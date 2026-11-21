//! The lock screen app: one `iced_sessionlock` application that paints the
//! Lock board on every output — wallpaper under a dark veil, a Jost clock, and
//! the unlock card. PAM auth runs on a worker thread (`pam`); the verdict
//! comes back as a message, and `UnLock` ends the process.

use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::widget::{self, operation::focusable};
use iced_core::window::{self, Id};
use iced_core::{Background, ContentFit, Event, Length, Padding, image};
use iced_futures::futures::Stream;
use iced_futures::futures::channel::{mpsc, oneshot};
use iced_futures::Subscription;
use iced_runtime::Task;
use iced_sessionlock::to_session_message;
use iced_widget::{Space, column, row, stack};
use triton_design::color::with_alpha;
use triton_design::{Icon, IconStyle, TypeStyle};
use triton_ui::button::{self, IconKind, Size};
use triton_ui::field::Field;
use triton_ui::progress::{self, WaveTone};
use triton_ui::surface::{self, Surface};
use triton_ui::text::{self, Tone};
use triton_ui::{Element, Theme};

use crate::pam;
use crate::user::{self, User};

/// The password field's widget id, so it can be focused once a surface maps.
fn field_id() -> widget::Id {
    widget::Id::from("lock-password")
}

// Board metrics that the tokens do not name (the type scale's display stops
// at 64 px; the Lock board's clock is ~110 px).
const CLOCK: TypeStyle = TypeStyle {
    size: 110.0,
    line_height: 120.0,
    ..TypeStyle::DISPLAY
};
/// The avatar wave, and the initials drawn inside it.
const AVATAR: f32 = 64.0;
const INITIALS: TypeStyle = TypeStyle {
    size: 24.0,
    line_height: 30.0,
    ..TypeStyle::TITLE_L
};
/// The unlock card's width on the board.
const CARD_WIDTH: f32 = 400.0;
/// The dark veil over the wallpaper (scheme.surface at ~0.55) so the card
/// reads; real blur is niri's job later.
const VEIL_ALPHA: f32 = 0.55;

#[to_session_message]
#[derive(Debug, Clone)]
enum Message {
    /// The minute tick, for the clock.
    Minute,
    /// A lock surface mapped on an output: put the caret in the field.
    Opened(Id),
    PasswordChanged(String),
    /// Enter or the arrow button.
    Submit,
    /// The visibility toggle (dots ⇄ plaintext).
    ToggleVisible,
    /// The PAM worker thread's verdict.
    AuthDone(bool),
    // `UnLock` is added by the macro and consumed by the runtime before it
    // reaches `update` (unlock + exit).
}

pub struct Lock {
    /// `--demo`: skip PAM; any non-empty password unlocks.
    demo: bool,
    user: User,
    /// The PAM service name chosen at boot.
    service: &'static str,
    wallpaper: Option<image::Handle>,
    password: String,
    show_password: bool,
    /// An auth attempt is in flight on the worker thread.
    busy: bool,
    error: Option<&'static str>,
    locked_at: chrono::DateTime<chrono::Local>,
    now: chrono::DateTime<chrono::Local>,
}

