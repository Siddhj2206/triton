//! CLI argument parsing for `triton` daemon and `triton msg` subcommands.

use std::path::PathBuf;

use thiserror::Error;

use crate::ipc::{BarCommand, IpcCommand, OsdCommand, ThemeCommand};
use crate::osd::OsdPayload;
use crate::state::BackendMode;

/// Usage text printed for `triton --help`.
pub const USAGE: &str = "\
triton — Triton desktop shell for niri

USAGE:
    triton [--real-services] [--socket <PATH>]
    triton msg [--socket <PATH>] [--json] <SUBCOMMAND>

DAEMON FLAGS:
    --real-services       Enable live system integrations (default: safe fake dev backend)
    --socket <PATH>       Override Unix socket path (default: $TRITON_SOCKET or $XDG_RUNTIME_DIR/triton.sock)
    -h, --help            Print this help message

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

    parse_daemon_args(&tokens).map(CliCommand::Daemon)
}

fn parse_daemon_args(tokens: &[String]) -> Result<DaemonArgs, CliError> {
    let mut backend = BackendMode::Fake;
    let mut socket = None;
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "--real-services" => {
                backend = BackendMode::RealServices;
                i += 1;
            }
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
            other => return Err(CliError::UnknownArgument(other.to_owned())),
        }
    }
    Ok(DaemonArgs { backend, socket })
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
