//! Startup's report of what was wrong in either configuration document.

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
}
