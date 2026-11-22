//! Integration tests for `triton::niri_config` — the owned niri config writer
//! (ADR-0004). Every test targets an ephemeral `TempDir`; nothing here may
//! touch the host's `~/.config/niri`.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::process::Command;

use triton::niri_config::{self, INCLUDE_LINE};
use triton::state::BarMode;
use triton_design::color::{Scheme, Variant};

fn owned_dir(config_dir: &Path) -> std::path::PathBuf {
    config_dir.join(niri_config::OWNED_DIR)
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).expect("read generated file")
}

/// Extract the text of the `layer-rule { ... }` block matching `namespace`.
fn layer_rule_block<'a>(doc: &'a str, namespace: &str) -> &'a str {
    let needle = format!("namespace=\"^{namespace}$\"");
    let start = doc
        .find(&needle)
        .unwrap_or_else(|| panic!("no layer-rule for {namespace}"));
    // Walk backwards to the enclosing `layer-rule` opener.
    let rule_start = doc[..start]
        .rfind("layer-rule")
        .expect("layer-rule opener before match");
    // The block ends at the first `\n}\n` after the match.
    let rest = &doc[rule_start..];
    let end = rest.find("\n}\n").map(|i| i + "\n}\n".len());
    match end {
        Some(end) => &rest[..end],
        None => &rest[start - rule_start..],
    }
}

#[test]
fn install_writes_all_owned_files_and_appends_include_line() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let config_dir = tmp.path().join("niri");

    niri_config::install(&config_dir, &Scheme::DARK, Variant::Dark, BarMode::Autohide)
        .expect("install");

    let owned = owned_dir(&config_dir);
    for name in ["index.kdl", "binds.kdl", "colors.kdl", "layer-rules.kdl"] {
        assert!(
            owned.join(name).is_file(),
            "expected owned file {name} under {}",
            owned.display()
        );
        assert!(
            read(&owned.join(name)).ends_with('\n'),
            "{name} ends with a newline"
        );
    }

    let config = read(&config_dir.join("config.kdl"));
    assert_eq!(
        config.matches(INCLUDE_LINE).count(),
        1,
        "config.kdl contains exactly one triton include line:\n{config}"
    );
    assert!(
        config.trim_end().ends_with(INCLUDE_LINE),
        "include line is appended at the end:\n{config}"
    );
}

#[test]
fn install_is_idempotent_and_preserves_user_content() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let config_dir = tmp.path().join("niri");
    fs::create_dir_all(&config_dir).unwrap();

    let user_config = "\
// My hand-written niri config. Do not touch!

input {
    keyboard {
        repeat-delay 300 // tuned by hand
    }
}

/* block comment mentioning triton/index.kdl must not count */
";
    fs::write(config_dir.join("config.kdl"), user_config).unwrap();

    for round in 0..3 {
        niri_config::install(&config_dir, &Scheme::DARK, Variant::Dark, BarMode::Autohide)
            .unwrap_or_else(|err| panic!("install round {round}: {err}"));
    }

    let config = read(&config_dir.join("config.kdl"));
    assert!(
        config.starts_with(user_config),
        "user content preserved verbatim:\n{config}"
    );
    assert_eq!(
        config.matches("triton/index.kdl").count(),
        2, // one in the block comment above, one appended line
        "exactly one active include line appended:\n{config}"
    );
    assert_eq!(config.matches(INCLUDE_LINE).count(), 1);
}

#[test]
fn install_skips_append_when_user_already_includes_index() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let config_dir = tmp.path().join("niri");
    fs::create_dir_all(&config_dir).unwrap();

    // A user who already includes our index (any spelling) must not get a
    // second line — `install` appends at most once, ever.
    let config_before = "// my config\ninclude \"triton/index.kdl\"\n";
    fs::write(config_dir.join("config.kdl"), config_before).unwrap();

    niri_config::install(&config_dir, &Scheme::DARK, Variant::Dark, BarMode::Autohide)
        .expect("install");

    let config = read(&config_dir.join("config.kdl"));
    assert_eq!(
        config.matches("triton/index.kdl").count(),
        1,
        "no duplicate include:\n{config}"
    );
}

