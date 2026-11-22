//! Triton Kit Showcase — an interactive Material 3 Futura component gallery.
//!
//! Run ONLY inside the isolated nested session (`mise run dev`), never against
//! the host shell!
//!
//! Surfaces:
//! 1. `LeftIsland`  — top-left Glass island: gallery button, spring-morphing
//!    workspace `pill`s, and a live `level_meter` media pill.
//! 2. `RightIsland` — top-right Glass island: clock, `status_pill`,
//!    `overflow_pill`, `badged` notification bell, Dark/Light theme toggle,
//!    and a `reduce_motion` toggle emitting `AppCommand::SetReduceMotion`.
//! 3. `Gallery`     — centered Thick Glass panel (`Layer::Overlay`, exclusive
//!    keyboard) with a `segmented` tab bar over six scrollable sections:
//!    Surfaces, Buttons & Groups, Controls, Inputs, Chrome & Progress, and
//!    Overlays & Toasts — every `triton-kit` component family, all live.
//!
//! Automation env vars: `SHOWCASE_OPEN` (auto-open the gallery and walk a
//! demo step), `SHOWCASE_QUIT_MS`, `SHOWCASE_REPORT`, `SHOWCASE_QUERY`
//! (initial search-field text), `SHOWCASE_NO_BLINK` (freeze the caret).

#![forbid(unsafe_code)]

use std::cell::Cell;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use triton_design::color::{STATE_LAYER, Variant};
use triton_design::icon::{Icon, IconStyle};
use triton_design::material::{Material, ShadowToken};
use triton_design::motion::{Animated, Motion, Spring};
use triton_design::shape::{RADII, SIZE};
use triton_design::typography::TypeStyle;
use triton_engine::{
    AppCommand, Edges, Element, FontSpec, HeadlessRenderer, HeadlessSession, ImageData, ImageId,
    ImageStore, ImageStyle, InputEvent, Key, NodeKey, Rgba, ShellApp, SubtreeTransform,
    SurfaceAnchor, SurfaceKeyboard, SurfaceLayer, SurfaceMetrics, SurfaceSize, SurfaceSpec, Theme,
    ViewContext, box_, col, icon, row, run, text,
};
use triton_kit::{
    Badge, Button, ButtonSize, ButtonTone, CheckState, Chip, ChipKind, Dialog, DividerStyle,
    IconButton, KeycapStyle, ListFill, Menu, MenuEntry, MenuItem, MeterMode, Orientation,
    PasswordField, PillStatus, PowerCellTone, PowerItem, Progress, ScrollAreaOptions, SearchField,
    SearchMode, SegmentedItem, SegmentedStyle, SliderMode, SliderOptions, SliderSurface, TextInput,
    TextInputAction, TextInputState, ToastAction, ToastActionTone, ToastCard, ToastIdentity,
    ToastVariant, ToggleTile, badge, badged, button, card, checkbox, chip, concentric_inner,
    dialog, divider, empty_state, hint_bar, icon_button, icon_disc, icon_disc_styled, island,
    keycap, level_meter, list_group, list_row, list_row_text, menu, overflow_pill, panel,
    password_field, pill, power_menu, radio, scroll_area, search_field, segmented, shadow_gutter,
    slider, status_pill, switch, text_input, toast_card, toast_is_hovered, toggle_tile, tooltip,
    trigger_password_shake, wave_avatar, wave_avatar_image, wavy_progress, wavy_ring,
};

const TIMER_OPEN_GALLERY: u32 = 1;
const TIMER_SELECT_WS: u32 = 2;
const TIMER_DEMO_STEP: u32 = 3;
const TIMER_CARET_BLINK: u32 = 4;
const TIMER_QUIT: u32 = 5;

const WORKSPACES: usize = 4;
const PANEL_RADIUS: f32 = RADII.xl;
const PANEL_PAD: f32 = 16.0;
const SECTION_HEIGHT: f32 = 520.0;
/// Live `wavy_ring` countdown drain (seconds); restart by clicking the ring.
const COUNTDOWN_SECS: f32 = 8.0;
/// The demo password that unlocks `password_field` without a shake.
const DEMO_PASSWORD: &str = "triton";
/// Sample contact-photo edge, generated in `sync_images` for
/// `wave_avatar_image`.
const AVATAR_PX: u32 = 96;

const SEARCH_MODES: [SearchMode; 4] = [
    SearchMode::Search,
    SearchMode::Math,
    SearchMode::Command,
    SearchMode::File,
];

// ── Gallery sections ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Surfaces,
    Buttons,
    Controls,
    Inputs,
    Chrome,
    Overlays,
}

const SECTIONS: [Section; 6] = [
    Section::Surfaces,
    Section::Buttons,
    Section::Controls,
    Section::Inputs,
    Section::Chrome,
    Section::Overlays,
];

impl Section {
    fn title(self) -> &'static str {
        match self {
            Self::Surfaces => "Surfaces",
            Self::Buttons => "Buttons",
            Self::Controls => "Controls",
            Self::Inputs => "Inputs",
            Self::Chrome => "Chrome",
            Self::Overlays => "Overlays",
        }
    }

    fn glyph(self) -> Icon {
        match self {
            Self::Surfaces => Icon::WebAsset,
            Self::Buttons => Icon::AdsClick,
            Self::Controls => Icon::Tune,
            Self::Inputs => Icon::Keyboard,
            Self::Chrome => Icon::Palette,
            Self::Overlays => Icon::Notifications,
        }
    }
}

/// Which demo text field a routed [`InputEvent`] belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldId {
    Text,
    Search,
    Password,
}

// ── Surfaces & messages ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SurfaceId {
    LeftIsland,
    RightIsland,
    Gallery,
}

#[derive(Debug, Clone)]
enum Msg {
    ToggleGallery,
    CloseGallery,
    SelectWorkspace(usize),
    ToggleTheme,
    ToggleReduceMotion,
    ToggleMedia,
    ToggleDnd,
    ToggleWifi,
    SelectSection(usize),
    OpenDialog,
    DismissDialog,
    ToggleIconButton(usize),
    ToggleChip(usize),
    DismissChip,
    ToggleTile(usize),
    TileSubPage(usize),
    SelectSegmented { group: usize, index: usize },
    FocusPower(usize),
    ToggleSwitch(usize),
    CycleCheck(usize),
    SelectRadio(usize),
    SetSlider { index: usize, value: f32 },
    FieldEvent(FieldId, InputEvent),
    ToggleInputError,
    SetSearchMode(usize),
    ClearSearch,
    TogglePasswordReveal,
    SubmitPassword,
    ToggleCapsSim,
    ToggleSubmenu,
    DismissToast(usize),
    ActivateToast(usize),
    ToastActionClicked { toast: usize, action: usize },
    ResetToasts,
    RestartCountdown,
    DemoAction(&'static str),
}

// ── App state ────────────────────────────────────────────────────────────────

#[expect(
    clippy::struct_excessive_bools,
    reason = "demo shell state: independent toggles, a state machine would obscure them"
)]
struct ShowcaseApp {
    theme: Theme,
    reduce_motion: bool,
    active_ws: usize,
    gallery_open: bool,
    gallery_anim: Animated,
    section: usize,
    dialog_open: bool,
    media_playing: bool,
    dnd: bool,
    wifi: bool,
    badge_count: u32,
    icon_toggles: [bool; 4],
    chip_filters: [bool; 3],
    chip_dismissed: bool,
    tile_on: [bool; 2],
    seg_sel: [usize; 3],
    power_focus: usize,
    switches: [bool; 2],
    checks: [CheckState; 2],
    radio_sel: usize,
    sliders: [f32; 4],
    text_input: TextInputState,
    input_error: bool,
    search: TextInputState,
    search_mode: usize,
    password: TextInputState,
    password_revealed: bool,
    password_error: Option<String>,
    caps_sim: bool,
    /// Incremented per failed password submit; `shake_seen` lets the `&self`
    /// view fire [`trigger_password_shake`] exactly once per failure.
    shake_seq: u64,
    shake_seen: Cell<u64>,
    submenu_open: bool,
    toast_dismissed: [bool; 3],
    countdown_end: Instant,
    avatar_image: Option<ImageId>,
    auto_open: bool,
    quit_ms: Option<u64>,
    report_path: String,
}

// ── Small shared view helpers ────────────────────────────────────────────────

fn on_surf(theme: &Theme) -> Rgba {
    Rgba::from_color(theme.scheme.on_surface)
}

fn onv(theme: &Theme) -> Rgba {
    Rgba::from_color(theme.scheme.on_surface_variant)
}

fn sec_label<M>(label: &str, theme: &Theme) -> Element<M> {
    text(label.to_owned(), TypeStyle::LABEL_S, onv(theme))
}

fn labeled<M>(control: Element<M>, label: &str, theme: &Theme) -> Element<M> {
    row().gap(10.0).items_center().child(control).child(text(
        label.to_owned(),
        TypeStyle::BODY_M,
        on_surf(theme),
    ))
}

fn gallery_redraw() -> Vec<AppCommand<SurfaceId>> {
    vec![AppCommand::Redraw(SurfaceId::Gallery)]
}

/// Target text color for a `status_pill` severity (matches the pill's own
/// animated content color).
fn status_fg(status: PillStatus, theme: &Theme) -> Rgba {
    let s = &theme.scheme;
    let c = match status {
        PillStatus::Primary => s.on_primary_container,
        PillStatus::Attention => s.on_tertiary_container,
        PillStatus::Urgent => s.on_error_container,
        PillStatus::Neutral | _ => s.on_surface,
    };
    Rgba::from_color(c)
}

/// Procedural 96×96 contact photo (diagonal indigo→teal gradient) uploaded via
/// `sync_images` for the `wave_avatar_image` demo — no asset files needed.
fn sample_avatar_rgba(size: u32) -> Vec<u8> {
    let mut px = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let u = x as f32 / size as f32;
            let v = y as f32 / size as f32;
            px.extend_from_slice(&[
                (64.0 + 110.0 * u) as u8,
                (120.0 + 90.0 * v) as u8,
                (235.0 - 70.0 * u) as u8,
                255,
            ]);
        }
    }
    px
}

