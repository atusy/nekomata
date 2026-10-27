//! Public commands: one definition publishes both a keymap kind and a palette entry.
use crate::{
    layout::{Axis, Direction},
    shortcuts::Action,
};

/// The closed set of label prefixes; see the ADR
/// name-palette-commands-by-category-and-verb-phrase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Category {
    CommandPalette,
    Workspace,
    Tab,
    Pane,
    Session,
    Scrollback,
    Clipboard,
    Font,
}

impl Category {
    fn name(self) -> &'static str {
        match self {
            Self::CommandPalette => "Command palette",
            Self::Workspace => "Workspace",
            Self::Tab => "Tab",
            Self::Pane => "Pane",
            Self::Session => "Session",
            Self::Scrollback => "Scrollback",
            Self::Clipboard => "Clipboard",
            Self::Font => "Font",
        }
    }
}

#[derive(Clone)]
pub(crate) struct Command {
    pub kind: &'static str,
    pub visible: bool,
    category: Category,
    phrase: &'static str,
    pub action: Action,
    pub detail: Option<&'static str>,
    /// Words that find the command without appearing in it.
    keywords: &'static [&'static str],
}

impl Command {
    const fn new(
        kind: &'static str,
        category: Category,
        phrase: &'static str,
        action: Action,
    ) -> Self {
        Self {
            kind,
            visible: true,
            category,
            phrase,
            action,
            detail: None,
            keywords: &[],
        }
    }

    const fn hidden(mut self) -> Self {
        self.visible = false;
        self
    }

    const fn keywords(mut self, keywords: &'static [&'static str]) -> Self {
        self.keywords = keywords;
        self
    }

    pub fn matches(&self, words: &[String]) -> bool {
        let text = format!("{} {}", self.label(), self.keywords.join(" ")).to_lowercase();
        words.iter().all(|word| text.contains(word.as_str()))
    }

    const fn detail(mut self, detail: &'static str) -> Self {
        self.detail = Some(detail);
        self
    }

    pub fn label(&self) -> String {
        format!("{}: {}", self.category.name(), self.phrase)
    }
}

pub(crate) fn commands() -> &'static [Command] {
    use Category::*;
    use felis_client_core::{Action::Scroll, Action::ScrollToPrompt, ScrollStep};
    use felis_protocol::messages::PromptJump;
    const COMMANDS: &[Command] = &[
        Command::new(
            "workspace:create",
            Workspace,
            "Create",
            Action::NewWorkspace,
        )
        .keywords(&["new"]),
        Command::new(
            "workspace:switch",
            Workspace,
            "Switch…",
            Action::OpenWorkspaces,
        )
        .keywords(&["open", "list"]),
        Command::new("tab:create", Tab, "Create", Action::NewTab).keywords(&["new"]),
        Command::new("tab:switch_next", Tab, "Switch to next", Action::NextTab),
        Command::new(
            "tab:switch_previous",
            Tab,
            "Switch to previous",
            Action::PreviousTab,
        ),
        Command::new("tab:close", Tab, "Close", Action::CloseTab)
            .detail("Detaches its sessions; the shells keep running")
            .keywords(&["detach"]),
        Command::new(
            "pane:split_right",
            Pane,
            "Split right",
            Action::Split(Axis::Horizontal),
        )
        .keywords(&["side by side"]),
        Command::new(
            "pane:split_below",
            Pane,
            "Split below",
            Action::Split(Axis::Vertical),
        )
        .keywords(&["down"]),
        Command::new("pane:focus_next", Pane, "Focus next", Action::NextPane),
        Command::new(
            "pane:focus_left",
            Pane,
            "Focus left",
            Action::Focus(Direction::Left),
        ),
        Command::new(
            "pane:focus_down",
            Pane,
            "Focus down",
            Action::Focus(Direction::Down),
        ),
        Command::new(
            "pane:focus_up",
            Pane,
            "Focus up",
            Action::Focus(Direction::Up),
        ),
        Command::new(
            "pane:focus_right",
            Pane,
            "Focus right",
            Action::Focus(Direction::Right),
        ),
        Command::new("pane:close", Pane, "Close", Action::ClosePane)
            .detail("Detaches its session; the shell keeps running")
            .keywords(&["detach"]),
        Command::new("session:attach", Session, "Attach…", Action::AttachSession)
            .detail("Opens a local daemon session in a new tab")
            .keywords(&["picker", "open"]),
        Command::new("scrollback:find", Scrollback, "Find…", Action::Search)
            .keywords(&["search", "grep"]),
        Command::new(
            "scrollback:jump_next_prompt",
            Scrollback,
            "Jump to next prompt",
            Action::Terminal(ScrollToPrompt(PromptJump::Next)),
        )
        .detail("Needs a shell that marks prompts with OSC 133"),
        Command::new(
            "scrollback:jump_previous_prompt",
            Scrollback,
            "Jump to previous prompt",
            Action::Terminal(ScrollToPrompt(PromptJump::Previous)),
        )
        .detail("Needs a shell that marks prompts with OSC 133"),
        Command::new(
            "scrollback:scroll_line_up",
            Scrollback,
            "Scroll one line up",
            Action::Terminal(Scroll(ScrollStep::LineUp)),
        ),
        Command::new(
            "scrollback:scroll_line_down",
            Scrollback,
            "Scroll one line down",
            Action::Terminal(Scroll(ScrollStep::LineDown)),
        ),
        Command::new(
            "scrollback:scroll_half_page_up",
            Scrollback,
            "Scroll half a page up",
            Action::Terminal(Scroll(ScrollStep::HalfPageUp)),
        ),
        Command::new(
            "scrollback:scroll_half_page_down",
            Scrollback,
            "Scroll half a page down",
            Action::Terminal(Scroll(ScrollStep::HalfPageDown)),
        ),
        Command::new(
            "scrollback:scroll_to_top",
            Scrollback,
            "Scroll to top",
            Action::Terminal(Scroll(ScrollStep::Home)),
        )
        .keywords(&["oldest", "home"]),
        Command::new(
            "scrollback:scroll_to_bottom",
            Scrollback,
            "Scroll to bottom",
            Action::Terminal(Scroll(ScrollStep::End)),
        )
        .keywords(&["live", "end"]),
        Command::new(
            "clipboard:copy_selection",
            Clipboard,
            "Copy selection",
            Action::Copy,
        ),
        Command::new("clipboard:paste", Clipboard, "Paste", Action::Paste),
        Command::new(
            "font:change_size",
            Font,
            "Change size…",
            Action::ChangeFontSize,
        )
        .keywords(&["zoom", "text"]),
        Command::new(
            "font:increase_size",
            Font,
            "Increase size",
            Action::FontScale(crate::font_size::STEP),
        )
        .keywords(&["zoom", "larger", "text"]),
        Command::new(
            "font:decrease_size",
            Font,
            "Decrease size",
            Action::FontScale(1.0 / crate::font_size::STEP),
        )
        .keywords(&["zoom", "smaller", "text"]),
        Command::new("font:reset_size", Font, "Reset size", Action::FontReset)
            .keywords(&["zoom", "text"]),
        Command::new(
            "command_palette:open",
            CommandPalette,
            "Open",
            Action::Palette,
        )
        .hidden(),
    ];
    COMMANDS
}