#[test]
fn install_respects_commented_out_include() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let config_dir = tmp.path().join("niri");
    fs::create_dir_all(&config_dir).unwrap();

    // A user who disabled Triton by commenting the include line keeps it
    // disabled; `install` does not fight them by appending another line.
    let config_before = "// include optional=true \"triton/index.kdl\" (disabled for now)\n";
    fs::write(config_dir.join("config.kdl"), config_before).unwrap();

    niri_config::install(&config_dir, &Scheme::DARK, Variant::Dark, BarMode::Autohide)
        .expect("install");

    let config = read(&config_dir.join("config.kdl"));
    assert_eq!(
        config, config_before,
        "commented include line left untouched"
    );
}

#[test]
fn install_appends_include_on_fresh_line_without_trailing_newline() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let config_dir = tmp.path().join("niri");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.kdl"),
        "prefer-no-csd // no trailing newline",
    )
    .unwrap();

    niri_config::install(&config_dir, &Scheme::DARK, Variant::Dark, BarMode::Autohide)
        .expect("install");

    let config = read(&config_dir.join("config.kdl"));
    assert!(
        config.contains(&format!("\n{INCLUDE_LINE}\n")),
        "include line starts on its own line:\n{config}"
    );
}

#[test]
fn index_includes_every_owned_file() {
    let index = niri_config::render_index();
    for name in ["layer-rules.kdl", "binds.kdl", "colors.kdl"] {
        assert!(
            index.contains(&format!("\"{name}\"")),
            "index.kdl includes {name}:\n{index}"
        );
    }
}

#[test]
fn binds_cover_resolved_board_shortcuts() {
    let binds = niri_config::render_binds();
    // Super+B toggles the bar through `triton msg`.
    assert!(binds.contains("Super+B"));
    assert!(binds.contains("spawn \"triton\" \"msg\" \"bar\" \"toggle\""));
    // Resolved board conflicts: Super+V clipboard, Super+Shift+V floating.
    assert!(binds.contains("Super+V"));
    assert!(binds.contains("Super+Shift+V"));
    assert!(binds.contains("toggle-window-floating"));
}

#[test]
fn layer_rules_never_emit_rule_side_blur() {
    for variant in [Variant::Dark, Variant::Light] {
        for mode in [BarMode::Autohide, BarMode::Pinned, BarMode::Hidden] {
            let rendered = niri_config::render_layer_rules(variant, mode);
            assert!(
                !rendered.contains("blur true"),
                "rule-side blur true in {variant:?}/{mode:?}:\n{rendered}"
            );
        }
    }
}

#[test]
fn layer_rules_xray_follows_material_spec() {
    // Autohide bar = Glass islands → xray false. OSD = Glass → xray false.
    let autohide = niri_config::render_layer_rules(Variant::Dark, BarMode::Autohide);
    assert!(layer_rule_block(&autohide, "triton-bar").contains("xray false"));
    assert!(layer_rule_block(&autohide, "triton-osd").contains("xray false"));

    // Pinned bar = Mica strip → xray true. OSD stays Glass → xray false.
    let pinned = niri_config::render_layer_rules(Variant::Dark, BarMode::Pinned);
    assert!(layer_rule_block(&pinned, "triton-bar").contains("xray true"));
    assert!(layer_rule_block(&pinned, "triton-osd").contains("xray false"));
}

#[test]
fn layer_rules_match_shell_surface_namespaces() {
    let rendered = niri_config::render_layer_rules(Variant::Dark, BarMode::Autohide);
    for namespace in ["triton-bar", "triton-osd"] {
        assert!(
            rendered.contains(&format!("namespace=\"^{namespace}$\"")),
            "rule for {namespace}:\n{rendered}"
        );
    }
}

