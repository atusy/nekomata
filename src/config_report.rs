//! Startup's report of what was wrong in either configuration document.
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The document is ignored; its layer uses every default.
    Error,
    /// One setting was skipped or adjusted; the rest of the document applies.
    Warning,
}

/// One problem a document had when startup read it.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub severity: Severity,
    pub source: PathBuf,
    /// Unescaped, as the document's reader produced it.
    pub message: String,
}

impl Problem {
    pub fn error(source: impl AsRef<Path>, message: impl Into<String>) -> Self {
        Self::new(Severity::Error, source, message)
    }

    pub fn warning(source: impl AsRef<Path>, message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, source, message)
    }

    fn new(severity: Severity, source: impl AsRef<Path>, message: impl Into<String>) -> Self {
        Self {
            severity,
            source: source.as_ref().to_path_buf(),
            message: message.into(),
        }
    }
}

/// Escaped, so the document cannot drive the terminal showing it.
impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let source = printable(&self.source.display().to_string());
        let message = printable(&self.message);
        match self.severity {
            Severity::Error => write!(f, "{source} is ignored: {message}"),
            Severity::Warning => write!(f, "{source}: {message}"),
        }
    }
}

/// Loads the terminal layer, then the application layer, keeping every
/// problem of both: a broken document never stops the other's load.
pub fn load_both<T, A>(
    terminal: impl FnOnce() -> (T, Vec<Problem>),
    app: impl FnOnce() -> (A, Vec<Problem>),
) -> (T, A, Vec<Problem>) {
    let (terminal, mut problems) = terminal();
    let (app, app_problems) = app();
    problems.extend(app_problems);
    (terminal, app, problems)
}

/// Escapes control characters, which TOML strings and quoted keys can
/// carry, so a config file cannot drive the terminal showing stderr.
/// Line breaks and tabs stay: they cannot, and toml's parse errors lay
/// out a source excerpt with them.
pub fn printable(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() && !matches!(c, '\n' | '\t') {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_characters_are_escaped_but_layout_and_unicode_stay() {
        assert_eq!(printable("a\nb\tc\u{1b}"), "a\nb\tc\\u{1b}");
        assert_eq!(printable("keymap.\"alt+x\" ✓"), "keymap.\"alt+x\" ✓");
    }

    #[test]
    fn an_error_says_its_document_is_ignored_and_a_warning_names_its_document() {
        let error = Problem::error("/c/config.toml", "TOML parse error");
        let warning = Problem::warning("/c/config.toml", "unknown key `x`");
        assert_eq!(
            error.to_string(),
            "/c/config.toml is ignored: TOML parse error"
        );
        assert_eq!(warning.to_string(), "/c/config.toml: unknown key `x`");
    }

    #[test]
    fn both_layers_report_their_errors_terminal_layer_first() {
        let (_, _, problems) = load_both(
            || ((), vec![Problem::error("/felis.toml", "broken")]),
            || ((), vec![Problem::error("/nekomata.toml", "broken")]),
        );
        assert_eq!(
            problems,
            [
                Problem::error("/felis.toml", "broken"),
                Problem::error("/nekomata.toml", "broken"),
            ]
        );
    }

    #[test]
    fn shown_problems_carry_no_control_characters() {
        let problem = Problem::warning("/c/\u{7}.toml", "key `\u{1b}]0;x`");
        let shown = problem.to_string();
        assert!(!shown.chars().any(char::is_control), "{shown}");
        assert!(shown.contains("\\u{1b}]0;x"), "{shown}");
        assert!(shown.contains("\\u{7}"), "{shown}");
    }
}
