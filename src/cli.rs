#[derive(Debug, PartialEq)]
pub enum Command {
    Gui,
    DaemonRun,
    DaemonStop,
    SessionsList,
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
nekomata sessions list\n\
nekomata daemon stop\n\
nekomata felis sessions list [--format human|json]\n\
nekomata panes list [--window WINDOW_ID]\n\
nekomata panes attach [PANE_ID] --session SESSION [--daemon SOCKET | --ssh HOST] [--window WINDOW_ID]";

pub fn parse(args: &[String]) -> Result<Command, &'static str> {
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => Ok(Command::Gui),
        ["daemon", "run"] => Ok(Command::DaemonRun),
        ["daemon", "stop"] => Ok(Command::DaemonStop),
        ["sessions", "list"] => Ok(Command::SessionsList),
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
    if ssh.is_some() && daemon.is_some() {
        return Err("--ssh and --daemon cannot be combined yet");
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
            vec![
                "panes",
                "attach",
                "--session",
                "abc",
                "--ssh",
                "dev",
                "--daemon",
                "/tmp/s",
            ],
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
