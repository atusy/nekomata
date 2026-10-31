//! The workspace roster as the CLI prints it and reads IDs back from it.
use anyhow::{Result, bail};

use crate::daemon::Summary;

/// The workspace whose ID starts with `prefix`, a lowercase hex string.
/// Workspaces other windows hold are candidates too: the switch then
/// reports the workspace as attached instead of as missing.
pub fn resolve(workspaces: &[Summary], prefix: &str) -> Result<u128> {
    let mut matches = workspaces
        .iter()
        .filter(|workspace| format!("{:032x}", workspace.id).starts_with(prefix));
    match (matches.next(), matches.count()) {
        (Some(workspace), 0) => Ok(workspace.id),
        (Some(_), others) => bail!(
            "workspace ID prefix {prefix} matches {} workspaces; type more digits",
            others + 1
        ),
        (None, _) => bail!("no workspace ID starts with {prefix}"),
    }
}

/// One object per workspace under a version key, as felis lists sessions;
/// `name` is omitted when unset.
/// `windows` is an array so that a workspace shown in several windows would
/// not change the shape.
pub fn json(workspaces: &[Summary]) -> String {
    let workspaces: Vec<_> = workspaces
        .iter()
        .map(|workspace| {
            let mut entry = serde_json::json!({
                "id": format!("{:032x}", workspace.id),
                "windows": workspace.window.into_iter().collect::<Vec<_>>(),
                "tabs": workspace.tabs,
                "panes": workspace.panes,
            });
            if let Some(name) = &workspace.name {
                entry["name"] = name.as_str().into();
            }
            entry
        })
        .collect();
    serde_json::json!({"v": 1, "workspaces": workspaces}).to_string()
}

/// Listed IDs are never shorter, so that they rarely change as workspaces
/// come and go; felis shortens session IDs the same way.
const MIN_LISTED_DIGITS: usize = 8;
const ID_DIGITS: usize = 32;
const HIGHLIGHT: &str = "\x1b[1;35m";
/// Bright black, as jj dims the part of a change ID that is not needed.
const DIM: &str = "\x1b[90m";
const RESET: &str = "\x1b[0m";

/// Whether to color output to a stream; see <https://no-color.org>.
pub fn color(terminal: bool, no_color: Option<std::ffi::OsString>) -> bool {
    terminal && no_color.is_none_or(|value| value.is_empty())
}

/// How many leading digits tell `id` apart from every other listed ID.
fn identifying_digits(id: u128, workspaces: &[Summary]) -> usize {
    let shared = workspaces
        .iter()
        .filter(|other| other.id != id)
        .map(|other| (id ^ other.id).leading_zeros() as usize / 4)
        .max()
        .unwrap_or(0);
    (shared + 1).min(ID_DIGITS)
}

