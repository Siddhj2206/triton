//! Spike 1 (docs/roadmap.md, ADR-0005 and ADR-0007): what a large animated Thick
//! Glass surface costs, per renderer.
//!
//! One overlay surface (full screen, or the panel's own size) runs four scenarios back to back, each kept in
//! motion by a spring that is retargeted before it settles:
//!
//! - `grow`: a Thick panel springs between 60% and 100% of the output (an opening
//!   panel; nearly every pixel is damaged every frame).
//! - `slide`: a full-size panel's content slides 240 px (scrolling, a page change).
//! - `select`: a still full-size panel; a selection moves across a tile grid (the
//!   damage is two tiles).
//! - `sized`: a launcher-sized panel (720 × 540) grows 92% to 100% (surfaces sized to
//!   their content).
//!
//! For each it reports the frame intervals and the CPU time per frame (the render
//! thread, from /proc/self/schedstat, and the whole process, from /proc/self/stat),
//! and memory (`smaps_rollup`, and the GPU memory in DRM `fdinfo`) before and after.
//!
//! Run it in the preview, never against the real session. `TRITON_CMD` is split on
//! spaces, so settings go in through `env`:
//!
//! ```sh
//! cargo build --release --example spike_glass
//! TRITON_CMD="env WGPU_BACKEND=vulkan SPIKE_LABEL=vulkan SPIKE_SURFACE=panel \
//!   /work/target/release/examples/spike_glass" mise run preview
//! cat target/preview/spike-glass.txt
//! ```
//!
//! `ICED_BACKEND=tiny-skia` selects the software renderer, `WGPU_BACKEND=gl` wgpu's
//! GL backend, and `SPIKE_SURFACE=full` (the default) a full-screen surface.

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::layout::{self, Layout};
use iced_core::widget::{Tree, Widget};
use iced_core::window::Id;
use iced_core::{Color, Length, Padding, Rectangle, Size, mouse, renderer};
use iced_layershell::reexport::{
    Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings, OutputOption, Task,
};
use iced_layershell::settings::{LayerShellSettings, Settings, StartMode};
use iced_layershell::to_layer_message;
use iced_widget::{Column, Row, Space, Stack, responsive};
use triton_design::motion::{Animated, Motion, Spring};
use triton_design::{Icon, Material, TypeStyle};
use triton_ui::field::{Field, search};
use triton_ui::surface::{icon_disc, surface};
use triton_ui::text::text;
use triton_ui::{Element, Renderer, Surface, Theme};

const SCENARIOS: [Scenario; 4] = [
    Scenario::Grow,
    Scenario::Slide,
    Scenario::Select,
    Scenario::Sized,
];
const RUN: Duration = Duration::from_secs(6);
/// Retarget before the spring settles, so every frame has motion.
const RETARGET: Duration = Duration::from_millis(450);
const COLUMNS: usize = 8;
const ROWS: usize = 4;
const TILE: f32 = 132.0;
const ICONS: [Icon; 8] = [
    Icon::Apps,
    Icon::Bluetooth,
    Icon::Calculate,
    Icon::Bolt,
    Icon::Bedtime,
    Icon::BlurOn,
    Icon::Animation,
    Icon::Add,
];

#[derive(Debug, Clone, Copy, PartialEq)]
enum Scenario {
    Grow,
    Slide,
    Select,
    Sized,
}

impl Scenario {
    fn name(self) -> &'static str {
        match self {
            Scenario::Grow => "grow",
            Scenario::Slide => "slide",
            Scenario::Select => "select",
            Scenario::Sized => "sized",
        }
    }

    /// The two ends the spring swings between.
    fn range(self) -> (f32, f32) {
        match self {
            Scenario::Grow => (0.6, 1.0),
            Scenario::Slide => (0.0, 240.0),
            Scenario::Select => (0.0, (COLUMNS - 1) as f32),
            Scenario::Sized => (0.92, 1.0),
        }
    }
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
enum Message {
    Frame(Instant),
}

/// CPU time so far: the main (render) thread in ns, the whole process in ticks.
fn cpu() -> (u64, u64) {
    let thread = std::fs::read_to_string("/proc/self/schedstat")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse().ok())
        .unwrap_or(0);
    // Fields 14 and 15 (utime, stime) follow the parenthesised command name.
    let process = std::fs::read_to_string("/proc/self/stat")
        .ok()
        .and_then(|s| {
            let rest = s.rsplit_once(')')?.1;
            let f: Vec<&str> = rest.split_whitespace().collect();
            Some(f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?)
        })
        .unwrap_or(0);
    (thread, process)
}

