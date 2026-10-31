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

/// One object per workspace under a version key, as felis lists sessions.
/// `windows` is an array so that a workspace shown in several windows would
/// not change the shape.
pub fn json(workspaces: &[Summary]) -> String {
    let workspaces: Vec<_> = workspaces
        .iter()
        .map(|workspace| {
            serde_json::json!({
                "id": format!("{:032x}", workspace.id),
                "windows": workspace.window.into_iter().collect::<Vec<_>>(),
                "tabs": workspace.tabs,
                "panes": workspace.panes,
            })
        })
        .collect();
    serde_json::json!({"v": 1, "workspaces": workspaces}).to_string()
}

/// The table shown to people, one workspace per row in the daemon's order.
pub fn human(workspaces: &[Summary]) -> String {
    let mut output = String::from("WORKSPACE\tWINDOW\tTABS\tPANES\n");
    for workspace in workspaces {
        output.push_str(&format!(
            "{:032x}\t{}\t{}\t{}\n",
            workspace.id,
            workspace
                .window
                .map(|w| w.to_string())
                .unwrap_or_else(|| "-".into()),
            workspace.tabs,
            workspace.panes
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
        let output: serde_json::Value =
            serde_json::from_str(&json(&[summary(1, Some(2), 3, 4), summary(5, None, 1, 1)]))
                .unwrap();
        assert_eq!(
            output,
            serde_json::json!({"v": 1, "workspaces": [
                {"id": "00000000000000000000000000000001", "windows": [2], "tabs": 3, "panes": 4},
                {"id": "00000000000000000000000000000005", "windows": [], "tabs": 1, "panes": 1},
            ]})
        );
    }

    #[test]
    fn the_table_names_each_workspace_and_its_window() {
        assert_eq!(
            human(&[summary(1, Some(2), 3, 4), summary(5, None, 1, 1)]),
            "WORKSPACE\tWINDOW\tTABS\tPANES\n\
             00000000000000000000000000000001\t2\t3\t4\n\
             00000000000000000000000000000005\t-\t1\t1"
        );
    }
}
