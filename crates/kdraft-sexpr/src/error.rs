use std::fmt;

/// An error with its position in the source text.
///
/// `line` and `column` are 1-based; `column` counts Unicode scalar values.
/// Errors from owned [`crate::Value`] trees carry no position and report
/// offset 0, line 0, column 0.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub struct Error {
    pub kind: ErrorKind,
    pub offset: usize,
    pub line: usize,
    pub column: usize,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(f, "{}", self.kind)
        } else {
            write!(
                f,
                "{} at {}:{} (byte {})",
                self.kind, self.line, self.column, self.offset
            )
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErrorKind {
    #[error("unexpected ')'")]
    UnexpectedClose,
    #[error("'(' at byte {open} is never closed")]
    UnclosedList { open: usize },
    #[error("unterminated string")]
    UnterminatedString,
    #[error("nesting deeper than {limit} levels")]
    TooDeep { limit: usize },
    #[error("input larger than 4 GiB is not supported")]
    TooLarge,
    #[error("expected a list")]
    ExpectedList,
    #[error("expected an atom at index {index}")]
    ExpectedAtom { index: usize },
    #[error("missing element at index {index}")]
    MissingChild { index: usize },
    #[error("missing ({head} ...)")]
    MissingList { head: String },
    #[error("invalid number {text:?}")]
    InvalidNumber { text: String },
    #[error("number is not finite")]
    NonFinite,
}

impl Error {
    pub(crate) fn unpositioned(kind: ErrorKind) -> Self {
        Error {
            kind,
            offset: 0,
            line: 0,
            column: 0,
        }
    }

    pub(crate) fn at(text: &str, offset: usize, kind: ErrorKind) -> Self {
        let (line, column) = line_col(text, offset);
        Error {
            kind,
            offset,
            line,
            column,
        }
    }
}

/// 1-based line and column of a byte offset. Lines break at `\n`, so CRLF
/// counts as one break. Offsets inside a multi-byte character resolve to
/// that character.
pub fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(text.len());
    let before = &text.as_bytes()[..offset];
    let line = before.iter().filter(|&&b| b == b'\n').count() + 1;
    let line_start = before
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |p| p + 1);
    // Count characters by counting non-continuation bytes.
    let column = before[line_start..]
        .iter()
        .filter(|&&b| (b & 0xC0) != 0x80)
        .count()
        + 1;
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_counts_lf_crlf_and_utf8() {
        let t = "ab\r\ncd\nµx";
        assert_eq!(line_col(t, 0), (1, 1));
        assert_eq!(line_col(t, 1), (1, 2));
        assert_eq!(line_col(t, 4), (2, 1));
        assert_eq!(line_col(t, 7), (3, 1));
        // 'x' after the two-byte 'µ'
        assert_eq!(line_col(t, 9), (3, 2));
        assert_eq!(line_col(t, 100), (3, 3));
    }
}
