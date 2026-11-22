//! Triton Engine Showcase — interactive multi-surface demo for visual & metric verification.
//!
//! Run ONLY inside the preview container (`mise run preview`), never against the host session!
//!
//! Surfaces:
//! 1. `LeftIsland`  — independent top-left Glass island (`Layer::Top`) with launcher button
//!    and 4 spring-morphing workspace pills.
//! 2. `RightIsland` — independent top-right Glass island (`Layer::Top`) with clock, status
//!    pill (Wi-Fi, volume, battery), and notification bell badge.
//! 3. `Launcher`    — floating centered Thick-glass panel (`Layer::Overlay`, exclusive keyboard,
//!    zero broken fullscreen dark scrim) combining a Launcher (`TextInput` + `ScrollArea` app list)
//!    and a mini Quick Settings column (`Slider`s for Brightness & Volume, spring-animated `Switch`es,
//!    concentric 28px/14px nested radii, smooth corners, shaped compositor blur).

use std::collections::HashMap;
use std::fmt::Write as _;
use std::time::{Duration, Instant};

use triton_design::icon::{Icon, IconStyle};
use triton_design::material::{Material, ShadowToken};
use triton_design::motion::{Animated, Motion, Spring};
use triton_design::shape::{CORNER_SMOOTHING, RADII, list_corners};
use triton_design::typography::TypeStyle;
use triton_engine::{
    AppCommand, Corners, DropShadow, Edges, Element, FontSpec, InputEvent, Key, NodeKey, Rgba,
    ShellApp, SubtreeTransform, SurfaceAnchor, SurfaceKeyboard, SurfaceLayer, SurfaceMetrics,
    SurfaceSize, SurfaceSpec, Theme, ViewContext, box_, col, icon, row, run, text,
};
use triton_kit::{
    Button, ButtonTone, IconButton, ScrollAreaOptions, SwitchState, TextInputAction,
    TextInputState, button, concentric_inner, icon_button, island, panel, pill, scroll_area,
    slider, switch, text_input,
};

const TIMER_OPEN_PANEL: u32 = 1;
const TIMER_SELECT_WS: u32 = 2;
const TIMER_TOGGLE_SWITCH: u32 = 3;
const TIMER_CARET_BLINK: u32 = 4;
const TIMER_QUIT: u32 = 5;

const WORKSPACES: usize = 4;
const PANEL_RADIUS: f32 = RADII.xl; // 28px
const PANEL_PAD: f32 = 14.0;
const INNER_RADIUS: f32 = PANEL_RADIUS - PANEL_PAD; // 14px concentric!

struct AppEntry {
    name: &'static str,
    subtitle: &'static str,
    icon: Icon,
    badge_rgba: [f32; 4],
    tag: Option<&'static str>,
}

