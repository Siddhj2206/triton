//! Smoke tests for the Rune-style headless dev flags on `triton`
//! (`--snap`, `--tree`, `--script`) — Phase 1B.9.
//!
//! Everything runs offscreen inside an ephemeral `TempDir`: no Wayland, no
//! compositor, and never the host `~/.config` or live session.

use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;
use triton::{BarMode, CliCommand, HeadlessArgs, SurfaceRole, headless, parse_args};
use triton_design::color::Variant;

const PNG_MAGIC: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

#[test]
fn parses_headless_dev_flags() {
    assert_eq!(
        parse_args(["--tree", "out/bar.json", "--surface", "bar"]).unwrap(),
        CliCommand::Headless(HeadlessArgs {
            tree: Some(PathBuf::from("out/bar.json")),
            surfaces: vec![SurfaceRole::Bar],
            ..HeadlessArgs::default()
        })
    );

    assert_eq!(
        parse_args([
            "--snap",
            "snaps/",
            "--surface",
            "all",
            "--variant",
            "both",
            "--mode",
            "pinned",
            "--scale",
            "2.0",
            "--reduce-motion",
        ])
        .unwrap(),
        CliCommand::Headless(HeadlessArgs {
            snap: Some(PathBuf::from("snaps/")),
            surfaces: vec![SurfaceRole::Background, SurfaceRole::Bar, SurfaceRole::Osd],
            variants: vec![Variant::Dark, Variant::Light],
            bar_mode: Some(BarMode::Pinned),
            scale: 2.0,
            reduce_motion: true,
            ..HeadlessArgs::default()
        })
    );

    // `--flag=value` form works too.
    assert_eq!(
        parse_args(["--snap=out.png"]).unwrap(),
        CliCommand::Headless(HeadlessArgs {
            snap: Some(PathBuf::from("out.png")),
            ..HeadlessArgs::default()
        })
    );
}

#[test]
fn headless_flags_are_incompatible_with_daemon_flags() {
    // The headless runner always uses the fake dev backend; `--real-services`
    // or `--socket` would be silently ignored, so they are rejected loudly.
    assert!(parse_args(["--real-services", "--snap", "out.png"]).is_err());
    assert!(parse_args(["--socket", "/tmp/t.sock", "--tree", "t.json"]).is_err());
    // Headless-only modifiers without a snap/tree/script target are rejected.
    assert!(parse_args(["--surface", "bar"]).is_err());
}

#[test]
fn headless_tree_writes_valid_semantic_json() {
    let dir = TempDir::new().expect("temp dir");
    let tree_path = dir.path().join("bar.json");

    headless::run(&HeadlessArgs {
        tree: Some(tree_path.clone()),
        surfaces: vec![SurfaceRole::Bar],
        bar_mode: Some(BarMode::Pinned),
        ..HeadlessArgs::default()
    })
    .expect("headless --tree run");

    let text = fs::read_to_string(&tree_path).expect("tree JSON file written");
    let doc: serde_json::Value = serde_json::from_str(&text).expect("valid tree JSON");
    let nodes = doc
        .get("nodes")
        .and_then(serde_json::Value::as_array)
        .expect("tree JSON has a nodes array");
    assert!(!nodes.is_empty(), "semantic tree must contain nodes");
    assert!(doc.get("content_size").is_some());
}

#[test]
fn headless_snap_writes_non_empty_png() {
    let dir = TempDir::new().expect("temp dir");
    let png_path = dir.path().join("bar.png");

    headless::run(&HeadlessArgs {
        snap: Some(png_path.clone()),
        surfaces: vec![SurfaceRole::Bar],
        bar_mode: Some(BarMode::Pinned),
        ..HeadlessArgs::default()
    })
    .expect("headless --snap run");

    let bytes = fs::read(&png_path).expect("PNG file written");
    assert!(bytes.len() > PNG_MAGIC.len(), "PNG must be non-empty");
    assert_eq!(bytes[..PNG_MAGIC.len()], PNG_MAGIC, "PNG magic header");
}

#[test]
fn headless_snap_covers_bar_osd_and_wallpaper() {
    let dir = TempDir::new().expect("temp dir");

    let report = headless::run(&HeadlessArgs {
        snap: Some(dir.path().to_path_buf()),
        surfaces: vec![SurfaceRole::Background, SurfaceRole::Bar, SurfaceRole::Osd],
        ..HeadlessArgs::default()
    })
    .expect("headless --snap run");

    for role in [SurfaceRole::Background, SurfaceRole::Bar, SurfaceRole::Osd] {
        let path = dir.path().join(format!("{}-dark.png", role.as_str()));
        let bytes = fs::read(&path).unwrap_or_else(|_| panic!("{path:?} written"));
        assert!(bytes.len() > PNG_MAGIC.len());
        assert_eq!(bytes[..PNG_MAGIC.len()], PNG_MAGIC);
    }
    assert_eq!(report.snapshots.len(), 3);
}

#[test]
fn headless_script_replays_input_events() {
    let dir = TempDir::new().expect("temp dir");
    let script_path = dir.path().join("script.json");
    fs::write(
        &script_path,
        r#"{"events": [
            {"t": 0.05, "type": "tap", "key": "bar-ws-eDP-1-2"},
            {"t": 0.40, "type": "mark", "name": "after-tap"}
        ]}"#,
    )
    .expect("write script");
    let tree_path = dir.path().join("bar.json");

    let report = headless::run(&HeadlessArgs {
        script: Some(script_path),
        tree: Some(tree_path.clone()),
        surfaces: vec![SurfaceRole::Bar],
        bar_mode: Some(BarMode::Pinned),
        ..HeadlessArgs::default()
    })
    .expect("headless --script run");

    assert_eq!(report.events_dispatched, 1);
    assert_eq!(report.marks.get("after-tap"), Some(&0.40));

    let text = fs::read_to_string(&tree_path).expect("tree JSON file written");
    let doc: serde_json::Value = serde_json::from_str(&text).expect("valid tree JSON");
    assert!(doc.get("nodes").is_some());
}
