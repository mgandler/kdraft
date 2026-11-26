//! Snapping as a pure function: where a dragged box lands, and which guide
//! lines to show.
//!
//! Per axis, the nearest object target within the tolerance wins: edges and
//! centres of other items, the page and the drawing area, and equal
//! spacing between neighbours. Without an object target the box's top-left
//! corner snaps to the grid if a grid line is within the tolerance. The
//! correction never exceeds the tolerance on either axis.

use kdraft_board::geom::{BBox, Point};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuideKind {
    /// Edge of another item.
    Edge,
    /// Centre of another item.
    Center,
    /// Equal spacing between neighbours (drawn as gap markers).
    Spacing,
    /// Page edge or centre.
    Page,
    /// Drawing-area edge or centre.
    Area,
}

/// A guide line segment in sheet mm.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Guide {
    pub kind: GuideKind,
    pub a: Point,
    pub b: Point,
}

#[derive(Debug, Clone)]
pub struct SnapInput<'a> {
    /// The moving box (union of the dragged items) at its raw position.
    pub moving: BBox,
    /// Boxes of the items that are not moving.
    pub others: &'a [BBox],
    pub page: BBox,
    pub area: Option<BBox>,
    /// Grid step in mm; `None` turns grid snapping off.
    pub grid: Option<f64>,
    /// Object, page and spacing snapping.
    pub objects: bool,
    /// Largest correction per axis, mm.
    pub tolerance: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapped {
    /// Correction to add to the raw position.
    pub delta: Point,
    pub guides: Vec<Guide>,
    /// Whether the grid decided the axis (x, y).
    pub on_grid: (bool, bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    X,
    Y,
}

fn lo(b: &BBox, a: Axis) -> f64 {
    match a {
        Axis::X => b.min.x,
        Axis::Y => b.min.y,
    }
}

fn hi(b: &BBox, a: Axis) -> f64 {
    match a {
        Axis::X => b.max.x,
        Axis::Y => b.max.y,
    }
}

fn mid(b: &BBox, a: Axis) -> f64 {
    (lo(b, a) + hi(b, a)) / 2.0
}

fn other(a: Axis) -> Axis {
    match a {
        Axis::X => Axis::Y,
        Axis::Y => Axis::X,
    }
}

/// A point with coordinate `along` on axis `a` and `across` on the other.
fn pt(a: Axis, along: f64, across: f64) -> Point {
    match a {
        Axis::X => Point::new(along, across),
        Axis::Y => Point::new(across, along),
    }
}

/// Boxes overlap in the cross direction (same row for X, column for Y).
fn same_row(m: &BBox, b: &BBox, a: Axis) -> bool {
    let c = other(a);
    lo(b, c) < hi(m, c) && hi(b, c) > lo(m, c)
}

struct Candidate {
    offset: f64,
    rank: u8,
    guides: Vec<Guide>,
}

fn consider(best: &mut Option<Candidate>, c: Candidate, tol: f64) {
    if !c.offset.is_finite() || c.offset.abs() > tol {
        return;
    }
    let better = match best {
        None => true,
        Some(b) => {
            let (d, e) = (c.offset.abs(), b.offset.abs());
            d < e - 1e-9 || ((d - e).abs() <= 1e-9 && c.rank < b.rank)
        }
    };
    if better {
        *best = Some(c);
    }
}

fn line_guide(kind: GuideKind, a: Axis, at: f64, from: f64, to: f64) -> Guide {
    Guide {
        kind,
        a: pt(a, at, from),
        b: pt(a, at, to),
    }
}

fn axis_snap(inp: &SnapInput<'_>, a: Axis) -> Option<Candidate> {
    let m = &inp.moving;
    let c = other(a);
    let mv = [lo(m, a), mid(m, a), hi(m, a)];
    let mut best: Option<Candidate> = None;
    for o in inp.others {
        let tv = [lo(o, a), mid(o, a), hi(o, a)];
        for (ti, t) in tv.iter().enumerate() {
            for (mi, v) in mv.iter().enumerate() {
                // Centres snap to centres; edges to edges.
                if (ti == 1) != (mi == 1) {
                    continue;
                }
                let kind = if ti == 1 {
                    GuideKind::Center
                } else {
                    GuideKind::Edge
                };
                let from = lo(m, c).min(lo(o, c));
                let to = hi(m, c).max(hi(o, c));
                consider(
                    &mut best,
                    Candidate {
                        offset: t - v,
                        rank: 0,
                        guides: vec![line_guide(kind, a, *t, from, to)],
                    },
                    inp.tolerance,
                );
            }
        }
    }
    let frames = [
        (Some(inp.page), GuideKind::Page, 2u8),
        (inp.area, GuideKind::Area, 1u8),
    ];
    for (b, kind, rank) in frames {
        let Some(b) = b else { continue };
        let tv = [lo(&b, a), mid(&b, a), hi(&b, a)];
        for (ti, t) in tv.iter().enumerate() {
            for (mi, v) in mv.iter().enumerate() {
                if (ti == 1) != (mi == 1) {
                    continue;
                }
                consider(
                    &mut best,
                    Candidate {
                        offset: t - v,
                        rank,
                        guides: vec![line_guide(kind, a, *t, lo(&b, c), hi(&b, c))],
                    },
                    inp.tolerance,
                );
            }
        }
    }
    spacing(inp, a, &mut best);
    best
}

/// Nearest neighbour before (`before`) or after the box `m` in its row.
fn neighbour<'b>(
    m: &BBox,
    others: &'b [BBox],
    a: Axis,
    before: bool,
    tol: f64,
) -> Option<&'b BBox> {
    others
        .iter()
        .filter(|o| same_row(m, o, a))
        .filter(|o| {
            if before {
                hi(o, a) <= lo(m, a) + tol
            } else {
                lo(o, a) >= hi(m, a) - tol
            }
        })
        .min_by(|x, y| {
            let dx = if before {
                lo(m, a) - hi(x, a)
            } else {
                lo(x, a) - hi(m, a)
            };
            let dy = if before {
                lo(m, a) - hi(y, a)
            } else {
                lo(y, a) - hi(m, a)
            };
            dx.total_cmp(&dy)
        })
}