/// Memory now, in MiB: resident, proportional (shared pages split between their
/// users), private, and GPU buffers the kernel driver charges to this process.
fn memory() -> String {
    let rollup = std::fs::read_to_string("/proc/self/smaps_rollup").unwrap_or_default();
    let kib = |key: &str| -> u64 {
        rollup
            .lines()
            .filter(|l| l.starts_with(key))
            .filter_map(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
            .sum()
    };
    let mib = |k: u64| k as f64 / 1024.0;
    format!(
        "rss {:.1}  pss {:.1}  private {:.1}  gpu {}",
        mib(kib("Rss:")),
        mib(kib("Pss:")),
        mib(kib("Private_Clean:") + kib("Private_Dirty:")),
        gpu_memory(),
    )
}

/// The DRM fdinfo memory keys (amdgpu: vram, gtt, cpu), summed over this
/// process's GPU clients, counting each client once.
fn gpu_memory() -> String {
    let mut seen = std::collections::BTreeSet::new();
    let mut totals = std::collections::BTreeMap::<String, u64>::new();
    let Ok(dir) = std::fs::read_dir("/proc/self/fdinfo") else {
        return "-".into();
    };
    for entry in dir.flatten() {
        let Ok(info) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Some(client) = info.lines().find_map(|l| {
            l.strip_prefix("drm-client-id:")
                .map(|v| v.trim().to_owned())
        }) else {
            continue;
        };
        if !seen.insert(client) {
            continue;
        }
        for line in info.lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let Some(region) = key.strip_prefix("drm-memory-") else {
                continue;
            };
            let mut parts = value.split_whitespace();
            let (Some(n), unit) = (
                parts.next().and_then(|n| n.parse::<u64>().ok()),
                parts.next(),
            ) else {
                continue;
            };
            let kib = match unit {
                Some("MiB") => n * 1024,
                Some("KiB") => n,
                _ => n / 1024,
            };
            *totals.entry(region.to_owned()).or_default() += kib;
        }
    }
    if totals.is_empty() {
        return "none".into();
    }
    totals
        .iter()
        .map(|(k, v)| format!("{k} {:.1}", *v as f64 / 1024.0))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Clock ticks per second; 100 on every Linux target we run on.
const TICKS: f64 = 100.0;

#[derive(Debug)]
struct Run {
    scenario: Scenario,
    started: Instant,
    cpu: (u64, u64),
    retargeted: Instant,
    value: Animated,
    last: Option<Instant>,
    intervals: Vec<f64>,
}

impl Run {
    fn new(scenario: Scenario, now: Instant) -> Self {
        let (from, to) = scenario.range();
        let mut value = Animated::new(from, Motion::Spring(Spring::SPATIAL), 0.001, now);
        value.set_target(to, now);
        Self {
            scenario,
            started: now,
            cpu: cpu(),
            retargeted: now,
            value,
            last: None,
            intervals: Vec::new(),
        }
    }

    fn frame(&mut self, now: Instant) {
        if let Some(last) = self.last {
            self.intervals
                .push(now.saturating_duration_since(last).as_secs_f64() * 1000.0);
        }
        self.last = Some(now);
        if now.saturating_duration_since(self.retargeted) >= RETARGET {
            let (from, to) = self.scenario.range();
            let next = if (self.value.target() - to).abs() < f32::EPSILON {
                from
            } else {
                to
            };
            self.value.set_target(next, now);
            self.retargeted = now;
        }
    }

    fn report(&self, now: Instant) -> String {
        let (thread, process) = cpu();
        let frames = self.intervals.len().max(1) as f64;
        let wall = now.saturating_duration_since(self.started).as_secs_f64();
        let thread_ms = (thread - self.cpu.0) as f64 / 1e6 / frames;
        let process_ms = (process - self.cpu.1) as f64 / TICKS * 1000.0 / frames;
        let mut sorted = self.intervals.clone();
        sorted.sort_by(f64::total_cmp);
        let pct = |p: f64| {
            sorted
                .get(((sorted.len() as f64 - 1.0) * p).round() as usize)
                .copied()
                .unwrap_or(0.0)
        };
        let mean = sorted.iter().sum::<f64>() / frames;
        // A frame is late when it took more than one and a half refresh periods.
        let period = pct(0.5);
        let late = sorted.iter().filter(|&&i| i > period * 1.5).count();
        format!(
            "{:<7} frames {:>4} in {wall:.1} s ({:.1} fps)  interval ms: mean {mean:.2} \
             p50 {:.2} p95 {:.2} p99 {:.2} max {:.2}  late {late}  \
             cpu ms/frame: render thread {thread_ms:.2}, process {process_ms:.2}\n",
            self.scenario.name(),
            sorted.len(),
            frames / wall,
            pct(0.5),
            pct(0.95),
            pct(0.99),
            pct(1.0),
        )
    }
}

fn panel_surface() -> bool {
    std::env::var("SPIKE_SURFACE").is_ok_and(|v| v == "panel")
}

#[derive(Debug, Default)]
struct Spike {
    run: Option<Run>,
    next: usize,
    report: String,
}

impl Spike {
    fn boot() -> (Self, Task<Message>) {
        let mut report = String::from("spike 1: Thick Glass frame cost\n");
        if let Ok(threads) = std::thread::available_parallelism() {
            let _ = writeln!(report, "cpus: {threads}");
        }
        let _ = writeln!(
            report,
            "renderer: {}  surface: {}",
            std::env::var("SPIKE_LABEL").unwrap_or_else(|_| "?".into()),
            if panel_surface() {
                "720x540 panel"
            } else {
                "full screen"
            },
        );
        // SPIKE_SURFACE=panel: the surface is the panel's own size (ADR-0007's rule),
        // and the scenarios animate inside it.
        let (size, anchor) = if panel_surface() {
            (Some((720, 540)), Anchor::empty())
        } else {
            (
                Some((0, 0)),
                Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
            )
        };
        let open = Task::done(Message::NewLayerShell {
            settings: NewLayerShellSettings {
                size,
                layer: Layer::Overlay,
                anchor,
                exclusive_zone: Some(-1),
                margin: None,
                keyboard_interactivity: KeyboardInteractivity::None,
                output_option: OutputOption::OutputName("winit".into()),
                events_transparent: false,
                namespace: Some("triton-spike".into()),
            },
            id: Id::unique(),
        });
        (
            Self {
                report,
                ..Self::default()
            },
            open,
        )
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "the daemon's update signature"
    )]
    fn update(&mut self, message: Message) -> Task<Message> {
        let Message::Frame(now) = message else {
            return Task::none();
        };
        if self.run.is_none() {
            let Some(&scenario) = SCENARIOS.get(self.next) else {
                return Task::none();
            };
            if self.next == 0 {
                // The renderer is up and has drawn one frame, nothing has moved yet.
                let _ = writeln!(self.report, "memory after first frame: {}", memory());
            }
            self.next += 1;
            self.run = Some(Run::new(scenario, now));
        }
        let Some(run) = self.run.as_mut() else {
            return Task::none();
        };
        run.frame(now);
        if now.saturating_duration_since(run.started) >= RUN {
            let line = run.report(now);
            tracing::info!("{}", line.trim_end());
            self.report.push_str(&line);
            self.run = None;
            if self.next == SCENARIOS.len() {
                let _ = writeln!(self.report, "memory after the scenarios: {}", memory());
                let path = std::env::var("SPIKE_REPORT")
                    .unwrap_or_else(|_| "/preview/spike-glass.txt".into());
                if let Err(e) = std::fs::write(&path, &self.report) {
                    tracing::warn!(%e, path, "cannot write the report");
                }
                print!("{}", self.report);
            }
        }
        Task::none()
    }

    fn view(&self, _id: Id) -> Element<'_, Message> {
        let active = self.next < SCENARIOS.len() || self.run.is_some();
        let now = Instant::now();
        let (scenario, value) = match &self.run {
            Some(run) => (run.scenario, run.value.value(now)),
            None => (Scenario::Select, 0.0),
        };
        let content = responsive(move |size| panel(scenario, value, size));
        Ticker {
            content: content.into(),
            active,
        }
        .into()
    }
}

