//! Board-mm to plot-SVG transform: derived from data, never assumed.
//!
//! Model: `u = sx * x + ox`, `v = sy * y + oy` with board `(x, y)` in mm and
//! `(u, v)` in the SVG's root user units. Negative `sx` with positive `sy`
//! is a mirrored plot; negative `sy` a Y-up plot. A full 6-parameter affine
//! least-squares fit is reported alongside, to detect rotation or shear the
//! axis model cannot express.

use std::collections::{BTreeMap, HashMap};

use kdraft_board::geom::{Affine, BBox, Point};
use kdraft_board::{Board, Pad, PadType};
use serde::Serialize;

use crate::svg::SvgDoc;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct PlotTransform {
    pub sx: f64,
    pub ox: f64,
    pub sy: f64,
    pub oy: f64,
}

impl PlotTransform {
    pub fn affine(&self) -> Affine {
        Affine::new(self.sx, 0.0, 0.0, self.sy, self.ox, self.oy)
    }

    pub fn apply(&self, p: Point) -> Point {
        Point::new(self.sx * p.x + self.ox, self.sy * p.y + self.oy)
    }

    pub fn inverse_apply(&self, q: Point) -> Point {
        Point::new((q.x - self.ox) / self.sx, (q.y - self.oy) / self.sy)
    }

    /// User units per board mm (geometric mean of the axes).
    pub fn scale(&self) -> f64 {
        (self.sx * self.sy).abs().sqrt()
    }

    /// True if the plot is a mirror image of the board as seen from the
    /// front (orientation reversed).
    pub fn mirrored(&self) -> bool {
        self.sx * self.sy < 0.0
    }

    /// True if SVG Y grows in the same direction as board Y (down).
    pub fn y_down(&self) -> bool {
        self.sy > 0.0
    }

    pub fn map_bbox(&self, b: &BBox) -> BBox {
        self.affine().apply_bbox(b)
    }

    pub fn unmap_bbox(&self, b: &BBox) -> Option<BBox> {
        self.affine().inverse().map(|i| i.apply_bbox(b))
    }
}

/// Bounding-box fit with chosen axis signs: maps `board` onto `svg`.
pub fn fit_bbox(board: &BBox, svg: &BBox, sign_x: f64, sign_y: f64) -> Option<PlotTransform> {
    let rx = (board.width() > 1e-9).then(|| svg.width() / board.width());
    let ry = (board.height() > 1e-9).then(|| svg.height() / board.height());
    let (ax, ay) = match (rx, ry) {
        (Some(x), Some(y)) => (x, y),
        (Some(x), None) => (x, x),
        (None, Some(y)) => (y, y),
        (None, None) => return None,
    };
    if !(ax > 0.0 && ay > 0.0 && ax.is_finite() && ay.is_finite()) {
        return None;
    }
    let sx = sign_x.signum() * ax;
    let sy = sign_y.signum() * ay;
    let ox = if sx > 0.0 {
        svg.min.x - sx * board.min.x
    } else {
        svg.max.x - sx * board.min.x
    };
    let oy = if sy > 0.0 {
        svg.min.y - sy * board.min.y
    } else {
        svg.max.y - sy * board.min.y
    };
    Some(PlotTransform { sx, ox, sy, oy })
}

/// A board point expected to appear in the plot, with a label for grouped
/// statistics (e.g. the pad shape).
#[derive(Debug, Clone, PartialEq)]
pub struct RefPoint {
    pub at: Point,
    pub label: String,
}

/// Spatial grid over candidate points in SVG user units.
pub struct PointIndex {
    cell: f64,
    cells: HashMap<(i64, i64), Vec<Point>>,
    len: usize,
}

impl PointIndex {
    pub fn new(points: impl IntoIterator<Item = Point>, cell: f64) -> PointIndex {
        let cell = if cell > 0.0 && cell.is_finite() {
            cell
        } else {
            1.0
        };
        let mut cells: HashMap<(i64, i64), Vec<Point>> = HashMap::new();
        let mut len = 0;
        for p in points {
            if !(p.x.is_finite() && p.y.is_finite()) {
                continue;
            }
            cells.entry(Self::key(cell, p)).or_default().push(p);
            len += 1;
        }
        PointIndex { cell, cells, len }
    }

