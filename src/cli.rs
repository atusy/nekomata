#[derive(Debug, PartialEq)]
pub enum Command {
    Gui,
    Doctor {
        json: bool,
    },
    DaemonRun,
    DaemonStop,
    WorkspacesList,
    Workspace {
        window: Option<u32>,
        workspace: Option<u64>,
    },
    List(Vec<String>),
    Help,
    Pane {
        window: Option<u32>,
        action: PaneAction,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum PaneAction {
    List,
    Attach {
        pane: Option<u64>,
        daemon: Option<String>,
        ssh: Option<String>,
        session: String,
    },
}

pub const HELP: &str = "nekomata\n\
nekomata workspaces list\n\
nekomata workspaces new [--window WINDOW_ID]\n\
nekomata workspaces attach WORKSPACE_ID [--window WINDOW_ID]\n\
nekomata doctor [--json]\n\
nekomata daemon stop\n\
nekomata felis sessions list [--format human|json]\n\
nekomata panes list [--window WINDOW_ID]\n\
nekomata panes attach [PANE_ID] --session SESSION [--ssh HOST] [--daemon SOCKET] [--window WINDOW_ID]";

/// Former name of `workspaces`, kept for scripts written before the rename.
const DEPRECATED_WORKSPACES: &str = "sessions";

pub(crate) fn is_deprecated(args: &[String]) -> bool {
    args.first().is_some_and(|arg| arg == DEPRECATED_WORKSPACES)
}

pub fn parse(args: &[String]) -> Result<Command, &'static str> {
    let mut args: Vec<_> = args.iter().map(String::as_str).collect();
    if let Some(first) = args.first_mut()
        && *first == DEPRECATED_WORKSPACES
    {
        *first = "workspaces";
    }
    match args.as_slice() {
        [] => Ok(Command::Gui),
        ["doctor"] => Ok(Command::Doctor { json: false }),
        ["doctor", "--json"] => Ok(Command::Doctor { json: true }),
        ["doctor", "--help" | "-h"] => Ok(Command::Help),
        ["daemon", "run"] => Ok(Command::DaemonRun),
        ["daemon", "stop"] => Ok(Command::DaemonStop),
        ["workspaces", "list"] => Ok(Command::WorkspacesList),
        ["workspaces", "--help" | "-h"]
        | ["workspaces", "list" | "new" | "attach", "--help" | "-h"]
        | ["daemon", "--help" | "-h"]
        | ["daemon", "run" | "stop", "--help" | "-h"] => Ok(Command::Help),
        ["workspaces", verb @ ("new" | "attach"), options @ ..] => parse_workspace(verb, options),
        ["--help" | "-h"]
        | ["felis", "--help" | "-h"]
        | ["felis", "sessions", "--help" | "-h"]
        | ["felis", "sessions", "list", "--help" | "-h"] => Ok(Command::Help),
        ["felis", "sessions", "list", options @ ..] => {
            Ok(Command::List(options.iter().map(|s| (*s).into()).collect()))
        }
        ["panes", "--help" | "-h"] | ["panes", "list" | "attach", "--help" | "-h"] => {
            Ok(Command::Help)
        }
        ["panes", verb @ ("list" | "attach"), options @ ..] => parse_pane(verb, options),
        _ => Err("unknown command"),
    }
}

fn parse_workspace(verb: &str, options: &[&str]) -> Result<Command, &'static str> {
    let mut window = None;
    let mut workspace = None;
    let mut args = options.iter().copied();
    while let Some(arg) = args.next() {
        match arg {
            "--window" if window.is_none() => {
                window = Some(
                    args.next()
                        .ok_or("missing window ID")?
                        .parse()
                        .map_err(|_| "invalid window ID")?,
                );
            }
            id if verb == "attach" && workspace.is_none() && !id.starts_with('-') => {
                workspace = Some(id.parse().map_err(|_| "invalid workspace ID")?);
            }
            _ => return Err("unknown or repeated workspace argument"),
        }
    }
    if verb == "attach" && workspace.is_none() {
        return Err("workspace ID is required");
    }
    Ok(Command::Workspace { window, workspace })
}

