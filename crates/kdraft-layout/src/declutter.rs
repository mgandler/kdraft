//! Reference designator placement for assembly views.
//!
//! All values are sheet millimetres. A label is placed, in this order:
//!
//! 1. **Inside** its body, centred, at the largest cap height between
//!    `min_h` and `max_h` that fits (horizontal, or rotated 90 degrees for
//!    a tall body when that gives a clearly larger text).
//! 2. **Outside**, at `outside_h`, in the nearest free spot of a ring of
//!    candidates around the body (above, right, below, left, corners), up
//!    to `near` away from the body.
//! 3. With a **leader** to a free spot further out, up to `leader_max`.
//! 4. If the board is too dense for a spot clear of all bodies, steps 2
//!    and 3 repeat allowing the label to cover other bodies (never other
//!    labels); such labels are flagged `over_bodies`.
//! 5. Otherwise **unplaced**: listed, never drawn overlapping.
//!
//! Free means: inside `area`, no overlap with any placed label, with any
//! body (except in step 4) or with an outline segment. Labels never
//! overlap each other. Text is horizontal or reads bottom to top (90
//! degrees), so it is never upside down. The result depends only on the
//! input (fixed candidate order, ties broken by input order).

use kdraft_board::geom::{BBox, Point};

use crate::grid::{inside, overlaps, BoxGrid, SegmentGrid};

/// Padding around the text's cap-height box, as a fraction of the height.
pub const PAD: f64 = 0.18;

#[derive(Debug, Clone, PartialEq)]
pub struct LabelReq {
    /// Body box of the part on the sheet.
    pub body: BBox,
    /// Text width at 1 mm cap height.
    pub unit_width: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    pub min_h: f64,
    pub max_h: f64,
    /// Cap height of labels placed outside their body.
    pub outside_h: f64,
    /// Farthest distance from the body without a leader.
    pub near: f64,
    /// Farthest distance from the body with a leader.
    pub leader_max: f64,
    /// Labels stay inside this box.
    pub area: BBox,
}

impl Params {
    pub fn new(min_h: f64, max_h: f64, area: BBox) -> Params {
        let min_h = min_h.max(0.1);
        let max_h = max_h.max(min_h);
        Params {
            min_h,
            max_h,
            outside_h: min_h,
            near: 1.6 * min_h,
            leader_max: 12.0 * min_h.max(1.0),
            area,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Inside,
    Outside,
    Leader,
    Unplaced,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub kind: Kind,
    /// Text centre.
    pub at: Point,
    /// Cap height.
    pub size: f64,
    /// 0 or 90 degrees (counter-clockwise, reads bottom to top).
    pub rotation: f64,
    /// Occupied box (text plus padding).
    pub rect: BBox,
    /// Leader from the body edge to the label edge.
    pub leader: Option<(Point, Point)>,
    /// Placed over other parts' bodies (dense fallback).
    pub over_bodies: bool,
}

/// Counts per placement kind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stats {
    pub inside: usize,
    pub outside: usize,
    pub leader: usize,
    pub over_bodies: usize,
    pub unplaced: usize,
}

pub fn stats(p: &[Placed]) -> Stats {
    let mut s = Stats::default();
    for l in p {
        match l.kind {
            Kind::Inside => s.inside += 1,
            Kind::Outside => s.outside += 1,
            Kind::Leader => s.leader += 1,
            Kind::Unplaced => s.unplaced += 1,
        }
        s.over_bodies += usize::from(l.over_bodies);
    }
    s
}

/// Box of a label of cap height `h` centred at `c`.
pub fn text_rect(c: Point, unit_width: f64, h: f64, rotation: f64) -> BBox {
    let w = unit_width * h + 2.0 * PAD * h;
    let t = h * (1.0 + 2.0 * PAD);
    let (hw, hh) = if rotation == 90.0 {
        (t / 2.0, w / 2.0)
    } else {
        (w / 2.0, t / 2.0)
    };
    BBox {
        min: Point::new(c.x - hw, c.y - hh),
        max: Point::new(c.x + hw, c.y + hh),
    }
}

/// Largest cap height (and rotation) at which the label fits inside
/// `body`, capped at `max_h`; `None` below `min_h`.
pub fn fit_inside(body: &BBox, unit_width: f64, min_h: f64, max_h: f64) -> Option<(f64, f64)> {
    let (w, h) = (body.width(), body.height());
    let unit = unit_width.max(1e-9) + 2.0 * PAD;
    let tall = 1.0 + 2.0 * PAD;
    // 90 % of the body, so the text does not touch the outline.
    let horiz = (0.9 * w / unit).min(0.9 * h / tall).min(max_h);
    let vert = (0.9 * h / unit).min(0.9 * w / tall).min(max_h);
    let (size, rot) = if vert > horiz * 1.15 {
        (vert, 90.0)
    } else {
        (horiz, 0.0)
    };
    (size >= min_h).then_some((size, rot))
}

struct State<'a> {
    labels: BoxGrid,
    bodies: BoxGrid,
    /// Small marks (pin-1 dots) no label may cover.
    marks: BoxGrid,
    outline: &'a SegmentGrid,
    area: BBox,
}