impl Lock {
    pub fn new(demo: bool) -> (Self, Task<Message>) {
        let now = chrono::Local::now();
        (
            Self {
                demo,
                user: user::current(),
                service: pam::service(),
                wallpaper: wallpaper(),
                password: String::new(),
                show_password: false,
                busy: false,
                error: None,
                locked_at: now,
                now,
            },
            Task::none(),
        )
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            minutes().map(|()| Message::Minute),
            iced_futures::event::listen_with(|event, _status, id| match event {
                Event::Window(window::Event::Opened { .. }) => Some(Message::Opened(id)),
                _ => None,
            }),
        ])
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Minute => {
                self.now = chrono::Local::now();
                Task::none()
            }
            Message::Opened(_) => iced_runtime::task::widget(focusable::focus(field_id())),
            Message::PasswordChanged(password) => {
                self.password = password;
                self.error = None;
                Task::none()
            }
            Message::ToggleVisible => {
                self.show_password = !self.show_password;
                Task::none()
            }
            Message::Submit => self.submit(),
            Message::AuthDone(ok) => {
                self.busy = false;
                if ok {
                    Task::done(Message::UnLock)
                } else {
                    // The field was already cleared by `submit`; keep the caret in it.
                    self.error = Some("Wrong password");
                    iced_runtime::task::widget(focusable::focus(field_id()))
                }
            }
            // `UnLock` is consumed by iced_sessionlock before it reaches update.
            Message::UnLock => Task::none(),
        }
    }

    /// Sends the password to PAM on a worker thread — `pam_authenticate` can
    /// take seconds and must never run on the UI thread.
    fn submit(&mut self) -> Task<Message> {
        if self.busy || self.password.is_empty() {
            return Task::none();
        }
        self.busy = true;
        self.error = None;
        let password = std::mem::take(&mut self.password);
        if self.demo {
            // --demo: no PAM call at all; any non-empty password unlocks. For
            // the headless preview, where PAM cannot authenticate.
            return Task::done(Message::AuthDone(!password.is_empty()));
        }
        let service = self.service;
        let user = self.user.name.clone();
        Task::perform(
            async move {
                let (tx, rx) = oneshot::channel();
                thread::Builder::new()
                    .name("pam-auth".into())
                    .spawn(move || {
                        let ok = pam::authenticate(service, &user, &password);
                        let _ = tx.send(ok);
                    })
                    .map_err(|e| tracing::error!(%e, "pam: cannot start the auth thread"))
                    .ok();
                // A failed spawn drops `tx`, which reads as a failed attempt.
                rx.await.unwrap_or(false)
            },
            Message::AuthDone,
        )
    }

    fn theme(&self) -> Theme {
        Theme::dark()
    }

    /// The lock surface fills its output; the scheme's surface shows where
    /// the wallpaper is missing and behind the veil.
    fn style(&self, theme: &Theme) -> iced_core::theme::Style {
        iced_core::theme::Style {
            background_color: theme.scheme.surface,
            text_color: theme.scheme.on_surface,
        }
    }

    pub fn view(&self, _id: Id) -> Element<'_, Message> {
        stack([self.backdrop(), Self::veil(), self.content()]).into()
    }

    /// The user's wallpaper covering the surface, or nothing — the window
    /// background (`style`) is the scheme's surface color underneath.
    fn backdrop(&self) -> Element<'_, Message> {
        match &self.wallpaper {
            Some(handle) => iced_widget::image(handle.clone())
                .content_fit(ContentFit::Cover)
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
            None => Space::new()
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
        }
    }

    /// The translucent dark veil over the wallpaper, so the card reads on a
    /// bright image. Not a `Surface` class: it's a full-bleed fill unique to
    /// the lock screen.
    fn veil() -> Element<'static, Message> {
        iced_widget::container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|theme: &Theme| iced_widget::container::Style {
                background: Some(Background::Color(with_alpha(
                    theme.scheme.surface,
                    VEIL_ALPHA,
                ))),
                ..iced_widget::container::Style::default()
            })
            .into()
    }

    /// Date and the huge Jost clock over the card, centered on the output.
    fn content(&self) -> Element<'_, Message> {
        let header = column![
            text::text(self.date(), TypeStyle::TITLE_L),
            text::text(self.clock(), CLOCK),
        ]
        .spacing(4)
        .align_x(Horizontal::Center);
        iced_widget::container(
            column![header, self.card()]
                .spacing(56)
                .align_x(Horizontal::Center),
        )
        .center(Length::Fill)
        .into()
    }

    /// The unlock card: wave avatar, name, lock age, the password row and the
    /// hint/error line.
    fn card(&self) -> Element<'_, Message> {
        let note = if self.busy {
            text::text("Checking…", TypeStyle::BODY_S).class(Tone::OnSurfaceVariant)
        } else if let Some(error) = self.error {
            text::text(error, TypeStyle::BODY_S).class(Tone::Error)
        } else {
            text::text("Enter your password", TypeStyle::BODY_S).class(Tone::OnSurfaceVariant)
        };
        surface::surface(
            column![
                self.avatar(),
                text::text(self.user.display.as_str(), TypeStyle::TITLE_L),
                text::text(self.locked_text(), TypeStyle::BODY_S).class(Tone::OnSurfaceVariant),
                self.password_row(),
                note,
            ]
            .spacing(16)
            .align_x(Horizontal::Center),
            Surface::Dialog,
        )
        .padding(24.0)
        .width(Length::Fixed(CARD_WIDTH))
        .into()
    }

    /// The nine-lobed wave as the avatar, with the user's initials in tertiary.
    fn avatar(&self) -> Element<'_, Message> {
        stack([
            progress::wave_shape(AVATAR, WaveTone::Tertiary),
            iced_widget::container(
                text::text(self.user.initials.as_str(), INITIALS).class(Tone::Tertiary),
            )
            .center(Length::Fill)
            .into(),
        ])
        .into()
    }

    /// Lock icon, the secret field (dots while hidden), the visibility toggle
    /// and the filled arrow that submits. Enter submits too (`on_submit`).
    fn password_row(&self) -> Element<'_, Message> {
        let input = iced_widget::text_input("Password", &self.password)
            .id(field_id())
            .icon(text_input::Icon {
                font: Icon::font(IconStyle::Outlined),
                code_point: Icon::Lock.codepoint(),
                size: Some(20.0.into()),
                spacing: 8.0,
                side: text_input::Side::Left,
            })
            .secure(!self.show_password)
            .size(16)
            .line_height(iced_core::text::LineHeight::Absolute(24.0.into()))
            // Same box as triton-ui's search field: 14 + 24 + 14 = the 52 px pill.
            .padding(Padding::from([14.0, 18.0]))
            .width(Length::Fill)
            .class(Field::Search)
            .on_input(Message::PasswordChanged)
            .on_submit(Message::Submit);
        let eye = button::icon_button(
            if self.show_password {
                Icon::VisibilityOff
            } else {
                Icon::Visibility
            },
            IconKind::Standard,
            Size::M,
            false,
        )
        .on_press(Message::ToggleVisible);
        let go = button::icon_button(Icon::ArrowForward, IconKind::Filled, Size::M, false)
            .on_press_maybe(
                (!self.busy && !self.password.is_empty()).then_some(Message::Submit),
            );
        row![input, eye, go]
            .spacing(8)
            .align_y(Vertical::Center)
            .into()
    }

    fn locked_text(&self) -> String {
        match (self.now - self.locked_at).num_minutes().max(0) {
            0 => "Locked just now".to_owned(),
            1 => "Locked a minute ago".to_owned(),
            n => format!("Locked {n} minutes ago"),
        }
    }

    /// "Saturday, 21 November".
    fn date(&self) -> String {
        self.now.format("%A, %-d %B").to_string()
    }

    fn clock(&self) -> String {
        self.now.format("%H:%M").to_string()
    }
}