fn parse_pane(verb: &str, options: &[&str]) -> Result<Command, &'static str> {
    let mut window = None;
    let mut pane = None;
    let mut daemon = None;
    let mut ssh = None;
    let mut session = None;
    let mut args = options.iter().copied();
    while let Some(arg) = args.next() {
        match arg {
            "--window" if window.is_none() => {
                window = Some(
                    args.next()
                        .ok_or("missing window ID")?
                        .parse::<u32>()
                        .map_err(|_| "invalid window ID")?,
                );
            }
            "--session" if verb == "attach" && session.is_none() => {
                session = Some(
                    args.next()
                        .filter(|s| !s.is_empty())
                        .ok_or("missing session ID")?
                        .to_owned(),
                )
            }
            "--daemon" if verb == "attach" && daemon.is_none() => {
                daemon = Some(
                    args.next()
                        .filter(|s| !s.is_empty())
                        .ok_or("missing daemon socket")?
                        .to_owned(),
                )
            }
            "--ssh" if verb == "attach" && ssh.is_none() => {
                let host = args
                    .next()
                    .filter(|s| !s.is_empty() && !s.starts_with('-'))
                    .ok_or("invalid SSH host")?;
                ssh = Some(host.to_owned());
            }
            id if verb == "attach" && !id.starts_with('-') && pane.is_none() => {
                pane = Some(id.parse::<u64>().map_err(|_| "invalid pane ID")?);
            }
            _ => return Err("unknown or repeated pane argument"),
        }
    }
    Ok(Command::Pane {
        window,
        action: if verb == "list" {
            PaneAction::List
        } else {
            PaneAction::Attach {
                pane,
                daemon,
                ssh,
                session: session.ok_or("--session is required")?,
            }
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctor_accepts_human_and_json_output_but_rejects_unknown_flags() {
        for (args, json) in [(vec!["doctor"], false), (vec!["doctor", "--json"], true)] {
            assert_eq!(
                parse(&args.into_iter().map(String::from).collect::<Vec<_>>()),
                Ok(Command::Doctor { json })
            );
        }
        assert!(parse(&["doctor", "--verbose"].map(String::from)).is_err());
    }

    #[test]
    fn ssh_attach_accepts_a_remote_daemon_socket() {
        let args = [
            "panes",
            "attach",
            "--ssh",
            "dev",
            "--daemon",
            "relative socket",
            "--session",
            "abc",
        ]
        .map(String::from);
        assert_eq!(
            parse(&args),
            Ok(Command::Pane {
                window: None,
                action: PaneAction::Attach {
                    pane: None,
                    daemon: Some("relative socket".into()),
                    ssh: Some("dev".into()),
                    session: "abc".into()
                },
            })
        );
    }

    #[test]
    fn workspace_switch_commands_select_the_destination_and_window() {
        assert_eq!(
            parse(&["workspaces", "list"].map(String::from)),
            Ok(Command::WorkspacesList)
        );
        assert_eq!(
            parse(&["workspaces", "new"].map(String::from)),
            Ok(Command::Workspace {
                window: None,
                workspace: None
            })
        );
        assert_eq!(
            parse(&["workspaces", "attach", "7", "--window", "42"].map(String::from)),
            Ok(Command::Workspace {
                window: Some(42),
                workspace: Some(7)
            })
        );
        for args in [
            vec!["workspaces", "attach"],
            vec!["workspaces", "attach", "bad"],
            vec!["workspaces", "attach", "1", "2"],
            vec!["workspaces", "new", "2"],
            vec!["workspaces", "new", "--window"],
            vec!["workspaces", "new", "--window", "1", "--window", "2"],
        ] {
            let args: Vec<_> = args.into_iter().map(String::from).collect();
            assert!(parse(&args).is_err(), "{args:?}");
        }
        for command in ["list", "new", "attach"] {
            assert_eq!(
                parse(&["workspaces", command, "--help"].map(String::from)),
                Ok(Command::Help)
            );
        }
        assert!(HELP.contains("nekomata workspaces attach WORKSPACE_ID"));
        assert!(!HELP.contains("nekomata sessions"));
    }

    #[test]
    fn deprecated_sessions_commands_still_manage_workspaces() {
        let args = |args: &[&str]| args.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        for command in [
            vec!["list"],
            vec!["new"],
            vec!["attach", "7", "--window", "42"],
            vec!["--help"],
            vec!["attach", "-h"],
            vec!["attach"],
            vec!["detach", "7"],
        ] {
            let legacy = args(&[&["sessions"], command.as_slice()].concat());
            let current = args(&[&["workspaces"], command.as_slice()].concat());
            assert_eq!(parse(&legacy), parse(&current), "{command:?}");
            assert!(is_deprecated(&legacy));
            assert!(!is_deprecated(&current));
        }
        assert!(!is_deprecated(&args(&["felis", "sessions", "list"])));
    }

    #[test]
    fn panes_list_selects_optional_window() {
        assert_eq!(
            parse(&["panes", "list", "--window", "42"].map(String::from)),
            Ok(Command::Pane {
                window: Some(42),
                action: PaneAction::List,
            })
        );
    }

    #[test]
    fn panes_commands_offer_help() {
        for command in [
            vec!["panes"],
            vec!["panes", "list"],
            vec!["panes", "attach"],
        ] {
            for flag in ["--help", "-h"] {
                let args = command
                    .iter()
                    .copied()
                    .chain([flag])
                    .map(String::from)
                    .collect::<Vec<_>>();
                assert_eq!(parse(&args), Ok(Command::Help));
            }
        }
        assert!(HELP.contains("nekomata panes list"));
        assert!(HELP.contains("nekomata panes attach"));
        assert!(!HELP.contains("nekomata pane "));
    }

    #[test]
    fn singular_pane_command_is_rejected() {
        for command in [
            vec!["pane", "list"],
            vec!["pane", "attach", "--session", "abc"],
            vec!["pane", "--help"],
        ] {
            let args = command.into_iter().map(String::from).collect::<Vec<_>>();
            assert_eq!(parse(&args), Err("unknown command"));
        }
    }

    #[test]
    fn panes_attach_selects_optional_pane_and_connection() {
        let parse_args =
            |args: &[&str]| parse(&args.iter().map(|s| (*s).into()).collect::<Vec<_>>());
        assert_eq!(
            parse_args(&[
                "panes",
                "attach",
                "3",
                "--session",
                "abc",
                "--ssh",
                "dev",
                "--window",
                "42"
            ]),
            Ok(Command::Pane {
                window: Some(42),
                action: PaneAction::Attach {
                    pane: Some(3),
                    session: "abc".into(),
                    ssh: Some("dev".into()),
                    daemon: None
                }
            })
        );
        assert_eq!(
            parse_args(&["panes", "attach", "--session", "abc"]),
            Ok(Command::Pane {
                window: None,
                action: PaneAction::Attach {
                    pane: None,
                    session: "abc".into(),
                    ssh: None,
                    daemon: None
                }
            })
        );
        for args in [
            vec!["panes", "attach"],
            vec!["panes", "attach", "--session", "abc", "--ssh", "-oops"],
            vec!["panes", "attach", "0", "1", "--session", "abc"],
        ] {
            assert!(parse_args(&args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn session_listing_requires_the_felis_namespace() {
        let args = |args: &[&str]| args.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            parse(&args(&["felis", "sessions", "list"])),
            Ok(Command::List(vec![]))
        );
        assert_eq!(
            parse(&args(&["felis", "sessions", "list", "--format", "json"])),
            Ok(Command::List(vec!["--format".into(), "json".into()]))
        );
        for command in [
            vec!["sessions", "attach"],
            vec!["sessions", "detach", "abc"],
            vec!["--attach", "abc"],
            vec!["felis", "sessions", "attach"],
            vec!["felis", "sessions", "detach", "abc"],
        ] {
            assert!(parse(&args(&command)).is_err(), "{command:?}");
        }
    }
}
