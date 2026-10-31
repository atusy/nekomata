//! The workspace roster as `nekomata workspaces list` prints it.
use crate::daemon::Summary;

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