impl ShowcaseApp {
    fn new(now: Instant) -> Self {
        let auto_open = std::env::var_os("SHOWCASE_OPEN").is_some();
        let quit_ms = std::env::var("SHOWCASE_QUIT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|&ms| ms > 0);
        let report_path = std::env::var("SHOWCASE_REPORT")
            .unwrap_or_else(|_| "/preview/triton-kit-showcase.txt".into());
        let initial_query = std::env::var("SHOWCASE_QUERY").unwrap_or_default();

        Self {
            theme: Theme::dark(),
            reduce_motion: false,
            active_ws: 0,
            gallery_open: false,
            gallery_anim: Animated::new(0.0, Motion::Spring(Spring::SPATIAL), 0.002, now),
            section: 0,
            dialog_open: false,
            media_playing: true,
            dnd: false,
            wifi: true,
            badge_count: 3,
            icon_toggles: [true, false, true, false],
            chip_filters: [true, false, false],
            chip_dismissed: false,
            tile_on: [true, false],
            seg_sel: [0, 1, 2],
            power_focus: 2,
            switches: [true, false],
            checks: [CheckState::Checked, CheckState::Indeterminate],
            radio_sel: 1,
            sliders: [0.72, 0.5, 0.4, 0.62],
            text_input: TextInputState::new(""),
            input_error: false,
            search: TextInputState::new(initial_query),
            search_mode: 0,
            password: TextInputState::new(""),
            password_revealed: false,
            password_error: None,
            caps_sim: false,
            shake_seq: 0,
            shake_seen: Cell::new(0),
            submenu_open: false,
            toast_dismissed: [false; 3],
            countdown_end: now + Duration::from_secs_f32(COUNTDOWN_SECS),
            avatar_image: None,
            auto_open,
            quit_ms,
            report_path,
        }
    }

    fn gallery_visible(&self, now: Instant) -> bool {
        self.gallery_open || !self.gallery_anim.is_settled(now)
    }

    fn countdown_progress(&self, now: Instant) -> f32 {
        (self
            .countdown_end
            .saturating_duration_since(now)
            .as_secs_f32()
            / COUNTDOWN_SECS)
            .clamp(0.0, 1.0)
    }

    fn open_gallery(&mut self, now: Instant) -> Vec<AppCommand<SurfaceId>> {
        self.gallery_open = true;
        self.gallery_anim.set_target(1.0, now);
        self.search.caret_visible = true;
        let mut cmds = vec![
            AppCommand::Redraw(SurfaceId::Gallery),
            AppCommand::Focus(SurfaceId::Gallery, NodeKey::named("demo-search")),
        ];
        if std::env::var_os("SHOWCASE_NO_BLINK").is_none() {
            cmds.push(AppCommand::ScheduleTimer {
                tag: TIMER_CARET_BLINK,
                delay: Duration::from_millis(530),
            });
        }
        cmds
    }

    fn close_gallery(&mut self, now: Instant) -> Vec<AppCommand<SurfaceId>> {
        self.gallery_open = false;
        self.dialog_open = false;
        self.gallery_anim.set_target(0.0, now);
        vec![AppCommand::Redraw(SurfaceId::Gallery)]
    }

    fn check_password(&mut self) -> Vec<AppCommand<SurfaceId>> {
        if self.password.value == DEMO_PASSWORD {
            self.password_error = None;
            self.password.clear();
            self.password_revealed = false;
        } else {
            self.password_error = Some(format!("Incorrect password — hint: {DEMO_PASSWORD}"));
            self.shake_seq += 1;
        }
        gallery_redraw()
    }

    fn handle_field(
        &mut self,
        field: FieldId,
        ev: &InputEvent,
        now: Instant,
    ) -> Vec<AppCommand<SurfaceId>> {
        let state = match field {
            FieldId::Text => &mut self.text_input,
            FieldId::Search => &mut self.search,
            FieldId::Password => &mut self.password,
        };
        match state.apply_key(ev) {
            Some(TextInputAction::Escape) => self.close_gallery(now),
            Some(TextInputAction::Submit) if field == FieldId::Password => self.check_password(),
            Some(_) => gallery_redraw(),
            None => Vec::new(),
        }
    }

    // ── Top-bar islands ──────────────────────────────────────────────────────

    fn view_left_island(&self, ctx: &mut ViewContext<'_>) -> Element<Msg> {
        let t = &self.theme;
        let mut bar = island(t).child(icon_button(
            t,
            ctx.interaction,
            ctx.anim,
            IconButton {
                key: NodeKey::named("btn-gallery"),
                icon: Icon::Apps,
                toggled: self.gallery_open,
                tone: ButtonTone::Veil,
                on_click: Some(Box::new(|| Msg::ToggleGallery)),
                ..Default::default()
            },
        ));
        bar = bar.child(box_().w(6.0).h(1.0));
        for i in 0..WORKSPACES {
            bar = bar.child(pill(
                NodeKey::from_pair("ws", i),
                (i + 1).to_string(),
                i == self.active_ws,
                t,
                ctx.anim,
                move || Msg::SelectWorkspace(i),
            ));
        }

        let meter_mode = if self.media_playing {
            MeterMode::Playing
        } else {
            MeterMode::Levels([0.3, 0.55, 0.4])
        };
        let fg = on_surf(t);
        let media = row()
            .id(NodeKey::named("media-pill"))
            .h(SIZE.control_m)
            .px(14.0)
            .gap(10.0)
            .items_center()
            .rounded(SIZE.control_m * 0.5)
            .smoothing(0.0)
            .bg(t.veil())
            .hover_bg(t.veil().over(fg, STATE_LAYER.hover))
            .press_bg(t.veil().over(fg, STATE_LAYER.press))
            .on_click(|| Msg::ToggleMedia)
            .child(icon(
                if self.media_playing {
                    Icon::Pause
                } else {
                    Icon::PlayArrow
                },
                IconStyle::Outlined,
                SIZE.icon_s,
                fg,
            ))
            .child(level_meter(
                NodeKey::named("media-meter"),
                meter_mode,
                (16.0, 16.0),
                t,
                ctx.anim,
            ))
            .child(text("Now Playing", FontSpec::ui(13.0).weight(600), fg));
        bar = bar.child(box_().w(6.0).h(1.0)).child(media);

        shadow_gutter(bar)
    }

    #[expect(
        clippy::too_many_lines,
        reason = "gallery section enumerates every demo inline; splitting per-widget helpers would scatter the showcase narrative"
    )]
    fn view_right_island(&self, ctx: &mut ViewContext<'_>) -> Element<Msg> {
        let t = &self.theme;
        let clock = row()
            .px(14.0)
            .gap(10.0)
            .items_center()
            .child(text(
                "21:41",
                FontSpec::display(22.0).weight(600),
                on_surf(t),
            ))
            .child(text("Sun 22 Nov", FontSpec::ui(12.0).weight(500), onv(t)));

        let status = if self.wifi {
            PillStatus::Primary
        } else {
            PillStatus::Attention
        };
        let sfg = status_fg(status, t);
        let stat = status_pill(
            NodeKey::named("status-pill"),
            status,
            t,
            ctx.anim,
            Some(Box::new(|| Msg::ToggleWifi)),
            [
                icon(
                    if self.wifi { Icon::Wifi } else { Icon::WifiOff },
                    IconStyle::Outlined,
                    SIZE.icon_m,
                    sfg,
                ),
                icon(Icon::VolumeUp, IconStyle::Outlined, SIZE.icon_m, sfg),
                row()
                    .gap(4.0)
                    .items_center()
                    .child(icon(
                        Icon::Battery5Bar,
                        IconStyle::Outlined,
                        SIZE.icon_m,
                        sfg,
                    ))
                    .child(text("82%", FontSpec::ui(14.0).weight(600), sfg)),
            ],
        );

        let bell = icon_button(
            t,
            ctx.interaction,
            ctx.anim,
            IconButton {
                key: NodeKey::named("btn-dnd"),
                icon: if self.dnd {
                    Icon::NotificationsOff
                } else {
                    Icon::Notifications
                },
                toggled: self.dnd,
                tone: ButtonTone::Veil,
                on_click: Some(Box::new(|| Msg::ToggleDnd)),
                ..Default::default()
            },
        );
        let bell = if self.dnd || self.badge_count == 0 {
            bell
        } else {
            badged(bell, Badge::Count(self.badge_count), t)
        };

        let theme_toggle = icon_button(
            t,
            ctx.interaction,
            ctx.anim,
            IconButton {
                key: NodeKey::named("btn-theme"),
                icon: match t.variant {
                    Variant::Dark => Icon::LightMode,
                    Variant::Light => Icon::DarkMode,
                },
                tone: ButtonTone::Veil,
                on_click: Some(Box::new(|| Msg::ToggleTheme)),
                ..Default::default()
            },
        );
        let motion_toggle = icon_button(
            t,
            ctx.interaction,
            ctx.anim,
            IconButton {
                key: NodeKey::named("btn-motion"),
                icon: Icon::Animation,
                toggled: self.reduce_motion,
                tone: ButtonTone::Veil,
                on_click: Some(Box::new(|| Msg::ToggleReduceMotion)),
                ..Default::default()
            },
        );

        let bar = island(t)
            .child(clock)
            .child(stat)
            .child(overflow_pill(NodeKey::named("overflow"), 2, t, || {
                Msg::ToggleGallery
            }))
            .child(bell)
            .child(theme_toggle)
            .child(motion_toggle);
        shadow_gutter(bar)
    }

    // ── Gallery panel ────────────────────────────────────────────────────────

    fn view_gallery(&self, ctx: &mut ViewContext<'_>) -> Element<Msg> {
        let t = &self.theme;
        let prog = self.gallery_anim.value(ctx.now);
        let tf = SubtreeTransform {
            translate_y: 14.0 * (1.0 - prog),
            scale: 0.96 + 0.04 * prog,
            opacity: prog.clamp(0.0, 1.0),
            ..SubtreeTransform::IDENTITY
        };

        let tabs = segmented(
            NodeKey::named("gallery-tabs"),
            SegmentedStyle::Standard,
            Some(self.section),
            t,
            SECTIONS
                .iter()
                .enumerate()
                .map(|(i, s)| SegmentedItem {
                    icon: Some(s.glyph()),
                    label: Some(s.title().to_owned()),
                    on_click: Some(Box::new(move || Msg::SelectSection(i))),
                })
                .collect(),
        );

        let card = panel(Material::Thick, PANEL_RADIUS, t)
            .shadow(ShadowToken::FLOATING)
            .w(1000.0)
            .p(Edges::all(PANEL_PAD))
            .gap(12.0)
            .transform(tf)
            .on_key(|ev| {
                matches!(
                    ev,
                    InputEvent::KeyDown {
                        key: Key::Escape,
                        ..
                    }
                )
                .then_some(Msg::CloseGallery)
            })
            .child(self.view_gallery_header(ctx))
            .child(tabs)
            .child(divider(Orientation::Horizontal, DividerStyle::OnGlass, t))
            .child(self.view_section(ctx))
            .child(divider(Orientation::Horizontal, DividerStyle::OnGlass, t))
            .child(self.view_gallery_footer(ctx));

        let mut root = box_().child(shadow_gutter(card));
        if self.dialog_open {
            root = root.child(self.view_dialog(ctx).absolute().inset_all(0.0));
        }
        root
    }

    fn view_gallery_header(&self, ctx: &mut ViewContext<'_>) -> Element<Msg> {
        let t = &self.theme;
        row()
            .w_full()
            .items_center()
            .gap(14.0)
            .child(
                col()
                    .flex_1()
                    .gap(2.0)
                    .child(text("Triton Kit", TypeStyle::TITLE_L, on_surf(t)))
                    .child(text(
                        "Material 3 Futura component gallery — every family, live",
                        TypeStyle::BODY_S,
                        onv(t),
                    )),
            )
            .child(chip(
                t,
                Chip {
                    key: NodeKey::named("theme-chip"),
                    label: match t.variant {
                        Variant::Dark => "Dark".to_owned(),
                        Variant::Light => "Light".to_owned(),
                    },
                    icon: Some(match t.variant {
                        Variant::Dark => Icon::DarkMode,
                        Variant::Light => Icon::LightMode,
                    }),
                    kind: ChipKind::StatusNeutral,
                    ..Default::default()
                },
            ))
            .child(icon_button(
                t,
                ctx.interaction,
                ctx.anim,
                IconButton {
                    key: NodeKey::named("btn-close-gallery"),
                    icon: Icon::Close,
                    tone: ButtonTone::Veil,
                    on_click: Some(Box::new(|| Msg::CloseGallery)),
                    ..Default::default()
                },
            ))
    }

    fn view_gallery_footer(&self, ctx: &mut ViewContext<'_>) -> Element<Msg> {
        let t = &self.theme;
        row()
            .w_full()
            .items_center()
            .child(hint_bar(
                [
                    ("Click", "interact"),
                    ("Scroll", "browse"),
                    ("Esc", "close"),
                ],
                t,
            ))
            .child(box_().flex_1())
            .child(text(
                format!(
                    "triton-kit · {:.2}× · concentric {}/{}px",
                    ctx.scale,
                    PANEL_RADIUS as i32,
                    concentric_inner(PANEL_RADIUS, PANEL_PAD) as i32,
                ),
                FontSpec::mono(11.5),
                onv(t).with_alpha(0.75),
            ))
    }

    fn view_dialog(&self, ctx: &mut ViewContext<'_>) -> Element<Msg> {
        let t = &self.theme;
        dialog(
            t,
            Dialog {
                key: NodeKey::named("gallery-dialog"),
                icon: Some(Icon::Delete),
                title: Some("Move to Trash?".to_owned()),
                body: Some(
                    "A live triton-kit `dialog`: a Thick Glass card floating over a 45% scrim. \
                     Destructive actions always use error_container — never primary."
                        .to_owned(),
                ),
                actions: vec![
                    button(
                        t,
                        ctx.interaction,
                        ctx.anim,
                        Button {
                            key: NodeKey::named("dialog-cancel"),
                            label: "Cancel".into(),
                            tone: ButtonTone::Text,
                            on_click: Some(Box::new(|| Msg::DismissDialog)),
                            ..Default::default()
                        },
                    ),
                    button(
                        t,
                        ctx.interaction,
                        ctx.anim,
                        Button {
                            key: NodeKey::named("dialog-confirm"),
                            label: "Move to Trash".into(),
                            lead_icon: Some(Icon::Delete),
                            tone: ButtonTone::Destructive,
                            on_click: Some(Box::new(|| Msg::DismissDialog)),
                            ..Default::default()
                        },
                    ),
                ],
                on_dismiss: Some(Box::new(|| Msg::DismissDialog)),
            },
        )
    }

    fn view_section(&self, ctx: &mut ViewContext<'_>) -> Element<Msg> {
        let t = &self.theme;
        let children = match SECTIONS[self.section] {
            Section::Surfaces => self.sec_surfaces(ctx),
            Section::Buttons => self.sec_buttons(ctx),
            Section::Controls => self.sec_controls(ctx),
            Section::Inputs => self.sec_inputs(ctx),
            Section::Chrome => self.sec_chrome(ctx),
            Section::Overlays => self.sec_overlays(ctx),
        };
        scroll_area(
            NodeKey::from_pair("section-scroll", self.section),
            SECTION_HEIGHT,
            ScrollAreaOptions {
                gap: 12.0,
                padding: Edges::new(6.0, 12.0, 6.0, 6.0),
                edge_fade: Some(Edges::xy(0.0, 24.0)),
            },
            t,
            ctx.interaction,
            ctx.anim,
            children,
        )
    }

    // ── Section 1: Surfaces ──────────────────────────────────────────────────

    #[expect(
        clippy::too_many_lines,
        reason = "gallery section enumerates every demo inline; splitting per-widget helpers would scatter the showcase narrative"
    )]
    fn sec_surfaces(&self, ctx: &mut ViewContext<'_>) -> Vec<Element<Msg>> {
        let t = &self.theme;
        let mats = [
            (Material::Solid, "Solid", "opaque · no blur"),
            (Material::Mica, "Mica", "α .72 · blur 48 · xray"),
            (Material::Glass, "Glass", "α .70 · blur 32 · hi"),
            (Material::Thick, "Thick", "α .88 · blur 40 · floats"),
        ];
        let mat_row = row()
            .gap(12.0)
            .p(Edges::new(4.0, 4.0, 12.0, 4.0))
            .children(mats.iter().map(|&(m, name, desc)| {
                let mut tile = panel(m, RADII.lg, t)
                    .w(218.0)
                    .h(92.0)
                    .p(14.0)
                    .gap(2.0)
                    .child(text(name, TypeStyle::TITLE_M, on_surf(t)))
                    .child(text(desc, TypeStyle::BODY_S, onv(t)));
                if matches!(m, Material::Glass | Material::Thick) {
                    tile = tile.shadow(ShadowToken::RAISED);
                }
                tile
            }));

        let demo_card = card(t)
            .flex_1()
            .gap(8.0)
            .child(sec_label("card · surface_container_low", t))
            .child(divider(Orientation::Horizontal, DividerStyle::Standard, t))
            .child(list_group(
                ListFill::Raised,
                t,
                [
                    list_row(
                        NodeKey::named("card-row-0"),
                        ListFill::Raised,
                        false,
                        t,
                        Some(Box::new(|| Msg::DemoAction("card row"))),
                        [
                            icon_disc(Icon::Folder, t),
                            list_row_text(
                                "Raised row",
                                Some("one tone up inside a card"),
                                false,
                                t,
                            ),
                        ],
                    ),
                    list_row(
                        NodeKey::named("card-row-1"),
                        ListFill::Raised,
                        true,
                        t,
                        None,
                        [
                            icon_disc(Icon::Star, t),
                            list_row_text("Selected row", Some("secondary_container .on"), true, t),
                        ],
                    ),
                ],
            ));

        let veil_col = col()
            .flex_1()
            .gap(8.0)
            .child(sec_label("list_group · Veil on Thick", t))
            .child(list_group(
                ListFill::Veil,
                t,
                [
                    list_row(
                        NodeKey::named("veil-row-0"),
                        ListFill::Veil,
                        false,
                        t,
                        Some(Box::new(|| Msg::DemoAction("veil row 0"))),
                        [
                            icon_disc(Icon::Wifi, t),
                            list_row_text("Veil row", Some("translucent on glass"), false, t),
                        ],
                    ),
                    list_row(
                        NodeKey::named("veil-row-1"),
                        ListFill::Veil,
                        true,
                        t,
                        Some(Box::new(|| Msg::DemoAction("veil row 1"))),
                        [
                            icon_disc(Icon::Bluetooth, t),
                            list_row_text("Selected", Some("list_corners joins"), true, t),
                        ],
                    ),
                    list_row(
                        NodeKey::named("veil-row-2"),
                        ListFill::Veil,
                        false,
                        t,
                        None,
                        [
                            icon_disc(Icon::Cast, t),
                            list_row_text("Read-only", Some("no click handler"), false, t),
                        ],
                    ),
                ],
            ))
            .child(sec_label("empty_state", t))
            .child(empty_state(
                Some(Icon::Search),
                "No matching applications",
                Some("try a different query"),
                t,
            ));

        let scroll_demo = col()
            .w_full()
            .gap(8.0)
            .child(sec_label("scroll_area · edge fade + 4→10px thumb", t))
            .child(scroll_area(
                NodeKey::named("surfaces-list"),
                220.0,
                ScrollAreaOptions {
                    gap: 4.0,
                    padding: Edges::new(0.0, 10.0, 0.0, 0.0),
                    edge_fade: Some(Edges::xy(0.0, 28.0)),
                },
                t,
                ctx.interaction,
                ctx.anim,
                (0..8).map(|i| {
                    let glyph = [
                        Icon::Folder,
                        Icon::Image,
                        Icon::MusicNote,
                        Icon::Description,
                    ][i % 4];
                    list_row(
                        NodeKey::from_pair("surf-row", i),
                        ListFill::Veil,
                        false,
                        t,
                        Some(Box::new(|| Msg::DemoAction("scroll row"))),
                        [
                            icon_disc(glyph, t),
                            list_row_text(
                                ["Documents", "Photos", "Music", "Notes"][i % 4],
                                Some("scrollable veil row"),
                                false,
                                t,
                            ),
                        ],
                    )
                }),
            ));

        let dialog_row = row()
            .gap(14.0)
            .items_center()
            .child(button(
                t,
                ctx.interaction,
                ctx.anim,
                Button {
                    key: NodeKey::named("btn-open-dialog"),
                    label: "Open dialog".into(),
                    lead_icon: Some(Icon::WebAsset),
                    tone: ButtonTone::Filled,
                    on_click: Some(Box::new(|| Msg::OpenDialog)),
                    ..Default::default()
                },
            ))
            .child(text(
                "live `dialog` over `scrim` — Esc or scrim-click dismisses",
                TypeStyle::BODY_S,
                onv(t),
            ));

        vec![
            sec_label("panel · materials", t),
            mat_row,
            row().gap(12.0).child(demo_card).child(veil_col),
            scroll_demo,
            divider(Orientation::Horizontal, DividerStyle::OnGlass, t),
            dialog_row,
        ]
    }

    // ── Section 2: Buttons & Groups ──────────────────────────────────────────

    fn sec_buttons(&self, ctx: &mut ViewContext<'_>) -> Vec<Element<Msg>> {
        let t = &self.theme;
        let tones = [
            (ButtonTone::Filled, "Filled", Some(Icon::Check)),
            (ButtonTone::Tonal, "Tonal", Some(Icon::Star)),
            (ButtonTone::Outlined, "Outlined", None),
            (ButtonTone::Text, "Text", None),
            (ButtonTone::Veil, "Veil", Some(Icon::BlurOn)),
            (ButtonTone::Destructive, "Destructive", Some(Icon::Delete)),
        ];
        let tone_row = row()
            .gap(10.0)
            .wrap()
            .children(tones.iter().map(|&(tone, label, ic)| {
                button(
                    t,
                    ctx.interaction,
                    ctx.anim,
                    Button {
                        key: NodeKey::named(format!("btn-tone-{label}").as_str()),
                        label: label.to_owned(),
                        lead_icon: ic,
                        tone,
                        on_click: Some(Box::new(|| Msg::DemoAction("tone"))),
                        ..Default::default()
                    },
                )
            }));

        let sizes = [
            (ButtonSize::S, "Small"),
            (ButtonSize::M, "Medium"),
            (ButtonSize::L, "Large"),
            (ButtonSize::Xl, "Extra large"),
        ];
        let mut size_row =
            row()
                .gap(10.0)
                .items_end()
                .children(sizes.iter().map(|&(size, label)| {
                    button(
                        t,
                        ctx.interaction,
                        ctx.anim,
                        Button {
                            key: NodeKey::named(format!("btn-size-{label}").as_str()),
                            label: label.to_owned(),
                            size,
                            tone: ButtonTone::Filled,
                            on_click: Some(Box::new(|| Msg::DemoAction("size"))),
                            ..Default::default()
                        },
                    )
                }));
        for (tone, label) in [
            (ButtonTone::Filled, "Disabled"),
            (ButtonTone::Outlined, "Disabled"),
        ] {
            size_row = size_row.child(button(
                t,
                ctx.interaction,
                ctx.anim,
                Button {
                    key: NodeKey::named(format!("btn-dis-{label}").as_str()),
                    label: label.to_owned(),
                    tone,
                    disabled: true,
                    ..Default::default()
                },
            ));
        }

        vec![
            sec_label("button · tones (press morphs to r12)", t),
            tone_row,
            sec_label("button · sizes + disabled", t),
            size_row,
            self.button_groups_extras(ctx),
        ]
    }

    #[expect(
        clippy::too_many_lines,
        reason = "gallery section enumerates every demo inline; splitting per-widget helpers would scatter the showcase narrative"
    )]
    fn button_groups_extras(&self, ctx: &mut ViewContext<'_>) -> Element<Msg> {
        let t = &self.theme;
        let ib_icons = [
            (Icon::Star, "star"),
            (Icon::Notifications, "notif"),
            (Icon::Wifi, "wifi"),
            (Icon::Settings, "settings"),
        ];
        let mut ib_row =
            row()
                .gap(10.0)
                .children(ib_icons.iter().enumerate().map(|(i, &(ic, name))| {
                    icon_button(
                        t,
                        ctx.interaction,
                        ctx.anim,
                        IconButton {
                            key: NodeKey::named(format!("ib-{name}").as_str()),
                            icon: ic,
                            toggled: self.icon_toggles[i],
                            tone: ButtonTone::Neutral,
                            on_click: Some(Box::new(move || Msg::ToggleIconButton(i))),
                            ..Default::default()
                        },
                    )
                }));
        ib_row = ib_row.child(icon_button(
            t,
            ctx.interaction,
            ctx.anim,
            IconButton {
                key: NodeKey::named("ib-disabled"),
                icon: Icon::Mic,
                disabled: true,
                tone: ButtonTone::Neutral,
                ..Default::default()
            },
        ));

        let mut chip_row = row().gap(8.0).wrap();
        for (i, label) in ["Wi-Fi", "Bluetooth", "Airplane"].iter().enumerate() {
            chip_row = chip_row.child(chip(
                t,
                Chip {
                    key: NodeKey::from_pair("filter-chip", i),
                    label: (*label).to_owned(),
                    kind: ChipKind::Filter,
                    selected: self.chip_filters[i],
                    on_click: Some(Box::new(move || Msg::ToggleChip(i))),
                    ..Default::default()
                },
            ));
        }
        for (kind, label, ic) in [
            (ChipKind::StatusNeutral, "Stable", Icon::CheckCircle),
            (ChipKind::StatusWarm, "Needs attention", Icon::Warning),
            (ChipKind::StatusError, "Failed", Icon::Error),
        ] {
            chip_row = chip_row.child(chip(
                t,
                Chip {
                    key: NodeKey::named(format!("status-chip-{label}").as_str()),
                    label: label.to_owned(),
                    icon: Some(ic),
                    kind,
                    ..Default::default()
                },
            ));
        }
        if !self.chip_dismissed {
            chip_row = chip_row.child(chip(
                t,
                Chip {
                    key: NodeKey::named("chip-dismissible"),
                    label: "Dismissible".to_owned(),
                    icon: Some(Icon::PushPin),
                    kind: ChipKind::Filter,
                    selected: true,
                    on_dismiss: Some(Box::new(|| Msg::DismissChip)),
                    ..Default::default()
                },
            ));
        }

        let tiles = row()
            .gap(12.0)
            .child(box_().flex_1().child(toggle_tile(
                t,
                ctx.anim,
                ToggleTile {
                    key: NodeKey::named("tile-wifi"),
                    icon: Icon::Wifi,
                    title: "Wi-Fi".into(),
                    subtitle: Some("Neptuno-5G".into()),
                    on: self.tile_on[0],
                    on_click: Some(Box::new(|| Msg::ToggleTile(0))),
                    on_sub_page: Some(Box::new(|| Msg::TileSubPage(0))),
                },
            )))
            .child(box_().flex_1().child(toggle_tile(
                t,
                ctx.anim,
                ToggleTile {
                    key: NodeKey::named("tile-bt"),
                    icon: Icon::Bluetooth,
                    title: "Bluetooth".into(),
                    subtitle: Some("2 devices".into()),
                    on: self.tile_on[1],
                    on_click: Some(Box::new(|| Msg::ToggleTile(1))),
                    on_sub_page: Some(Box::new(|| Msg::TileSubPage(1))),
                },
            )));

        let segs = col()
            .w_full()
            .gap(10.0)
            .child(segmented(
                NodeKey::named("seg-std"),
                SegmentedStyle::Standard,
                Some(self.seg_sel[0]),
                t,
                vec![
                    SegmentedItem {
                        icon: Some(Icon::ViewList),
                        label: Some("List".into()),
                        on_click: Some(Box::new(|| Msg::SelectSegmented { group: 0, index: 0 })),
                    },
                    SegmentedItem {
                        icon: Some(Icon::GridView),
                        label: Some("Grid".into()),
                        on_click: Some(Box::new(|| Msg::SelectSegmented { group: 0, index: 1 })),
                    },
                    SegmentedItem {
                        icon: Some(Icon::Public),
                        label: Some("World".into()),
                        on_click: Some(Box::new(|| Msg::SelectSegmented { group: 0, index: 2 })),
                    },
                ],
            ))
            .child(segmented(
                NodeKey::named("seg-lg"),
                SegmentedStyle::Large,
                Some(self.seg_sel[1]),
                t,
                vec![
                    SegmentedItem {
                        icon: None,
                        label: Some("Week".into()),
                        on_click: Some(Box::new(|| Msg::SelectSegmented { group: 1, index: 0 })),
                    },
                    SegmentedItem {
                        icon: None,
                        label: Some("Month".into()),
                        on_click: Some(Box::new(|| Msg::SelectSegmented { group: 1, index: 1 })),
                    },
                ],
            ))
            .child(segmented(
                NodeKey::named("seg-veil"),
                SegmentedStyle::Veil,
                Some(self.seg_sel[2]),
                t,
                vec![
                    SegmentedItem {
                        icon: None,
                        label: Some("On".into()),
                        on_click: Some(Box::new(|| Msg::SelectSegmented { group: 2, index: 0 })),
                    },
                    SegmentedItem {
                        icon: None,
                        label: Some("Off".into()),
                        on_click: Some(Box::new(|| Msg::SelectSegmented { group: 2, index: 1 })),
                    },
                    SegmentedItem {
                        icon: None,
                        label: Some("Auto".into()),
                        on_click: Some(Box::new(|| Msg::SelectSegmented { group: 2, index: 2 })),
                    },
                ],
            ));

        let power = power_menu(
            NodeKey::named("power-menu"),
            self.power_focus,
            t,
            ctx.anim,
            vec![
                PowerItem {
                    icon: Icon::Lock,
                    label: "Lock".into(),
                    hint: Some("Super+L".into()),
                    tone: PowerCellTone::Neutral,
                    on_click: Some(Box::new(|| Msg::FocusPower(0))),
                },
                PowerItem {
                    icon: Icon::Bedtime,
                    label: "Suspend".into(),
                    hint: Some("systemctl suspend".into()),
                    tone: PowerCellTone::Neutral,
                    on_click: Some(Box::new(|| Msg::FocusPower(1))),
                },
                PowerItem {
                    icon: Icon::RestartAlt,
                    label: "Restart".into(),
                    hint: Some("Press Enter".into()),
                    tone: PowerCellTone::Tertiary,
                    on_click: Some(Box::new(|| Msg::FocusPower(2))),
                },
                PowerItem {
                    icon: Icon::PowerSettingsNew,
                    label: "Shut down".into(),
                    hint: Some("Hold to confirm".into()),
                    tone: PowerCellTone::Neutral,
                    on_click: Some(Box::new(|| Msg::FocusPower(3))),
                },
            ],
        );

        col()
            .w_full()
            .gap(12.0)
            .child(sec_label("icon_button · circle → squircle toggle", t))
            .child(ib_row)
            .child(sec_label("chip · filter + status + dismissible", t))
            .child(chip_row)
            .child(sec_label("toggle_tile · r36 veil ↔ r20 primary", t))
            .child(tiles)
            .child(sec_label("segmented · Standard / Large / Veil", t))
            .child(segs)
            .child(sec_label("power_menu · click a cell to expand", t))
            .child(power)
    }

    // ── Section 3: Controls ──────────────────────────────────────────────────

    #[expect(
        clippy::too_many_lines,
        reason = "gallery section enumerates every demo inline; splitting per-widget helpers would scatter the showcase narrative"
    )]
    fn sec_controls(&self, ctx: &mut ViewContext<'_>) -> Vec<Element<Msg>> {
        let t = &self.theme;
        let switch_row = row().gap(28.0).wrap().children([
            labeled(
                switch(
                    NodeKey::named("sw-0"),
                    self.switches[0],
                    false,
                    t,
                    ctx.interaction,
                    ctx.anim,
                    || Msg::ToggleSwitch(0),
                ),
                "Wi-Fi",
                t,
            ),
            labeled(
                switch(
                    NodeKey::named("sw-1"),
                    self.switches[1],
                    false,
                    t,
                    ctx.interaction,
                    ctx.anim,
                    || Msg::ToggleSwitch(1),
                ),
                "Night Light",
                t,
            ),
            labeled(
                switch(
                    NodeKey::named("sw-dis"),
                    true,
                    true,
                    t,
                    ctx.interaction,
                    ctx.anim,
                    || Msg::DemoAction("disabled switch"),
                ),
                "Disabled",
                t,
            ),
        ]);

        let check_row = row().gap(28.0).wrap().children([
            labeled(
                checkbox(
                    NodeKey::named("chk-0"),
                    CheckState::Unchecked,
                    false,
                    t,
                    ctx.interaction,
                    ctx.anim,
                    || Msg::DemoAction("unchecked"),
                ),
                "Unchecked",
                t,
            ),
            labeled(
                checkbox(
                    NodeKey::named("chk-1"),
                    self.checks[0],
                    false,
                    t,
                    ctx.interaction,
                    ctx.anim,
                    || Msg::CycleCheck(0),
                ),
                "Click to cycle",
                t,
            ),
            labeled(
                checkbox(
                    NodeKey::named("chk-2"),
                    self.checks[1],
                    false,
                    t,
                    ctx.interaction,
                    ctx.anim,
                    || Msg::CycleCheck(1),
                ),
                "Indeterminate →",
                t,
            ),
            labeled(
                checkbox(
                    NodeKey::named("chk-dis"),
                    CheckState::Checked,
                    true,
                    t,
                    ctx.interaction,
                    ctx.anim,
                    || Msg::DemoAction("disabled check"),
                ),
                "Disabled",
                t,
            ),
        ]);

        let radio_row = row().gap(28.0).wrap().children((0..3).map(|i| {
            labeled(
                radio(
                    NodeKey::from_pair("radio", i),
                    self.radio_sel == i,
                    false,
                    t,
                    ctx.interaction,
                    ctx.anim,
                    move || Msg::SelectRadio(i),
                ),
                ["Option A", "Option B", "Option C"][i],
                t,
            )
        }));

        vec![
            sec_label("switch", t),
            switch_row,
            sec_label("checkbox", t),
            check_row,
            sec_label("radio", t),
            radio_row,
            self.slider_demos(ctx),
        ]
    }

    fn slider_demos(&self, ctx: &mut ViewContext<'_>) -> Element<Msg> {
        let t = &self.theme;
        let caption = |name: &str, value: f32| {
            row()
                .w_full()
                .justify_between()
                .child(text(name.to_owned(), TypeStyle::BODY_M, on_surf(t)))
                .child(text(
                    format!("{}%", (value * 100.0).round() as i32),
                    FontSpec::mono(12.5),
                    onv(t),
                ))
        };
        col()
            .w_full()
            .gap(12.0)
            .child(sec_label("slider · continuous", t))
            .child(caption("Volume", self.sliders[0]))
            .child(slider(
                NodeKey::named("sl-cont"),
                self.sliders[0],
                620.0,
                SliderOptions::default(),
                t,
                ctx.interaction,
                ctx.anim,
                |value| Msg::SetSlider { index: 0, value },
            ))
            .child(sec_label("slider · stepped (stop dots)", t))
            .child(slider(
                NodeKey::named("sl-step"),
                self.sliders[1],
                620.0,
                SliderOptions {
                    mode: SliderMode::Stepped { steps: 4 },
                    ..Default::default()
                },
                t,
                ctx.interaction,
                ctx.anim,
                |value| Msg::SetSlider { index: 1, value },
            ))
            .child(sec_label("slider · on Glass + disabled", t))
            .child(
                panel(Material::Glass, RADII.md, t)
                    .shadow(ShadowToken::RAISED)
                    .w(660.0)
                    .p(Edges::xy(20.0, 12.0))
                    .m(Edges::new(2.0, 0.0, 10.0, 8.0))
                    .child(slider(
                        NodeKey::named("sl-glass"),
                        self.sliders[2],
                        620.0,
                        SliderOptions {
                            surface: SliderSurface::OnGlass,
                            ..Default::default()
                        },
                        t,
                        ctx.interaction,
                        ctx.anim,
                        |value| Msg::SetSlider { index: 2, value },
                    )),
            )
            .child(slider(
                NodeKey::named("sl-dis"),
                0.6,
                620.0,
                SliderOptions {
                    disabled: true,
                    ..Default::default()
                },
                t,
                ctx.interaction,
                ctx.anim,
                |value| Msg::SetSlider { index: 0, value },
            ))
    }

    // ── Section 4: Inputs ────────────────────────────────────────────────────

    #[expect(
        clippy::too_many_lines,
        reason = "gallery section enumerates every demo inline; splitting per-widget helpers would scatter the showcase narrative"
    )]
    fn sec_inputs(&self, ctx: &mut ViewContext<'_>) -> Vec<Element<Msg>> {
        let t = &self.theme;
        // Fire the wrong-password shake exactly once per failed submit — the
        // `&self` view records which sequence it already triggered via a Cell.
        if self.shake_seen.get() != self.shake_seq {
            self.shake_seen.set(self.shake_seq);
            trigger_password_shake(ctx.anim, NodeKey::named("demo-password"));
        }

        let field = text_input(
            &self.text_input,
            TextInput {
                key: NodeKey::named("demo-input"),
                placeholder: "Type something…".into(),
                lead_icon: Some(Icon::Edit),
                error: self
                    .input_error
                    .then(|| "Something went wrong — fix me".to_owned()),
                on_event: Some(Box::new(|ev| {
                    Some(Msg::FieldEvent(FieldId::Text, ev.clone()))
                })),
                ..Default::default()
            },
            t,
            ctx.interaction,
            ctx.text,
        );

        let search = search_field(
            &self.search,
            SearchField {
                key: NodeKey::named("demo-search"),
                placeholder: "Search apps, math, commands, files…".into(),
                mode: SEARCH_MODES[self.search_mode],
                keycap_hint: Some("Tab".into()),
                on_clear: Some(Box::new(|| Msg::ClearSearch)),
                on_event: Some(Box::new(|ev| {
                    Some(Msg::FieldEvent(FieldId::Search, ev.clone()))
                })),
            },
            t,
            ctx.interaction,
            ctx.text,
        );

        let mode_row = row()
            .gap(8.0)
            .children(SEARCH_MODES.iter().enumerate().map(|(i, m)| {
                chip(
                    t,
                    Chip {
                        key: NodeKey::from_pair("mode-chip", i),
                        label: match m {
                            SearchMode::Search => "Search",
                            SearchMode::Math => "Math",
                            SearchMode::Command => "Command",
                            SearchMode::File => "File",
                            _ => "Mode",
                        }
                        .to_owned(),
                        icon: Some(m.icon()),
                        kind: ChipKind::Filter,
                        selected: self.search_mode == i,
                        on_click: Some(Box::new(move || Msg::SetSearchMode(i))),
                        ..Default::default()
                    },
                )
            }));

        let pw = password_field(
            &self.password,
            PasswordField {
                key: NodeKey::named("demo-password"),
                placeholder: format!("Password — hint: {DEMO_PASSWORD}"),
                revealed: self.password_revealed,
                error: self.password_error.clone(),
                caps_lock_override: self.caps_sim.then_some(true),
                on_toggle_reveal: Some(Box::new(|| Msg::TogglePasswordReveal)),
                on_submit: Some(Box::new(|| Msg::SubmitPassword)),
                on_event: Some(Box::new(|ev| {
                    Some(Msg::FieldEvent(FieldId::Password, ev.clone()))
                })),
            },
            t,
            ctx.interaction,
            ctx.anim,
            ctx.text,
        );

        let caps_row = row()
            .gap(18.0)
            .items_center()
            .child(labeled(
                switch(
                    NodeKey::named("sw-caps-sim"),
                    self.caps_sim,
                    false,
                    t,
                    ctx.interaction,
                    ctx.anim,
                    || Msg::ToggleCapsSim,
                ),
                "Simulate Caps Lock",
                t,
            ))
            .child(text(
                "submit a wrong password to shake; reveal with the eye",
                TypeStyle::BODY_S,
                onv(t),
            ));

        vec![
            sec_label("text_input · inline error", t),
            field,
            row()
                .gap(12.0)
                .items_center()
                .child(button(
                    t,
                    ctx.interaction,
                    ctx.anim,
                    Button {
                        key: NodeKey::named("btn-input-error"),
                        label: "Toggle error".into(),
                        tone: ButtonTone::Text,
                        on_click: Some(Box::new(|| Msg::ToggleInputError)),
                        ..Default::default()
                    },
                ))
                .child(text(
                    "the ring swaps primary → error",
                    TypeStyle::BODY_S,
                    onv(t),
                )),
            sec_label("search_field · mode-aware Wave disc + keycap + clear", t),
            search,
            mode_row,
            sec_label("password_field · masked dots + shake + Caps Lock", t),
            pw,
            caps_row,
        ]
    }

    // ── Section 5: Chrome & Progress ─────────────────────────────────────────

    fn sec_chrome(&self, ctx: &mut ViewContext<'_>) -> Vec<Element<Msg>> {
        let t = &self.theme;
        let ws_row = row()
            .gap(8.0)
            .items_center()
            .children((0..WORKSPACES).map(|i| {
                pill(
                    NodeKey::from_pair("demo-ws", i),
                    (i + 1).to_string(),
                    i == self.active_ws,
                    t,
                    ctx.anim,
                    move || Msg::SelectWorkspace(i),
                )
            }));

        vec![
            sec_label("pill · workspace morph", t),
            ws_row,
            self.status_pill_row(ctx),
            self.identity_row(),
            self.keycap_row(),
            self.progress_demos(ctx),
        ]
    }

    fn status_pill_row(&self, ctx: &mut ViewContext<'_>) -> Element<Msg> {
        let t = &self.theme;
        let mut mk = |status: PillStatus, key: &'static str, glyph: Icon, label: &'static str| {
            let fg = status_fg(status, t);
            status_pill(
                NodeKey::named(key),
                status,
                t,
                ctx.anim,
                Some(Box::new(|| Msg::DemoAction("status pill"))),
                [
                    icon(glyph, IconStyle::Outlined, SIZE.icon_m, fg),
                    text(label, FontSpec::ui(14.0).weight(600), fg),
                ],
            )
        };
        col()
            .w_full()
            .gap(12.0)
            .child(sec_label("status_pill · overflow_pill · badged bell", t))
            .child(
                row()
                    .gap(10.0)
                    .wrap()
                    .items_center()
                    .child(mk(
                        PillStatus::Primary,
                        "stat-primary",
                        Icon::Wifi,
                        "Neptuno-5G",
                    ))
                    .child(mk(
                        PillStatus::Attention,
                        "stat-attn",
                        Icon::Warning,
                        "2 updates",
                    ))
                    .child(mk(
                        PillStatus::Urgent,
                        "stat-urgent",
                        Icon::BatteryAlert,
                        "12%",
                    ))
                    .child(mk(PillStatus::Neutral, "stat-veil", Icon::Info, "Veil"))
                    .child(overflow_pill(NodeKey::named("overflow-demo"), 3, t, || {
                        Msg::DemoAction("overflow")
                    }))
                    .child(badged(
                        icon_button(
                            t,
                            ctx.interaction,
                            ctx.anim,
                            IconButton {
                                key: NodeKey::named("ib-badge"),
                                icon: Icon::Notifications,
                                tone: ButtonTone::Neutral,
                                on_click: Some(Box::new(|| Msg::DemoAction("bell"))),
                                ..Default::default()
                            },
                        ),
                        Badge::Count(7),
                        t,
                    ))
                    .child(badged(
                        icon_disc(Icon::Mail, t),
                        Badge::Dot(Rgba::from_color(t.scheme.surface)),
                        t,
                    ))
                    .child(badge(Badge::Count(120), t)),
            )
    }

    fn identity_row(&self) -> Element<Msg> {
        let t = &self.theme;
        row()
            .gap(16.0)
            .items_center()
            .child(icon_disc(Icon::Folder, t))
            .child(icon_disc_styled(
                Icon::Call,
                48.0,
                t.scheme.tertiary,
                t.scheme.on_tertiary,
            ))
            .child(wave_avatar("SK", 48.0, t))
            .child_opt(
                self.avatar_image
                    .map(|id| wave_avatar_image(id, ImageStyle::default(), 48.0)),
            )
            .child(text(
                "icon_disc · call disc · wave_avatar · wave_avatar_image (sync_images)",
                TypeStyle::BODY_S,
                onv(t),
            ))
    }

    fn keycap_row(&self) -> Element<Msg> {
        let t = &self.theme;
        col()
            .w_full()
            .gap(12.0)
            .child(sec_label("keycap · hint_bar", t))
            .child(
                row()
                    .gap(10.0)
                    .items_center()
                    .child(keycap("Ctrl", KeycapStyle::Flat, t))
                    .child(keycap("K", KeycapStyle::Flat, t))
                    .child(keycap("Shift", KeycapStyle::Raised, t))
                    .child(keycap("Enter", KeycapStyle::Raised, t))
                    .child(box_().w(24.0))
                    .child(hint_bar(
                        [("↑↓", "move"), ("Enter", "open"), ("Esc", "close")],
                        t,
                    )),
            )
    }

    fn progress_demos(&self, ctx: &mut ViewContext<'_>) -> Element<Msg> {
        let t = &self.theme;
        let remaining = self
            .countdown_end
            .saturating_duration_since(ctx.now)
            .as_secs_f32();
        col()
            .w_full()
            .gap(12.0)
            .child(sec_label("wavy_progress · determinate + indeterminate", t))
            .child(wavy_progress(
                NodeKey::named("prog-det"),
                640.0,
                3.5,
                Progress::Determinate(self.sliders[3]),
                t,
                ctx.anim,
            ))
            .child(row().gap(14.0).items_center().child(slider(
                NodeKey::named("sl-progress"),
                self.sliders[3],
                640.0,
                SliderOptions::default(),
                t,
                ctx.interaction,
                ctx.anim,
                |value| Msg::SetSlider { index: 3, value },
            )))
            .child(wavy_progress(
                NodeKey::named("prog-ind"),
                640.0,
                3.5,
                Progress::Indeterminate,
                t,
                ctx.anim,
            ))
            .child(sec_label("wavy_ring · level_meter", t))
            .child(
                row()
                    .gap(20.0)
                    .items_center()
                    .child(
                        box_()
                            .id(NodeKey::named("countdown-ring"))
                            .size(72.0, 72.0)
                            .items_center()
                            .justify_center()
                            .on_click(|| Msg::RestartCountdown)
                            .child(wavy_ring(
                                64.0,
                                4.0,
                                self.countdown_progress(ctx.now),
                                t,
                                ctx.anim,
                            )),
                    )
                    .child(
                        col()
                            .gap(2.0)
                            .child(text(
                                format!("{remaining:.0}s — countdown drain"),
                                TypeStyle::BODY_M,
                                on_surf(t),
                            ))
                            .child(text("click the ring to restart", TypeStyle::BODY_S, onv(t))),
                    )
                    .child(box_().w(32.0))
                    .child(level_meter(
                        NodeKey::named("meter-live"),
                        MeterMode::Playing,
                        (30.0, 26.0),
                        t,
                        ctx.anim,
                    ))
                    .child(level_meter(
                        NodeKey::named("meter-static"),
                        MeterMode::Levels([0.85, 0.5, 0.65]),
                        (30.0, 26.0),
                        t,
                        ctx.anim,
                    ))
                    .child(text("playing · levels", TypeStyle::BODY_S, onv(t))),
            )
    }

    // ── Section 6: Overlays & Toasts ─────────────────────────────────────────

    fn sec_overlays(&self, ctx: &mut ViewContext<'_>) -> Vec<Element<Msg>> {
        let t = &self.theme;
        vec![
            row()
                .gap(32.0)
                .p(Edges::new(32.0, 48.0, 48.0, 48.0))
                .child(self.menu_demo(t))
                .child(Self::tooltip_block(t)),
            sec_label("toast_card · hover pauses auto-dismiss", t),
            self.toast_row(ctx),
        ]
    }

    fn menu_demo(&self, t: &Theme) -> Element<Msg> {
        let submenu = Menu {
            key: NodeKey::named("demo-submenu"),
            width: 208.0,
            entries: vec![
                MenuEntry::Item(MenuItem {
                    key: NodeKey::named("sub-mail"),
                    icon: Some(Icon::Mail),
                    label: "Mail".into(),
                    on_click: Some(Box::new(|| Msg::DemoAction("share mail"))),
                    ..Default::default()
                }),
                MenuEntry::Item(MenuItem {
                    key: NodeKey::named("sub-chat"),
                    icon: Some(Icon::Chat),
                    label: "Chat".into(),
                    on_click: Some(Box::new(|| Msg::DemoAction("share chat"))),
                    ..Default::default()
                }),
                MenuEntry::Item(MenuItem {
                    key: NodeKey::named("sub-cast"),
                    icon: Some(Icon::Cast),
                    label: "Cast".into(),
                    on_click: Some(Box::new(|| Msg::DemoAction("share cast"))),
                    ..Default::default()
                }),
            ],
            ..Default::default()
        };
        menu(
            t,
            Menu {
                key: NodeKey::named("demo-menu"),
                width: 264.0,
                open_submenu: self.submenu_open.then(|| NodeKey::named("mi-share")),
                entries: vec![
                    MenuEntry::Item(MenuItem {
                        key: NodeKey::named("mi-new"),
                        icon: Some(Icon::Add),
                        label: "New tab".into(),
                        shortcut: Some("Ctrl+T".into()),
                        on_click: Some(Box::new(|| Msg::DemoAction("new tab"))),
                        ..Default::default()
                    }),
                    MenuEntry::Item(MenuItem {
                        key: NodeKey::named("mi-copy"),
                        icon: Some(Icon::ContentCopy),
                        label: "Copy link".into(),
                        shortcut: Some("Ctrl+C".into()),
                        on_click: Some(Box::new(|| Msg::DemoAction("copy"))),
                        ..Default::default()
                    }),
                    MenuEntry::Separator,
                    MenuEntry::Item(MenuItem {
                        key: NodeKey::named("mi-share"),
                        icon: Some(Icon::Send),
                        label: "Share…".into(),
                        submenu: Some(Box::new(submenu)),
                        on_click: Some(Box::new(|| Msg::ToggleSubmenu)),
                        ..Default::default()
                    }),
                    MenuEntry::Item(MenuItem {
                        key: NodeKey::named("mi-disabled"),
                        icon: Some(Icon::Cancel),
                        label: "Disabled item".into(),
                        disabled: true,
                        ..Default::default()
                    }),
                    MenuEntry::Separator,
                    MenuEntry::Item(MenuItem {
                        key: NodeKey::named("mi-trash"),
                        icon: Some(Icon::Delete),
                        label: "Move to Trash".into(),
                        shortcut: Some("Del".into()),
                        danger: true,
                        on_click: Some(Box::new(|| Msg::DemoAction("trash"))),
                        ..Default::default()
                    }),
                ],
            },
        )
    }

    fn tooltip_block(t: &Theme) -> Element<Msg> {
        col()
            .flex_1()
            .gap(14.0)
            .child(sec_label("tooltip · glass", t))
            .child(shadow_gutter(tooltip("Glass tooltip · r12 · 32px", t)))
            .child(text(
                "menu: shortcuts, separators, a disabled row, and a nested \
                 submenu (click “Share…”)",
                TypeStyle::BODY_S,
                onv(t),
            ))
    }

    #[expect(
        clippy::too_many_lines,
        reason = "gallery section enumerates every demo inline; splitting per-widget helpers would scatter the showcase narrative"
    )]
    fn toast_row(&self, ctx: &mut ViewContext<'_>) -> Element<Msg> {
        let t = &self.theme;
        let mut cards = col()
            .w_full()
            .gap(32.0)
            .p(Edges::new(32.0, 48.0, 64.0, 48.0));

        if self.toast_dismissed.iter().all(|&d| d) {
            cards = cards.child(row().gap(12.0).items_center().child(button(
                t,
                ctx.interaction,
                ctx.anim,
                Button {
                    key: NodeKey::named("btn-reset-toasts"),
                    label: "Reset toasts".into(),
                    tone: ButtonTone::Tonal,
                    lead_icon: Some(Icon::RestartAlt),
                    on_click: Some(Box::new(|| Msg::ResetToasts)),
                    ..Default::default()
                },
            )));
            return cards;
        }

        if !self.toast_dismissed[0] {
            let paused = toast_is_hovered(NodeKey::named("toast-std"), ctx.interaction);
            cards = cards
                .child(toast_card(
                    t,
                    ctx.anim,
                    ToastCard {
                        key: NodeKey::named("toast-std"),
                        width: 460.0,
                        variant: ToastVariant::Standard,
                        identity: Some(ToastIdentity::Initials("SK".into())),
                        title: Some("Sid".into()),
                        timestamp: Some("now".into()),
                        body: Some("Every triton-kit family renders live on Thick Glass.".into()),
                        actions: vec![
                            ToastAction {
                                key: NodeKey::named("ta-reply"),
                                label: "Reply".into(),
                                tone: ToastActionTone::Primary,
                                on_click: Some(Box::new(|| Msg::ToastActionClicked {
                                    toast: 0,
                                    action: 0,
                                })),
                                ..Default::default()
                            },
                            ToastAction {
                                key: NodeKey::named("ta-archive"),
                                label: "Archive".into(),
                                tone: ToastActionTone::Tonal,
                                on_click: Some(Box::new(|| Msg::ToastActionClicked {
                                    toast: 0,
                                    action: 1,
                                })),
                                ..Default::default()
                            },
                        ],
                        on_dismiss: Some(Box::new(|| Msg::DismissToast(0))),
                        on_activate: Some(Box::new(|| Msg::ActivateToast(0))),
                    },
                ))
                .child(text(
                    if paused {
                        "auto-dismiss paused — pointer is over the toast"
                    } else {
                        "auto-dismiss running — hover the card to pause"
                    },
                    TypeStyle::BODY_S,
                    if paused {
                        Rgba::from_color(t.scheme.tertiary)
                    } else {
                        onv(t)
                    },
                ));
        }
        if !self.toast_dismissed[1] {
            cards = cards.child(toast_card(
                t,
                ctx.anim,
                ToastCard {
                    key: NodeKey::named("toast-crit"),
                    width: 460.0,
                    variant: ToastVariant::Critical,
                    identity: Some(ToastIdentity::AppIcon(Icon::Error)),
                    title: Some("Update failed".into()),
                    timestamp: Some("2m".into()),
                    body: Some("error_container at 78% — stays until dismissed.".into()),
                    actions: vec![ToastAction {
                        key: NodeKey::named("ta-retry"),
                        label: "Retry".into(),
                        tone: ToastActionTone::Primary,
                        on_click: Some(Box::new(|| Msg::ToastActionClicked {
                            toast: 1,
                            action: 0,
                        })),
                        ..Default::default()
                    }],
                    on_dismiss: Some(Box::new(|| Msg::DismissToast(1))),
                    on_activate: Some(Box::new(|| Msg::ActivateToast(1))),
                },
            ));
        }
        if !self.toast_dismissed[2] {
            cards = cards.child(toast_card(
                t,
                ctx.anim,
                ToastCard {
                    key: NodeKey::named("toast-prog"),
                    width: 460.0,
                    variant: ToastVariant::Progress {
                        value: 1.0 - self.countdown_progress(ctx.now),
                        label: None,
                    },
                    identity: Some(ToastIdentity::AppIcon(Icon::Download)),
                    title: Some("Fetching assets".into()),
                    timestamp: Some("live".into()),
                    body: Some("progress follows the countdown ring above.".into()),
                    actions: vec![
                        ToastAction {
                            key: NodeKey::named("ta-pause"),
                            label: "Pause".into(),
                            tone: ToastActionTone::Tonal,
                            on_click: Some(Box::new(|| Msg::ToastActionClicked {
                                toast: 2,
                                action: 0,
                            })),
                            ..Default::default()
                        },
                        ToastAction {
                            key: NodeKey::named("ta-cancel"),
                            label: "Cancel".into(),
                            tone: ToastActionTone::Destructive,
                            on_click: Some(Box::new(|| Msg::DismissToast(2))),
                            ..Default::default()
                        },
                    ],
                    on_dismiss: Some(Box::new(|| Msg::DismissToast(2))),
                    on_activate: Some(Box::new(|| Msg::ActivateToast(2))),
                },
            ));
        }
        cards
    }
}

