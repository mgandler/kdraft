//! Callout label numbering: the next label on placement, bumping a label's
//! number, renumbering in reading order.
//!
//! A label's number is its last run of ASCII digits; what comes before it
//! is the prefix, what follows the suffix ("A12" = "A" + 12, "3a" = "" +
//! 3 + "a"). Labels without digits have no number.

use kdraft_board::geom::Point;

use crate::model::{CalloutKind, Document, Item};

/// A label split around its number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelParts<'a> {
    pub prefix: &'a str,
    /// The digits (keeps leading zeros), empty when there is no number.
    pub digits: &'a str,
    pub suffix: &'a str,
}

impl LabelParts<'_> {
    pub fn number(&self) -> Option<u64> {
        self.digits.parse().ok()
    }
}

/// Splits `label` around its last run of digits.
pub fn split(label: &str) -> LabelParts<'_> {
    let b = label.as_bytes();
    let Some(end) = b.iter().rposition(u8::is_ascii_digit).map(|i| i + 1) else {
        return LabelParts {
            prefix: label,
            digits: "",
            suffix: "",
        };
    };
    let start = b[..end]
        .iter()
        .rposition(|c| !c.is_ascii_digit())
        .map_or(0, |i| i + 1);
    LabelParts {
        prefix: &label[..start],
        digits: &label[start..end],
        suffix: &label[end..],
    }
}

/// `prefix` + `n` (zero-padded to `width`) + `suffix`.
fn join(prefix: &str, n: u64, width: usize, suffix: &str) -> String {
    format!("{prefix}{n:0width$}{suffix}")
}

/// The label after `existing` (in placement order, most recent last):
/// one more than the highest number among the labels with the same prefix
/// and suffix as the most recent one; "1" when there are none.
pub fn next_label(existing: &[String]) -> String {
    let Some(last) = existing.last() else {
        return "1".to_string();
    };
    let lp = split(last);
    let max = existing
        .iter()
        .map(|l| split(l))
        .filter(|p| p.prefix == lp.prefix && p.suffix == lp.suffix)
        .filter_map(|p| p.number())
        .max()
        .unwrap_or(0);
    join(lp.prefix, max.saturating_add(1), lp.digits.len(), lp.suffix)
}

/// `label` with its number changed by `delta` (not below 0). A label
/// without a number gets one appended for a positive delta.
pub fn bump(label: &str, delta: i64) -> String {
    let p = split(label);
    match p.number() {
        Some(n) => {
            let v = (n as i128 + delta as i128).clamp(0, u64::MAX as i128) as u64;
            join(p.prefix, v, p.digits.len(), p.suffix)
        }
        None if delta > 0 => format!("{label}{delta}"),
        None => label.to_string(),
    }
}

