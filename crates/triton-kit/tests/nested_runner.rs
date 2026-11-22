//! Isolation-invariant tests for `dev/nested/`: the nested-niri visual runner
//! used by `mise run dev`. These run headlessly — they verify the *script's*
//! contract (private runtime dir, scratch XDG dirs, never `niri --session`,
//! POSIX `sh -n` clean) without ever launching a compositor.

use std::process::Command;

const SESSION_SH: &str = include_str!("../../../dev/nested/session.sh");
const NIRI_KDL: &str = include_str!("../../../dev/nested/niri.kdl");
const MISE_TOML: &str = include_str!("../../../mise.toml");

/// Run `sh -n` (syntax check only — never executes the script).
fn sh_syntax_check(script: &str) -> std::process::ExitStatus {
    let mut child = Command::new("sh")
        .arg("-n")
        .arg("-c")
        .arg(script)
        .spawn()
        .expect("spawn sh -n");
    child.wait().expect("wait for sh -n")
}

#[test]
fn session_script_passes_sh_n() {
    let status = sh_syntax_check(SESSION_SH);
    assert!(status.success(), "sh -n failed for dev/nested/session.sh");
}

#[test]
fn session_script_is_posix_and_strict() {
    assert!(
        SESSION_SH.starts_with("#!") || SESSION_SH.starts_with('#'),
        "script should document its interpreter"
    );
    assert!(
        SESSION_SH.contains("set -eu"),
        "script must fail fast: `set -eu` missing"
    );
    // POSIX only — no bashisms in the runner contract.
    for bashism in ["[[", "source ", "declare ", "local ", "arrays"] {
        assert!(
            !SESSION_SH.contains(bashism),
            "non-POSIX construct {bashism:?} found"
        );
    }
}

#[test]
fn session_script_resolves_host_socket_before_overriding_runtime_dir() {
    // The host WAYLAND_DISPLAY must be turned into an absolute socket path
    // BEFORE XDG_RUNTIME_DIR is replaced, or niri cannot reach the parent
    // compositor and opens a drm session instead of a nested window.
    let resolve = SESSION_SH
        .find("WAYLAND_DISPLAY")
        .expect("WAYLAND_DISPLAY handling missing");
    let runtime = SESSION_SH
        .find("export XDG_RUNTIME_DIR")
        .expect("XDG_RUNTIME_DIR export missing");
    assert!(
        resolve < runtime,
        "WAYLAND_DISPLAY must be resolved before XDG_RUNTIME_DIR is overridden"
    );
}

#[test]
fn session_script_uses_private_runtime_and_scratch_xdg_dirs() {
    // Private runtime dir under target/ with mode 700.
    assert!(
        SESSION_SH.contains("target/nested-niri"),
        "private dirs must live under target/nested-niri"
    );
    assert!(
        SESSION_SH.contains("export XDG_RUNTIME_DIR"),
        "must export a private XDG_RUNTIME_DIR"
    );
    assert!(
        SESSION_SH.contains("-m 700") || SESSION_SH.contains("chmod 700"),
        "private runtime dir must be mode 700"
    );
    // Scratch HOME and every XDG user dir — the script must never touch the
    // real $HOME or $HOME/.config/niri.
    for var in [
        "export HOME=",
        "export XDG_CONFIG_HOME=",
        "export XDG_DATA_HOME=",
        "export XDG_CACHE_HOME=",
        "export XDG_STATE_HOME=",
    ] {
        assert!(
            SESSION_SH.contains(var),
            "missing scratch {var} — host config must never be used"
        );
    }
    // The niri config must be copied into the scratch config home.
    assert!(
        SESSION_SH.contains("niri.kdl") && SESSION_SH.contains("config.kdl"),
        "dev/nested/niri.kdl must be copied to $XDG_CONFIG_HOME/niri/config.kdl"
    );
    // Private runtime dir is cleaned up on exit.
    assert!(
        SESSION_SH.contains("trap"),
        "missing trap cleanup for the private runtime dir"
    );
}

#[test]
fn session_script_never_uses_niri_session() {
    // `niri --session` exports the nested env to systemd/the real session bus —
    // the single biggest isolation break. `niri -c cfg -- BIN` is the contract.
    // (Comments mention the forbidden form — strip them before checking code.)
    let code: String = SESSION_SH
        .lines()
        .map(|line| line.split('#').next().unwrap_or_default())
        .collect();
    assert!(
        !code.contains("niri --session"),
        "niri --session is forbidden in the nested runner"
    );
    assert!(
        code.contains("niri -c"),
        "expected `niri -c <config> -- <bin>` launch"
    );
}

#[test]
fn session_script_runs_niri_as_a_child_so_cleanup_trap_fires() {
    // `exec niri` would replace the shell entirely, so the `trap cleanup EXIT`
    // would never run and the private runtime dir would leak into target/.
    // The contract is a foreground child + `wait` so EXIT/INT/TERM traps fire.
    let code: String = SESSION_SH
        .lines()
        .map(|line| line.split('#').next().unwrap_or_default())
        .collect();
    assert!(
        !code.contains("exec niri"),
        "exec replaces the shell — the cleanup trap can never run"
    );
    assert!(
        code.contains("trap cleanup"),
        "missing `trap cleanup` for the private runtime dir"
    );
    assert!(
        code.contains("wait"),
        "niri must be awaited so the script (and its traps) outlives it"
    );
}

#[test]
fn niri_config_requests_blur_for_triton_namespaces() {
    // Glass/Thick materials are only real when the compositor blurs behind the
    // layer surface; every triton namespace is matched by the caret prefix.
    assert!(NIRI_KDL.contains("layer-rule"), "missing layer-rule");
    assert!(
        NIRI_KDL.contains("namespace=\"^triton\""),
        "layer-rule must match ^triton namespaces"
    );
    assert!(NIRI_KDL.contains("blur true"), "blur must be enabled");
    assert!(
        NIRI_KDL.contains("xray false"),
        "xray must stay off so wallpaper never bleeds through"
    );
    assert!(NIRI_KDL.contains("prefer-no-csd"), "missing prefer-no-csd");
    assert!(
        NIRI_KDL.contains("skip-at-startup"),
        "hotkey overlay should not block the first frame"
    );
    assert!(
        NIRI_KDL.contains("output \"winit\"") && NIRI_KDL.contains("scale"),
        "nested window needs a winit output scale"
    );
    assert!(
        NIRI_KDL.contains("quit"),
        "a quit bind keeps the nested session easy to leave"
    );
}

#[test]
fn mise_dev_task_runs_the_nested_session() {
    assert!(
        MISE_TOML.contains("[tasks.dev]"),
        "mise.toml must define [tasks.dev]"
    );
    assert!(
        MISE_TOML.contains("dev/nested/session.sh"),
        "dev task must invoke dev/nested/session.sh"
    );
    assert!(
        MISE_TOML.contains("example showcase"),
        "dev task must build the showcase example"
    );
    // The nested session gets its own bus so it never touches the real
    // session's org.freedesktop namespace.
    assert!(
        MISE_TOML.contains("dbus-run-session"),
        "dev task must run under a private session bus"
    );
}
