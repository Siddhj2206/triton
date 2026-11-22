//! CLI argument parsing for `triton` daemon and `triton msg` subcommands.

use std::path::PathBuf;

use thiserror::Error;
use triton_design::color::Variant;

use crate::headless::HeadlessArgs;
use crate::ipc::{BarCommand, IpcCommand, OsdCommand, ThemeCommand};
use crate::osd::OsdPayload;
use crate::state::{BackendMode, BarMode, SurfaceRole};

/// Usage text printed for `triton --help`.
pub const USAGE: &str = "\
triton — Triton desktop shell for niri

USAGE:
    triton [--real-services] [--socket <PATH>]
    triton --snap <FILE.png|DIR> [HEADLESS FLAGS]
    triton --tree <FILE.json|DIR|-> [HEADLESS FLAGS]
    triton --script <FILE.json> [--snap ...] [--tree ...]
    triton msg [--socket <PATH>] [--json] <SUBCOMMAND>

DAEMON FLAGS:
    --real-services       Enable live system integrations (default: safe fake dev backend)
    --socket <PATH>       Override Unix socket path (default: $TRITON_SOCKET or $XDG_RUNTIME_DIR/triton.sock)
    -h, --help            Print this help message

HEADLESS FLAGS (offscreen dev loop; always the fake backend, no Wayland):
    --snap <PATH>         Write offscreen EGL PNG snapshot(s) to a file or directory
    --tree <PATH>         Write semantic tree JSON to a file, directory, or `-` for stdout
    --script <PATH>       Replay a timestamped input-event script (JSON `events` array)
    --surface <ROLE>      bar | osd | wallpaper | all (repeatable; default: bar)
    --variant <V>         dark | light | both (default: dark)
    --mode <MODE>         autohide | pinned | hidden (Bar mode during the run)
    --osd <KIND>          volume | brightness | mic | layout | workspace | battery
    --reveal              Simulate a hover on the Autohide reveal edge (shows islands)
    --scale <N>           Logical to physical scale (default: 1.75)
    --size <WxH>          Canvas for Stretch surfaces (default: 1920x1080)
    --settle <MS>         Virtual settle budget before output (default: 800)
    --reduce-motion       Use 120ms linear fades instead of springs
    --transparent         Keep alpha in PNGs instead of a flat backdrop

MSG SUBCOMMANDS:
    status                                 Show daemon status and active surfaces
    bar <autohide|pinned|hidden|toggle>    Change or toggle bar mode
    osd volume <0..100|0..1> [--muted] [--output <NAME>]
    osd brightness <0..100|0..1> [--output <NAME>]
    osd mic <muted|unmuted> [--output <NAME>]
    osd layout <NAME> [--output <NAME>]
    osd workspace <LABEL> [--output <NAME>]
    osd battery <PERCENT> [--output <NAME>]
    osd hide
    wallpaper <PATH> [--output <NAME>]     Set wallpaper globally or per output
    theme <dark|light|toggle>              Switch theme variant
    theme reduce-motion <on|off>           Toggle reduced motion
    quit                                   Shut down the running daemon
";

/// Errors produced while parsing CLI arguments.
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum CliError {
    #[error("unknown argument: {0}")]
    UnknownArgument(String),
    #[error("missing value for {0}")]
    MissingValue(String),
    #[error("invalid value for {flag}: {value}")]
    InvalidValue { flag: String, value: String },
    #[error("missing subcommand for `triton msg`")]
    MissingMsgSubcommand,
    #[error("{0} cannot be combined with headless dev flags (--snap/--tree/--script)")]
    IncompatibleWithHeadless(String),
    #[error("{0} only applies to a headless run; pass --snap, --tree, or --script")]
    MissingHeadlessTarget(String),
}

/// Options for running the `triton` daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonArgs {
    pub backend: BackendMode,
    pub socket: Option<PathBuf>,
}

/// Options for running `triton msg`.
#[derive(Debug, Clone, PartialEq)]
pub struct MsgArgs {
    pub socket: Option<PathBuf>,
    pub command: IpcCommand,
    pub json: bool,
}

/// Top-level parsed CLI command.
#[derive(Debug, Clone, PartialEq)]
pub enum CliCommand {
    Daemon(DaemonArgs),
    Msg(MsgArgs),
    /// Offscreen dev loop (`--snap`/`--tree`/`--script`) — no daemon, no Wayland.
    Headless(HeadlessArgs),
    Help,
}