// ── ShellApp wiring ──────────────────────────────────────────────────────────

impl ShellApp for ShowcaseApp {
    type SurfaceId = SurfaceId;
    type Message = Msg;

    fn surfaces(&self) -> Vec<(SurfaceId, SurfaceSpec)> {
        let now = Instant::now();
        // `SurfaceSpec::island`'s default max_h (120) would clip the FLOATING
        // shadow gutter (blur 40 → pad 80, offset 16 → 64 top / 96 bottom);
        // grow it to fit the 56px pill plus gutter, and use the spec's 8px
        // top offset (docs/design-spec.md).
        let island_spec = |namespace: &str, anchor: SurfaceAnchor, margin: (i32, i32, i32, i32)| {
            let mut spec = SurfaceSpec::island(namespace, anchor, margin);
            spec.sizing = SurfaceSize::FitContent {
                max_w: Some(1400),
                max_h: Some(56 + 64 + 96),
            };
            spec
        };
        let mut out = vec![
            (
                SurfaceId::LeftIsland,
                island_spec(
                    "triton-bar-left",
                    SurfaceAnchor::TOP | SurfaceAnchor::LEFT,
                    (8, 0, 0, 0),
                ),
            ),
            (
                SurfaceId::RightIsland,
                island_spec(
                    "triton-bar-right",
                    SurfaceAnchor::TOP | SurfaceAnchor::RIGHT,
                    (8, 4, 0, 0),
                ),
            ),
        ];

        if self.gallery_visible(now) {
            out.push((
                SurfaceId::Gallery,
                SurfaceSpec {
                    namespace: "triton-gallery".into(),
                    layer: SurfaceLayer::Overlay,
                    anchor: SurfaceAnchor::empty(), // centered when unanchored
                    margin: (0, 0, 0, 0),
                    keyboard: SurfaceKeyboard::Exclusive,
                    exclusive_zone: -1,
                    // 1000px card + FLOATING gutter (blur 40 → 80 sides,
                    // offset 16 → 64 top / 96 bottom) so the full surface fits
                    // cleanly inside 1645×1028 logical (2880×1800 @ 1.75×) below
                    // the top bar islands without shadow clipping.
                    sizing: SurfaceSize::FitContent {
                        max_w: Some(1000 + 2 * 80),
                        max_h: Some(760 + 64 + 96),
                    },
                    shaped_input: true,
                    output: None,
                },
            ));
        }

        out
    }

