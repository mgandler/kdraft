//! Edit handles of the single selected item and what dragging them does:
//! line and polyline vertices, rectangle corners and edges, circle radius,
//! arc points, text rotation and wrap width, dimension anchors and text,
//! callout anchors. Pure geometry; the app turns results into engine
//! commands (one gesture per drag).

use kdraft_board::geom::{BBox, Point};
use kdraft_doc::{GraphicShape, Id};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandleKind {
    /// Defining point `i` (line ends, polyline/polygon vertices, arc
    /// start/mid/end).
    Vertex(usize),
    /// Rectangle corner: 0 top-left, 1 top-right, 2 bottom-right,
    /// 3 bottom-left.
    Corner(usize),
    /// Rectangle edge: 0 top, 1 right, 2 bottom, 3 left.
    Edge(usize),
    /// A point on a circle.
    Radius,
    /// Text rotation.
    Rotate,
    /// Text wrap width (right edge of the block).
    Wrap,
    /// Dimension anchor 0 (from) or 1 (to).
    DimAnchor(usize),
    /// Dimension text (moves along the line).
    DimText,
    /// The anchored end of a callout's leader.
    CalloutAnchor,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EditHandle {
    pub item: Id,
    pub kind: HandleKind,
    /// Sheet position.
    pub at: Point,
}

impl EditHandle {
    /// Handles that drop onto board anchors.
    pub fn picks_anchor(&self) -> bool {
        matches!(
            self.kind,
            HandleKind::DimAnchor(_) | HandleKind::CalloutAnchor
        )
    }

    /// Accessible / tooltip name.
    pub fn label(&self) -> String {
        match self.kind {
            HandleKind::Vertex(i) => format!("Point {}", i + 1),
            HandleKind::Corner(i) => [
                "Top-left corner",
                "Top-right corner",
                "Bottom-right corner",
                "Bottom-left corner",
            ][i % 4]
                .into(),
            HandleKind::Edge(i) => {
                ["Top edge", "Right edge", "Bottom edge", "Left edge"][i % 4].into()
            }
            HandleKind::Radius => "Radius".into(),
            HandleKind::Rotate => "Rotation".into(),
            HandleKind::Wrap => "Wrap width".into(),
            HandleKind::DimAnchor(0) => "Dimension start anchor".into(),
            HandleKind::DimAnchor(_) => "Dimension end anchor".into(),
            HandleKind::DimText => "Dimension text".into(),
            HandleKind::CalloutAnchor => "Callout anchor".into(),
        }
    }
}

/// Corners of a box: top-left, top-right, bottom-right, bottom-left.
pub fn corners(b: &BBox) -> [Point; 4] {
    [
        b.min,
        Point::new(b.max.x, b.min.y),
        b.max,
        Point::new(b.min.x, b.max.y),
    ]
}

/// Handles of a graphic shape (text handles come from its layout).
pub fn shape_handles(id: &Id, s: &GraphicShape) -> Vec<EditHandle> {
    let h = |kind, at| EditHandle {
        item: id.clone(),
        kind,
        at,
    };
    match s {
        GraphicShape::Line { .. }
        | GraphicShape::Polyline { .. }
        | GraphicShape::Polygon { .. }
        | GraphicShape::Arc { .. } => s
            .points()
            .into_iter()
            .enumerate()
            .map(|(i, p)| h(HandleKind::Vertex(i), p))
            .collect(),
        GraphicShape::Rect { start, end, .. } => {
            let Some(b) = BBox::from_points([*start, *end]) else {
                return Vec::new();
            };
            let c = corners(&b);
            let mut v: Vec<EditHandle> = (0..4).map(|i| h(HandleKind::Corner(i), c[i])).collect();
            for i in 0..4 {
                let (a, b2) = (c[i], c[(i + 1) % 4]);
                v.push(h(
                    HandleKind::Edge(i),
                    Point::new((a.x + b2.x) / 2.0, (a.y + b2.y) / 2.0),
                ));
            }
            v
        }
        GraphicShape::Circle { center, radius } => {
            vec![h(
                HandleKind::Radius,
                Point::new(center.x + radius, center.y),
            )]
        }
        GraphicShape::Text { .. } | GraphicShape::Image { .. } => Vec::new(),
    }
}

/// Text handles from the block's rotated corners (top-left, top-right,
/// bottom-right, bottom-left): rotation above the top-right corner, wrap
/// on the right edge. `px` is one screen pixel in mm.
pub fn text_handles(id: &Id, corners: &[Point; 4], px: f64) -> Vec<EditHandle> {
    let [_, tr, br, _] = *corners;
    // Up along the right edge (bottom to top).
    let e = Point::new(tr.x - br.x, tr.y - br.y);
    let le = e.x.hypot(e.y).max(1e-9);
    let up = Point::new(e.x / le, e.y / le);
    vec![
        EditHandle {
            item: id.clone(),
            kind: HandleKind::Rotate,
            at: Point::new(tr.x + up.x * 14.0 * px, tr.y + up.y * 14.0 * px),
        },
        EditHandle {
            item: id.clone(),
            kind: HandleKind::Wrap,
            at: Point::new((tr.x + br.x) / 2.0, (tr.y + br.y) / 2.0),
        },
    ]
}

/// Points constrained to 0/45/90 degree steps from `from` (Shift).
pub fn constrain45(from: Point, p: Point) -> Point {
    let d = Point::new(p.x - from.x, p.y - from.y);
    let len = d.x.hypot(d.y);
    if len < 1e-12 {
        return p;
    }
    let a = d.y.atan2(d.x);
    let step = std::f64::consts::FRAC_PI_4;
    let s = (a / step).round() * step;
    Point::new(from.x + len * s.cos(), from.y + len * s.sin())
}

/// New defining points of a shape when handle `k` is dragged to `p`
/// (`shift`: constrain line/polyline segments to 45 degree steps).
pub fn edited_points(s: &GraphicShape, k: HandleKind, p: Point, shift: bool) -> Option<Vec<Point>> {
    let mut pts = s.points();
    match (s, k) {
        (
            GraphicShape::Line { .. }
            | GraphicShape::Polyline { .. }
            | GraphicShape::Polygon { .. }
            | GraphicShape::Arc { .. },
            HandleKind::Vertex(i),
        ) => {
            if i >= pts.len() {
                return None;
            }
            let q = if shift && !matches!(s, GraphicShape::Arc { .. }) {
                let n = pts.len();
                let nb = if i > 0 {
                    pts[i - 1]
                } else if matches!(s, GraphicShape::Polygon { .. }) {
                    pts[n - 1]
                } else {
                    pts.get(1).copied()?
                };
                constrain45(nb, p)
            } else {
                p
            };
            pts[i] = q;
            Some(pts)
        }
        (GraphicShape::Rect { start, end, .. }, HandleKind::Corner(i)) => {
            let b = BBox::from_points([*start, *end])?;
            let c = corners(&b);
            let opp = c[(i + 2) % 4];
            Some(vec![opp, p])
        }
        (GraphicShape::Rect { start, end, .. }, HandleKind::Edge(i)) => {
            let mut b = BBox::from_points([*start, *end])?;
            match i % 4 {
                0 => b.min.y = p.y,
                1 => b.max.x = p.x,
                2 => b.max.y = p.y,
                _ => b.min.x = p.x,
            }
            Some(vec![b.min, b.max])
        }
        (GraphicShape::Circle { center, .. }, HandleKind::Radius) => {
            (center.dist(p) > 1e-6).then(|| vec![*center, p])
        }
        _ => None,
    }
}

/// Text rotation (degrees CCW, snapped to 15 degrees unless `free`)
/// when the rotation handle that started at `start` (with rotation `r0`)
/// is dragged to `p` around anchor `at`.
pub fn text_rotation(at: Point, start: Point, p: Point, r0: f64, free: bool) -> f64 {
    let ang = |q: Point| (-(q.y - at.y)).atan2(q.x - at.x).to_degrees();
    let a = r0 + ang(p) - ang(start);
    let a = if free { a } else { (a / 15.0).round() * 15.0 };
    let mut r = a % 360.0;
    if r > 180.0 {
        r -= 360.0;
    } else if r <= -180.0 {
        r += 360.0;
    }
    if r.abs() < 1e-9 {
        0.0
    } else {
        r
    }
}

/// Wrap width for the wrap handle at `p`: distance from the block's left
/// edge (through `tl`, `bl`) along the text direction `dir` (unit).
pub fn wrap_width(tl: Point, dir: Point, p: Point) -> f64 {
    ((p.x - tl.x) * dir.x + (p.y - tl.y) * dir.y).max(2.0)
}

/// The handle under `p` within `tol`, nearest first.
pub fn hit(hs: &[EditHandle], p: Point, tol: f64) -> Option<&EditHandle> {
    hs.iter()
        .filter(|h| h.at.dist(p) <= tol)
        .min_by(|a, b| a.at.dist(p).total_cmp(&b.at.dist(p)))
}

/// Index at which a vertex inserted at `p` goes (the nearest segment of
/// an open or closed point list), with the distance to that segment.
pub fn insert_index(pts: &[Point], closed: bool, p: Point) -> Option<(usize, f64)> {
    let n = pts.len();
    if n < 2 {
        return None;
    }
    let segs = if closed { n } else { n - 1 };
    (0..segs)
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            (
                i + 1,
                kdraft_layout::dimension::point_segment_distance(p, a, b),
            )
        })
        .min_by(|x, y| x.1.total_cmp(&y.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> Id {
        Id("g".into())
    }

    #[test]
    fn rect_corner_and_edge_drags() {
        let r = GraphicShape::Rect {
            start: Point::new(10.0, 10.0),
            end: Point::new(30.0, 20.0),
            radius: 0.0,
        };
        let hs = shape_handles(&id(), &r);
        assert_eq!(hs.len(), 8);
        let v = edited_points(&r, HandleKind::Corner(2), Point::new(40.0, 25.0), false).unwrap();
        assert_eq!(v, vec![Point::new(10.0, 10.0), Point::new(40.0, 25.0)]);
        let v = edited_points(&r, HandleKind::Edge(3), Point::new(5.0, 99.0), false).unwrap();
        assert_eq!(v, vec![Point::new(5.0, 10.0), Point::new(30.0, 20.0)]);
    }

    #[test]
    fn line_shift_constrains_to_45_degrees() {
        let l = GraphicShape::Line {
            start: Point::new(0.0, 0.0),
            end: Point::new(10.0, 0.0),
        };
        let v = edited_points(&l, HandleKind::Vertex(1), Point::new(10.0, 1.0), true).unwrap();
        assert!(
            (v[1].y).abs() < 1e-9 && (v[1].x - 10.05).abs() < 0.01,
            "{:?}",
            v[1]
        );
        let v = edited_points(&l, HandleKind::Vertex(1), Point::new(10.0, 9.0), true).unwrap();
        assert!((v[1].x - v[1].y).abs() < 1e-9);
    }

    #[test]
    fn insert_index_finds_the_nearest_segment() {
        let pts = [
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(10.0, 10.0),
        ];
        assert_eq!(
            insert_index(&pts, false, Point::new(5.0, 1.0)).unwrap().0,
            1
        );
        assert_eq!(
            insert_index(&pts, false, Point::new(9.0, 5.0)).unwrap().0,
            2
        );
        assert_eq!(insert_index(&pts, true, Point::new(4.0, 5.0)).unwrap().0, 3);
        let o = Point::ORIGIN;
        assert_eq!(
            text_rotation(o, Point::new(1.0, -1.0), Point::new(1.0, -1.0), 0.0, false),
            0.0
        );
        assert_eq!(
            text_rotation(o, Point::new(1.0, -1.0), Point::new(-1.0, -1.0), 0.0, false),
            90.0
        );
        assert_eq!(
            text_rotation(o, Point::new(1.0, 0.0), Point::new(0.0, 1.0), 30.0, false),
            -60.0
        );
    }
}
