//! Spike: does iced's own wgpu path antialias a canvas path on this driver?
//!
//! `spike_msaa` proved raw wgpu MSAA resolves fine on radv. This runs the same
//! test through iced's real pipeline: the fallback renderer's headless mode
//! (`iced_wgpu`, which hardcodes MSAA x4 for headless), a canvas `Path::circle`
//! filled exactly like the wave shape, drawn offscreen and read back through
//! `Headless::screenshot`. Edge coverage values tell whether the mesh's MSAA
//! offscreen + resolve + blit chain produces antialiasing.
//!
//! `ICED_BACKEND` selects the renderer (`wgpu`, `tiny-skia`, `wgpu,tiny-skia`).
//! `WGPU_BACKEND` selects the wgpu backend. Runs headless, so the preview
//! container works:
//!
//! ```sh
//! cargo build --release --example spike_wave
//! podman run --rm --device /dev/dri --security-opt label=disable \
//!   -v "$PWD:/work:ro" -v "$(readlink -f target):$(readlink -f target):ro" \
//!   -v "$PWD/target/preview:/preview" \
//!   -e ICED_BACKEND=wgpu -e WGPU_BACKEND=vulkan \
//!   localhost/triton-preview:latest /work/target/release/examples/spike_wave
//! ```

use iced_core::renderer::{self, Headless};
use iced_core::{Color, Length, Rectangle, Size, Theme, mouse};
use iced_futures::futures::executor::block_on;
use iced_runtime::UserInterface;
use iced_widget::canvas::{self, Frame, Geometry, Path};

type Renderer = iced_renderer::Renderer;

const LOGICAL: f32 = 240.0;
const SCALE: f32 = 1.75;

struct Filled;

impl canvas::Program<(), Theme, Renderer> for Filled {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry<Renderer>> {
        let mut frame = Frame::new(renderer, bounds.size());
        // A circle: its rim is tessellated triangles, so MSAA is what
        // antialiases it (same path as the wave shape).
        frame.fill(
            &Path::circle(bounds.center(), bounds.width.min(bounds.height) / 2.0),
            Color::WHITE,
        );
        vec![frame.into_geometry()]
    }
}

fn main() {
    let backend = std::env::var("ICED_BACKEND").ok();
    let Some(mut renderer) = block_on(Renderer::new(
        iced_core::Font::default(),
        iced_core::Pixels(16.0),
        backend.as_deref(),
    )) else {
        println!("no headless renderer for {backend:?}");
        return;
    };
    println!("renderer: {}", renderer.name());

    let element: iced_core::Element<'_, (), Theme, Renderer> = iced_widget::canvas(Filled)
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
    let mut ui = UserInterface::build(
        element,
        Size::new(LOGICAL, LOGICAL),
        iced_runtime::user_interface::Cache::default(),
        &mut renderer,
    );
    ui.draw(
        &mut renderer,
        &Theme::Dark,
        &renderer::Style::default(),
        mouse::Cursor::Unavailable,
    );

    let physical = Size::new((LOGICAL * SCALE) as u32, (LOGICAL * SCALE) as u32);
    let rgba = renderer.screenshot(physical, SCALE, Color::BLACK);

    // Middle row crosses the circle's rim once on each side.
    let w = physical.width as usize;
    let mid = physical.height as usize / 2;
    let value = |x: usize| rgba[(mid * w + x) * 4];
    let run: Vec<u8> = (w / 2 - 20..w / 2 + 20).map(&value).collect();
    let mut distinct: Vec<u8> = (0..w * physical.height as usize)
        .map(|i| rgba[i * 4])
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    distinct.sort_unstable();
    let left_edge: Vec<u8> = (20..60).map(value).collect();
    println!("scale {SCALE}, {w}x{}", physical.height);
    println!("distinct values: {distinct:?}");
    println!("left edge x=20..60: {left_edge:?}");
    println!("around center: {run:?}");
    let intermediate = distinct.iter().filter(|&&v| v > 4 && v < 250).count();
    println!(
        "verdict: {}",
        if intermediate > 0 {
            "antialiased edges"
        } else {
            "NO antialiasing"
        }
    );

    if let Ok(path) = std::env::var("SPIKE_PPM") {
        let mut out = format!("P6\n{} {}\n255\n", w, physical.height);
        out.push_str(&String::from_utf8_lossy(&[]));
        let mut bytes = out.into_bytes();
        for px in rgba.chunks(4) {
            bytes.extend_from_slice(&px[..3]);
        }
        std::fs::write(&path, bytes).expect("write ppm");
    }
}