    fn view(&self, id: SurfaceId, ctx: &mut ViewContext<'_>) -> Element<Msg> {
        match id {
            SurfaceId::LeftIsland => self.view_left_island(ctx),
            SurfaceId::RightIsland => self.view_right_island(ctx),
            SurfaceId::Gallery => self.view_gallery(ctx),
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "gallery section enumerates every demo inline; splitting per-widget helpers would scatter the showcase narrative"
    )]
    fn update(&mut self, _id: SurfaceId, msg: Msg, now: Instant) -> Vec<AppCommand<SurfaceId>> {
        match msg {
            Msg::ToggleGallery => {
                let mut cmds = if self.gallery_open {
                    self.close_gallery(now)
                } else {
                    self.open_gallery(now)
                };
                cmds.push(AppCommand::Redraw(SurfaceId::LeftIsland));
                cmds
            }
            Msg::CloseGallery => {
                let mut cmds = self.close_gallery(now);
                cmds.push(AppCommand::Redraw(SurfaceId::LeftIsland));
                cmds
            }
            Msg::SelectWorkspace(i) => {
                if i < WORKSPACES {
                    self.active_ws = i;
                }
                vec![
                    AppCommand::Redraw(SurfaceId::LeftIsland),
                    AppCommand::Redraw(SurfaceId::Gallery),
                ]
            }
            Msg::ToggleTheme => {
                self.theme = if matches!(self.theme.variant, Variant::Dark) {
                    Theme::light()
                } else {
                    Theme::dark()
                };
                vec![AppCommand::RedrawAll]
            }
            Msg::ToggleReduceMotion => {
                self.reduce_motion = !self.reduce_motion;
                self.gallery_anim
                    .set_motion(Motion::pick(Spring::SPATIAL, self.reduce_motion), now);
                vec![
                    AppCommand::SetReduceMotion(self.reduce_motion),
                    AppCommand::RedrawAll,
                ]
            }
            Msg::ToggleMedia => {
                self.media_playing = !self.media_playing;
                vec![
                    AppCommand::Redraw(SurfaceId::LeftIsland),
                    AppCommand::Redraw(SurfaceId::Gallery),
                ]
            }
            Msg::ToggleDnd => {
                self.dnd = !self.dnd;
                vec![AppCommand::Redraw(SurfaceId::RightIsland)]
            }
            Msg::ToggleWifi => {
                self.wifi = !self.wifi;
                vec![
                    AppCommand::Redraw(SurfaceId::RightIsland),
                    AppCommand::Redraw(SurfaceId::Gallery),
                ]
            }
            Msg::SelectSection(i) => {
                self.section = i.min(SECTIONS.len() - 1);
                gallery_redraw()
            }
            Msg::OpenDialog => {
                self.dialog_open = true;
                gallery_redraw()
            }
            Msg::DismissDialog => {
                self.dialog_open = false;
                gallery_redraw()
            }
            Msg::ToggleIconButton(i) => {
                if let Some(v) = self.icon_toggles.get_mut(i) {
                    *v = !*v;
                }
                gallery_redraw()
            }
            Msg::ToggleChip(i) => {
                if let Some(v) = self.chip_filters.get_mut(i) {
                    *v = !*v;
                }
                gallery_redraw()
            }
            Msg::DismissChip => {
                self.chip_dismissed = true;
                gallery_redraw()
            }
            Msg::ToggleTile(i) => {
                if let Some(v) = self.tile_on.get_mut(i) {
                    *v = !*v;
                }
                gallery_redraw()
            }
            Msg::TileSubPage(i) => {
                tracing::info!(tile = i, "tile sub-page");
                gallery_redraw()
            }
            Msg::SelectSegmented { group, index } => {
                if let Some(sel) = self.seg_sel.get_mut(group) {
                    *sel = index;
                }
                gallery_redraw()
            }
            Msg::FocusPower(i) => {
                self.power_focus = i;
                gallery_redraw()
            }
            Msg::ToggleSwitch(i) => {
                if let Some(v) = self.switches.get_mut(i) {
                    *v = !*v;
                }
                gallery_redraw()
            }
            Msg::CycleCheck(i) => {
                if let Some(v) = self.checks.get_mut(i) {
                    *v = match *v {
                        CheckState::Unchecked => CheckState::Checked,
                        CheckState::Checked => CheckState::Indeterminate,
                        _ => CheckState::Unchecked,
                    };
                }
                gallery_redraw()
            }
            Msg::SelectRadio(i) => {
                self.radio_sel = i;
                gallery_redraw()
            }
            Msg::SetSlider { index, value } => {
                if let Some(v) = self.sliders.get_mut(index) {
                    *v = value;
                }
                gallery_redraw()
            }
            Msg::FieldEvent(field, ev) => self.handle_field(field, &ev, now),
            Msg::ToggleInputError => {
                self.input_error = !self.input_error;
                gallery_redraw()
            }
            Msg::SetSearchMode(i) => {
                self.search_mode = i.min(SEARCH_MODES.len() - 1);
                gallery_redraw()
            }
            Msg::ClearSearch => {
                self.search.clear();
                gallery_redraw()
            }
            Msg::TogglePasswordReveal => {
                self.password_revealed = !self.password_revealed;
                gallery_redraw()
            }
            Msg::SubmitPassword => self.check_password(),
            Msg::ToggleCapsSim => {
                self.caps_sim = !self.caps_sim;
                gallery_redraw()
            }
            Msg::ToggleSubmenu => {
                self.submenu_open = !self.submenu_open;
                gallery_redraw()
            }
            Msg::DismissToast(i) => {
                if let Some(v) = self.toast_dismissed.get_mut(i) {
                    *v = true;
                }
                gallery_redraw()
            }
            Msg::ActivateToast(i) => {
                tracing::info!(toast = i, "toast activated");
                gallery_redraw()
            }
            Msg::ToastActionClicked { toast, action } => {
                tracing::info!(toast, action, "toast action");
                gallery_redraw()
            }
            Msg::ResetToasts => {
                self.toast_dismissed = [false; 3];
                gallery_redraw()
            }
            Msg::RestartCountdown => {
                self.countdown_end = now + Duration::from_secs_f32(COUNTDOWN_SECS);
                gallery_redraw()
            }
            Msg::DemoAction(name) => {
                tracing::info!(action = name, "demo action");
                Vec::new()
            }
        }
    }