    fn key(cell: f64, p: Point) -> (i64, i64) {
        ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64)
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Nearest point within `max_dist` (user units).
    pub fn nearest(&self, q: Point, max_dist: f64) -> Option<(Point, f64)> {
        if !(q.x.is_finite() && q.y.is_finite()) {
            return None;
        }
        let (cx, cy) = Self::key(self.cell, q);
        let rings = (max_dist / self.cell).ceil().clamp(1.0, 64.0) as i64;
        let mut best: Option<(Point, f64)> = None;
        for r in 0..=rings {
            for i in -r..=r {
                for j in -r..=r {
                    if i.abs() != r && j.abs() != r {
                        continue;
                    }
                    if let Some(v) = self.cells.get(&(cx + i, cy + j)) {
                        for p in v {
                            let d = p.dist(q);
                            if d <= max_dist && best.is_none_or(|(_, bd)| d < bd) {
                                best = Some((*p, d));
                            }
                        }
                    }
                }
            }
            // Points in ring r+1 are at least r * cell away.
            if let Some((_, d)) = best {
                if d <= r as f64 * self.cell {
                    break;
                }
            }
        }
        best
    }
}

/// Centres of plotted shapes: each element's box centre, each closed
/// subpath's box centre, and each open subpath's box centre (the midpoint
/// of a stroked segment, which is where an oval pad plotted as a thick line
/// has its centre).
pub fn shape_centres(doc: &SvgDoc) -> Vec<Point> {
    let mut v = Vec::new();
    for s in doc.content() {
        if let Some(b) = s.bbox() {
            v.push(b.center());
        }
        if s.subpaths.len() > 1 {
            for sp in &s.subpaths {
                v.push(sp.bbox().center());
            }
        }
    }
    v
}

/// All path vertices of the content (for outline matching).
pub fn shape_vertices(doc: &SvgDoc) -> Vec<Point> {
    doc.content()
        .flat_map(|s| s.subpaths.iter().flat_map(|sp| sp.vertices()))
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct GroupResidual {
    pub count: usize,
    pub within_0_05: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Residuals {
    /// Reference points checked.
    pub count: usize,
    /// Points with any candidate within the search radius.
    pub matched: usize,
    /// Over matched points, mm.
    pub max_mm: f64,
    pub rms_mm: f64,
    pub median_mm: f64,
    /// Fractions of `count`.
    pub within_0_01: f64,
    pub within_0_05: f64,
    pub by_label: BTreeMap<String, GroupResidual>,
}

/// Search radius for residuals, mm. Pads farther than this from any shape
/// centre count as unmatched.
pub const RESIDUAL_RADIUS_MM: f64 = 1.0;

pub fn residuals(t: &PlotTransform, refs: &[RefPoint], index: &PointIndex) -> Residuals {
    let s = t.scale();
    let mut d_mm: Vec<f64> = Vec::new();
    let mut by_label: BTreeMap<String, GroupResidual> = BTreeMap::new();
    let (mut w1, mut w5) = (0usize, 0usize);
    for r in refs {
        let g = by_label.entry(r.label.clone()).or_default();
        g.count += 1;
        if let Some((_, d)) = index.nearest(t.apply(r.at), RESIDUAL_RADIUS_MM * s) {
            let d = d / s;
            d_mm.push(d);
            if d <= 0.01 {
                w1 += 1;
            }
            if d <= 0.05 {
                w5 += 1;
                g.within_0_05 += 1;
            }
        }
    }
    let n = refs.len();
    let mut sorted = d_mm.clone();
    sorted.sort_by(f64::total_cmp);
    let frac = |k: usize| if n == 0 { 0.0 } else { k as f64 / n as f64 };
    Residuals {
        count: n,
        matched: d_mm.len(),
        max_mm: sorted.last().copied().unwrap_or(0.0),
        rms_mm: if d_mm.is_empty() {
            0.0
        } else {
            (d_mm.iter().map(|d| d * d).sum::<f64>() / d_mm.len() as f64).sqrt()
        },
        median_mm: sorted.get(sorted.len() / 2).copied().unwrap_or(0.0),
        within_0_01: frac(w1),
        within_0_05: frac(w5),
        by_label,
    }
}

/// Pairs (board point, nearest plot point) within `radius_mm`.
fn matches(
    t: &PlotTransform,
    refs: &[RefPoint],
    index: &PointIndex,
    radius_mm: f64,
) -> Vec<(Point, Point)> {
    let s = t.scale();
    refs.iter()
        .filter_map(|r| {
            index
                .nearest(t.apply(r.at), radius_mm * s)
                .map(|(q, _)| (r.at, q))
        })
        .collect()
}

/// Least squares `v = a u + b`; `None` without spread in `u`.
fn line_fit(pairs: impl Iterator<Item = (f64, f64)>) -> Option<(f64, f64)> {
    let (mut n, mut su, mut sv, mut suu, mut suv) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (u, v) in pairs {
        n += 1.0;
        su += u;
        sv += v;
        suu += u * u;
        suv += u * v;
    }
    if n < 2.0 {
        return None;
    }
    let var = suu - su * su / n;
    if var.abs() < 1e-9 * (1.0 + suu.abs()) {
        return None;
    }
    let a = (suv - su * sv / n) / var;
    Some((a, (sv - a * su) / n))
}

/// Refines a transform by matching reference points with shrinking search
/// radii and per-axis least squares. Axes without spread keep their value.
pub fn refine(t0: &PlotTransform, refs: &[RefPoint], index: &PointIndex) -> PlotTransform {
    let mut t = *t0;
    for radius in [2.0, 0.5, 0.2, 0.1, 0.1] {
        let m = matches(&t, refs, index, radius);
        if m.len() < 3 {
            continue;
        }
        let mut next = t;
        if let Some((a, b)) = line_fit(m.iter().map(|(p, q)| (p.x, q.x))) {
            if a.signum() == t.sx.signum() {
                next.sx = a;
                next.ox = b;
            }
        }
        if let Some((a, b)) = line_fit(m.iter().map(|(p, q)| (p.y, q.y))) {
            if a.signum() == t.sy.signum() {
                next.sy = a;
                next.oy = b;
            }
        }
        t = next;
    }
    t
}

/// Full affine least squares on matched pairs (diagnostic).
pub fn affine_lsq(pairs: &[(Point, Point)]) -> Option<Affine> {
    if pairs.len() < 3 {
        return None;
    }
    // Normal equations for [x y 1] * [a c e]^T = u, and the same for v.
    let mut m = [[0.0f64; 3]; 3];
    let mut ru = [0.0f64; 3];
    let mut rv = [0.0f64; 3];
    for (p, q) in pairs {
        let r = [p.x, p.y, 1.0];
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] += r[i] * r[j];
            }
            ru[i] += r[i] * q.x;
            rv[i] += r[i] * q.y;
        }
    }
    let su = solve3(m, ru)?;
    let sv = solve3(m, rv)?;
    Some(Affine::new(su[0], sv[0], su[1], sv[1], su[2], sv[2]))
}