/// Label callouts of the numbering scope of page `page` (the page, or the
/// whole document when the document numbers across pages), in placement
/// order: other pages first in page order, then `page` (so its most
/// recent callout is last).
pub fn scope_labels(doc: &Document, page: usize) -> Vec<String> {
    let labels = |i: usize| -> Vec<String> {
        doc.pages
            .get(i)
            .map(|p| {
                p.items
                    .iter()
                    .filter_map(|it| match it {
                        Item::Callout(c) if c.kind() == CalloutKind::Label => c.label.clone(),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut out = Vec::new();
    if doc.styles.callouts_per_document {
        for i in (0..doc.pages.len()).filter(|i| *i != page) {
            out.extend(labels(i));
        }
    }
    out.extend(labels(page));
    out
}

/// The label a new callout on page `page` gets.
pub fn next_on_page(doc: &Document, page: usize) -> String {
    next_label(&scope_labels(doc, page))
}

/// Order of [`reading_order`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadingOrder {
    /// Top to bottom, then left to right within a row.
    Rows,
    /// Left to right, then top to bottom within a column.
    Columns,
}

/// Indices of `points` in reading order; points within `band` mm of a
/// row's (column's) first point count as one row (column).
pub fn reading_order(points: &[Point], order: ReadingOrder, band: f64) -> Vec<usize> {
    type Coord = fn(Point) -> f64;
    let (major, minor): (Coord, Coord) = match order {
        ReadingOrder::Rows => (|p| p.y, |p| p.x),
        ReadingOrder::Columns => (|p| p.x, |p| p.y),
    };
    let mut idx: Vec<usize> = (0..points.len()).collect();
    idx.sort_by(|a, b| {
        major(points[*a])
            .total_cmp(&major(points[*b]))
            .then(minor(points[*a]).total_cmp(&minor(points[*b])))
    });
    let mut out = Vec::with_capacity(idx.len());
    let mut row: Vec<usize> = Vec::new();
    let mut row_start = f64::NAN;
    for i in idx {
        let m = major(points[i]);
        if !row.is_empty() && m - row_start > band {
            row.sort_by(|a, b| minor(points[*a]).total_cmp(&minor(points[*b])));
            out.append(&mut row);
        }
        if row.is_empty() {
            row_start = m;
        }
        row.push(i);
    }
    row.sort_by(|a, b| minor(points[*a]).total_cmp(&minor(points[*b])));
    out.append(&mut row);
    out
}

/// Gaps in the numbers of `labels` with the most common prefix and
/// suffix: the missing numbers between 1 (or the lowest) and the highest.
pub fn gaps(labels: &[String]) -> Vec<u64> {
    let mut nums: Vec<u64> = labels
        .iter()
        .filter_map(|l| {
            let p = split(l);
            (p.prefix.is_empty() && p.suffix.is_empty())
                .then(|| p.number())
                .flatten()
        })
        .collect();
    nums.sort_unstable();
    nums.dedup();
    let Some(&max) = nums.last() else {
        return Vec::new();
    };
    let lo = nums[0].min(1);
    (lo.max(1)..max)
        .filter(|n| nums.binary_search(n).is_err())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: &[&str]) -> Vec<String> {
        x.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn splitting() {
        assert_eq!(split("A12").prefix, "A");
        assert_eq!(split("A12").number(), Some(12));
        assert_eq!(split("3a").number(), Some(3));
        assert_eq!(split("3a").suffix, "a");
        assert_eq!(split("X").number(), None);
        assert_eq!(split("1.2").prefix, "1.");
    }

    #[test]
    fn next_labels() {
        assert_eq!(next_label(&[]), "1");
        assert_eq!(next_label(&v(&["1", "2", "3"])), "4");
        assert_eq!(next_label(&v(&["1", "7", "3"])), "8");
        assert_eq!(next_label(&v(&["5", "A1", "A2"])), "A3");
        assert_eq!(next_label(&v(&["A1", "A2", "9"])), "10");
        assert_eq!(next_label(&v(&["07"])), "08");
        assert_eq!(next_label(&v(&["X"])), "X1");
    }

    #[test]
    fn bumping() {
        assert_eq!(bump("1", 1), "2");
        assert_eq!(bump("A9", 1), "A10");
        assert_eq!(bump("1", -5), "0");
        assert_eq!(bump("3a", 2), "5a");
        assert_eq!(bump("X", 1), "X1");
        assert_eq!(bump("X", -1), "X");
    }

    #[test]
    fn reading() {
        let p = |x, y| Point::new(x, y);
        // Two rows (y 10 and 30), slightly ragged.
        let pts = [p(50.0, 30.0), p(10.0, 11.0), p(30.0, 10.0), p(5.0, 31.0)];
        assert_eq!(
            reading_order(&pts, ReadingOrder::Rows, 3.0),
            vec![1, 2, 3, 0]
        );
        assert_eq!(
            reading_order(&pts, ReadingOrder::Columns, 3.0),
            vec![3, 1, 2, 0]
        );
    }

    #[test]
    fn gap_finding() {
        assert_eq!(gaps(&v(&["1", "2", "4", "7"])), vec![3, 5, 6]);
        assert!(gaps(&v(&["1", "2", "2"])).is_empty());
        assert_eq!(gaps(&v(&["2", "3"])), vec![1]);
    }
}