    fn is_animating(&self, id: SurfaceId, now: Instant) -> bool {
        match id {
            // Clock-driven indicators (media meter, indeterminate progress,
            // countdown ring) register continuous frames on the host Animator
            // themselves — the app only drives its own gallery open/close spring.
            SurfaceId::Gallery => self.gallery_visible(now) && !self.gallery_anim.is_settled(now),
            _ => false,
        }
    }

    fn on_start(&mut self, _now: Instant) -> Vec<AppCommand<SurfaceId>> {
        let mut cmds = Vec::new();
        if self.auto_open {
            for (tag, ms) in [
                (TIMER_SELECT_WS, 450),
                (TIMER_OPEN_GALLERY, 900),
                (TIMER_DEMO_STEP, 1800),
            ] {
                cmds.push(AppCommand::ScheduleTimer {
                    tag,
                    delay: Duration::from_millis(ms),
                });
            }
        }
        if let Some(ms) = self.quit_ms {
            cmds.push(AppCommand::ScheduleTimer {
                tag: TIMER_QUIT,
                delay: Duration::from_millis(ms),
            });
        }
        cmds
    }

    fn on_timer(&mut self, tag: u32, now: Instant) -> Vec<AppCommand<SurfaceId>> {
        match tag {
            TIMER_SELECT_WS => {
                self.active_ws = 1;
                vec![AppCommand::Redraw(SurfaceId::LeftIsland)]
            }
            TIMER_OPEN_GALLERY => {
                let mut cmds = self.open_gallery(now);
                cmds.push(AppCommand::Redraw(SurfaceId::LeftIsland));
                cmds
            }
            TIMER_DEMO_STEP => {
                self.section = 1;
                vec![AppCommand::Redraw(SurfaceId::Gallery)]
            }
            TIMER_CARET_BLINK => {
                if self.gallery_open {
                    self.search.caret_visible = !self.search.caret_visible;
                    vec![
                        AppCommand::Redraw(SurfaceId::Gallery),
                        AppCommand::ScheduleTimer {
                            tag: TIMER_CARET_BLINK,
                            delay: Duration::from_millis(530),
                        },
                    ]
                } else {
                    Vec::new()
                }
            }
            TIMER_QUIT => vec![AppCommand::Quit],
            _ => Vec::new(),
        }
    }

