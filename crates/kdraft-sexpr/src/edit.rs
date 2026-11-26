//! Minimal text edits on top of the lossless tree: replace byte ranges
//! (an empty range inserts) and keep every other byte.

use std::ops::Range;

/// Replace `range` of the original text with `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub range: Range<usize>,
    pub text: String,
}

impl Edit {
    pub fn insert(at: usize, text: impl Into<String>) -> Edit {
        Edit {
            range: at..at,
            text: text.into(),
        }
    }

    pub fn replace(range: Range<usize>, text: impl Into<String>) -> Edit {
        Edit {
            range,
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    #[error("edit {0:?} is outside the text or not on a character boundary")]
    OutOfRange(Range<usize>),
    #[error("edits {0:?} and {1:?} overlap")]
    Overlap(Range<usize>, Range<usize>),
}

/// Applies non-overlapping edits (any order) to `text`. Returns the new
/// text and the inverse edits, which applied to the new text give `text`
/// back.
pub fn apply(text: &str, edits: &[Edit]) -> Result<(String, Vec<Edit>), EditError> {
    let mut sorted: Vec<&Edit> = edits.iter().collect();
    sorted.sort_by_key(|e| (e.range.start, e.range.end));
    for e in &sorted {
        if e.range.start > e.range.end
            || e.range.end > text.len()
            || !text.is_char_boundary(e.range.start)
            || !text.is_char_boundary(e.range.end)
        {
            return Err(EditError::OutOfRange(e.range.clone()));
        }
    }
    for w in sorted.windows(2) {
        // Two inserts at the same point would be ambiguous too.
        if w[1].range.start < w[0].range.end || (w[0].range == w[1].range && w[0].range.is_empty())
        {
            return Err(EditError::Overlap(w[0].range.clone(), w[1].range.clone()));
        }
    }
    let mut out = String::with_capacity(text.len());
    let mut inverse = Vec::with_capacity(sorted.len());
    let mut pos = 0;
    for e in sorted {
        out.push_str(&text[pos..e.range.start]);
        let start = out.len();
        out.push_str(&e.text);
        inverse.push(Edit {
            range: start..out.len(),
            text: text[e.range.clone()].to_string(),
        });
        pos = e.range.end;
    }
    out.push_str(&text[pos..]);
    Ok((out, inverse))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Tree;
    use proptest::prelude::*;

    #[test]
    fn insert_and_replace() {
        let t = "(a (b 1) (c 2))";
        let tree = Tree::parse(t).unwrap();
        let root = tree.root().unwrap();
        let b = root.find("b").unwrap();
        let c = root.find("c").unwrap();
        let (s, inv) = apply(
            t,
            &[
                Edit::insert(b.span().end, " (hide yes)"),
                Edit::replace(c.span(), "(c 3)"),
            ],
        )
        .unwrap();
        assert_eq!(s, "(a (b 1) (hide yes) (c 3))");
        assert_eq!(apply(&s, &inv).unwrap().0, t);
        assert!(Tree::parse(s.as_str()).is_ok());
        assert!(apply(t, &[Edit::insert(99, "x")]).is_err());
        assert!(apply(t, &[Edit::replace(0..3, "x"), Edit::replace(2..4, "y")]).is_err());
    }

    proptest! {
        #[test]
        fn edit_then_inverse_is_identity(
            text in "[ -~\n]{0,200}",
            raw in proptest::collection::vec((0usize..220, 0usize..6, "[ -~]{0,8}"), 0..8),
        ) {
            // Build non-overlapping edits from random positions.
            let mut edits: Vec<Edit> = Vec::new();
            let mut taken: Vec<Range<usize>> = Vec::new();
            for (s, len, t) in raw {
                let s = s.min(text.len());
                let e = (s + len).min(text.len());
                // Ranges that do not even touch: adjacent edits (a
                // deletion next to an insert) have inverses at the same
                // point, which `apply` rightly rejects as ambiguous.
                if taken.iter().any(|r| s <= r.end && r.start <= e) {
                    continue;
                }
                taken.push(s..e);
                edits.push(Edit::replace(s..e, t));
            }
            let (out, inv) = apply(&text, &edits).unwrap();
            let (back, _) = apply(&out, &inv).unwrap();
            prop_assert_eq!(back, text);
        }
    }
}