#[test]
fn colors_kdl_renders_reference_scheme_roles() {
    let rendered = niri_config::render_colors(&Scheme::DARK);
    // tokens.json dark scheme literals (RGBA hex): primary, outline, error, surface.
    for hex in ["#b0c6ffff", "#8e9099ff", "#ffb4abff", "#111318ff"] {
        assert!(
            rendered.contains(hex),
            "colors.kdl contains {hex}:\n{rendered}"
        );
    }
}

#[test]
fn write_colors_updates_only_colors_kdl() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let config_dir = tmp.path().join("niri");

    let path = niri_config::write_colors(&config_dir, &Scheme::LIGHT).expect("write_colors");
    assert_eq!(path, owned_dir(&config_dir).join("colors.kdl"));

    let rendered = read(&path);
    // Light scheme primary #2F5AC0 → RGBA hex.
    assert!(
        rendered.contains("#2f5ac0ff"),
        "light primary in colors.kdl:\n{rendered}"
    );
    // No other owned files are created by write_colors.
    assert!(!owned_dir(&config_dir).join("index.kdl").exists());
}

#[test]
fn write_layer_rules_updates_only_layer_rules_kdl() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let config_dir = tmp.path().join("niri");

    let path = niri_config::write_layer_rules(&config_dir, Variant::Dark, BarMode::Pinned)
        .expect("write_layer_rules");
    assert_eq!(path, owned_dir(&config_dir).join("layer-rules.kdl"));
    assert!(read(&path).contains("xray true"));
}

#[test]
fn renders_are_deterministic() {
    assert_eq!(niri_config::render_index(), niri_config::render_index());
    assert_eq!(niri_config::render_binds(), niri_config::render_binds());
    assert_eq!(
        niri_config::render_colors(&Scheme::DARK),
        niri_config::render_colors(&Scheme::DARK)
    );
    assert_eq!(
        niri_config::render_layer_rules(Variant::Dark, BarMode::Autohide),
        niri_config::render_layer_rules(Variant::Dark, BarMode::Autohide)
    );
}

#[test]
fn owned_writes_are_atomic_and_leave_no_temp_files() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let config_dir = tmp.path().join("niri");

    niri_config::install(&config_dir, &Scheme::DARK, Variant::Dark, BarMode::Autohide)
        .expect("install");

    let mut leftovers = Vec::new();
    for entry in fs::read_dir(owned_dir(&config_dir)).unwrap() {
        let name = entry.unwrap().file_name();
        let name = name.to_string_lossy();
        if name.contains(".tmp") || name.starts_with('.') {
            leftovers.push(name.into_owned());
        }
    }
    assert!(
        leftovers.is_empty(),
        "temp files left behind: {leftovers:?}"
    );
}

#[test]
fn niri_validate_accepts_the_generated_config() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let config_dir = tmp.path().join("niri");
    niri_config::install(&config_dir, &Scheme::DARK, Variant::Dark, BarMode::Autohide)
        .expect("install");
    let config = config_dir.join("config.kdl");

    run_niri_validate(&config);
}

/// Runs read-only `niri validate -c` when niri is installed; skips silently
/// otherwise. Exposed per acceptance criteria: the generated config must be a
/// valid niri configuration.
#[expect(
    clippy::disallowed_methods,
    reason = "read-only `niri validate` is the spec'd config oracle; tests only"
)]
fn run_niri_validate(config: &Path) {
    let status = match Command::new("niri")
        .arg("validate")
        .arg("-c")
        .arg(config)
        .status()
    {
        Ok(status) => status,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            eprintln!("niri not on PATH; skipping niri validate check");
            return;
        }
        Err(err) => panic!("failed to spawn niri validate: {err}"),
    };
    assert!(
        status.success(),
        "niri validate -c {} failed",
        config.display()
    );
}