    fn sync_images(&mut self, images: &mut ImageStore) {
        if self.avatar_image.is_none()
            && let Some(data) =
                ImageData::from_rgba(AVATAR_PX, AVATAR_PX, sample_avatar_rgba(AVATAR_PX))
        {
            self.avatar_image = Some(images.insert(data));
        }
    }

    fn on_exit(&mut self, metrics: &HashMap<SurfaceId, SurfaceMetrics>) {
        let mut out = String::from("triton-kit showcase report\n");
        for id in [
            SurfaceId::LeftIsland,
            SurfaceId::RightIsland,
            SurfaceId::Gallery,
        ] {
            if let Some(m) = metrics.get(&id) {
                let _ = writeln!(
                    out,
                    "{id:?}: {} frames, logical={}x{}, buffer={}x{}, scale={:.2}, interval_ms mean={:.2} p50={:.2} p95={:.2} max={:.2}",
                    m.frames,
                    m.logical_size.0,
                    m.logical_size.1,
                    m.buffer_size.0,
                    m.buffer_size.1,
                    m.scale,
                    m.interval_ms_mean,
                    m.interval_ms_p50,
                    m.interval_ms_p95,
                    m.interval_ms_max,
                );
            }
        }
        if let Ok(mem) = std::fs::read_to_string("/proc/self/smaps_rollup") {
            for key in ["Rss:", "Pss:", "Private_Dirty:"] {
                if let Some(line) = mem.lines().find(|l| l.starts_with(key)) {
                    let _ = writeln!(out, "{line}");
                }
            }
        }
        eprint!("{out}");
        let _ = std::fs::write(&self.report_path, &out);
    }
}

