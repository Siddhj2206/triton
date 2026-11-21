//! Renders the Component index board reproduction (src/index.rs) on tiny-skia
//! and compares it with the committed snapshot. `mise run pixel` copies that
//! PNG to target/boards/ and diffs it against the browser render of the board.

use iced_test::simulator::Simulator;
use triton_ui::{Theme, index, settings};

#[test]
fn index_dark() {
    let mut ui = Simulator::with_size(
        settings(),
        (index::WIDTH, index::HEIGHT),
        index::view::<()>(),
    );
    let snap = ui.snapshot(&Theme::dark()).expect("render");
    let path = format!("{}/tests/snapshots/index-dark", env!("CARGO_MANIFEST_DIR"));
    assert!(
        snap.matches_image(&path).expect("compare"),
        "index-dark changed: delete the snapshot to re-bless"
    );
}
