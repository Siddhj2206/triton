//! Renders the wave shape on whatever `ICED_TEST_BACKEND` selects and writes the
//! snapshot next to the given output path, for inspecting edge antialiasing per
//! backend (`wgpu` in the preview container exercises the MSAA resolve path).

use iced_test::simulator::Simulator;
use triton_ui::progress::{WaveTone, wave_shape};
use triton_ui::{Theme, settings};

#[test]
fn wave_edge() {
    let mut ui = Simulator::with_size(
        settings(),
        (120.0, 120.0),
        wave_shape::<()>(120.0, WaveTone::Primary),
    );
    let snap = ui.snapshot(&Theme::dark()).expect("render");
    let path = std::env::var("WAVE_SNAPSHOT").unwrap_or_else(|_| {
        format!(
            "{}/wave-edge",
            std::env::var("CARGO_TARGET_TMPDIR").unwrap_or_else(|_| "/tmp".into())
        )
    });
    snap.matches_image(path).expect("compare");
}