/// Parse CLI arguments (excluding `argv[0]`).
pub fn parse_args<I, S>(args: I) -> Result<CliCommand, CliError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let tokens: Vec<String> = args.into_iter().map(|s| s.as_ref().to_owned()).collect();
    let Some(first) = tokens.first().map(String::as_str) else {
        return Ok(CliCommand::Daemon(DaemonArgs {
            backend: BackendMode::Fake,
            socket: None,
        }));
    };

    if matches!(first, "-h" | "--help" | "help") {
        return Ok(CliCommand::Help);
    }

    if first == "msg" {
        return parse_msg_args(&tokens[1..]).map(CliCommand::Msg);
    }

    parse_daemon_args(&tokens)
}

/// Fetch a flag's value: inline `--flag=value`, or the next token `--flag v`.
fn flag_value<'a>(
    i: &mut usize,
    tokens: &'a [String],
    inline: Option<&'a str>,
    flag: &str,
) -> Result<&'a str, CliError> {
    if let Some(value) = inline {
        if value.is_empty() {
            return Err(CliError::MissingValue(flag.to_owned()));
        }
        return Ok(value);
    }
    *i += 1;
    tokens
        .get(*i)
        .map(String::as_str)
        .ok_or_else(|| CliError::MissingValue(flag.to_owned()))
}

fn parse_daemon_args(tokens: &[String]) -> Result<CliCommand, CliError> {
    let mut backend = BackendMode::Fake;
    let mut socket = None;
    let mut headless = HeadlessArgs::default();
    let mut saw_headless_modifier: Option<String> = None;
    let mut i = 0;
    while i < tokens.len() {
        let arg = tokens[i].as_str();
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v)),
            _ => (arg, None),
        };
        match name {
            "--real-services" => {
                backend = BackendMode::RealServices;
            }
            "--socket" => {
                socket = Some(PathBuf::from(flag_value(&mut i, tokens, inline, name)?));
            }
            _ => {
                let matched = parse_headless_flag(
                    &mut i,
                    tokens,
                    name,
                    inline,
                    &mut headless,
                    &mut saw_headless_modifier,
                )?;
                if !matched {
                    return Err(CliError::UnknownArgument(name.to_owned()));
                }
            }
        }
        i += 1;
    }

    let headless_requested =
        headless.snap.is_some() || headless.tree.is_some() || headless.script.is_some();
    if !headless_requested {
        if let Some(flag) = saw_headless_modifier {
            return Err(CliError::MissingHeadlessTarget(flag));
        }
        return Ok(CliCommand::Daemon(DaemonArgs { backend, socket }));
    }
    if backend == BackendMode::RealServices {
        return Err(CliError::IncompatibleWithHeadless(
            "--real-services".to_owned(),
        ));
    }
    if socket.is_some() {
        return Err(CliError::IncompatibleWithHeadless("--socket".to_owned()));
    }
    Ok(CliCommand::Headless(headless))
}