fn gap_marker(a: Axis, from: f64, to: f64, across: f64) -> Guide {
    Guide {
        kind: GuideKind::Spacing,
        a: pt(a, from, across),
        b: pt(a, to, across),
    }
}

fn spacing(inp: &SnapInput<'_>, a: Axis, best: &mut Option<Candidate>) {
    let m = &inp.moving;
    let c = other(a);
    let w = hi(m, a) - lo(m, a);
    let tol = inp.tolerance;
    let l = neighbour(m, inp.others, a, true, tol);
    let r = neighbour(m, inp.others, a, false, tol);
    let across = |o: &BBox| (lo(m, c).max(lo(o, c)) + hi(m, c).min(hi(o, c))) / 2.0;
    // Centred between both neighbours.
    if let (Some(l), Some(r)) = (l, r) {
        let target = (hi(l, a) + lo(r, a) - w) / 2.0;
        if target >= hi(l, a) {
            consider(
                best,
                Candidate {
                    offset: target - lo(m, a),
                    rank: 0,
                    guides: vec![
                        gap_marker(a, hi(l, a), target, across(l)),
                        gap_marker(a, target + w, lo(r, a), across(r)),
                    ],
                },
                tol,
            );
        }
    }
    // Continue the spacing of a neighbour pair in the same row.
    if let Some(l) = l {
        if let Some(ll) = neighbour(l, inp.others, a, true, 0.0) {
            let g = lo(l, a) - hi(ll, a);
            if g > 0.0 {
                let target = hi(l, a) + g;
                consider(
                    best,
                    Candidate {
                        offset: target - lo(m, a),
                        rank: 0,
                        guides: vec![
                            gap_marker(a, hi(ll, a), lo(l, a), across(ll)),
                            gap_marker(a, hi(l, a), target, across(l)),
                        ],
                    },
                    tol,
                );
            }
        }
    }
    if let Some(r) = r {
        if let Some(rr) = neighbour(r, inp.others, a, false, 0.0) {
            let g = lo(rr, a) - hi(r, a);
            if g > 0.0 {
                let target = lo(r, a) - g;
                consider(
                    best,
                    Candidate {
                        offset: target - hi(m, a),
                        rank: 0,
                        guides: vec![
                            gap_marker(a, hi(r, a), lo(rr, a), across(rr)),
                            gap_marker(a, target, lo(r, a), across(r)),
                        ],
                    },
                    tol,
                );
            }
        }
    }
}

fn grid_snap(v: f64, g: f64, tol: f64) -> Option<f64> {
    if !(g > 0.0 && g.is_finite()) {
        return None;
    }
    let off = (v / g).round() * g - v;
    (off.abs() <= tol).then_some(off)
}

/// Snaps the moving box. See the module docs for the rules.
pub fn snap(inp: &SnapInput<'_>) -> Snapped {
    let mut out = Snapped {
        delta: Point::ORIGIN,
        guides: Vec::new(),
        on_grid: (false, false),
    };
    if inp.tolerance.is_nan() || inp.tolerance < 0.0 {
        return out;
    }
    for a in [Axis::X, Axis::Y] {
        let cand = if inp.objects { axis_snap(inp, a) } else { None };
        let off = match cand {
            Some(c) => {
                out.guides.extend(c.guides);
                c.offset
            }
            None => match inp
                .grid
                .and_then(|g| grid_snap(lo(&inp.moving, a), g, inp.tolerance))
            {
                Some(o) => {
                    match a {
                        Axis::X => out.on_grid.0 = true,
                        Axis::Y => out.on_grid.1 = true,
                    }
                    o
                }
                None => 0.0,
            },
        };
        match a {
            Axis::X => out.delta.x = off,
            Axis::Y => out.delta.y = off,
        }
    }
    out
}