#[derive(Debug)]
struct HeadlessArgs {
    snap: Option<PathBuf>,
    tree: Option<PathBuf>,
    sections: Vec<usize>,
    themes: Vec<Variant>,
    reduce_motion: bool,
    scale: f32,
    scroll_y: f32,
    dialog: bool,
    transparent: bool,
}

fn parse_section_arg(raw: &str) -> Result<Vec<usize>, String> {
    match raw.to_ascii_lowercase().as_str() {
        "all" => Ok((0..SECTIONS.len()).collect()),
        "surfaces" => Ok(vec![0]),
        "buttons" => Ok(vec![1]),
        "controls" => Ok(vec![2]),
        "inputs" => Ok(vec![3]),
        "chrome" => Ok(vec![4]),
        "overlays" => Ok(vec![5]),
        other => Err(format!(
            "unknown --section '{other}' (expected surfaces|buttons|controls|inputs|chrome|overlays|all)"
        )),
    }
}

fn parse_headless_args(args: &[String]) -> Result<Option<HeadlessArgs>, String> {
    let mut snap: Option<PathBuf> = None;
    let mut tree: Option<PathBuf> = None;
    let mut sections: Option<Vec<usize>> = None;
    let mut themes: Vec<Variant> = vec![Variant::Dark];
    let mut reduce_motion = false;
    let mut scale = 1.75_f32;
    let mut scroll_y = 0.0_f32;
    let mut dialog = false;
    let mut transparent = false;

    let mut i = 0;
    while i < args.len() {
        let next = |i: &mut usize, flag: &str| -> Result<String, String> {
            *i += 1;
            args.get(*i)
                .cloned()
                .ok_or_else(|| format!("missing value for {flag}"))
        };
        match args[i].as_str() {
            "--snap" => snap = Some(PathBuf::from(next(&mut i, "--snap")?)),
            "--tree" => tree = Some(PathBuf::from(next(&mut i, "--tree")?)),
            "--section" => sections = Some(parse_section_arg(&next(&mut i, "--section")?)?),
            "--theme" => {
                let v = next(&mut i, "--theme")?;
                themes = match v.to_ascii_lowercase().as_str() {
                    "dark" => vec![Variant::Dark],
                    "light" => vec![Variant::Light],
                    "both" => vec![Variant::Dark, Variant::Light],
                    other => {
                        return Err(format!(
                            "unknown --theme '{other}' (expected dark|light|both)"
                        ));
                    }
                };
            }
            "--reduce-motion" => reduce_motion = true,
            "--scale" => {
                let v = next(&mut i, "--scale")?;
                scale = v
                    .parse::<f32>()
                    .map_err(|e| format!("invalid --scale '{v}': {e}"))?;
            }
            "--scroll" => {
                let v = next(&mut i, "--scroll")?;
                scroll_y = v
                    .parse::<f32>()
                    .map_err(|e| format!("invalid --scroll '{v}': {e}"))?;
            }
            "--dialog" => dialog = true,
            "--transparent" => transparent = true,
            other => return Err(format!("unknown argument '{other}'")),
        }
        i += 1;
    }

    if snap.is_none() && tree.is_none() {
        return Ok(None);
    }

    let default_sections = if snap
        .as_ref()
        .is_some_and(|p| p.extension().is_some_and(|e| e == "png"))
    {
        vec![0]
    } else {
        (0..SECTIONS.len()).collect()
    };

    Ok(Some(HeadlessArgs {
        snap,
        tree,
        sections: sections.unwrap_or(default_sections),
        themes,
        reduce_motion,
        scale,
        scroll_y,
        dialog,
        transparent,
    }))
}

fn section_slug(index: usize) -> &'static str {
    match SECTIONS[index] {
        Section::Surfaces => "surfaces",
        Section::Buttons => "buttons",
        Section::Controls => "controls",
        Section::Inputs => "inputs",
        Section::Chrome => "chrome",
        Section::Overlays => "overlays",
    }
}

fn theme_slug(variant: Variant) -> &'static str {
    match variant {
        Variant::Dark => "dark",
        Variant::Light => "light",
    }
}

fn run_headless(opts: &HeadlessArgs) -> Result<(), String> {
    let mut renderer = if opts.snap.is_some() {
        Some(HeadlessRenderer::new().map_err(|e| format!("HeadlessRenderer::new: {e}"))?)
    } else {
        None
    };

    let single_target = opts.sections.len() == 1 && opts.themes.len() == 1;
    let snap_is_file = opts
        .snap
        .as_ref()
        .is_some_and(|p| p.extension().is_some_and(|e| e == "png"));
    let tree_is_file = opts
        .tree
        .as_ref()
        .is_some_and(|p| p.extension().is_some_and(|e| e == "json"));

    let write_artifact = |base: &Path, is_file: bool, stem: &str, ext: &str| -> PathBuf {
        if single_target && is_file {
            base.to_path_buf()
        } else {
            base.join(format!("{stem}.{ext}"))
        }
    };

    for &variant in &opts.themes {
        let backdrop = if opts.transparent {
            Rgba::TRANSPARENT
        } else {
            match variant {
                Variant::Dark => Rgba::new(0.11, 0.12, 0.15, 1.0),
                Variant::Light => Rgba::new(0.84, 0.86, 0.90, 1.0),
            }
        };

        for &sec_idx in &opts.sections {
            let mut session = HeadlessSession::<Msg>::new((1200.0, 900.0), opts.scale);
            session.set_reduce_motion(opts.reduce_motion);
            let now = session.now();
            let mut app = ShowcaseApp::new(now);
            app.theme = match variant {
                Variant::Dark => Theme::dark(),
                Variant::Light => Theme::light(),
            };
            app.reduce_motion = opts.reduce_motion;
            app.gallery_open = true;
            app.gallery_anim.snap(1.0, now);
            app.section = sec_idx;
            app.dialog_open = opts.dialog;

            if opts.scroll_y > 0.0 {
                session.interaction_mut().set_scroll_offset(
                    NodeKey::from_pair("section-scroll", sec_idx),
                    opts.scroll_y,
                );
            }

            let sizing = app
                .surfaces()
                .into_iter()
                .find(|(id, _)| *id == SurfaceId::Gallery)
                .map_or(
                    SurfaceSize::FitContent {
                        max_w: Some(1160),
                        max_h: Some(920),
                    },
                    |(_, spec)| spec.sizing,
                );
            // Render once to seed springs, then advance 450ms so clock-driven
            // indicators (indeterminate wavy progress, level meter) show mid-flight.
            session.render_surface(&mut app, SurfaceId::Gallery, sizing);
            if !opts.reduce_motion {
                session.advance(Duration::from_millis(450));
                session.render_surface(&mut app, SurfaceId::Gallery, sizing);
            }

            let stem = format!("{}-{}", section_slug(sec_idx), theme_slug(variant));
            if let (Some(snap_base), Some(r)) = (&opts.snap, renderer.as_mut()) {
                let out_path = write_artifact(snap_base, snap_is_file, &stem, "png");
                let (w, h) = r
                    .snap_png_with_backdrop(&mut session, backdrop, &out_path)
                    .map_err(|e| format!("snap_png {}: {e}", out_path.display()))?;
                eprintln!("snapped {} ({w}x{h})", out_path.display());
            }
            if let Some(tree_base) = &opts.tree {
                let out_path = write_artifact(tree_base, tree_is_file, &stem, "json");
                if let Some(parent) = out_path.parent()
                    && !parent.as_os_str().is_empty()
                {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("create_dir_all {}: {e}", parent.display()))?;
                }
                std::fs::write(&out_path, session.tree_json())
                    .map_err(|e| format!("write {}: {e}", out_path.display()))?;
            }
        }

        // When snapping multiple sections into a directory, also snap both bar islands.
        if opts.sections.len() > 1
            && let (Some(snap_base), Some(r)) = (&opts.snap, renderer.as_mut())
            && !snap_is_file
        {
            snap_islands(opts, variant, backdrop, snap_base, r)?;
        }
    }

    Ok(())
}

