//! The workspace roster as `nekomata workspaces list` prints it.
use crate::daemon::Summary;

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
    fn the_table_names_each_workspace_and_its_window() {
        assert_eq!(
            human(&[summary(1, Some(2), 3, 4), summary(5, None, 1, 1)]),
            "WORKSPACE\tWINDOW\tTABS\tPANES\n\
             00000000000000000000000000000001\t2\t3\t4\n\
             00000000000000000000000000000005\t-\t1\t1"
        );
    }
}