/// The user's wallpaper: `$XDG_CONFIG_HOME/triton/wallpaper` holds one path.
/// Missing or unreadable → `style`'s scheme surface shows instead.
fn wallpaper() -> Option<image::Handle> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    let contents = std::fs::read_to_string(config.join("triton").join("wallpaper")).ok()?;
    let path = contents.trim();
    if path.is_empty() || !std::path::Path::new(path).is_file() {
        return None;
    }
    Some(image::Handle::from_path(path))
}

/// A message at every minute boundary, for the clock — the pattern copied
/// from `crates/triton/src/niri.rs`'s `minutes()`.
fn minutes() -> Subscription<()> {
    fn stream() -> impl Stream<Item = ()> {
        let (tx, rx) = mpsc::unbounded();
        thread::Builder::new()
            .name("clock".into())
            .spawn(move || {
                loop {
                    let now = chrono::Local::now();
                    let into = u64::from(now.timestamp_subsec_millis())
                        + 1000 * u64::from(chrono::Timelike::second(&now));
                    #[expect(
                        clippy::disallowed_methods,
                        reason = "a wait for a known deadline (the next minute), not polling"
                    )]
                    thread::sleep(Duration::from_millis(60_000 - into.min(59_999)));
                    if tx.unbounded_send(()).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| tracing::error!(%e, "clock: cannot start thread"))
            .ok();
        rx
    }
    Subscription::run(stream)
}