fn solve3(m: [[f64; 3]; 3], r: [f64; 3]) -> Option<[f64; 3]> {
    let det = |m: &[[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let d = det(&m);
    let scale: f64 = m.iter().flatten().map(|x| x.abs()).fold(0.0, f64::max);
    if d.abs() <= 1e-12 * scale.powi(3).max(1e-300) {
        return None;
    }
    let mut out = [0.0; 3];
    for (k, o) in out.iter_mut().enumerate() {
        let mut mk = m;
        for i in 0..3 {
            mk[i][k] = r[i];
        }
        *o = det(&mk) / d;
    }
    Some(out)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SignTrial {
    pub sign_x: f64,
    pub sign_y: f64,
    pub transform: PlotTransform,
    pub within_0_05: f64,
    pub rms_mm: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Fit {
    pub transform: PlotTransform,
    /// The bounding-box estimate the refinement started from.
    pub initial: PlotTransform,
    /// What the refinement matched: `pads` or `outline-vertices`.
    pub method: String,
    pub trials: Vec<SignTrial>,
    pub residuals: Residuals,
    /// 6-parameter least squares on the final matches, for diagnosis.
    pub affine_lsq: Option<Affine>,
}

impl Fit {
    /// At least 95 % of at least 3 reference points within 0.05 mm.
    pub fn is_good(&self) -> bool {
        self.residuals.count >= 3 && self.residuals.within_0_05 >= 0.95
    }
}

/// Fits the transform: bounding-box estimate for each of the four axis
/// sign combinations, refined on the reference points; the combination
/// with the most points within 0.05 mm (then lowest RMS) wins.
pub fn fit(
    board_box: &BBox,
    svg_box: &BBox,
    refs: &[RefPoint],
    index: &PointIndex,
    method: &str,
) -> Option<Fit> {
    let mut trials = Vec::new();
    for (sxs, sys) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
        let Some(t0) = fit_bbox(board_box, svg_box, sxs, sys) else {
            continue;
        };
        let t = refine(&t0, refs, index);
        let r = residuals(&t, refs, index);
        trials.push((
            t0,
            SignTrial {
                sign_x: sxs,
                sign_y: sys,
                transform: t,
                within_0_05: r.within_0_05,
                rms_mm: r.rms_mm,
            },
            r,
        ));
    }
    let best = trials
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| {
            a.1.within_0_05
                .total_cmp(&b.1.within_0_05)
                .then(a.2.matched.cmp(&b.2.matched))
                .then(b.1.rms_mm.total_cmp(&a.1.rms_mm))
        })
        .map(|(i, _)| i)?;
    let (initial, trial, res) = trials[best].clone();
    let pairs = matches(&trial.transform, refs, index, 0.1);
    Some(Fit {
        transform: trial.transform,
        initial,
        method: method.to_string(),
        affine_lsq: affine_lsq(&pairs),
        trials: trials.into_iter().map(|(_, t, _)| t).collect(),
        residuals: res,
    })
}

/// True if `pad` has copper on `layer` (`F.Cu`, `In1.Cu`, ...).
pub fn pad_on_layer(pad: &Pad, layer: &str) -> bool {
    pad.layers.iter().any(|l| {
        l == layer
            || (l == "*.Cu" && layer.ends_with(".Cu"))
            || (l == "F&B.Cu" && (layer == "F.Cu" || layer == "B.Cu"))
    })
}

/// Pad centres with copper on `layer`, labelled by pad shape. Pads without
/// size and NPTH pads not larger than their hole (no copper) are left out.
pub fn pad_refs(board: &Board, layer: &str) -> Vec<RefPoint> {
    let mut v = Vec::new();
    for fp in &board.footprints {
        for pad in &fp.pads {
            if pad.size.w <= 0.0 || pad.size.h <= 0.0 || !pad_on_layer(pad, layer) {
                continue;
            }
            if pad.kind == PadType::NpThruHole {
                let d = pad.drill.map_or(0.0, |d| d.shape.tool_diameter());
                if pad.size.w.min(pad.size.h) <= d {
                    continue;
                }
            }
            v.push(RefPoint {
                at: pad.position,
                label: pad.shape.clone(),
            });
        }
    }
    v
}

/// Outline vertices of every contour, plus circle extremes.
pub fn outline_refs(board: &Board) -> Vec<RefPoint> {
    board
        .outline
        .contours
        .iter()
        .flat_map(|c| c.vertices.iter())
        .map(|p| RefPoint {
            at: *p,
            label: "vertex".to_string(),
        })
        .collect()
}

/// Deviation of a mapped board box from a plot box, per side, in board mm.
/// Positive values mean the plot box extends beyond the mapped board box.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct BoxDeviation {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
}

impl BoxDeviation {
    pub fn max_abs(&self) -> f64 {
        [self.left, self.right, self.top, self.bottom]
            .iter()
            .fold(0.0f64, |m, v| m.max(v.abs()))
    }

    /// Spread between sides: a uniform inflation (stroke width) is small.
    pub fn spread(&self) -> f64 {
        let v = [self.left, self.right, self.top, self.bottom];
        let max = v.iter().copied().fold(f64::MIN, f64::max);
        let min = v.iter().copied().fold(f64::MAX, f64::min);
        max - min
    }
}

pub fn box_deviation(t: &PlotTransform, board: &BBox, svg: &BBox) -> Option<BoxDeviation> {
    let plot_in_board = t.unmap_bbox(svg)?;
    Some(BoxDeviation {
        left: board.min.x - plot_in_board.min.x,
        right: plot_in_board.max.x - board.max.x,
        top: board.min.y - plot_in_board.min.y,
        bottom: plot_in_board.max.y - board.max.y,
    })
}

/// Fits the board-to-plot transform for one export mode from an Edge.Cuts
/// plot and, if available, a copper plot with its layer name. Pads are the
/// preferred references; without them, outline vertices are used.
pub fn fit_board(board: &Board, edge: &SvgDoc, copper: Option<(&SvgDoc, &str)>) -> Option<Fit> {
    let board_box = board.outline.bbox?;
    let svg_box = edge.content_bbox()?;
    let scale_guess = fit_bbox(&board_box, &svg_box, 1.0, 1.0)?.scale();
    if let Some((cu, layer)) = copper {
        let refs = pad_refs(board, layer);
        if refs.len() >= 3 {
            let index = PointIndex::new(shape_centres(cu), scale_guess);
            if let Some(f) = fit(&board_box, &svg_box, &refs, &index, "pads") {
                if f.is_good() {
                    return Some(f);
                }
                let outline = fit_outline(board, edge, &board_box, &svg_box, scale_guess);
                return Some(match outline {
                    Some(o) if o.residuals.within_0_05 > f.residuals.within_0_05 => o,
                    _ => f,
                });
            }
        }
    }
    fit_outline(board, edge, &board_box, &svg_box, scale_guess)
}

fn fit_outline(
    board: &Board,
    edge: &SvgDoc,
    board_box: &BBox,
    svg_box: &BBox,
    scale_guess: f64,
) -> Option<Fit> {
    let refs = outline_refs(board);
    let index = PointIndex::new(shape_vertices(edge), scale_guess);
    fit(board_box, svg_box, &refs, &index, "outline-vertices")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs(points: &[(f64, f64)]) -> Vec<RefPoint> {
        points
            .iter()
            .map(|&(x, y)| RefPoint {
                at: Point::new(x, y),
                label: "p".into(),
            })
            .collect()
    }

    #[test]
    fn nearest_lookup_across_cells() {
        let idx = PointIndex::new(
            [
                Point::new(0.0, 0.0),
                Point::new(5.2, 5.0),
                Point::new(10.0, 0.0),
            ],
            1.0,
        );
        let (p, d) = idx.nearest(Point::new(5.0, 5.0), 3.0).unwrap();
        assert!(p.approx_eq(Point::new(5.2, 5.0), 1e-12));
        assert!((d - 0.2).abs() < 1e-12);
        assert!(idx.nearest(Point::new(20.0, 20.0), 3.0).is_none());
    }

    #[test]
    fn recovers_mirrored_scaled_transform() {
        let truth = PlotTransform {
            sx: -10.0,
            ox: 900.0,
            sy: 10.0,
            oy: 50.0,
        };
        let pts = [
            (1.0, 1.0),
            (30.0, 2.0),
            (12.0, 17.0),
            (25.0, 19.0),
            (5.0, 9.0),
        ];
        let r = refs(&pts);
        let plotted: Vec<Point> = r.iter().map(|p| truth.apply(p.at)).collect();
        let board_box = BBox::from_points(r.iter().map(|p| p.at)).unwrap();
        // Plot box inflated by a stroke of 0.2 mm on each side.
        let mut svg_box = BBox::from_points(plotted.iter().copied()).unwrap();
        svg_box.min = Point::new(svg_box.min.x - 2.0, svg_box.min.y - 2.0);
        svg_box.max = Point::new(svg_box.max.x + 2.0, svg_box.max.y + 2.0);
        let idx = PointIndex::new(plotted, 10.0);
        let f = fit(&board_box, &svg_box, &r, &idx, "pads").unwrap();
        let t = f.transform;
        assert!((t.sx - truth.sx).abs() < 1e-9, "{t:?}");
        assert!((t.oy - truth.oy).abs() < 1e-6, "{t:?}");
        assert!(t.mirrored() && t.y_down());
        assert_eq!(f.residuals.within_0_01, 1.0);
        assert!(f.is_good());
        let a = f.affine_lsq.unwrap();
        assert!(a.c.abs() < 1e-9 && a.b.abs() < 1e-9);
    }

    #[test]
    fn deviation_detects_uniform_inflation() {
        let t = PlotTransform {
            sx: 2.0,
            ox: 0.0,
            sy: 2.0,
            oy: 0.0,
        };
        let board = BBox {
            min: Point::new(0.0, 0.0),
            max: Point::new(10.0, 5.0),
        };
        let svg = BBox {
            min: Point::new(-0.2, -0.2),
            max: Point::new(20.2, 10.2),
        };
        let d = box_deviation(&t, &board, &svg).unwrap();
        assert!((d.left - 0.1).abs() < 1e-12 && (d.bottom - 0.1).abs() < 1e-12);
        assert!(d.spread() < 1e-12);
    }
}