/// `b` moved by `d`.
pub fn moved(b: &BBox, d: Point) -> BBox {
    BBox {
        min: b.min + d,
        max: b.max + d,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn bx(x: f64, y: f64, w: f64, h: f64) -> BBox {
        BBox {
            min: Point::new(x, y),
            max: Point::new(x + w, y + h),
        }
    }

    fn page() -> BBox {
        bx(0.0, 0.0, 420.0, 297.0)
    }

    fn input<'a>(moving: BBox, others: &'a [BBox]) -> SnapInput<'a> {
        SnapInput {
            moving,
            others,
            page: page(),
            area: None,
            grid: Some(0.5),
            objects: true,
            tolerance: 1.0,
        }
    }

    #[test]
    fn edge_and_centre_snaps_with_guides() {
        let others = [bx(100.0, 100.0, 20.0, 10.0)];
        // Left edge 0.4 mm right of the other's left edge.
        let s = snap(&input(bx(100.4, 150.3, 5.0, 5.0), &others));
        assert!((s.delta.x + 0.4).abs() < 1e-9, "{s:?}");
        assert!(s
            .guides
            .iter()
            .any(|g| g.kind == GuideKind::Edge && g.a.x == 100.0));
        // Y has no object target: grid.
        assert!(s.on_grid.1 && (s.delta.y - 0.2).abs() < 1e-9, "{s:?}");
        // Centre to centre.
        let s = snap(&input(bx(107.7, 150.0, 5.0, 5.0), &others));
        assert!((s.delta.x - (-0.2)).abs() < 1e-9 || (s.delta.x - 0.3).abs() < 1e-9);
    }

    #[test]
    fn equal_spacing_between_neighbours() {
        let others = [bx(0.0, 50.0, 10.0, 10.0), bx(40.0, 50.0, 10.0, 10.0)];
        // Gap 10 on the left would be 15 - 10 = 5..; centre position is 20..30.
        let s = snap(&SnapInput {
            grid: None,
            page: bx(-1000.0, -1000.0, 1.0, 1.0),
            ..input(bx(20.6, 52.0, 10.0, 4.0), &others)
        });
        assert!((s.delta.x + 0.6).abs() < 1e-9, "{s:?}");
        assert_eq!(
            s.guides
                .iter()
                .filter(|g| g.kind == GuideKind::Spacing)
                .count(),
            2
        );
    }

    #[test]
    fn page_and_area_targets() {
        let mut i = input(bx(209.6, 10.0, 0.8, 1.0), &[]);
        i.grid = None;
        let s = snap(&i);
        // Box centre 210 already on the page centre: no correction.
        assert!(s.delta.x.abs() < 1e-9);
        assert!(s.guides.iter().any(|g| g.kind == GuideKind::Page));
        i.area = Some(bx(10.0, 10.0, 300.0, 200.0));
        i.moving = bx(10.7, 50.0, 5.0, 5.0);
        let s = snap(&i);
        assert!((s.delta.x + 0.7).abs() < 1e-9);
        assert!(s.guides.iter().any(|g| g.kind == GuideKind::Area));
    }

    fn arb_box() -> impl Strategy<Value = BBox> {
        (-50.0..450.0f64, -50.0..350.0f64, 0.0..80.0f64, 0.0..80.0f64)
            .prop_map(|(x, y, w, h)| bx(x, y, w, h))
    }

    proptest! {
        #[test]
        fn never_farther_than_tolerance(
            m in arb_box(),
            others in prop::collection::vec(arb_box(), 0..8),
            tol in 0.0..5.0f64,
            grid in prop::option::of(0.1..5.0f64),
            objects in any::<bool>(),
        ) {
            let s = snap(&SnapInput {
                moving: m,
                others: &others,
                page: page(),
                area: Some(bx(10.0, 10.0, 380.0, 230.0)),
                grid,
                objects,
                tolerance: tol,
            });
            prop_assert!(s.delta.x.abs() <= tol + 1e-9 && s.delta.y.abs() <= tol + 1e-9, "{:?}", s);
        }

        #[test]
        fn grid_only_lands_on_grid(m in arb_box(), g in 0.1..5.0f64) {
            let s = snap(&SnapInput {
                moving: m,
                others: &[],
                page: page(),
                area: None,
                grid: Some(g),
                objects: false,
                tolerance: g / 2.0 + 1e-9,
            });
            let r = moved(&m, s.delta);
            for v in [r.min.x, r.min.y] {
                let k = v / g;
                prop_assert!((k - k.round()).abs() < 1e-6, "{} {}", v, g);
            }
            prop_assert!(s.guides.is_empty());
        }
    }
}
