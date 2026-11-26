//! Holes and the aggregated drill table.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::geom::Point;
use crate::model::{Board, DrillShape, PadType};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HoleKind {
    Via,
    Pad,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum HoleSource {
    Pad { footprint: usize, pad: usize },
    Via { via: usize },
}

/// A drilled hole. `index` is the hole's position in [`holes`] order: pads
/// in footprint and pad file order, then vias in file order. This is the
/// number used by `(hole N)` anchors.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hole {
    pub index: usize,
    /// Hole centre (pad centre plus the rotated drill offset).
    pub position: Point,
    pub shape: DrillShape,
    /// Slot rotation in degrees (the pad's absolute rotation; 0 for vias).
    pub rotation: f64,
    pub plated: bool,
    pub kind: HoleKind,
    pub source: HoleSource,
}

/// One drill tool the way KiCad's drill files group holes (hypothesis from
/// the S1 run on 10.0.8, see `docs/STATUS.md`): by tool diameter, an oval
/// taking its smaller dimension, split by plated / non-plated. Vias and
/// pads of the same diameter share a tool here.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolGroup {
    /// Tool diameter rounded to 1 um, mm.
    pub diameter: f64,
    pub plated: bool,
    pub holes: usize,
    pub slots: usize,
}

/// Holes grouped as KiCad's drill files group them ([`ToolGroup`]); plated
/// first, then by diameter. kdraft's own [`drill_table`] is finer (slots
/// and vias get rows of their own).
pub fn kicad_tool_groups(holes: &[Hole]) -> Vec<ToolGroup> {
    let mut m: BTreeMap<(bool, i64), (usize, usize)> = BTreeMap::new();
    for h in holes {
        let um = (h.shape.tool_diameter() * 1000.0).round() as i64;
        let e = m.entry((!h.plated, um)).or_default();
        e.0 += 1;
        if matches!(h.shape, DrillShape::Oval { .. }) {
            e.1 += 1;
        }
    }
    m.into_iter()
        .map(|((np, um), (holes, slots))| ToolGroup {
            diameter: um as f64 / 1000.0,
            plated: !np,
            holes,
            slots,
        })
        .collect()
}

/// All holes of the board. Pads other than `thru_hole` and `np_thru_hole`,
/// and drills of zero size, produce no hole.
pub fn holes(board: &Board) -> Vec<Hole> {
    let mut out = Vec::new();
    for (fi, fp) in board.footprints.iter().enumerate() {
        for (pi, pad) in fp.pads.iter().enumerate() {
            let plated = match pad.kind {
                PadType::ThruHole => true,
                PadType::NpThruHole => false,
                _ => continue,
            };
            let Some(drill) = pad.drill else { continue };
            if drill.shape.tool_diameter() <= 0.0 {
                continue;
            }
            // UNVERIFIED: the drill offset is taken in the pad's rotated frame,
            // matching KiCad's PAD::ShapePos(); not cross-checked on a file.
            let position = pad.position + drill.offset.rotate_kicad(pad.rotation);
            out.push(Hole {
                index: out.len(),
                position,
                shape: drill.shape,
                rotation: pad.rotation,
                plated,
                kind: HoleKind::Pad,
                source: HoleSource::Pad {
                    footprint: fi,
                    pad: pi,
                },
            });
        }
    }
    for (vi, via) in board.vias.iter().enumerate() {
        if via.drill <= 0.0 {
            continue;
        }
        out.push(Hole {
            index: out.len(),
            position: via.position,
            shape: DrillShape::Round {
                diameter: via.drill,
            },
            rotation: 0.0,
            plated: true,
            kind: HoleKind::Via,
            source: HoleSource::Via { via: vi },
        });
    }
    out
}

/// One drill table row.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DrillRow {
    /// Round, or oval with `width <= height` (slot orientation ignored).
    pub shape: DrillShape,
    pub plated: bool,
    pub kind: HoleKind,
    pub count: usize,
}

/// Sizes are grouped at 1 nm resolution.
fn nm(v: f64) -> i64 {
    (v * 1e6).round() as i64
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct RowKey {
    non_plated: bool,
    primary: i64,
    oval: bool,
    secondary: i64,
    kind: HoleKind,
}

fn key(h: &Hole) -> (RowKey, DrillShape) {
    match h.shape {
        DrillShape::Round { diameter } => (
            RowKey {
                non_plated: !h.plated,
                primary: nm(diameter),
                oval: false,
                secondary: 0,
                kind: h.kind,
            },
            h.shape,
        ),
        DrillShape::Oval { width, height } => {
            let (a, b) = if width <= height {
                (width, height)
            } else {
                (height, width)
            };
            (
                RowKey {
                    non_plated: !h.plated,
                    primary: nm(a),
                    oval: true,
                    secondary: nm(b),
                    kind: h.kind,
                },
                DrillShape::Oval {
                    width: a,
                    height: b,
                },
            )
        }
    }
}

/// Groups holes by size, plating and via/pad. Rows are ordered plated
/// first, then by tool diameter, round before oval, then slot length, then
/// vias before pads.
pub fn drill_table(holes: &[Hole]) -> Vec<DrillRow> {
    let mut groups: BTreeMap<RowKey, DrillRow> = BTreeMap::new();
    for h in holes {
        let (k, shape) = key(h);
        groups
            .entry(k)
            .or_insert(DrillRow {
                shape,
                plated: h.plated,
                kind: h.kind,
                count: 0,
            })
            .count += 1;
    }
    groups.into_values().collect()
}

/// For each hole (in [`holes`] order), the index of its row in
/// [`drill_table`] of the same holes.
pub fn drill_row_index(holes: &[Hole]) -> Vec<usize> {
    let keys: Vec<RowKey> = holes.iter().map(|h| key(h).0).collect();
    let mut sorted = keys.clone();
    sorted.sort();
    sorted.dedup();
    keys.iter()
        .map(|k| sorted.binary_search(k).unwrap_or(0))
        .collect()
}

/// Order-independent comparison helper for hole sets: sorted list of
/// (x nm, y nm, shape key, plated).
pub(crate) fn hole_signature(holes: &[Hole]) -> Vec<(i64, i64, i64, i64, bool)> {
    let mut v: Vec<_> = holes
        .iter()
        .map(|h| {
            let (k, _) = key(h);
            (
                nm(h.position.x),
                nm(h.position.y),
                k.primary,
                k.secondary,
                h.plated,
            )
        })
        .collect();
    v.sort_unstable();
    v
}
