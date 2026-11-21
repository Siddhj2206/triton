//! Spike 3 (docs/roadmap.md): does ext-session-lock-v1 work through
//! `iced_sessionlock` on niri?
//!
//! A minimal lock client: locks the session, paints a full-screen surface on
//! every output, counts down, then unlocks itself and exits. The headless
//! preview has no keyboard, so the unlock is a timer; real PAM auth belongs to
//! Phase 1's lock screen.
//!
//! While locked niri renders ONLY lock surfaces: a mid-run screenshot showing
//! this fill — and no bar — proves the protocol round-tripped. If niri did not
//! advertise `ext_session_lock_manager_v1` the app exits with an error instead.
//!
//! Run inside the preview, never the real session. Let niri start triton
//! normally, then run this client on niri's socket (wayland-2 is niri's;
//! wayland-1 is the host sway's):
//!
//! ```sh
//! cargo build --example spike_lock
//! TRITON_CMD=/work/target/debug/triton mise run preview
//! podman exec -d -e WAYLAND_DISPLAY=wayland-2 -e XDG_RUNTIME_DIR=/run/triton-preview \
//!   triton-preview /work/target/debug/examples/spike_lock
//! ```

use std::time::Duration;

use iced_core::Length;
use iced_futures::Subscription;
use iced_runtime::Task;
use iced_sessionlock::{application, to_session_message};
use iced_widget::{column, container};
use triton_design::TypeStyle;
use triton_ui::text::{self, Tone};
use triton_ui::{Element, Theme};

/// Seconds the session stays locked before the spike unlocks itself.
const LOCK_SECS: u64 = 6;

fn main() -> iced_sessionlock::Result {
    application(Spike::new, Spike::update, Spike::view)
        .subscription(Spike::subscription)
        .theme(Spike::theme)
        .style(Spike::style)
        .run()
}

struct Spike {
    remaining: u64,
}

#[to_session_message]
#[derive(Debug, Clone, Copy)]
enum Message {
    Tick,
}

impl Spike {
    fn new() -> (Self, Task<Message>) {
        (
            Self {
                remaining: LOCK_SECS,
            },
            Task::none(),
        )
    }

    fn subscription(_spike: &Self) -> Subscription<Message> {
        iced_futures::backend::default::time::every(Duration::from_secs(1)).map(|_| Message::Tick)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => {
                self.remaining = self.remaining.saturating_sub(1);
                if self.remaining == 0 {
                    return Task::done(Message::UnLock);
                }
                Task::none()
            }
            // The macro adds UnLock; the runtime consumes it before update.
            Message::UnLock => Task::none(),
        }
    }

    fn theme(_spike: &Self) -> Theme {
        Theme::dark()
    }

    /// The lock surface fills its output; a solid scheme background reads
    /// clearly when niri drops every other surface while locked.
    fn style(_spike: &Self, theme: &Theme) -> iced_core::theme::Style {
        iced_core::theme::Style {
            background_color: theme.scheme.surface,
            text_color: theme.scheme.on_surface,
        }
    }

    fn view(&self, _id: iced_core::window::Id) -> Element<'_, Message> {
        container(
            column![
                text::text("TRITON — SESSION LOCKED", TypeStyle::DISPLAY).class(Tone::Primary),
                text::text(
                    format!("the spike unlocks itself in {}s", self.remaining),
                    TypeStyle::TITLE_M
                )
                .class(Tone::OnSurfaceVariant),
            ]
            .spacing(12)
            .align_x(iced_core::alignment::Horizontal::Center),
        )
        .center(Length::Fill)
        .into()
    }
}