/// The table shown to people, one workspace per row in the daemon's order.
/// IDs are cut to one length that keeps all of them distinct; with `color`,
/// each row highlights the shorter prefix that alone identifies it and dims
/// the digits after it.
pub fn human(workspaces: &[Summary], color: bool) -> String {
    let identifying: Vec<_> = workspaces
        .iter()
        .map(|workspace| identifying_digits(workspace.id, workspaces))
        .collect();
    let width = identifying
        .iter()
        .copied()
        .fold(MIN_LISTED_DIGITS, usize::max);
    // NAME comes last: it is the only column of varying width.
    let mut output = String::from("WORKSPACE\tWINDOW\tTABS\tPANES\tNAME\n");
    for (workspace, identifying) in workspaces.iter().zip(identifying) {
        let id = format!("{:032x}", workspace.id);
        let id = if color {
            let rest = &id[identifying..width];
            let rest = if rest.is_empty() {
                String::new()
            } else {
                format!("{DIM}{rest}{RESET}")
            };
            format!("{HIGHLIGHT}{}{RESET}{rest}", &id[..identifying])
        } else {
            id[..width].to_owned()
        };
        output.push_str(&format!(
            "{id}\t{}\t{}\t{}\t{}\n",
            workspace
                .window
                .map(|w| w.to_string())
                .unwrap_or_else(|| "-".into()),
            workspace.tabs,
            workspace.panes,
            workspace.name.as_deref().unwrap_or("-"),
        ));
    }
    output.trim_end().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(id: u128, window: Option<u32>, tabs: usize, panes: usize) -> Summary {
        Summary {
            id,
            window,
            tabs,
            panes,
            last_used: 0,
            instance: String::new(),
            name: None,
        }
    }

    #[test]
    fn a_prefix_names_the_one_workspace_whose_id_starts_with_it() {
        let workspaces = [
            summary(0x3f2a << 112, Some(1), 1, 1),
            summary(0x3f2b << 112, None, 1, 1),
            summary(0x91bc << 112, None, 1, 1),
        ];
        assert_eq!(resolve(&workspaces, "91").unwrap(), 0x91bc << 112);
        assert_eq!(
            resolve(&workspaces, "3f2a").unwrap(),
            0x3f2a << 112,
            "a workspace another window holds still counts, so that attaching \
             reports it as attached rather than as missing"
        );
        assert_eq!(
            resolve(&workspaces, "3f").unwrap_err().to_string(),
            "workspace ID prefix 3f matches 2 workspaces; type more digits"
        );
        assert_eq!(
            resolve(&workspaces, "a0").unwrap_err().to_string(),
            "no workspace ID starts with a0"
        );
    }

    #[test]
    fn json_lists_full_ids_and_every_window_showing_each_workspace() {
        let output: serde_json::Value = serde_json::from_str(&json(&[
            named("my work", summary(1, Some(2), 3, 4)),
            summary(5, None, 1, 1),
        ]))
        .unwrap();
        assert_eq!(
            output,
            serde_json::json!({"v": 1, "workspaces": [
                {"id": "00000000000000000000000000000001", "name": "my work", "windows": [2], "tabs": 3, "panes": 4},
                {"id": "00000000000000000000000000000005", "windows": [], "tabs": 1, "panes": 1},
            ]})
        );
    }

    fn named(name: &str, workspace: Summary) -> Summary {
        Summary {
            name: Some(name.into()),
            ..workspace
        }
    }

    #[test]
    fn the_table_names_each_workspace_and_its_window() {
        assert_eq!(
            human(
                &[
                    named("my work", summary(1 << 124, Some(2), 3, 4)),
                    summary(5 << 124, None, 1, 1)
                ],
                false
            ),
            "WORKSPACE\tWINDOW\tTABS\tPANES\tNAME\n\
             10000000\t2\t3\t4\tmy work\n\
             50000000\t-\t1\t1\t-"
        );
    }

    #[test]
    fn listed_ids_share_the_length_the_closest_pair_needs_but_at_least_eight() {
        let ids = |workspaces: &[Summary]| {
            human(workspaces, false)
                .lines()
                .skip(1)
                .map(|row| row.split('\t').next().unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&[summary(u128::MAX, None, 1, 1)]), ["ffffffff"]);
        let close = 0x3f2a_91bc_04e7 << 80;
        assert_eq!(
            ids(&[
                summary(close, None, 1, 1),
                summary(close | 1 << 79, None, 1, 1),
                summary(0xa0 << 120, None, 1, 1),
            ]),
            ["3f2a91bc04e70", "3f2a91bc04e78", "a000000000000"]
        );
        assert_eq!(
            ids(&[summary(7, None, 1, 1), summary(6, None, 1, 1)]),
            [
                "00000000000000000000000000000007",
                "00000000000000000000000000000006"
            ]
        );
    }

    #[test]
    fn color_marks_the_prefix_that_identifies_each_workspace_and_dims_the_rest() {
        let table = human(
            &[
                summary(0x3f2a << 112, None, 1, 1),
                summary(0x3f2b << 112, None, 1, 1),
                summary(0x91bc << 112, None, 1, 1),
            ],
            true,
        );
        let ids: Vec<_> = table
            .lines()
            .skip(1)
            .map(|row| row.split('\t').next().unwrap())
            .collect();
        assert_eq!(
            ids,
            [
                "\x1b[1;35m3f2a\x1b[0m\x1b[90m0000\x1b[0m",
                "\x1b[1;35m3f2b\x1b[0m\x1b[90m0000\x1b[0m",
                "\x1b[1;35m9\x1b[0m\x1b[90m1bc0000\x1b[0m",
            ]
        );
    }

    #[test]
    fn color_is_for_terminals_that_have_not_opted_out() {
        assert!(color(true, None));
        assert!(color(true, Some("".into())), "an empty NO_COLOR is unset");
        assert!(!color(true, Some("1".into())));
        assert!(!color(false, None));
    }
}
