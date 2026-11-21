//! Renders the component gallery on tiny-skia, in both schemes, and compares it
//! with the committed snapshot. Delete a snapshot to re-bless it, then look at it.

use iced_test::simulator::Simulator;
use triton_ui::{Theme, gallery, settings};

fn snapshot(theme: &Theme, name: &str) {
    let mut ui = Simulator::with_size(settings(), (1440.0, 1640.0), gallery::view(()));
    let snap = ui.snapshot(theme).expect("render");
    let path = format!("{}/tests/snapshots/{name}", env!("CARGO_MANIFEST_DIR"));
    assert!(
        snap.matches_image(&path).expect("compare"),
        "{name} changed: delete the snapshot to re-bless"
    );
}

#[test]
fn gallery_dark() {
    snapshot(&Theme::dark(), "gallery-dark");
}

#[test]
fn gallery_light() {
    snapshot(&Theme::light(), "gallery-light");
}