impl State<'_> {
    fn free(&self, r: &BBox, check_bodies: bool) -> bool {
        inside(r, &self.area)
            && !self.labels.any(r, None)
            && !self.marks.any(r, None)
            && !(check_bodies && self.bodies.any(r, None))
            && !self.outline.hits(r)
    }
}

/// Candidate centres around `body` at clearance `d` for a `w` x `h` box,
/// in preference order.
fn ring(body: &BBox, w: f64, h: f64, d: f64) -> [Point; 10] {
    let c = body.center();
    let above = body.min.y - d - h / 2.0;
    let below = body.max.y + d + h / 2.0;
    let left = body.min.x - d - w / 2.0;
    let right = body.max.x + d + w / 2.0;
    [
        Point::new(c.x, above),
        Point::new(right, c.y),
        Point::new(c.x, below),
        Point::new(left, c.y),
        Point::new(body.min.x + w / 2.0, above),
        Point::new(body.max.x - w / 2.0, above),
        Point::new(right, above),
        Point::new(right, below),
        Point::new(left, below),
        Point::new(left, above),
    ]
}

/// Nearest point of box `b` to `p`.
fn clamp_to(b: &BBox, p: Point) -> Point {
    Point::new(p.x.clamp(b.min.x, b.max.x), p.y.clamp(b.min.y, b.max.y))
}

fn try_outside(
    st: &State<'_>,
    req: &LabelReq,
    p: &Params,
    check_bodies: bool,
) -> Option<(Point, f64, BBox, bool)> {
    let h = p.outside_h;
    let probe = text_rect(Point::ORIGIN, req.unit_width, h, 0.0);
    let (w, t) = (probe.width(), probe.height());
    let gap = 0.25 * h;
    let step = (0.5 * t).max(0.05);
    // Near: rings without leader.
    let mut d = gap;
    while d <= p.near + 1e-9 {
        for c in ring(&req.body, w, t, d) {
            let r = text_rect(c, req.unit_width, h, 0.0);
            if !overlaps(&r, &req.body) && st.free(&r, check_bodies) {
                return Some((c, 0.0, r, false));
            }
        }
        // Vertical text beside tall bodies.
        if req.body.height() > req.body.width() {
            for c in ring(&req.body, t, w, d)
                .into_iter()
                .take(4)
                .skip(1)
                .step_by(2)
            {
                let r = text_rect(c, req.unit_width, h, 90.0);
                if !overlaps(&r, &req.body) && st.free(&r, check_bodies) {
                    return Some((c, 90.0, r, false));
                }
            }
        }
        d += step;
    }
    // Leader: 16 directions on growing rings.
    let bc = req.body.center();
    let base = 0.5 * (req.body.width().hypot(req.body.height()));
    let mut d = p.near + step;
    while d <= p.leader_max + 1e-9 {
        for k in 0..16 {
            // Start upwards, then alternate right/left of it.
            let idx = [0, 4, 12, 2, 14, 6, 10, 8, 1, 15, 3, 13, 5, 11, 7, 9][k];
            let a = f64::from(idx) * std::f64::consts::PI / 8.0;
            let rad = base + d + 0.5 * w.max(t);
            let c = Point::new(bc.x + rad * a.sin(), bc.y - rad * a.cos());
            let r = text_rect(c, req.unit_width, h, 0.0);
            if !overlaps(&r, &req.body) && st.free(&r, check_bodies) {
                return Some((c, 0.0, r, true));
            }
        }
        d += 2.0 * step;
    }
    None
}

/// Places all labels. `outline` holds the board outline as segments (sheet
/// mm). The output has one entry per request, in request order.
pub fn place(reqs: &[LabelReq], outline: &SegmentGrid, p: &Params) -> Vec<Placed> {
    place_with_marks(reqs, &[], outline, p)
}

/// As [`place`], with small `marks` (pin-1 dots) that no label covers,
/// inside its body or not.
pub fn place_with_marks(
    reqs: &[LabelReq],
    marks: &[BBox],
    outline: &SegmentGrid,
    p: &Params,
) -> Vec<Placed> {
    place_with_options(reqs, &[], marks, outline, p)
}