/// Parse one headless dev flag (`--snap`, `--tree`, `--script`, and the
/// modifiers that configure them) into `headless`. Returns `false` when
/// `name` is not a headless flag so the caller can report it as unknown.
/// `saw_modifier` records the first modifier flag seen so a bare
/// `triton --surface bar` can fail with a targeted error.
fn parse_headless_flag(
    i: &mut usize,
    tokens: &[String],
    name: &str,
    inline: Option<&str>,
    headless: &mut HeadlessArgs,
    saw_modifier: &mut Option<String>,
) -> Result<bool, CliError> {
    let mut modifier = || {
        saw_modifier.get_or_insert_with(|| name.to_owned());
    };
    match name {
        "--snap" => {
            headless.snap = Some(PathBuf::from(flag_value(i, tokens, inline, name)?));
        }
        "--tree" => {
            headless.tree = Some(PathBuf::from(flag_value(i, tokens, inline, name)?));
        }
        "--script" => {
            headless.script = Some(PathBuf::from(flag_value(i, tokens, inline, name)?));
        }
        "--surface" => {
            headless
                .surfaces
                .extend(parse_surface_flag(flag_value(i, tokens, inline, name)?)?);
            modifier();
        }
        "--variant" => {
            headless.variants = parse_variant_flag(flag_value(i, tokens, inline, name)?)?;
            modifier();
        }
        "--mode" => {
            headless.bar_mode = Some(parse_mode_flag(flag_value(i, tokens, inline, name)?)?);
            modifier();
        }
        "--osd" => {
            headless.osd = Some(parse_osd_flag(flag_value(i, tokens, inline, name)?)?);
            modifier();
        }
        "--scale" => {
            let raw = flag_value(i, tokens, inline, name)?;
            headless.scale = raw.parse::<f32>().map_err(|_| CliError::InvalidValue {
                flag: name.to_owned(),
                value: raw.to_owned(),
            })?;
            if headless.scale <= 0.0 || !headless.scale.is_finite() {
                return Err(CliError::InvalidValue {
                    flag: name.to_owned(),
                    value: raw.to_owned(),
                });
            }
            modifier();
        }
        "--size" => {
            headless.size = Some(parse_size_flag(flag_value(i, tokens, inline, name)?)?);
            modifier();
        }
        "--settle" => {
            let raw = flag_value(i, tokens, inline, name)?;
            headless.settle_ms = raw.parse::<u64>().map_err(|_| CliError::InvalidValue {
                flag: name.to_owned(),
                value: raw.to_owned(),
            })?;
            modifier();
        }
        "--reduce-motion" => {
            headless.reduce_motion = true;
            modifier();
        }
        "--reveal" => {
            headless.reveal = true;
            modifier();
        }
        "--transparent" => {
            headless.transparent = true;
            modifier();
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// `--surface bar|osd|wallpaper|all`; `all` expands in `surfaces()` order.
fn parse_surface_flag(raw: &str) -> Result<Vec<SurfaceRole>, CliError> {
    match raw {
        "all" => Ok(vec![
            SurfaceRole::Background,
            SurfaceRole::Bar,
            SurfaceRole::Osd,
        ]),
        "bar" => Ok(vec![SurfaceRole::Bar]),
        "osd" => Ok(vec![SurfaceRole::Osd]),
        "wallpaper" | "background" => Ok(vec![SurfaceRole::Background]),
        other => Err(CliError::InvalidValue {
            flag: "--surface".to_owned(),
            value: other.to_owned(),
        }),
    }
}

fn parse_variant_flag(raw: &str) -> Result<Vec<Variant>, CliError> {
    match raw {
        "dark" => Ok(vec![Variant::Dark]),
        "light" => Ok(vec![Variant::Light]),
        "both" => Ok(vec![Variant::Dark, Variant::Light]),
        other => Err(CliError::InvalidValue {
            flag: "--variant".to_owned(),
            value: other.to_owned(),
        }),
    }
}

fn parse_mode_flag(raw: &str) -> Result<BarMode, CliError> {
    match raw {
        "autohide" => Ok(BarMode::Autohide),
        "pinned" => Ok(BarMode::Pinned),
        "hidden" => Ok(BarMode::Hidden),
        other => Err(CliError::InvalidValue {
            flag: "--mode".to_owned(),
            value: other.to_owned(),
        }),
    }
}

/// `--osd <kind>`: a canned payload so the OSD pill has something to show.
fn parse_osd_flag(raw: &str) -> Result<OsdPayload, CliError> {
    match raw {
        "volume" => Ok(OsdPayload::Volume {
            level: 0.65,
            muted: false,
        }),
        "volume-muted" | "muted" => Ok(OsdPayload::Volume {
            level: 0.65,
            muted: true,
        }),
        "brightness" => Ok(OsdPayload::Brightness { level: 0.7 }),
        "mic" => Ok(OsdPayload::Mic { muted: true }),
        "layout" => Ok(OsdPayload::KeyboardLayout {
            name: "English (US)".to_owned(),
        }),
        "workspace" => Ok(OsdPayload::Workspace {
            label: "3".to_owned(),
        }),
        "battery" => Ok(OsdPayload::LowBattery { percent: 14 }),
        other => Err(CliError::InvalidValue {
            flag: "--osd".to_owned(),
            value: other.to_owned(),
        }),
    }
}

/// `--size 1920x1080` -> `(1920, 1080)`.
fn parse_size_flag(raw: &str) -> Result<(u32, u32), CliError> {
    let invalid = || CliError::InvalidValue {
        flag: "--size".to_owned(),
        value: raw.to_owned(),
    };
    let (w, h) = raw.split_once(['x', 'X']).ok_or_else(invalid)?;
    let w = w.parse::<u32>().map_err(|_| invalid())?;
    let h = h.parse::<u32>().map_err(|_| invalid())?;
    if w == 0 || h == 0 {
        return Err(invalid());
    }
    Ok((w, h))
}

fn parse_msg_args(tokens: &[String]) -> Result<MsgArgs, CliError> {
    let mut socket = None;
    let mut json = false;
    let mut i = 0;

    while i < tokens.len() {
        match tokens[i].as_str() {
            "--socket" => {
                let value = tokens
                    .get(i + 1)
                    .ok_or_else(|| CliError::MissingValue("--socket".to_owned()))?;
                socket = Some(PathBuf::from(value));
                i += 2;
            }
            arg if arg.starts_with("--socket=") => {
                let value = &arg["--socket=".len()..];
                if value.is_empty() {
                    return Err(CliError::MissingValue("--socket".to_owned()));
                }
                socket = Some(PathBuf::from(value));
                i += 1;
            }
            "--json" => {
                json = true;
                i += 1;
            }
            _ => break,
        }
    }

    let rest = &tokens[i..];
    let Some(sub) = rest.first().map(String::as_str) else {
        return Err(CliError::MissingMsgSubcommand);
    };

    let command = match sub {
        "status" => {
            ensure_no_extra(&rest[1..])?;
            IpcCommand::Status
        }
        "quit" => {
            ensure_no_extra(&rest[1..])?;
            IpcCommand::Quit
        }
        "bar" => parse_bar_command(&rest[1..])?,
        "osd" => parse_osd_command(&rest[1..])?,
        "wallpaper" => parse_wallpaper_command(&rest[1..])?,
        "theme" => parse_theme_command(&rest[1..])?,
        other => return Err(CliError::UnknownArgument(other.to_owned())),
    };

    Ok(MsgArgs {
        socket,
        command,
        json,
    })
}

fn parse_bar_command(tokens: &[String]) -> Result<IpcCommand, CliError> {
    let action_str = tokens
        .first()
        .ok_or_else(|| CliError::MissingValue("bar <autohide|pinned|hidden|toggle>".to_owned()))?;
    ensure_no_extra(&tokens[1..])?;
    let action = match action_str.as_str() {
        "autohide" => BarCommand::Autohide,
        "pinned" => BarCommand::Pinned,
        "hidden" => BarCommand::Hidden,
        "toggle" => BarCommand::Toggle,
        other => {
            return Err(CliError::InvalidValue {
                flag: "bar".to_owned(),
                value: other.to_owned(),
            });
        }
    };
    Ok(IpcCommand::Bar { action })
}

fn parse_osd_command(tokens: &[String]) -> Result<IpcCommand, CliError> {
    let kind = tokens
        .first()
        .ok_or_else(|| CliError::MissingValue("osd <kind>".to_owned()))?;
    match kind.as_str() {
        "hide" => {
            ensure_no_extra(&tokens[1..])?;
            Ok(IpcCommand::Osd {
                action: OsdCommand::Hide,
            })
        }
        "volume" => parse_osd_volume(&tokens[1..]),
        "brightness" => {
            let raw = tokens
                .get(1)
                .ok_or_else(|| CliError::MissingValue("osd brightness <level>".to_owned()))?;
            let level = parse_normalized_level("osd brightness", raw)?;
            let output = parse_optional_output(&tokens[2..])?;
            Ok(IpcCommand::Osd {
                action: OsdCommand::Show {
                    payload: OsdPayload::Brightness { level },
                    output,
                },
            })
        }
        "mic" => {
            let raw = tokens
                .get(1)
                .ok_or_else(|| CliError::MissingValue("osd mic <muted|unmuted>".to_owned()))?;
            let muted = match raw.as_str() {
                "muted" | "on" | "true" => true,
                "unmuted" | "off" | "false" => false,
                other => {
                    return Err(CliError::InvalidValue {
                        flag: "osd mic".to_owned(),
                        value: other.to_owned(),
                    });
                }
            };
            let output = parse_optional_output(&tokens[2..])?;
            Ok(IpcCommand::Osd {
                action: OsdCommand::Show {
                    payload: OsdPayload::Mic { muted },
                    output,
                },
            })
        }
        "layout" => {
            let name = tokens
                .get(1)
                .ok_or_else(|| CliError::MissingValue("osd layout <name>".to_owned()))?
                .clone();
            let output = parse_optional_output(&tokens[2..])?;
            Ok(IpcCommand::Osd {
                action: OsdCommand::Show {
                    payload: OsdPayload::KeyboardLayout { name },
                    output,
                },
            })
        }
        "workspace" => {
            let label = tokens
                .get(1)
                .ok_or_else(|| CliError::MissingValue("osd workspace <label>".to_owned()))?
                .clone();
            let output = parse_optional_output(&tokens[2..])?;
            Ok(IpcCommand::Osd {
                action: OsdCommand::Show {
                    payload: OsdPayload::Workspace { label },
                    output,
                },
            })
        }
        "battery" => {
            let raw = tokens
                .get(1)
                .ok_or_else(|| CliError::MissingValue("osd battery <percent>".to_owned()))?;
            let percent = raw.parse::<u8>().map_err(|_| CliError::InvalidValue {
                flag: "osd battery".to_owned(),
                value: raw.clone(),
            })?;
            let output = parse_optional_output(&tokens[2..])?;
            Ok(IpcCommand::Osd {
                action: OsdCommand::Show {
                    payload: OsdPayload::LowBattery { percent },
                    output,
                },
            })
        }
        other => Err(CliError::UnknownArgument(other.to_owned())),
    }
}

fn parse_osd_volume(tokens: &[String]) -> Result<IpcCommand, CliError> {
    let raw = tokens
        .first()
        .ok_or_else(|| CliError::MissingValue("osd volume <level>".to_owned()))?;
    let level = parse_normalized_level("osd volume", raw)?;
    let mut muted = false;
    let mut output = None;
    let mut i = 1;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "--muted" => {
                muted = true;
                i += 1;
            }
            "--output" => {
                let out = tokens
                    .get(i + 1)
                    .ok_or_else(|| CliError::MissingValue("--output".to_owned()))?;
                output = Some(out.clone());
                i += 2;
            }
            other => return Err(CliError::UnknownArgument(other.to_owned())),
        }
    }
    Ok(IpcCommand::Osd {
        action: OsdCommand::Show {
            payload: OsdPayload::Volume { level, muted },
            output,
        },
    })
}

fn parse_wallpaper_command(tokens: &[String]) -> Result<IpcCommand, CliError> {
    let path = tokens
        .first()
        .ok_or_else(|| CliError::MissingValue("wallpaper <path>".to_owned()))?;
    let output = parse_optional_output(&tokens[1..])?;
    Ok(IpcCommand::Wallpaper {
        path: PathBuf::from(path),
        output,
    })
}

fn parse_theme_command(tokens: &[String]) -> Result<IpcCommand, CliError> {
    let action_str = tokens.first().ok_or_else(|| {
        CliError::MissingValue("theme <dark|light|toggle|reduce-motion>".to_owned())
    })?;
    let action = match action_str.as_str() {
        "dark" => {
            ensure_no_extra(&tokens[1..])?;
            ThemeCommand::Dark
        }
        "light" => {
            ensure_no_extra(&tokens[1..])?;
            ThemeCommand::Light
        }
        "toggle" => {
            ensure_no_extra(&tokens[1..])?;
            ThemeCommand::ToggleVariant
        }
        "reduce-motion" => {
            let raw = tokens
                .get(1)
                .ok_or_else(|| CliError::MissingValue("theme reduce-motion <on|off>".to_owned()))?;
            ensure_no_extra(&tokens[2..])?;
            let enabled = match raw.as_str() {
                "on" | "true" | "1" => true,
                "off" | "false" | "0" => false,
                other => {
                    return Err(CliError::InvalidValue {
                        flag: "theme reduce-motion".to_owned(),
                        value: other.to_owned(),
                    });
                }
            };
            ThemeCommand::ReduceMotion { enabled }
        }
        other => {
            return Err(CliError::InvalidValue {
                flag: "theme".to_owned(),
                value: other.to_owned(),
            });
        }
    };
    Ok(IpcCommand::Theme { action })
}

fn parse_normalized_level(flag: &str, raw: &str) -> Result<f32, CliError> {
    let value = raw.parse::<f32>().map_err(|_| CliError::InvalidValue {
        flag: flag.to_owned(),
        value: raw.to_owned(),
    })?;
    if !value.is_finite() || value < 0.0 || value > 100.0 {
        return Err(CliError::InvalidValue {
            flag: flag.to_owned(),
            value: raw.to_owned(),
        });
    }
    if value > 1.0 {
        Ok((value / 100.0).clamp(0.0, 1.0))
    } else {
        Ok(value)
    }
}

fn parse_optional_output(tokens: &[String]) -> Result<Option<String>, CliError> {
    match tokens {
        [] => Ok(None),
        [flag, value] if flag == "--output" => Ok(Some(value.clone())),
        [flag] if flag == "--output" => Err(CliError::MissingValue("--output".to_owned())),
        [first, ..] => Err(CliError::UnknownArgument(first.clone())),
    }
}

fn ensure_no_extra(tokens: &[String]) -> Result<(), CliError> {
    if let Some(first) = tokens.first() {
        return Err(CliError::UnknownArgument(first.clone()));
    }
    Ok(())
}