fn tile<'a>(i: usize) -> Element<'a, Message> {
    Column::new()
        .width(TILE)
        .height(TILE)
        .spacing(12)
        .align_x(Horizontal::Center)
        .push(Space::new().height(14))
        .push(icon_disc(ICONS[i % ICONS.len()]))
        .push(text(format!("App {}", i + 1), TypeStyle::LABEL_L))
        .into()
}

fn grid<'a>() -> Element<'a, Message> {
    let mut rows = Column::new().spacing(16);
    for r in 0..ROWS {
        let mut row = Row::new().spacing(16);
        for c in 0..COLUMNS {
            row = row.push(tile(r * COLUMNS + c));
        }
        rows = rows.push(row);
    }
    rows.into()
}

/// The selection: a tonal tile behind grid column `at` (fractional) of row 0.
fn selection<'a>(at: f32) -> Element<'a, Message> {
    let x = at * (TILE + 16.0);
    let highlight = surface(
        Space::new().width(TILE).height(TILE),
        Surface::ListRow {
            index: 0,
            count: 1,
            selected: true,
            on_card: false,
        },
    );
    Column::new()
        .push(Space::new().height(0))
        .push(Row::new().push(Space::new().width(x)).push(highlight))
        .into()
}

fn body<'a>(scenario: Scenario, value: f32) -> Element<'a, Message> {
    let field: Element<'a, Message> =
        search("Search apps, windows and files", "", Field::SearchOnGlass).into();
    let content: Element<'a, Message> = if scenario == Scenario::Select {
        Stack::new().push(selection(value)).push(grid()).into()
    } else {
        grid()
    };
    let offset = if scenario == Scenario::Slide {
        value
    } else {
        0.0
    };
    Column::new()
        .spacing(32)
        .padding(Padding {
            top: 48.0 + offset,
            ..Padding::from(48.0)
        })
        .push(text("Launcher", TypeStyle::HEADLINE_M))
        .push(Row::new().width(720).push(field))
        .push(content)
        .into()
}