fn snap_islands(
    opts: &HeadlessArgs,
    variant: Variant,
    backdrop: Rgba,
    snap_base: &Path,
    renderer: &mut HeadlessRenderer,
) -> Result<(), String> {
    for (surface_id, name) in [
        (SurfaceId::LeftIsland, "left-island"),
        (SurfaceId::RightIsland, "right-island"),
    ] {
        let mut session = HeadlessSession::<Msg>::new((1200.0, 220.0), opts.scale);
        session.set_reduce_motion(opts.reduce_motion);
        let now = session.now();
        let mut app = ShowcaseApp::new(now);
        app.theme = match variant {
            Variant::Dark => Theme::dark(),
            Variant::Light => Theme::light(),
        };
        app.reduce_motion = opts.reduce_motion;
        let sizing = app
            .surfaces()
            .into_iter()
            .find(|(id, _)| *id == surface_id)
            .map_or(
                SurfaceSize::FitContent {
                    max_w: Some(1400),
                    max_h: Some(220),
                },
                |(_, spec)| spec.sizing,
            );
        session.render_surface(&mut app, surface_id, sizing);
        let stem = format!("{name}-{}", theme_slug(variant));
        let out_path = snap_base.join(format!("{stem}.png"));
        let (w, h) = renderer
            .snap_png_with_backdrop(&mut session, backdrop, &out_path)
            .map_err(|e| format!("snap_png {}: {e}", out_path.display()))?;
        eprintln!("snapped {} ({w}x{h})", out_path.display());
    }
    Ok(())
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("triton_engine=info,info")),
        )
        .init();

    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    match parse_headless_args(&raw_args) {
        Ok(Some(opts)) => {
            if let Err(err) = run_headless(&opts) {
                eprintln!("triton-kit showcase headless error: {err}");
                std::process::exit(1);
            }
            return;
        }
        Err(err) => {
            eprintln!("triton-kit showcase: {err}");
            std::process::exit(2);
        }
        Ok(None) => {}
    }

    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        eprintln!(
            "triton-kit showcase: WAYLAND_DISPLAY is not set — run `mise run dev` for an isolated nested session, or pass `--snap <dir>` for headless PNG rendering!"
        );
        std::process::exit(2);
    }

    let app = ShowcaseApp::new(Instant::now());
    if let Err(err) = run(app) {
        tracing::error!(%err, "showcase exited with error");
        std::process::exit(1);
    }
}

// ── Headless tests ───────────────────────────────────────────────────────────
// Every gallery section is rendered through `layout_and_paint` — pure CPU, no
// Wayland/EGL — in both `Theme` variants and with `reduce_motion` on and off.
// Run via `cargo test -p triton-kit --examples` (also covered by
// `cargo test --workspace --all-targets` in `mise run check`).

#[cfg(test)]
mod tests {
    use super::*;
    use triton_engine::anim::Animator;
    use triton_engine::gpu::DrawList;
    use triton_engine::text::TextEngine;
    use triton_engine::ui::{BuiltFrame, InteractionState, layout_and_paint};

    const SURFACES: [SurfaceId; 3] = [
        SurfaceId::LeftIsland,
        SurfaceId::RightIsland,
        SurfaceId::Gallery,
    ];

    fn open_app() -> ShowcaseApp {
        let t0 = Instant::now();
        let mut app = ShowcaseApp::new(t0);
        app.gallery_open = true;
        app.gallery_anim.snap(1.0, t0);
        app
    }

    /// Paint one surface of `app` headlessly at 1.75×.
    fn paint(
        app: &mut ShowcaseApp,
        id: SurfaceId,
        reduce_motion: bool,
    ) -> (BuiltFrame<Msg>, InteractionState, DrawList) {
        let now = Instant::now();
        let mut text = TextEngine::new();
        let mut images = ImageStore::new();
        app.sync_images(&mut images);
        let mut anim = Animator::new();
        anim.begin_frame(now, reduce_motion);
        let interaction = InteractionState::default();
        let root = {
            let mut ctx = ViewContext {
                now,
                logical_size: (1200, 880),
                scale: 1.75,
                interaction: &interaction,
                text: &mut text,
                anim: &mut anim,
                images: &images,
            };
            app.view(id, &mut ctx)
        };
        anim.end_frame();
        let mut dl = DrawList::default();
        let frame = layout_and_paint(
            root,
            (Some(1200.0), Some(880.0)),
            1.75,
            &interaction,
            &mut text,
            &images,
            &mut dl,
        );
        (frame, interaction, dl)
    }

    #[test]
    fn every_section_paints_in_both_themes_and_motion_modes() {
        for section in 0..SECTIONS.len() {
            for dark in [true, false] {
                for reduce in [false, true] {
                    let mut app = open_app();
                    app.theme = if dark { Theme::dark() } else { Theme::light() };
                    app.section = section;
                    let (frame, _interaction, dl) = paint(&mut app, SurfaceId::Gallery, reduce);
                    assert!(
                        frame.content_size.0 > 0.0 && frame.content_size.1 > 0.0,
                        "section {section} dark={dark} reduce={reduce} painted empty"
                    );
                    assert!(
                        !frame.blur_rects.is_empty(),
                        "section {section}: Thick panel must emit blur rects"
                    );
                    assert!(!frame.hits.is_empty(), "section {section}: no hit targets");
                    assert!(dl.quad_count() > 0, "section {section}: no quads emitted");
                }
            }
        }
    }

    #[test]
    fn islands_paint_with_blur_and_hits() {
        let mut app = open_app();
        for id in [SurfaceId::LeftIsland, SurfaceId::RightIsland] {
            let (frame, _i, _dl) = paint(&mut app, id, false);
            assert!(
                !frame.blur_rects.is_empty(),
                "{id:?}: island must emit blur"
            );
            assert!(
                frame.hits.iter().any(|e| e.on_click.is_some()),
                "{id:?}: island needs clickable targets"
            );
        }
    }

    #[test]
    fn theme_toggle_flips_variant_and_redraws_all() {
        let mut app = open_app();
        let cmds = app.update(SurfaceId::RightIsland, Msg::ToggleTheme, Instant::now());
        assert!(matches!(app.theme.variant, Variant::Light));
        assert!(cmds.contains(&AppCommand::RedrawAll));
        let cmds = app.update(SurfaceId::RightIsland, Msg::ToggleTheme, Instant::now());
        assert!(matches!(app.theme.variant, Variant::Dark));
        assert!(cmds.contains(&AppCommand::RedrawAll));
    }

    #[test]
    fn reduce_motion_toggle_emits_set_reduce_motion() {
        let mut app = open_app();
        let cmds = app.update(
            SurfaceId::RightIsland,
            Msg::ToggleReduceMotion,
            Instant::now(),
        );
        assert!(app.reduce_motion);
        assert!(cmds.contains(&AppCommand::SetReduceMotion(true)));
        let cmds = app.update(
            SurfaceId::RightIsland,
            Msg::ToggleReduceMotion,
            Instant::now(),
        );
        assert!(!app.reduce_motion);
        assert!(cmds.contains(&AppCommand::SetReduceMotion(false)));
    }

    #[test]
    fn dialog_paints_over_scrim_and_scrim_dismisses() {
        let mut app = open_app();
        app.update(SurfaceId::Gallery, Msg::OpenDialog, Instant::now());
        assert!(app.dialog_open);
        let (frame, mut interaction, _dl) = paint(&mut app, SurfaceId::Gallery, false);
        assert!(
            frame
                .hits
                .iter()
                .any(|e| e.key == Some(NodeKey::named("gallery-dialog"))),
            "dialog card must register a hit target"
        );
        // A click in the far corner lands on the scrim -> dismiss.
        frame.dispatch_event(
            &InputEvent::PointerDown {
                x: 4.0,
                y: 4.0,
                button: 272,
            },
            &mut interaction,
        );
        let (msgs, _d) = frame.dispatch_event(
            &InputEvent::PointerUp {
                x: 4.0,
                y: 4.0,
                button: 272,
            },
            &mut interaction,
        );
        assert!(
            msgs.iter().any(|m| matches!(m, Msg::DismissDialog)),
            "scrim click should dismiss, got {msgs:?}"
        );
    }

    #[test]
    fn section_tabs_dispatch_select_section() {
        let mut app = open_app();
        app.section = 0;
        let (frame, mut interaction, _dl) = paint(&mut app, SurfaceId::Gallery, false);
        // Tab cells derive their keys from the group key via XOR salt — click
        // each hit whose rect top matches the tab bar row.
        let tab_hits: Vec<_> = frame
            .hits
            .iter()
            .filter(|e| e.on_click.is_some() && e.rect.h > 40.0 && e.rect.h < 48.0)
            .map(|e| e.rect)
            .collect();
        assert_eq!(
            tab_hits.len(),
            SECTIONS.len(),
            "expected one hit per tab cell"
        );
        let r = tab_hits[3];
        let (x, y) = (r.x + r.w * 0.5, r.y + r.h * 0.5);
        frame.dispatch_event(
            &InputEvent::PointerDown { x, y, button: 272 },
            &mut interaction,
        );
        let (msgs, _d) = frame.dispatch_event(
            &InputEvent::PointerUp { x, y, button: 272 },
            &mut interaction,
        );
        assert!(
            msgs.iter().any(|m| matches!(m, Msg::SelectSection(3))),
            "expected SelectSection(3), got {msgs:?}"
        );
    }

    #[test]
    fn wrong_password_sets_error_and_shakes() {
        let mut app = open_app();
        app.password.insert_str("hunter2");
        let before = app.shake_seq;
        app.update(SurfaceId::Gallery, Msg::SubmitPassword, Instant::now());
        assert!(app.password_error.is_some());
        assert_eq!(app.shake_seq, before + 1);
        // Painting the Inputs section must not panic while a shake is armed.
        app.section = SECTIONS.iter().position(|s| s == &Section::Inputs).unwrap();
        let (_f, _i, _dl) = paint(&mut app, SurfaceId::Gallery, false);
        // The correct password clears the error and the field.
        app.password = TextInputState::new(DEMO_PASSWORD);
        app.update(SurfaceId::Gallery, Msg::SubmitPassword, Instant::now());
        assert!(app.password_error.is_none());
        assert!(app.password.value.is_empty());
    }

    #[test]
    fn search_mode_switch_and_field_events() {
        let mut app = open_app();
        app.update(SurfaceId::Gallery, Msg::SetSearchMode(2), Instant::now());
        assert_eq!(app.search_mode, 2);
        app.section = SECTIONS.iter().position(|s| s == &Section::Inputs).unwrap();
        let (_f, _i, _dl) = paint(&mut app, SurfaceId::Gallery, false);
        // Key routing: typing into the search field updates its state.
        let ev = InputEvent::KeyDown {
            key: Key::Char('x'),
            utf8: Some("x".into()),
            ctrl: false,
            shift: false,
            alt: false,
            caps_lock: false,
        };
        app.update(
            SurfaceId::Gallery,
            Msg::FieldEvent(FieldId::Search, ev),
            Instant::now(),
        );
        assert!(app.search.value.contains('x'));
    }

    #[test]
    fn toast_hover_pause_is_live_through_dispatch() {
        let mut app = open_app();
        app.section = SECTIONS
            .iter()
            .position(|s| s == &Section::Overlays)
            .unwrap();
        let (frame, mut interaction, _dl) = paint(&mut app, SurfaceId::Gallery, false);
        let rect = frame
            .hits
            .iter()
            .find(|e| e.key == Some(NodeKey::named("toast-std")))
            .map(|e| e.rect)
            .expect("standard toast hit target");
        frame.dispatch_event(
            &InputEvent::PointerMove {
                x: rect.x + rect.w * 0.5,
                y: rect.y + rect.h * 0.5,
            },
            &mut interaction,
        );
        assert!(toast_is_hovered(NodeKey::named("toast-std"), &interaction));
    }

    #[test]
    fn all_surfaces_declared_and_reported() {
        let mut app = open_app();
        let declared: Vec<_> = app.surfaces().iter().map(|(id, _)| *id).collect();
        for id in SURFACES {
            assert!(declared.contains(&id), "{id:?} missing from surfaces()");
        }
        // A closed + settled gallery is torn down entirely.
        let now = Instant::now();
        app.gallery_open = false;
        app.gallery_anim.snap(0.0, now);
        let declared: Vec<_> = app.surfaces().iter().map(|(id, _)| *id).collect();
        assert!(!declared.contains(&SurfaceId::Gallery));
    }
}
