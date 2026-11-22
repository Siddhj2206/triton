//! CLI parsing and `systemd/triton.service` packaging tests.

use std::path::PathBuf;

use triton::{
    BackendMode, BarCommand, CliCommand, CliError, DaemonArgs, IpcCommand, MsgArgs, OsdCommand,
    OsdPayload, ThemeCommand, parse_args,
};

#[test]
fn daemon_defaults_to_fake_backend_and_supports_real_services_and_socket_override() {
    let default = parse_args(Vec::<&str>::new()).expect("default args");
    assert_eq!(
        default,
        CliCommand::Daemon(DaemonArgs {
            backend: BackendMode::Fake,
            socket: None,
        })
    );

    let real = parse_args(["--real-services", "--socket", "/tmp/triton-custom.sock"])
        .expect("real-services + socket");
    assert_eq!(
        real,
        CliCommand::Daemon(DaemonArgs {
            backend: BackendMode::RealServices,
            socket: Some(PathBuf::from("/tmp/triton-custom.sock")),
        })
    );
}

#[test]
fn parses_status_bar_wallpaper_theme_and_quit_subcommands() {
    assert_eq!(
        parse_args(["msg", "status"]).unwrap(),
        CliCommand::Msg(MsgArgs {
            socket: None,
            command: IpcCommand::Status,
            json: false,
        })
    );

    assert_eq!(
        parse_args(["msg", "--json", "--socket", "/tmp/t.sock", "bar", "pinned"]).unwrap(),
        CliCommand::Msg(MsgArgs {
            socket: Some(PathBuf::from("/tmp/t.sock")),
            command: IpcCommand::Bar {
                action: BarCommand::Pinned,
            },
            json: true,
        })
    );

    assert_eq!(
        parse_args(["msg", "bar", "toggle"]).unwrap(),
        CliCommand::Msg(MsgArgs {
            socket: None,
            command: IpcCommand::Bar {
                action: BarCommand::Toggle,
            },
            json: false,
        })
    );

    assert_eq!(
        parse_args(["msg", "wallpaper", "/tmp/bg.png", "--output", "eDP-1"]).unwrap(),
        CliCommand::Msg(MsgArgs {
            socket: None,
            command: IpcCommand::Wallpaper {
                path: PathBuf::from("/tmp/bg.png"),
                output: Some("eDP-1".to_owned()),
            },
            json: false,
        })
    );

    assert_eq!(
        parse_args(["msg", "theme", "light"]).unwrap(),
        CliCommand::Msg(MsgArgs {
            socket: None,
            command: IpcCommand::Theme {
                action: ThemeCommand::Light,
            },
            json: false,
        })
    );

    assert_eq!(
        parse_args(["msg", "theme", "reduce-motion", "on"]).unwrap(),
        CliCommand::Msg(MsgArgs {
            socket: None,
            command: IpcCommand::Theme {
                action: ThemeCommand::ReduceMotion { enabled: true },
            },
            json: false,
        })
    );

    assert_eq!(
        parse_args(["msg", "quit"]).unwrap(),
        CliCommand::Msg(MsgArgs {
            socket: None,
            command: IpcCommand::Quit,
            json: false,
        })
    );
}

#[test]
fn parses_all_osd_subcommands() {
    assert_eq!(
        parse_args(["msg", "osd", "volume", "75", "--muted"]).unwrap(),
        CliCommand::Msg(MsgArgs {
            socket: None,
            command: IpcCommand::Osd {
                action: OsdCommand::Show {
                    payload: OsdPayload::Volume {
                        level: 0.75,
                        muted: true,
                    },
                    output: None,
                },
            },
            json: false,
        })
    );

    assert_eq!(
        parse_args(["msg", "osd", "brightness", "0.5", "--output", "DP-1"]).unwrap(),
        CliCommand::Msg(MsgArgs {
            socket: None,
            command: IpcCommand::Osd {
                action: OsdCommand::Show {
                    payload: OsdPayload::Brightness { level: 0.5 },
                    output: Some("DP-1".to_owned()),
                },
            },
            json: false,
        })
    );

    assert_eq!(
        parse_args(["msg", "osd", "mic", "muted"]).unwrap(),
        CliCommand::Msg(MsgArgs {
            socket: None,
            command: IpcCommand::Osd {
                action: OsdCommand::Show {
                    payload: OsdPayload::Mic { muted: true },
                    output: None,
                },
            },
            json: false,
        })
    );

    assert_eq!(
        parse_args(["msg", "osd", "layout", "US"]).unwrap(),
        CliCommand::Msg(MsgArgs {
            socket: None,
            command: IpcCommand::Osd {
                action: OsdCommand::Show {
                    payload: OsdPayload::KeyboardLayout {
                        name: "US".to_owned(),
                    },
                    output: None,
                },
            },
            json: false,
        })
    );

    assert_eq!(
        parse_args(["msg", "osd", "workspace", "3"]).unwrap(),
        CliCommand::Msg(MsgArgs {
            socket: None,
            command: IpcCommand::Osd {
                action: OsdCommand::Show {
                    payload: OsdPayload::Workspace {
                        label: "3".to_owned(),
                    },
                    output: None,
                },
            },
            json: false,
        })
    );

    assert_eq!(
        parse_args(["msg", "osd", "battery", "15"]).unwrap(),
        CliCommand::Msg(MsgArgs {
            socket: None,
            command: IpcCommand::Osd {
                action: OsdCommand::Show {
                    payload: OsdPayload::LowBattery { percent: 15 },
                    output: None,
                },
            },
            json: false,
        })
    );

    assert_eq!(
        parse_args(["msg", "osd", "hide"]).unwrap(),
        CliCommand::Msg(MsgArgs {
            socket: None,
            command: IpcCommand::Osd {
                action: OsdCommand::Hide,
            },
            json: false,
        })
    );
}

#[test]
fn rejects_invalid_arguments_cleanly() {
    assert_eq!(parse_args(["msg"]), Err(CliError::MissingMsgSubcommand));
    assert!(matches!(
        parse_args(["--nope"]),
        Err(CliError::UnknownArgument(_))
    ));
    assert!(matches!(
        parse_args(["msg", "osd", "volume", "not-a-number"]),
        Err(CliError::InvalidValue { .. })
    ));
}

#[test]
fn systemd_user_unit_is_shipped_and_bound_to_graphical_session_target() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let unit_path = manifest_dir.join("../../systemd/triton.service");
    let content = std::fs::read_to_string(&unit_path)
        .expect("systemd/triton.service must be shipped in the repository");

    assert!(content.contains("PartOf=graphical-session.target"));
    assert!(content.contains("After=graphical-session.target"));
    assert!(content.contains("WantedBy=graphical-session.target"));
    assert!(content.contains("triton --real-services"));
}