const APPS: &[AppEntry] = &[
    AppEntry {
        name: "Console",
        subtitle: "Terminal emulator · GNOME",
        icon: Icon::Terminal,
        badge_rgba: [0.0, 0.26, 0.61, 1.0],
        tag: Some("Work"),
    },
    AppEntry {
        name: "Files",
        subtitle: "Browse folders and documents",
        icon: Icon::Folder,
        badge_rgba: [0.49, 0.19, 0.08, 1.0],
        tag: None,
    },
    AppEntry {
        name: "Web Browser",
        subtitle: "Fast, private browsing",
        icon: Icon::Public,
        badge_rgba: [0.16, 0.34, 0.56, 1.0],
        tag: Some("Open"),
    },
    AppEntry {
        name: "Code Editor",
        subtitle: "Rust & Wayland workspace",
        icon: Icon::Code,
        badge_rgba: [0.25, 0.28, 0.35, 1.0],
        tag: Some("Work"),
    },
    AppEntry {
        name: "Music Player",
        subtitle: "Lossless audio library",
        icon: Icon::MusicNote,
        badge_rgba: [0.42, 0.21, 0.48, 1.0],
        tag: None,
    },
    AppEntry {
        name: "Calculator",
        subtitle: "Scientific & unit math",
        icon: Icon::Calculate,
        badge_rgba: [0.14, 0.42, 0.36, 1.0],
        tag: None,
    },
    AppEntry {
        name: "Wallpaper",
        subtitle: "Dynamic matugen themes",
        icon: Icon::Wallpaper,
        badge_rgba: [0.35, 0.31, 0.48, 1.0],
        tag: None,
    },
    AppEntry {
        name: "System Settings",
        subtitle: "Displays, sound, niri layout",
        icon: Icon::Settings,
        badge_rgba: [0.24, 0.27, 0.32, 1.0],
        tag: None,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SurfaceId {
    LeftIsland,
    RightIsland,
    Launcher,
}

#[derive(Debug, Clone)]
enum Msg {
    ToggleLauncher,
    CloseLauncher,
    SelectWorkspace(usize),
    SearchEvent(InputEvent),
    LaunchApp(usize),
    SetBrightness(f32),
    SetVolume(f32),
    ToggleWifi,
    ToggleNightLight,
    ToggleDnd,
}

struct ShowcaseApp {
    theme: Theme,
    active_ws: usize,
    launcher_open: bool,
    launcher_anim: Animated,
    search: TextInputState,
    selected_app: usize,
    brightness: f32,
    volume: f32,
    wifi: SwitchState,
    night_light: SwitchState,
    dnd: SwitchState,
    status_note: String,
    auto_open: bool,
    quit_ms: Option<u64>,
    report_path: String,
}

impl ShowcaseApp {
    fn new(now: Instant) -> Self {
        let auto_open = std::env::var_os("SHOWCASE_OPEN").is_some();
        let quit_ms = std::env::var("SHOWCASE_QUIT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|&ms| ms > 0);
        let report_path = std::env::var("SHOWCASE_REPORT")
            .unwrap_or_else(|_| "/preview/triton-engine-showcase.txt".into());

        let initial_query = std::env::var("SHOWCASE_QUERY").unwrap_or_default();

        Self {
            theme: Theme::dark(),
            active_ws: 0,
            launcher_open: false,
            launcher_anim: Animated::new(0.0, Motion::Spring(Spring::SPATIAL), 0.002, now),
            search: TextInputState::new(initial_query),
            selected_app: 0,
            brightness: 0.72,
            volume: 0.48,
            wifi: SwitchState::new(true, now),
            night_light: SwitchState::new(true, now),
            dnd: SwitchState::new(false, now),
            status_note: "Neptuno-5G".into(),
            auto_open,
            quit_ms,
            report_path,
        }
    }

    fn filtered_apps(&self) -> Vec<usize> {
        let q = self.search.value.trim().to_lowercase();
        APPS.iter()
            .enumerate()
            .filter(|(_, a)| {
                q.is_empty()
                    || a.name.to_lowercase().contains(&q)
                    || a.subtitle.to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn launcher_visible(&self, now: Instant) -> bool {
        self.launcher_open || !self.launcher_anim.is_settled(now)
    }

    fn open_launcher(&mut self, now: Instant) -> Vec<AppCommand<SurfaceId>> {
        self.launcher_open = true;
        self.launcher_anim.set_target(1.0, now);
        self.search.caret_visible = true;
        let mut cmds = vec![
            AppCommand::Redraw(SurfaceId::Launcher),
            AppCommand::Focus(SurfaceId::Launcher, NodeKey::named("launcher-search")),
        ];
        if std::env::var_os("SHOWCASE_NO_BLINK").is_none() {
            cmds.push(AppCommand::ScheduleTimer {
                tag: TIMER_CARET_BLINK,
                delay: Duration::from_millis(530),
            });
        }
        cmds
    }

    fn close_launcher(&mut self, now: Instant) -> Vec<AppCommand<SurfaceId>> {
        self.launcher_open = false;
        self.launcher_anim.set_target(0.0, now);
        vec![AppCommand::Redraw(SurfaceId::Launcher)]
    }

    fn select_workspace(&mut self, idx: usize) {
        if idx >= WORKSPACES || idx == self.active_ws {
            return;
        }
        self.active_ws = idx;
    }

    // ── Views ────────────────────────────────────────────────────────────────

    fn view_left_island(&self, ctx: &mut ViewContext<'_>) -> Element<Msg> {
        let t = &self.theme;
        // Shadow pad around the island ensures drop shadow never clips at the surface edge!
        let mut bar = island(t).child(icon_button(
            t,
            ctx.interaction,
            ctx.anim,
            IconButton {
                key: "btn-launcher".into(),
                icon: Icon::Apps,
                toggled: self.launcher_open,
                tone: ButtonTone::Filled,
                on_click: Some(Box::new(|| Msg::ToggleLauncher)),
                ..Default::default()
            },
        ));

        bar = bar.child(box_().w(6.0).h(1.0));

        for i in 0..WORKSPACES {
            let key = NodeKey::from_pair("ws", i);
            let target = f32::from(u8::from(i == self.active_ws));
            let m = ctx.anim.spring(key, target, Spring::SPATIAL_FAST);
            bar = bar.child(pill(
                key,
                (i + 1).to_string(),
                m,
                40.0,
                58.0,
                40.0,
                t,
                move || Msg::SelectWorkspace(i),
            ));
        }

        // Subtle active workspace title chip inside the left island
        bar = bar
            .child(
                box_()
                    .w(1.0)
                    .h(24.0)
                    .m(Edges::xy(8.0, 0.0))
                    .bg(Rgba::new(1.0, 1.0, 1.0, 0.16)),
            )
            .child(
                row()
                    .px(10.0)
                    .gap(6.0)
                    .items_center()
                    .child(icon(
                        Icon::Terminal,
                        IconStyle::Filled,
                        18.0,
                        Rgba::from_color(t.scheme.primary),
                    ))
                    .child(text(
                        format!("Workspace {}", self.active_ws + 1),
                        FontSpec::ui(14.0).weight(600),
                        Rgba::from_color(t.scheme.on_surface),
                    )),
            );

        // Wrap in transparent shadow gutter so the island's soft shadow is never clipped.
        box_()
            .p(DropShadow::from(ShadowToken::FLOATING).gutter())
            .child(bar)
    }

    fn view_right_island(&self, ctx: &mut ViewContext<'_>) -> Element<Msg> {
        let t = &self.theme;
        let target_bg = if self.wifi.checked {
            Rgba::from_color(t.scheme.primary_container)
        } else {
            Rgba::from_color(t.scheme.tertiary_container)
        };
        let target_fg = if self.wifi.checked {
            Rgba::from_color(t.scheme.on_primary_container)
        } else {
            Rgba::from_color(t.scheme.on_tertiary_container)
        };
        let status_bg = ctx
            .anim
            .color("status-pill-bg", target_bg, Spring::EFFECTS_FAST);
        let status_fg = ctx
            .anim
            .color("status-pill-fg", target_fg, Spring::EFFECTS_FAST);

        let clock = row()
            .px(14.0)
            .gap(10.0)
            .items_center()
            .child(text(
                "21:41",
                FontSpec::display(24.0).weight(600),
                Rgba::from_color(t.scheme.on_surface),
            ))
            .child(text(
                "Sun 22 Nov",
                FontSpec::ui(13.0).weight(500),
                Rgba::from_color(t.scheme.on_surface_variant),
            ));

        let status_pill = row()
            .id("status-pill")
            .h(44.0)
            .px(16.0)
            .gap(12.0)
            .items_center()
            .rounded(22.0)
            .smoothing(0.0)
            .bg(status_bg)
            .hover_bg(status_bg.over(status_fg, 0.08))
            .on_click(|| Msg::ToggleLauncher)
            .child(icon(
                if self.wifi.checked {
                    Icon::Wifi
                } else {
                    Icon::WifiOff
                },
                IconStyle::Outlined,
                20.0,
                status_fg,
            ))
            .child(icon(
                if self.volume > 0.01 {
                    Icon::VolumeUp
                } else {
                    Icon::VolumeOff
                },
                IconStyle::Outlined,
                20.0,
                status_fg,
            ))
            .child(
                row()
                    .gap(4.0)
                    .items_center()
                    .child(icon(
                        Icon::Battery5Bar,
                        IconStyle::Outlined,
                        20.0,
                        status_fg,
                    ))
                    .child(text("82%", FontSpec::ui(14.0).weight(600), status_fg)),
            );

        let bell = icon_button(
            t,
            ctx.interaction,
            ctx.anim,
            IconButton {
                key: "btn-dnd".into(),
                icon: if self.dnd.checked {
                    Icon::NotificationsOff
                } else {
                    Icon::Notifications
                },
                toggled: self.dnd.checked,
                tone: ButtonTone::Veil,
                on_click: Some(Box::new(|| Msg::ToggleDnd)),
                ..Default::default()
            },
        );

        let bar = island(t).child(clock).child(status_pill).child(bell);
        box_()
            .p(DropShadow::from(ShadowToken::FLOATING).gutter())
            .child(bar)
    }

    fn view_launcher(&self, ctx: &mut ViewContext<'_>) -> Element<Msg> {
        let t = &self.theme;
        let prog = self.launcher_anim.value(ctx.now);
        let tf = SubtreeTransform {
            translate_x: 0.0,
            translate_y: 14.0 * (1.0 - prog),
            scale: 0.94 + 0.06 * prog,
            opacity: prog.clamp(0.0, 1.0),
        };

        let search_field = text_input(
            "launcher-search",
            &self.search,
            "Search apps, settings, commands...",
            Some(Icon::Search),
            INNER_RADIUS, // Concentric: 28px panel - 14px pad = 14px!
            t,
            ctx.interaction,
            ctx.text,
            Msg::SearchEvent,
        );

        let header = row()
            .w_full()
            .gap(12.0)
            .items_center()
            .child(box_().flex_1().child(search_field))
            .child(icon_button(
                t,
                ctx.interaction,
                ctx.anim,
                IconButton {
                    key: "btn-close-launcher".into(),
                    icon: Icon::Close,
                    tone: ButtonTone::Veil,
                    on_click: Some(Box::new(|| Msg::CloseLauncher)),
                    ..Default::default()
                },
            ));

        let footer = row()
            .w_full()
            .pt_top_border()
            .gap(16.0)
            .items_center()
            .child(kbd_hint("Enter", "Open", t))
            .child(kbd_hint("Scroll", "Browse list", t))
            .child(kbd_hint("Esc", "Close", t))
            .child(box_().flex_1())
            .child(text(
                format!(
                    "triton-engine · {:.2}× · concentric {}/{}px",
                    ctx.scale,
                    PANEL_RADIUS as i32,
                    concentric_inner(PANEL_RADIUS, PANEL_PAD) as i32,
                ),
                FontSpec::mono(11.5),
                Rgba::from_color(t.scheme.on_surface_variant).with_alpha(0.75),
            ));

        let card = panel(Material::Thick, PANEL_RADIUS, t)
            .w(840.0)
            .p(PANEL_PAD)
            .gap(14.0)
            .transform(tf)
            .child(header)
            .child(box_().w_full().h(1.0).bg(Rgba::new(1.0, 1.0, 1.0, 0.10)))
            .child(
                row()
                    .w_full()
                    .gap(14.0)
                    .child(self.view_app_list(ctx.interaction, ctx.anim))
                    .child(self.view_quick_controls(ctx)),
            )
            .child(box_().w_full().h(1.0).bg(Rgba::new(1.0, 1.0, 1.0, 0.10)))
            .child(footer);

        // Transparent shadow gutter around the Thick panel — no broken fullscreen dark scrim!
        box_()
            .p(DropShadow::from(ShadowToken::FLOATING).gutter())
            .child(card)
    }

    fn view_app_list(
        &self,
        interaction: &triton_engine::InteractionState,
        anim: &mut triton_engine::Animator,
    ) -> Element<Msg> {
        let t = &self.theme;
        let filtered = self.filtered_apps();
        let list_count = filtered.len().max(1);
        let list_items: Vec<Element<Msg>> = if filtered.is_empty() {
            vec![
                box_()
                    .w_full()
                    .h(80.0)
                    .items_center()
                    .justify_center()
                    .child(text(
                        "No matching applications",
                        TypeStyle::BODY_M,
                        Rgba::from_color(t.scheme.on_surface_variant),
                    )),
            ]
        } else {
            filtered
                .iter()
                .enumerate()
                .map(|(row_idx, &app_idx)| self.view_app_row(row_idx, app_idx, list_count))
                .collect()
        };

        col()
            .flex_1()
            .gap(8.0)
            .child(text(
                format!("APPLICATIONS ({})", filtered.len()),
                TypeStyle::LABEL_S,
                Rgba::from_color(t.scheme.on_surface_variant),
            ))
            .child(scroll_area(
                "launcher-scroll",
                336.0,
                ScrollAreaOptions {
                    gap: 3.0,
                    padding: Edges::new(0.0, 10.0, 0.0, 0.0),
                    edge_fade: Some(Edges::xy(0.0, 20.0)),
                },
                t,
                interaction,
                anim,
                list_items,
            ))
    }

    fn view_app_row(&self, row_idx: usize, app_idx: usize, list_count: usize) -> Element<Msg> {
        let t = &self.theme;
        let app = &APPS[app_idx];
        let selected = row_idx == self.selected_app;
        let row_bg = if selected {
            Rgba::from_color(t.scheme.secondary_container)
        } else {
            t.veil()
        };
        let title_col = if selected {
            Rgba::from_color(t.scheme.on_secondary_container)
        } else {
            Rgba::from_color(t.scheme.on_surface)
        };
        let sub_col = if selected {
            Rgba::from_color(t.scheme.on_secondary_container).with_alpha(0.82)
        } else {
            Rgba::from_color(t.scheme.on_surface_variant)
        };

        let mut r = row()
            .id(NodeKey::from_pair("app-row", app_idx))
            .w_full()
            .min_h(58.0)
            .px(14.0)
            .py(8.0)
            .gap(14.0)
            .items_center()
            .rounded(Corners(list_corners(row_idx, list_count)))
            .smoothing(CORNER_SMOOTHING)
            .bg(row_bg)
            .hover_bg(row_bg.over(Rgba::WHITE, 0.08))
            .press_bg(row_bg.over(Rgba::WHITE, 0.14))
            .on_click(move || Msg::LaunchApp(app_idx))
            .child(
                box_()
                    .size(38.0, 38.0)
                    .items_center()
                    .justify_center()
                    .rounded(19.0)
                    .smoothing(0.0)
                    .bg(Rgba::from(app.badge_rgba))
                    .child(icon(
                        app.icon,
                        IconStyle::Filled,
                        20.0,
                        Rgba::from_color(t.scheme.on_surface),
                    )),
            )
            .child(
                col()
                    .flex_1()
                    .gap(2.0)
                    .child(text(app.name, FontSpec::ui(15.0).weight(600), title_col))
                    .child(text(app.subtitle, FontSpec::ui(12.5), sub_col)),
            );

        if let Some(tag) = app.tag {
            r = r.child(
                box_()
                    .px(10.0)
                    .py(4.0)
                    .rounded(10.0)
                    .bg(Rgba::from_color(t.scheme.primary_container))
                    .child(text(
                        tag,
                        FontSpec::ui(11.5).weight(600),
                        Rgba::from_color(t.scheme.on_primary_container),
                    )),
            );
        }
        r
    }

    fn view_quick_controls(&self, ctx: &mut ViewContext<'_>) -> Element<Msg> {
        let now = ctx.now;
        let t = &self.theme;
        col()
            .w(320.0)
            .p(16.0)
            .gap(14.0)
            .rounded(INNER_RADIUS)
            .smoothing(CORNER_SMOOTHING)
            .bg(Rgba::new(1.0, 1.0, 1.0, 0.06))
            .border(1.0, Rgba::new(1.0, 1.0, 1.0, 0.08))
            .child(
                row()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .child(text(
                        "Quick Controls",
                        FontSpec::display(20.0).weight(600),
                        Rgba::from_color(t.scheme.on_surface),
                    ))
                    .child(text(
                        "Thick Glass",
                        FontSpec::mono(11.5),
                        Rgba::from_color(t.scheme.primary),
                    )),
            )
            .child(self.slider_row(
                "slider-brightness",
                Icon::LightMode,
                "Brightness",
                self.brightness,
                Msg::SetBrightness,
            ))
            .child(self.slider_row(
                "slider-volume",
                Icon::VolumeUp,
                "Volume",
                self.volume,
                Msg::SetVolume,
            ))
            .child(box_().w_full().h(1.0).bg(Rgba::new(1.0, 1.0, 1.0, 0.10)))
            .child(self.switch_row(
                "sw-wifi",
                Icon::Wifi,
                "Wi-Fi",
                if self.wifi.checked {
                    &self.status_note
                } else {
                    "Disconnected"
                },
                &self.wifi,
                now,
                Msg::ToggleWifi,
            ))
            .child(self.switch_row(
                "sw-night",
                Icon::Bedtime,
                "Night Light",
                if self.night_light.checked {
                    "Warm · Until 06:30"
                } else {
                    "Off"
                },
                &self.night_light,
                now,
                Msg::ToggleNightLight,
            ))
            .child(
                row().w_full().gap(8.0).child(
                    box_().flex_1().child(
                        button(
                            t,
                            ctx.interaction,
                            ctx.anim,
                            Button {
                                key: "btn-settings".into(),
                                label: "All Settings".into(),
                                lead_icon: Some(Icon::Tune),
                                tone: ButtonTone::Tonal,
                                on_click: Some(Box::new(|| Msg::CloseLauncher)),
                                ..Default::default()
                            },
                        )
                        .w_full(),
                    ),
                ),
            )
    }

    fn slider_row(
        &self,
        key: &'static str,
        ic: Icon,
        label: &str,
        value: f32,
        on_change: impl Fn(f32) -> Msg + 'static,
    ) -> Element<Msg> {
        let t = &self.theme;
        col()
            .w_full()
            .gap(4.0)
            .child(
                row()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .child(
                        row()
                            .gap(8.0)
                            .items_center()
                            .child(icon(
                                ic,
                                IconStyle::Outlined,
                                18.0,
                                Rgba::from_color(t.scheme.on_surface_variant),
                            ))
                            .child(text(
                                label,
                                FontSpec::ui(13.5).weight(500),
                                Rgba::from_color(t.scheme.on_surface),
                            )),
                    )
                    .child(text(
                        format!("{}%", (value * 100.0).round() as i32),
                        FontSpec::mono(12.5),
                        Rgba::from_color(t.scheme.on_surface_variant),
                    )),
            )
            .child(slider(key, value, 288.0, t, on_change))
    }

    #[allow(clippy::too_many_arguments)]
    fn switch_row(
        &self,
        key: &'static str,
        ic: Icon,
        title: &str,
        sub: &str,
        state: &SwitchState,
        now: Instant,
        msg: Msg,
    ) -> Element<Msg> {
        let t = &self.theme;
        row()
            .w_full()
            .items_center()
            .gap(10.0)
            .child(icon(
                ic,
                if state.checked {
                    IconStyle::Filled
                } else {
                    IconStyle::Outlined
                },
                20.0,
                if state.checked {
                    Rgba::from_color(t.scheme.primary)
                } else {
                    Rgba::from_color(t.scheme.on_surface_variant)
                },
            ))
            .child(
                col()
                    .flex_1()
                    .gap(1.0)
                    .child(text(
                        title,
                        FontSpec::ui(14.0).weight(600),
                        Rgba::from_color(t.scheme.on_surface),
                    ))
                    .child(text(
                        sub,
                        FontSpec::ui(12.0),
                        Rgba::from_color(t.scheme.on_surface_variant),
                    )),
            )
            .child(switch(key, state, now, t, move || msg.clone()))
    }
}

trait ElementExt {
    fn pt_top_border(self) -> Self;
}

impl<M> ElementExt for Element<M> {
    fn pt_top_border(self) -> Self {
        self.px(4.0)
    }
}

fn kbd_hint<M>(key_label: &str, desc: &str, theme: &Theme) -> Element<M> {
    row()
        .gap(6.0)
        .items_center()
        .child(
            box_()
                .h(22.0)
                .px(7.0)
                .items_center()
                .justify_center()
                .rounded(6.0)
                .border(1.0, Rgba::new(1.0, 1.0, 1.0, 0.28))
                .child(text(
                    key_label,
                    FontSpec::mono(11.0).weight(600),
                    Rgba::from_color(theme.scheme.on_surface),
                )),
        )
        .child(text(
            desc,
            FontSpec::ui(12.5),
            Rgba::from_color(theme.scheme.on_surface_variant),
        ))
}

impl ShellApp for ShowcaseApp {
    type SurfaceId = SurfaceId;
    type Message = Msg;

    fn surfaces(&self) -> Vec<(SurfaceId, SurfaceSpec)> {
        let now = Instant::now();
        let mut out = vec![
            (
                SurfaceId::LeftIsland,
                SurfaceSpec::island(
                    "triton-bar-left",
                    SurfaceAnchor::TOP | SurfaceAnchor::LEFT,
                    (4, 0, 0, 0),
                ),
            ),
            (
                SurfaceId::RightIsland,
                SurfaceSpec::island(
                    "triton-bar-right",
                    SurfaceAnchor::TOP | SurfaceAnchor::RIGHT,
                    (4, 0, 0, 0),
                ),
            ),
        ];

        if self.launcher_visible(now) {
            // Centered floating Thick-glass overlay: zero fullscreen scrim, crisp shaped blur!
            out.push((
                SurfaceId::Launcher,
                SurfaceSpec {
                    namespace: "triton-launcher".into(),
                    layer: SurfaceLayer::Overlay,
                    anchor: SurfaceAnchor::empty(), // centered by layer-shell when unanchored
                    margin: (0, 0, 0, 0),
                    keyboard: SurfaceKeyboard::Exclusive,
                    exclusive_zone: -1,
                    sizing: SurfaceSize::FitContent {
                        max_w: Some(1100),
                        max_h: Some(820),
                    },
                    shaped_input: true,
                },
            ));
        }

        out
    }

    fn view(&self, id: SurfaceId, ctx: &mut ViewContext<'_>) -> Element<Msg> {
        match id {
            SurfaceId::LeftIsland => self.view_left_island(ctx),
            SurfaceId::RightIsland => self.view_right_island(ctx),
            SurfaceId::Launcher => self.view_launcher(ctx),
        }
    }

    fn update(&mut self, _id: SurfaceId, msg: Msg, now: Instant) -> Vec<AppCommand<SurfaceId>> {
        match msg {
            Msg::ToggleLauncher => {
                let mut cmds = if self.launcher_open {
                    self.close_launcher(now)
                } else {
                    self.open_launcher(now)
                };
                cmds.push(AppCommand::Redraw(SurfaceId::LeftIsland));
                cmds
            }
            Msg::CloseLauncher => {
                let mut cmds = self.close_launcher(now);
                cmds.push(AppCommand::Redraw(SurfaceId::LeftIsland));
                cmds
            }
            Msg::SelectWorkspace(i) => {
                self.select_workspace(i);
                vec![AppCommand::Redraw(SurfaceId::LeftIsland)]
            }
            Msg::SearchEvent(ev) => {
                if let InputEvent::KeyDown { key, .. } = &ev {
                    match key {
                        Key::Down => {
                            let len = self.filtered_apps().len();
                            if len > 0 {
                                self.selected_app = (self.selected_app + 1) % len;
                            }
                            return vec![AppCommand::Redraw(SurfaceId::Launcher)];
                        }
                        Key::Up => {
                            let len = self.filtered_apps().len();
                            if len > 0 {
                                self.selected_app = (self.selected_app + len - 1) % len;
                            }
                            return vec![AppCommand::Redraw(SurfaceId::Launcher)];
                        }
                        _ => {}
                    }
                }
                match self.search.apply_key(&ev) {
                    Some(TextInputAction::Escape) => self.close_launcher(now),
                    Some(TextInputAction::Submit) => {
                        if let Some(&idx) = self.filtered_apps().get(self.selected_app) {
                            tracing::info!(app = APPS[idx].name, "launching selected app");
                        }
                        self.close_launcher(now)
                    }
                    Some(_) => {
                        self.selected_app = 0;
                        vec![AppCommand::Redraw(SurfaceId::Launcher)]
                    }
                    None => Vec::new(),
                }
            }
            Msg::LaunchApp(idx) => {
                tracing::info!(app = APPS[idx].name, "launched app");
                self.close_launcher(now)
            }
            Msg::SetBrightness(v) => {
                self.brightness = v;
                vec![AppCommand::Redraw(SurfaceId::Launcher)]
            }
            Msg::SetVolume(v) => {
                self.volume = v;
                vec![
                    AppCommand::Redraw(SurfaceId::Launcher),
                    AppCommand::Redraw(SurfaceId::RightIsland),
                ]
            }
            Msg::ToggleWifi => {
                self.wifi.toggle(now);
                vec![
                    AppCommand::Redraw(SurfaceId::Launcher),
                    AppCommand::Redraw(SurfaceId::RightIsland),
                ]
            }
            Msg::ToggleNightLight => {
                self.night_light.toggle(now);
                vec![AppCommand::Redraw(SurfaceId::Launcher)]
            }
            Msg::ToggleDnd => {
                self.dnd.toggle(now);
                vec![AppCommand::Redraw(SurfaceId::RightIsland)]
            }
        }
    }

    fn is_animating(&self, id: SurfaceId, now: Instant) -> bool {
        match id {
            SurfaceId::LeftIsland => false,
            SurfaceId::RightIsland => self.dnd.is_animating(now),
            SurfaceId::Launcher => {
                !self.launcher_anim.is_settled(now)
                    || self.wifi.is_animating(now)
                    || self.night_light.is_animating(now)
            }
        }
    }

    fn on_start(&mut self, _now: Instant) -> Vec<AppCommand<SurfaceId>> {
        let mut cmds = Vec::new();
        if self.auto_open {
            cmds.push(AppCommand::ScheduleTimer {
                tag: TIMER_SELECT_WS,
                delay: Duration::from_millis(450),
            });
            cmds.push(AppCommand::ScheduleTimer {
                tag: TIMER_OPEN_PANEL,
                delay: Duration::from_millis(900),
            });
            cmds.push(AppCommand::ScheduleTimer {
                tag: TIMER_TOGGLE_SWITCH,
                delay: Duration::from_millis(1800),
            });
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
                self.select_workspace(1);
                vec![AppCommand::Redraw(SurfaceId::LeftIsland)]
            }
            TIMER_OPEN_PANEL => {
                let mut cmds = self.open_launcher(now);
                cmds.push(AppCommand::Redraw(SurfaceId::LeftIsland));
                cmds
            }
            TIMER_TOGGLE_SWITCH => {
                self.night_light.toggle(now);
                vec![AppCommand::Redraw(SurfaceId::Launcher)]
            }
            TIMER_CARET_BLINK => {
                if self.launcher_open {
                    self.search.caret_visible = !self.search.caret_visible;
                    vec![
                        AppCommand::Redraw(SurfaceId::Launcher),
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

    fn on_exit(&mut self, metrics: &HashMap<SurfaceId, SurfaceMetrics>) {
        let mut out = String::from("triton-engine showcase report\n");
        for id in [
            SurfaceId::LeftIsland,
            SurfaceId::RightIsland,
            SurfaceId::Launcher,
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

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("triton_engine=info,info")),
        )
        .init();

    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        eprintln!(
            "triton-engine showcase: WAYLAND_DISPLAY is not set — run inside the preview container!"
        );
        std::process::exit(2);
    }

    let app = ShowcaseApp::new(Instant::now());
    if let Err(err) = run(app) {
        tracing::error!(%err, "showcase exited with error");
        std::process::exit(1);
    }
}
