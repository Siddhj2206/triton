//! `triton doctor` — self-diagnostics in the DMS `dms doctor` mold:
//! categorized checks (env, niri, generated config, systemd unit, tools)
//! that print `ok`/`WARN`/`FAIL` lines. Exits nonzero on any FAIL.

use std::path::{Path, PathBuf};
use std::process::exit;

#[derive(Clone, Copy)]
enum Check {
    Ok,
    Warn,
    Fail,
}

fn report(all_ok: &mut bool, level: Check, name: &str, detail: &str) {
    let tag = match level {
        Check::Ok => "ok  ",
        Check::Warn => "WARN",
        Check::Fail => "FAIL",
    };
    println!("{tag}  {name:28} {detail}");
    *all_ok &= !matches!(level, Check::Fail);
}

fn in_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
}

fn check(all_ok: &mut bool, level: Check, name: &str, detail: impl Into<String>) {
    report(all_ok, level, name, &detail.into());
}

pub fn run() -> ! {
    let mut all_ok = true;

    // ── environment ───────────────────────────────────────────────────
    for var in ["XDG_RUNTIME_DIR", "HOME"] {
        check(
            &mut all_ok,
            if std::env::var_os(var).is_some() {
                Check::Ok
            } else {
                Check::Fail
            },
            var,
            std::env::var(var).unwrap_or_else(|_| "(unset)".into()),
        );
    }
    let display = std::env::var_os("WAYLAND_DISPLAY")
        .map(|d| d.into_string().unwrap_or_default())
        .filter(|d| !d.is_empty());
    let wayland_up = display.as_deref().is_some_and(|d| {
        std::env::var_os("XDG_RUNTIME_DIR").is_some_and(|r| PathBuf::from(r).join(d).exists())
    });
    check(
        &mut all_ok,
        if wayland_up { Check::Ok } else { Check::Warn },
        "wayland socket",
        display.unwrap_or_else(|| "(unset)".into()),
    );

    // ── niri ──────────────────────────────────────────────────────────
    match std::env::var_os("NIRI_SOCKET").map(PathBuf::from) {
        Some(p) if p.exists() => {
            check(
                &mut all_ok,
                Check::Ok,
                "NIRI_SOCKET",
                p.display().to_string(),
            );
        }
        _ => check(
            &mut all_ok,
            Check::Warn,
            "NIRI_SOCKET",
            "unset — not inside niri",
        ),
    }

    // ── generated config + systemd ────────────────────────────────────
    config_and_systemd(&mut all_ok);
    tools(&mut all_ok);

    exit(i32::from(!all_ok));
}

fn config_and_systemd(all_ok: &mut bool) {
    let Some(root) = crate::util::config_root() else {
        check(
            all_ok,
            Check::Fail,
            "config root",
            "no HOME/XDG_CONFIG_HOME",
        );
        exit(1);
    };
    let config_kdl = std::fs::read_to_string(root.join("niri/config.kdl")).unwrap_or_default();
    check(
        all_ok,
        if crate::niri_config::has_include(&config_kdl) {
            Check::Ok
        } else {
            Check::Warn
        },
        "niri include line",
        "triton/index.kdl referenced from config.kdl",
    );
    for f in ["index.kdl", "binds.kdl", "colors.kdl", "layer-rules.kdl"] {
        let p = root.join("niri/triton").join(f);
        check(
            all_ok,
            if p.is_file() { Check::Ok } else { Check::Warn },
            f,
            if p.is_file() {
                "present".to_owned()
            } else {
                "missing — run triton once".to_owned()
            },
        );
    }
    let unit = root.join("systemd/user/triton.service");
    if unit.is_file() {
        let text = std::fs::read_to_string(&unit).unwrap_or_default();
        let exec = text
            .lines()
            .find_map(|l| l.strip_prefix("ExecStart="))
            .unwrap_or("?");
        let exe = Path::new(exec.split_whitespace().next().unwrap_or(exec));
        check(
            all_ok,
            if exe.is_file() {
                Check::Ok
            } else {
                Check::Fail
            },
            "unit ExecStart",
            format!(
                "{exec} — {}",
                if exe.is_file() {
                    "resolves"
                } else {
                    "stale: re-run triton from the installed path"
                }
            ),
        );
        check(
            all_ok,
            if root
                .join("systemd/user/niri.service.wants/triton.service")
                .exists()
            {
                Check::Ok
            } else {
                Check::Warn
            },
            "unit enabled",
            "niri.service.wants link",
        );
    } else {
        check(all_ok, Check::Warn, "triton.service", "not installed");
    }
}

fn tools(all_ok: &mut bool) {
    for (bin, why) in [
        ("triton-lock", "session lock"),
        ("nmcli", "wifi join"),
        ("wpctl", "volume backend"),
        ("systemctl", "daemon-reload"),
    ] {
        check(
            all_ok,
            if in_path(bin) { Check::Ok } else { Check::Warn },
            bin,
            why,
        );
    }
    // The setuid helper lives in a libexec dir, never on PATH — the same
    // resolution `polkit.rs` uses.
    match crate::polkit::helper_path() {
        Some(p) => check(
            all_ok,
            Check::Ok,
            "polkit-agent-helper-1",
            p.display().to_string(),
        ),
        None => check(
            all_ok,
            Check::Fail,
            "polkit-agent-helper-1",
            "not found — polkit auth cannot run",
        ),
    }
}
