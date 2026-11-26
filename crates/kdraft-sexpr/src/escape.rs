use std::borrow::Cow;

/// Decodes the body of a quoted string (without the quotes).
///
/// Recognised escapes: `\"`, `\\`, `\n`, `\r`, `\t`. Any other backslash
/// sequence is kept verbatim, backslash included.
// UNVERIFIED: KiCad's DSNLEXER also decodes octal/hex escapes; KiCad 9/10
// writers only emit the five above, so the rest are kept as-is.
pub fn unescape(body: &str) -> Cow<'_, str> {
    if !body.contains('\\') {
        return Cow::Borrowed(body);
    }
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    Cow::Owned(out)
}

/// Encodes text as the body of a quoted string.
pub fn escape_into(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
}

/// True if `text` cannot be written as a bare atom and read back unchanged.
pub fn needs_quotes(text: &str) -> bool {
    text.is_empty()
        || text
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '(' | ')' | '"' | '\\'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_unescape_inverse() {
        for s in [
            "",
            "plain",
            "a\"b",
            "back\\slash",
            "nl\nx",
            "tab\tcr\r",
            "µΩ",
        ] {
            let mut e = String::new();
            escape_into(s, &mut e);
            assert_eq!(unescape(&e), s);
        }
    }

    #[test]
    fn unknown_escape_kept() {
        assert_eq!(unescape("a\\qb"), "a\\qb");
        assert_eq!(unescape("end\\"), "end\\");
    }

    #[test]
    fn quoting_rules() {
        assert!(needs_quotes(""));
        assert!(needs_quotes("a b"));
        assert!(needs_quotes("a(b"));
        assert!(needs_quotes("a\"b"));
        assert!(needs_quotes("a\\b"));
        assert!(!needs_quotes("F.Cu"));
        assert!(!needs_quotes("-1.5"));
        assert!(!needs_quotes("µ"));
    }
}