/// [`place_with_marks`] where `occupied[i]` marks a body whose inside is
/// taken (a dense pad field): its label goes outside.
pub fn place_with_options(
    reqs: &[LabelReq],
    occupied: &[bool],
    marks: &[BBox],
    outline: &SegmentGrid,
    p: &Params,
) -> Vec<Placed> {
    let cell = (4.0 * p.min_h).max(1.0);
    let mut st = State {
        labels: BoxGrid::new(cell),
        bodies: BoxGrid::new(cell),
        marks: BoxGrid::new(cell),
        outline,
        area: p.area,
    };
    for r in reqs {
        st.bodies.insert(r.body);
    }
    for m in marks {
        st.marks.insert(*m);
    }
    let mut out: Vec<Option<Placed>> = vec![None; reqs.len()];

    // 1. Inside, larger bodies first (they rarely conflict).
    let mut order: Vec<usize> = (0..reqs.len()).collect();
    let area = |i: usize| reqs[i].body.width() * reqs[i].body.height();
    order.sort_by(|&a, &b| area(b).total_cmp(&area(a)).then(a.cmp(&b)));
    for &i in &order {
        let req = &reqs[i];
        if occupied.get(i).copied().unwrap_or(false) {
            continue;
        }
        if let Some((fit, rot)) = fit_inside(&req.body, req.unit_width, p.min_h, p.max_h) {
            let c = req.body.center();
            // Shrink (down to min_h) to clear a pin-1 mark or another label.
            let mut size = fit;
            let mut r = text_rect(c, req.unit_width, size, rot);
            while (st.labels.any(&r, None) || st.marks.any(&r, None)) && size > p.min_h {
                size = (size * 0.85).max(p.min_h);
                r = text_rect(c, req.unit_width, size, rot);
            }
            if !st.labels.any(&r, None) && !st.marks.any(&r, None) {
                st.labels.insert(r);
                out[i] = Some(Placed {
                    kind: Kind::Inside,
                    at: c,
                    size,
                    rotation: rot,
                    rect: r,
                    leader: None,
                    over_bodies: false,
                });
            }
        }
    }

    // 2-4. Outside, smaller bodies first (they have the fewest options).
    let mut rest: Vec<usize> = (0..reqs.len()).filter(|&i| out[i].is_none()).collect();
    rest.sort_by(|&a, &b| area(a).total_cmp(&area(b)).then(a.cmp(&b)));
    for &i in &rest {
        let req = &reqs[i];
        let found = try_outside(&st, req, p, true)
            .map(|f| (f, false))
            .or_else(|| try_outside(&st, req, p, false).map(|f| (f, true)));
        out[i] = Some(match found {
            Some(((c, rot, r, leader), over)) => {
                st.labels.insert(r);
                let leader = leader.then(|| {
                    let from = clamp_to(&req.body, c);
                    (from, clamp_to(&r, from))
                });
                Placed {
                    kind: if leader.is_some() {
                        Kind::Leader
                    } else {
                        Kind::Outside
                    },
                    at: c,
                    size: p.outside_h,
                    rotation: rot,
                    rect: r,
                    leader,
                    over_bodies: over,
                }
            }
            None => Placed {
                kind: Kind::Unplaced,
                at: req.body.center(),
                size: p.outside_h,
                rotation: 0.0,
                rect: text_rect(req.body.center(), req.unit_width, p.outside_h, 0.0),
                leader: None,
                over_bodies: false,
            },
        });
    }
    out.into_iter()
        .map(|o| {
            o.unwrap_or(Placed {
                kind: Kind::Unplaced,
                at: Point::ORIGIN,
                size: 0.0,
                rotation: 0.0,
                rect: BBox::from_point(Point::ORIGIN),
                leader: None,
                over_bodies: false,
            })
        })
        .collect()
}

/// Fraction of labels that fit inside their body at `min_h` when the
/// bodies are scaled by `scale` (bodies given in board mm). Used to pick
/// a view scale for dense boards.
pub fn inside_fraction(bodies: &[(BBox, f64)], scale: f64, min_h: f64) -> f64 {
    if bodies.is_empty() {
        return 1.0;
    }
    let n = bodies
        .iter()
        .filter(|(b, unit)| {
            let s = BBox {
                min: Point::new(b.min.x * scale, b.min.y * scale),
                max: Point::new(b.max.x * scale, b.max.y * scale),
            };
            fit_inside(&s, *unit, min_h, f64::INFINITY).is_some()
        })
        .count();
    n as f64 / bodies.len() as f64
}

