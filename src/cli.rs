#[derive(Debug, PartialEq)]
pub enum Command {
    Gui,
    List(Vec<String>),
    Help,
}

pub const HELP: &str = "nekomata\n\
nekomata felis sessions list [--format human|json]";

pub fn parse(args: &[String]) -> Result<Command, &'static str> {
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => Ok(Command::Gui),
        ["--help" | "-h"]
        | ["felis", "--help" | "-h"]
        | ["felis", "sessions", "--help" | "-h"]
        | ["felis", "sessions", "list", "--help" | "-h"] => Ok(Command::Help),
        ["felis", "sessions", "list", options @ ..] => {
            Ok(Command::List(options.iter().map(|s| (*s).into()).collect()))
        }
        _ => Err("unknown command"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            vec!["sessions", "list"],
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