fn panel<'a>(scenario: Scenario, value: f32, size: Size) -> Element<'a, Message> {
    let (w, h) = match scenario {
        Scenario::Grow => (size.width * value, size.height * value),
        Scenario::Slide | Scenario::Select => (size.width, size.height),
        Scenario::Sized => (720.0 * value, 540.0 * value),
    };
    // SPIKE_SHADOW=1 paints the Thick shadow in iced, as a plain floating panel does.
    let class = if std::env::var_os("SPIKE_SHADOW").is_some() {
        Surface::Panel(Material::Thick)
    } else {
        Surface::LayerPanel(Material::Thick)
    };
    let glass = surface(body(scenario, value), class).width(w).height(h);
    iced_widget::container(glass)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .into()
}

/// Asks for the next frame after every redraw while `active`. The frames reach the
/// app through `window::frames()`: a message a widget publishes during a redraw
/// waits in `iced_layershell` 0.19 until some other event wakes the loop.
struct Ticker<'a> {
    content: Element<'a, Message>,
    active: bool,
}

impl Widget<Message, Theme, Renderer> for Ticker<'_> {
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &iced_core::Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn iced_core::Clipboard,
        shell: &mut iced_core::Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if let iced_core::Event::Window(iced_core::window::Event::RedrawRequested(_)) = event
            && self.active
        {
            shell.request_redraw();
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }
}

impl<'a> From<Ticker<'a>> for Element<'a, Message> {
    fn from(t: Ticker<'a>) -> Self {
        iced_core::Element::new(t)
    }
}

fn main() -> Result<(), iced_layershell::Error> {
    // niri does not pass a child's output on, so the log goes next to the report.
    let log = std::env::var("SPIKE_LOG").unwrap_or_else(|_| "/preview/spike-glass.log".into());
    let filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    match std::fs::File::create(&log) {
        Ok(file) => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(file))
            .init(),
        Err(_) => tracing_subscriber::fmt().with_env_filter(filter).init(),
    }
    let base = triton_ui::settings();
    iced_layershell::daemon(Spike::boot, "triton-spike", Spike::update, Spike::view)
        .subscription(|_: &Spike| iced_runtime::window::frames().map(Message::Frame))
        .theme(|_: &Spike, _| Theme::dark())
        .style(|_: &Spike, theme: &Theme| iced_core::theme::Style {
            background_color: Color::TRANSPARENT,
            text_color: theme.scheme.on_surface,
        })
        .settings(Settings {
            fonts: base.fonts,
            default_font: base.default_font,
            default_text_size: base.default_text_size,
            layer_settings: LayerShellSettings {
                start_mode: StartMode::Background,
                ..LayerShellSettings::default()
            },
            ..Settings::default()
        })
        .run()
}