/// Pairs of placed labels that overlap (should always be empty).
pub fn overlapping_pairs(p: &[Placed]) -> Vec<(usize, usize)> {
    let mut v = Vec::new();
    for i in 0..p.len() {
        if p[i].kind == Kind::Unplaced {
            continue;
        }
        for j in i + 1..p.len() {
            if p[j].kind != Kind::Unplaced && overlaps(&p[i].rect, &p[j].rect) {
                v.push((i, j));
            }
        }
    }
    v
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

    fn area() -> BBox {
        bx(-50.0, -50.0, 200.0, 200.0)
    }

    #[test]
    fn big_body_inside_small_body_outside() {
        let reqs = vec![
            LabelReq {
                body: bx(0.0, 0.0, 20.0, 10.0),
                unit_width: 1.6,
            },
            LabelReq {
                body: bx(30.0, 0.0, 1.0, 0.5),
                unit_width: 1.6,
            },
            LabelReq {
                body: bx(50.0, 0.0, 2.0, 12.0),
                unit_width: 2.4,
            },
        ];
        let sg = SegmentGrid::new(1.0, vec![]);
        let p = Params::new(0.9, 2.5, area());
        let out = place(&reqs, &sg, &p);
        assert_eq!(out[0].kind, Kind::Inside);
        assert_eq!(out[0].size, 2.5);
        assert_eq!(out[1].kind, Kind::Outside);
        assert!(!overlaps(&out[1].rect, &reqs[1].body));
        assert_eq!(out[2].kind, Kind::Inside);
        assert_eq!(out[2].rotation, 90.0);
        assert_eq!(stats(&out).unplaced, 0);
        // Deterministic.
        assert_eq!(place(&reqs, &sg, &p), out);
    }

    #[test]
    fn crowded_row_uses_leaders_or_dense_fallback_without_overlap() {
        // 40 tiny parts in a tight row: not everything fits beside them.
        let reqs: Vec<LabelReq> = (0..40)
            .map(|i| LabelReq {
                body: bx(f64::from(i) * 1.2, 0.0, 1.0, 0.5),
                unit_width: 2.5,
            })
            .collect();
        let sg = SegmentGrid::new(1.0, vec![]);
        let out = place(&reqs, &sg, &Params::new(0.9, 2.5, area()));
        assert!(overlapping_pairs(&out).is_empty());
        let s = stats(&out);
        assert!(s.leader > 0, "{s:?}");
        assert_eq!(s.unplaced, 0, "{s:?}");
    }

    #[test]
    fn outline_and_area_are_respected() {
        let reqs = vec![LabelReq {
            body: bx(0.0, 0.0, 0.5, 0.5),
            unit_width: 2.0,
        }];
        // Outline segments right around the body except below.
        let segs = vec![
            (Point::new(-5.0, -0.1), Point::new(5.0, -0.1)),
            (Point::new(-0.1, -5.0), Point::new(-0.1, 5.0)),
            (Point::new(0.6, -5.0), Point::new(0.6, 5.0)),
        ];
        let sg = SegmentGrid::new(1.0, segs);
        let out = place(
            &reqs,
            &sg,
            &Params::new(0.9, 2.5, bx(-0.1, -0.1, 0.7, 50.0)),
        );
        // Nothing fits between the lines (narrow channel): unplaced is
        // reported, not drawn over the outline.
        assert_eq!(out[0].kind, Kind::Unplaced);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]
        #[test]
        fn labels_never_overlap(parts in proptest::collection::vec(
            (0.0f64..60.0, 0.0f64..40.0, 0.3f64..8.0, 0.3f64..8.0, 1.0f64..5.0), 1..80)) {
            let reqs: Vec<LabelReq> = parts
                .iter()
                .map(|&(x, y, w, h, u)| LabelReq { body: bx(x, y, w, h), unit_width: u })
                .collect();
            let sg = SegmentGrid::new(2.0, vec![]);
            let p = Params::new(0.9, 2.5, bx(-20.0, -20.0, 100.0, 80.0));
            let out = place(&reqs, &sg, &p);
            prop_assert_eq!(out.len(), reqs.len());
            prop_assert!(overlapping_pairs(&out).is_empty());
            for (l, r) in out.iter().zip(&reqs) {
                if l.kind == Kind::Inside {
                    prop_assert!(inside(&l.rect, &r.body));
                    prop_assert!(l.size >= p.min_h && l.size <= p.max_h);
                }
                if l.kind != Kind::Unplaced {
                    prop_assert!(inside(&l.rect, &p.area));
                    prop_assert!(l.rotation == 0.0 || l.rotation == 90.0);
                }
                if matches!(l.kind, Kind::Outside | Kind::Leader) {
                    prop_assert!(!overlaps(&l.rect, &r.body));
                }
            }
            prop_assert_eq!(place(&reqs, &sg, &p), out);
        }
    }
}
