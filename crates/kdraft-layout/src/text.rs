//! Multi-line text: line breaking and block layout.
//!
//! Callers pass a width measure (mm for a string at the block's text
//! height); everything here is font independent.

/// Horizontal alignment of lines within a text block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

impl Align {
    /// Fraction of the width left of the anchor: 0, 0.5, 1.
    pub fn factor(self) -> f64 {
        match self {
            Align::Left => 0.0,
            Align::Center => 0.5,
            Align::Right => 1.0,
        }
    }
}

/// Breaks `text` into lines: at every newline (`\n`, `\r\n`, `\r`), and,
/// with a `wrap` width, greedily between words so that no line is wider
/// than `wrap` unless it is a single word. Wrapped paragraphs collapse
/// runs of spaces; unwrapped lines are kept as they are. An empty text is
/// one empty line.
pub fn break_lines(text: &str, wrap: Option<f64>, measure: &dyn Fn(&str) -> f64) -> Vec<String> {
    let norm = text.replace("\r\n", "\n").replace('\r', "\n");
    let wrap = wrap.filter(|w| w.is_finite() && *w > 0.0);
    let mut out = Vec::new();
    for para in norm.split('\n') {
        let Some(w) = wrap else {
            out.push(para.to_string());
            continue;
        };
        let mut cur = String::new();
        let mut any = false;
        for word in para.split_whitespace() {
            any = true;
            if cur.is_empty() {
                cur = word.to_string();
                continue;
            }
            let cand = format!("{cur} {word}");
            if measure(&cand) <= w {
                cur = cand;
            } else {
                out.push(std::mem::replace(&mut cur, word.to_string()));
            }
        }
        if any {
            out.push(cur);
        } else {
            out.push(String::new());
        }
    }
    out
}

/// A laid-out block: line origins (baseline, left end) relative to the
/// anchor, before rotation; the anchor is on the first baseline.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub lines: Vec<(String, f64, f64)>,
    /// Block width: the wrap width when set, else the widest line.
    pub width: f64,
    /// Unrotated extent relative to the anchor: (min x, min y, max x,
    /// max y), Y down, from the first cap top to the last baseline.
    pub extent: (f64, f64, f64, f64),
}

/// Lays `lines` out at cap height `h` with baseline distance
/// `spacing * h`. The block's left edge is `align.factor() * width` left
/// of the anchor; each line is aligned within the block.
pub fn layout_block(
    lines: Vec<String>,
    wrap: Option<f64>,
    h: f64,
    spacing: f64,
    align: Align,
    measure: &dyn Fn(&str) -> f64,
) -> Block {
    let widths: Vec<f64> = lines.iter().map(|l| measure(l)).collect();
    let widest = widths.iter().copied().fold(0.0_f64, f64::max);
    let width = wrap.filter(|w| *w > 0.0).unwrap_or(widest);
    let left = -align.factor() * width;
    let step = spacing.max(0.1) * h;
    let n = lines.len().max(1);
    let placed: Vec<(String, f64, f64)> = lines
        .into_iter()
        .zip(widths)
        .enumerate()
        .map(|(i, (l, w))| {
            let x = left + align.factor() * (width - w);
            (l, x, i as f64 * step)
        })
        .collect();
    let min_x = placed.iter().map(|p| p.1).fold(left, f64::min);
    let max_x = left + width.max(widest);
    Block {
        lines: placed,
        width,
        extent: (min_x, -h, max_x, (n - 1) as f64 * step),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn m(s: &str) -> f64 {
        s.chars().count() as f64
    }

    #[test]
    fn newlines_and_wrap() {
        assert_eq!(break_lines("a\nb\r\nc", None, &m), vec!["a", "b", "c"]);
        assert_eq!(break_lines("", None, &m), vec![""]);
        assert_eq!(break_lines("a\n\nb", Some(5.0), &m), vec!["a", "", "b"]);
        assert_eq!(
            break_lines("aa bb cc dd", Some(5.0), &m),
            vec!["aa bb", "cc dd"]
        );
        assert_eq!(
            break_lines("verylongword x", Some(5.0), &m),
            vec!["verylongword", "x"]
        );
    }

    #[test]
    fn alignment() {
        let b = layout_block(
            vec!["ab".into(), "abcd".into()],
            None,
            1.0,
            1.5,
            Align::Right,
            &m,
        );
        assert_eq!(b.width, 4.0);
        assert_eq!(b.lines[0].1, -2.0);
        assert_eq!(b.lines[1].1, -4.0);
        assert_eq!(b.lines[1].2, 1.5);
        assert_eq!(b.extent, (-4.0, -1.0, 0.0, 1.5));
        let c = layout_block(vec!["ab".into()], Some(10.0), 1.0, 1.5, Align::Center, &m);
        assert_eq!(c.lines[0].1, -1.0);
    }

    proptest! {
        #[test]
        fn wrapped_lines_fit_unless_single_word(
            words in proptest::collection::vec("[a-zµ°±Ø]{1,12}", 0..30),
            breaks in proptest::collection::vec(any::<bool>(), 0..30),
            w in 1.0f64..40.0,
        ) {
            let mut text = String::new();
            for (i, word) in words.iter().enumerate() {
                if i > 0 {
                    text.push(if breaks.get(i).copied().unwrap_or(false) { '\n' } else { ' ' });
                }
                text.push_str(word);
            }
            let lines = break_lines(&text, Some(w), &m);
            for l in &lines {
                prop_assert!(m(l) <= w || !l.contains(' '), "{l:?} wider than {w}");
            }
            // No word lost or reordered.
            let a: Vec<&str> = text.split_whitespace().collect();
            let joined = lines.join(" ");
            let b: Vec<&str> = joined.split_whitespace().collect();
            prop_assert_eq!(a, b);
            prop_assert!(lines.len() > text.matches('\n').count());
        }
    }
}
