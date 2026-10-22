#[derive(Debug, PartialEq)]
pub enum Command {
    Gui { attach: Option<String>, pick: bool },
    List(Vec<String>),
    Detach(Option<String>),
    Help,
}

pub const HELP: &str = "nekomata\n\
nekomata sessions list [--format human|json]\n\
nekomata sessions attach [SESSION_ID]\n\
nekomata sessions detach [SESSION_ID]\n\n\
attach without an ID opens the session picker.\n\
detach without an ID uses FELIS_SESSION_ID and detaches all views of that session.\n\
Sessions keep running after detach.";

pub fn parse(args: &[String]) -> Result<Command, &'static str> {
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => Ok(Command::Gui {
            attach: None,
            pick: false,
        }),
        ["--help" | "-h"]
        | ["sessions", "--help" | "-h"]
        | ["sessions", "list" | "attach" | "detach", "--help" | "-h"] => Ok(Command::Help),
        ["--attach", id] | ["sessions", "attach", id] => Ok(Command::Gui {
            attach: Some((*id).into()),
            pick: false,
        }),
        ["sessions", "attach"] => Ok(Command::Gui {
            attach: None,
            pick: true,
        }),
        ["sessions", "list", options @ ..] => {
            Ok(Command::List(options.iter().map(|s| (*s).into()).collect()))
        }
        ["sessions", "detach"] => Ok(Command::Detach(None)),
        ["sessions", "detach", id] => Ok(Command::Detach(Some((*id).into()))),
        _ => Err("unknown command"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_commands_use_the_sessions_namespace() {
        for (args, expected) in [
            (vec!["sessions", "list"], Command::List(vec![])),
            (
                vec!["sessions", "attach"],
                Command::Gui {
                    attach: None,
                    pick: true,
                },
            ),
            (
                vec!["sessions", "attach", "abc"],
                Command::Gui {
                    attach: Some("abc".into()),
                    pick: false,
                },
            ),
            (vec!["sessions", "detach"], Command::Detach(None)),
            (
                vec!["sessions", "detach", "abc"],
                Command::Detach(Some("abc".into())),
            ),
        ] {
            assert_eq!(
                parse(&args.into_iter().map(String::from).collect::<Vec<_>>()),
                Ok(expected)
            );
        }
    }
}
