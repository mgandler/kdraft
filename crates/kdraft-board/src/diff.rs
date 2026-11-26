//! Semantic difference between two board models.

use std::collections::HashMap;

use serde::Serialize;

use crate::drill::{hole_signature, holes};
use crate::geom::{normalize_deg, BBox, Point};
use crate::model::{Board, Footprint, Side};
use crate::outline::{EdgeShape, PolyItem};

/// Positions closer than this (mm) count as unchanged.
pub const POSITION_TOLERANCE: f64 = 1e-6;
/// Rotations closer than this (degrees) count as unchanged.
pub const ROTATION_TOLERANCE: f64 = 1e-6;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FootprintId {
    pub reference: Option<String>,
    pub uuid: Option<String>,
    pub lib_id: String,
}

impl FootprintId {
    fn of(fp: &Footprint) -> FootprintId {
        FootprintId {
            reference: fp.reference.clone(),
            uuid: fp.uuid.clone(),
            lib_id: fp.lib_id.clone(),
        }
    }

    pub fn label(&self) -> String {
        match (&self.reference, &self.uuid) {
            (Some(r), _) => r.clone(),
            (None, Some(u)) => format!("<{u}>"),
            (None, None) => format!("<{}>", self.lib_id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum FootprintChange {
    Moved {
        from: Point,
        to: Point,
    },
    Rotated {
        from: f64,
        to: f64,
    },
    Flipped {
        from: Side,
        to: Side,
    },
    Value {
        old: Option<String>,
        new: Option<String>,
    },
    Reference {
        old: Option<String>,
        new: Option<String>,
    },
    LibId {
        old: String,
        new: String,
    },
    Dnp {
        old: bool,
        new: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChangedFootprint {
    pub id: FootprintId,
    pub changes: Vec<FootprintChange>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OutlineChange {
    pub old_bbox: Option<BBox>,
    pub new_bbox: Option<BBox>,
    pub old_primitives: usize,
    pub new_primitives: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HoleChange {
    pub old_count: usize,
    pub new_count: usize,
    /// Holes in the new board without an identical (position, size,
    /// plating) hole in the old one.
    pub added: usize,
    pub removed: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FieldChange {
    pub field: String,
    pub old: Option<String>,
    pub new: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct BoardDiff {
    pub footprints_added: Vec<FootprintId>,
    pub footprints_removed: Vec<FootprintId>,
    pub footprints_changed: Vec<ChangedFootprint>,
    pub outline: Option<OutlineChange>,
    pub holes: Option<HoleChange>,
    pub stackup_changed: bool,
    pub thickness_changed: Option<(Option<f64>, Option<f64>)>,
    pub title_block: Vec<FieldChange>,
}

impl BoardDiff {
    /// True if the boards are semantically identical for kdraft's purposes.
    pub fn is_empty(&self) -> bool {
        self.footprints_added.is_empty()
            && self.footprints_removed.is_empty()
            && self.footprints_changed.is_empty()
            && self.outline.is_none()
            && self.holes.is_none()
            && !self.stackup_changed
            && self.thickness_changed.is_none()
            && self.title_block.is_empty()
    }
}

/// Computes the semantic difference from `old` to `new`.
///
/// Footprints are matched by UUID first; the rest by reference designator
/// (in file order when references repeat).
pub fn diff(old: &Board, new: &Board) -> BoardDiff {
    let mut d = BoardDiff::default();
    diff_footprints(old, new, &mut d);

    let (os, ns) = (outline_signature(old), outline_signature(new));
    if os != ns {
        d.outline = Some(OutlineChange {
            old_bbox: old.outline.bbox,
            new_bbox: new.outline.bbox,
            old_primitives: old.outline.primitives.len(),
            new_primitives: new.outline.primitives.len(),
        });
    }

    let (oh, nh) = (holes(old), holes(new));
    let (osig, nsig) = (hole_signature(&oh), hole_signature(&nh));
    if osig != nsig {
        let (added, removed) = multiset_diff(&nsig, &osig);
        d.holes = Some(HoleChange {
            old_count: oh.len(),
            new_count: nh.len(),
            added,
            removed,
        });
    }

    d.stackup_changed = old.stackup != new.stackup;
    if old.thickness != new.thickness {
        d.thickness_changed = Some((old.thickness, new.thickness));
    }

    let (ot, nt) = (&old.title_block, &new.title_block);
    let mut field = |name: &str, a: &Option<String>, b: &Option<String>| {
        if a != b {
            d.title_block.push(FieldChange {
                field: name.to_string(),
                old: a.clone(),
                new: b.clone(),
            });
        }
    };
    field("title", &ot.title, &nt.title);
    field("date", &ot.date, &nt.date);
    field("rev", &ot.rev, &nt.rev);
    field("company", &ot.company, &nt.company);
    let mut keys: Vec<u32> = ot
        .comments
        .keys()
        .chain(nt.comments.keys())
        .copied()
        .collect();
    keys.sort_unstable();
    keys.dedup();
    for k in keys {
        field(
            &format!("comment {k}"),
            &ot.comments.get(&k).cloned(),
            &nt.comments.get(&k).cloned(),
        );
    }
    d
}

/// Counts of elements in `a` not in `b` and in `b` not in `a` (sorted input).
fn multiset_diff<T: Ord>(a: &[T], b: &[T]) -> (usize, usize) {
    let (mut i, mut j, mut only_a, mut only_b) = (0, 0, 0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => {
                only_a += 1;
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                only_b += 1;
                j += 1;
            }
            std::cmp::Ordering::Equal => {
                i += 1;
                j += 1;
            }
        }
    }
    (only_a + a.len() - i, only_b + b.len() - j)
}

fn diff_footprints(old: &Board, new: &Board, d: &mut BoardDiff) {
    let mut new_matched = vec![false; new.footprints.len()];
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    let mut old_unmatched: Vec<usize> = Vec::new();

    let mut by_uuid: HashMap<&str, usize> = HashMap::new();
    for (i, fp) in new.footprints.iter().enumerate() {
        if let Some(u) = fp.uuid.as_deref() {
            by_uuid.entry(u).or_insert(i);
        }
    }
    for (i, fp) in old.footprints.iter().enumerate() {
        match fp.uuid.as_deref().and_then(|u| by_uuid.get(u)) {
            Some(&j) if !new_matched[j] => {
                new_matched[j] = true;
                pairs.push((i, j));
            }
            _ => old_unmatched.push(i),
        }
    }

    let mut by_ref: HashMap<&str, Vec<usize>> = HashMap::new();
    for (j, fp) in new.footprints.iter().enumerate().rev() {
        if new_matched[j] {
            continue;
        }
        if let Some(r) = fp.reference.as_deref() {
            by_ref.entry(r).or_default().push(j);
        }
    }
    for i in old_unmatched {
        let fp = &old.footprints[i];
        let m = fp
            .reference
            .as_deref()
            .and_then(|r| by_ref.get_mut(r))
            .and_then(|v| v.pop());
        match m {
            Some(j) => {
                new_matched[j] = true;
                pairs.push((i, j));
            }
            None => d.footprints_removed.push(FootprintId::of(fp)),
        }
    }
    for (j, fp) in new.footprints.iter().enumerate() {
        if !new_matched[j] {
            d.footprints_added.push(FootprintId::of(fp));
        }
    }

    pairs.sort_by_key(|&(_, j)| j);
    for (i, j) in pairs {
        let (a, b) = (&old.footprints[i], &new.footprints[j]);
        let mut changes = Vec::new();
        if !a.position.approx_eq(b.position, POSITION_TOLERANCE) {
            changes.push(FootprintChange::Moved {
                from: a.position,
                to: b.position,
            });
        }
        if normalize_deg(a.rotation - b.rotation).abs() > ROTATION_TOLERANCE {
            changes.push(FootprintChange::Rotated {
                from: a.rotation,
                to: b.rotation,
            });
        }
        if a.side != b.side {
            changes.push(FootprintChange::Flipped {
                from: a.side,
                to: b.side,
            });
        }
        if a.value != b.value {
            changes.push(FootprintChange::Value {
                old: a.value.clone(),
                new: b.value.clone(),
            });
        }
        if a.reference != b.reference {
            changes.push(FootprintChange::Reference {
                old: a.reference.clone(),
                new: b.reference.clone(),
            });
        }
        if a.lib_id != b.lib_id {
            changes.push(FootprintChange::LibId {
                old: a.lib_id.clone(),
                new: b.lib_id.clone(),
            });
        }
        if a.attributes.dnp != b.attributes.dnp {
            changes.push(FootprintChange::Dnp {
                old: a.attributes.dnp,
                new: b.attributes.dnp,
            });
        }
        if !changes.is_empty() {
            d.footprints_changed.push(ChangedFootprint {
                id: FootprintId::of(b),
                changes,
            });
        }
    }
}

/// Order-independent outline fingerprint at 1 nm resolution.
fn outline_signature(b: &Board) -> Vec<Vec<i64>> {
    let q = |v: f64| (v * 1e6).round() as i64;
    let qp = |out: &mut Vec<i64>, p: &Point| {
        out.push(q(p.x));
        out.push(q(p.y));
    };
    let mut sig: Vec<Vec<i64>> = b
        .outline
        .primitives
        .iter()
        .map(|p| {
            let mut v = Vec::new();
            match &p.shape {
                EdgeShape::Line { start, end } => {
                    v.push(0);
                    // Direction-independent.
                    let (s, e) = if (q(start.x), q(start.y)) <= (q(end.x), q(end.y)) {
                        (start, end)
                    } else {
                        (end, start)
                    };
                    qp(&mut v, s);
                    qp(&mut v, e);
                }
                EdgeShape::Arc { start, mid, end } => {
                    v.push(1);
                    qp(&mut v, start);
                    qp(&mut v, mid);
                    qp(&mut v, end);
                }
                EdgeShape::Circle { center, radius } => {
                    v.push(2);
                    qp(&mut v, center);
                    v.push(q(*radius));
                }
                EdgeShape::Rect { start, end } => {
                    v.push(3);
                    qp(&mut v, start);
                    qp(&mut v, end);
                }
                EdgeShape::Poly { items } => {
                    v.push(4);
                    for it in items {
                        match it {
                            PolyItem::Point { at } => qp(&mut v, at),
                            PolyItem::Arc { start, mid, end } => {
                                qp(&mut v, start);
                                qp(&mut v, mid);
                                qp(&mut v, end);
                            }
                        }
                    }
                }
                EdgeShape::Curve { points } => {
                    v.push(5);
                    for p in points {
                        qp(&mut v, p);
                    }
                }
            }
            v
        })
        .collect();
    sig.sort_unstable();
    sig
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiset_counts() {
        assert_eq!(multiset_diff(&[1, 2, 2, 3], &[2, 3, 4]), (2, 1));
        assert_eq!(multiset_diff::<i32>(&[], &[1]), (0, 1));
        assert_eq!(multiset_diff(&[1, 2], &[1, 2]), (0, 0));
    }
}
