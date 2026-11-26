//! Planar geometry in KiCad board coordinates: millimetres, Y pointing down.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const ORIGIN: Point = Point { x: 0.0, y: 0.0 };

    pub fn new(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    pub fn dist(self, o: Point) -> f64 {
        (self.x - o.x).hypot(self.y - o.y)
    }

    /// Rotates by `deg` degrees the way KiCad rotates footprints: positive
    /// angles turn counter-clockwise as seen on screen, where Y points down.
    /// In Y-down coordinates that is `(x cos + y sin, -x sin + y cos)`.
    pub fn rotate_kicad(self, deg: f64) -> Point {
        if deg == 0.0 {
            return self;
        }
        let (s, c) = sin_cos_deg(deg);
        Point::new(self.x * c + self.y * s, -self.x * s + self.y * c)
    }

    pub fn approx_eq(self, o: Point, tol: f64) -> bool {
        (self.x - o.x).abs() <= tol && (self.y - o.y).abs() <= tol
    }
}

impl std::ops::Add for Point {
    type Output = Point;
    fn add(self, o: Point) -> Point {
        Point::new(self.x + o.x, self.y + o.y)
    }
}

impl std::ops::Sub for Point {
    type Output = Point;
    fn sub(self, o: Point) -> Point {
        Point::new(self.x - o.x, self.y - o.y)
    }
}

/// Sine and cosine of an angle in degrees, exact for multiples of 90.
pub fn sin_cos_deg(deg: f64) -> (f64, f64) {
    let r = deg.rem_euclid(360.0);
    if r == 0.0 {
        (0.0, 1.0)
    } else if r == 90.0 {
        (1.0, 0.0)
    } else if r == 180.0 {
        (0.0, -1.0)
    } else if r == 270.0 {
        (-1.0, 0.0)
    } else {
        r.to_radians().sin_cos()
    }
}

/// Normalises an angle to `(-180, 180]` degrees.
pub fn normalize_deg(deg: f64) -> f64 {
    let r = deg.rem_euclid(360.0);
    if r > 180.0 {
        r - 360.0
    } else {
        r
    }
}

/// Placement of a footprint: local coordinates to board coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub origin: Point,
    pub rotation: f64,
}

impl Placement {
    pub fn apply(&self, local: Point) -> Point {
        self.origin + local.rotate_kicad(self.rotation)
    }
}

/// 2D affine map in SVG matrix order: `(x, y)` maps to
/// `(a x + c y + e, b x + d y + f)`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Affine {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Default for Affine {
    fn default() -> Self {
        Affine::IDENTITY
    }
}

impl Affine {
    pub const IDENTITY: Affine = Affine {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    pub const fn new(a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) -> Affine {
        Affine { a, b, c, d, e, f }
    }

    pub fn translate(tx: f64, ty: f64) -> Affine {
        Affine::new(1.0, 0.0, 0.0, 1.0, tx, ty)
    }

    pub fn scale(sx: f64, sy: f64) -> Affine {
        Affine::new(sx, 0.0, 0.0, sy, 0.0, 0.0)
    }

    /// Rotation by `deg` with the SVG sense: positive angles turn +X towards
    /// +Y (clockwise on a Y-down screen).
    pub fn rotate_deg(deg: f64) -> Affine {
        let (s, c) = sin_cos_deg(deg);
        Affine::new(c, s, -s, c, 0.0, 0.0)
    }

    /// `self` followed by `next`: `next.apply(self.apply(p))`.
    pub fn then(&self, next: &Affine) -> Affine {
        let m = self;
        let n = next;
        Affine {
            a: n.a * m.a + n.c * m.b,
            b: n.b * m.a + n.d * m.b,
            c: n.a * m.c + n.c * m.d,
            d: n.b * m.c + n.d * m.d,
            e: n.a * m.e + n.c * m.f + n.e,
            f: n.b * m.e + n.d * m.f + n.f,
        }
    }

    pub fn apply(&self, p: Point) -> Point {
        Point::new(
            self.a * p.x + self.c * p.y + self.e,
            self.b * p.x + self.d * p.y + self.f,
        )
    }

    /// Applies only the linear part (for vectors).
    pub fn apply_vector(&self, p: Point) -> Point {
        Point::new(self.a * p.x + self.c * p.y, self.b * p.x + self.d * p.y)
    }

    pub fn det(&self) -> f64 {
        self.a * self.d - self.b * self.c
    }

    /// Geometric mean scale factor, `sqrt(|det|)`; maps stroke widths.
    pub fn mean_scale(&self) -> f64 {
        self.det().abs().sqrt()
    }

    /// True if the map reverses orientation.
    pub fn is_mirroring(&self) -> bool {
        self.det() < 0.0
    }

    pub fn inverse(&self) -> Option<Affine> {
        let det = self.det();
        if !det.is_finite() || det.abs() < 1e-300 {
            return None;
        }
        let a = self.d / det;
        let b = -self.b / det;
        let c = -self.c / det;
        let d = self.a / det;
        Some(Affine {
            a,
            b,
            c,
            d,
            e: -(a * self.e + c * self.f),
            f: -(b * self.e + d * self.f),
        })
    }

    /// Box of the four mapped corners.
    pub fn apply_bbox(&self, b: &BBox) -> BBox {
        let pts = [
            self.apply(b.min),
            self.apply(Point::new(b.max.x, b.min.y)),
            self.apply(b.max),
            self.apply(Point::new(b.min.x, b.max.y)),
        ];
        let mut r = BBox::from_point(pts[0]);
        for p in &pts[1..] {
            r.include(*p);
        }
        r
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct BBox {
    pub min: Point,
    pub max: Point,
}

impl BBox {
    pub fn from_point(p: Point) -> BBox {
        BBox { min: p, max: p }
    }

    pub fn from_points(points: impl IntoIterator<Item = Point>) -> Option<BBox> {
        let mut it = points.into_iter();
        let mut b = BBox::from_point(it.next()?);
        for p in it {
            b.include(p);
        }
        Some(b)
    }

    pub fn include(&mut self, p: Point) {
        self.min.x = self.min.x.min(p.x);
        self.min.y = self.min.y.min(p.y);
        self.max.x = self.max.x.max(p.x);
        self.max.y = self.max.y.max(p.y);
    }

    pub fn union(&self, o: &BBox) -> BBox {
        let mut b = *self;
        b.include(o.min);
        b.include(o.max);
        b
    }

    pub fn width(&self) -> f64 {
        self.max.x - self.min.x
    }

    pub fn height(&self) -> f64 {
        self.max.y - self.min.y
    }

    pub fn center(&self) -> Point {
        Point::new(
            (self.min.x + self.max.x) / 2.0,
            (self.min.y + self.max.y) / 2.0,
        )
    }

    pub fn approx_eq(&self, o: &BBox, tol: f64) -> bool {
        self.min.approx_eq(o.min, tol) && self.max.approx_eq(o.max, tol)
    }
}

/// Unions optional boxes.
pub fn union_opt(a: Option<BBox>, b: Option<BBox>) -> Option<BBox> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.union(&b)),
        (a, None) => a,
        (None, b) => b,
    }
}

/// Centre and radius of the circle through three points, `None` if they are
/// (nearly) collinear.
pub fn circle_through(a: Point, b: Point, c: Point) -> Option<(Point, f64)> {
    let d = 2.0 * (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y));
    let scale = a.dist(b).max(b.dist(c)).max(a.dist(c));
    if scale == 0.0 || d.abs() <= 1e-12 * scale * scale {
        return None;
    }
    let a2 = a.x * a.x + a.y * a.y;
    let b2 = b.x * b.x + b.y * b.y;
    let c2 = c.x * c.x + c.y * c.y;
    let ux = (a2 * (b.y - c.y) + b2 * (c.y - a.y) + c2 * (a.y - b.y)) / d;
    let uy = (a2 * (c.x - b.x) + b2 * (a.x - c.x) + c2 * (b.x - a.x)) / d;
    let center = Point::new(ux, uy);
    Some((center, center.dist(a)))
}

/// Bounding box of the arc from `start` through `mid` to `end`. Collinear
/// points degrade to the box of the three points.
pub fn arc_bbox(start: Point, mid: Point, end: Point) -> BBox {
    let mut b = BBox::from_point(start);
    b.include(mid);
    b.include(end);
    let Some((c, r)) = circle_through(start, mid, end) else {
        return b;
    };
    let ang = |p: Point| (p.y - c.y).atan2(p.x - c.x);
    let tau = std::f64::consts::TAU;
    let (a0, am, a1) = (ang(start), ang(mid), ang(end));
    let ccw_span = |from: f64, to: f64| (to - from).rem_euclid(tau);
    // Walk from a0 in increasing angle; if mid is passed before end the arc
    // runs that way, otherwise it runs from a1 to a0 in increasing angle.
    let (from, span) = if ccw_span(a0, am) <= ccw_span(a0, a1) {
        (a0, ccw_span(a0, a1))
    } else {
        (a1, ccw_span(a1, a0))
    };
    for (k, dir) in [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)]
        .into_iter()
        .enumerate()
    {
        let t = k as f64 * std::f64::consts::FRAC_PI_2;
        if ccw_span(from, t) <= span {
            b.include(Point::new(c.x + r * dir.0, c.y + r * dir.1));
        }
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Point, b: Point) -> bool {
        a.approx_eq(b, 1e-9)
    }

    #[test]
    fn kicad_rotation_is_ccw_on_screen() {
        // +X rotated by 90 (CCW on a Y-down screen) points up, i.e. -Y.
        assert!(close(
            Point::new(1.0, 0.0).rotate_kicad(90.0),
            Point::new(0.0, -1.0)
        ));
        assert!(close(
            Point::new(0.0, 1.0).rotate_kicad(90.0),
            Point::new(1.0, 0.0)
        ));
        assert!(close(
            Point::new(1.0, 2.0).rotate_kicad(180.0),
            Point::new(-1.0, -2.0)
        ));
        assert!(close(
            Point::new(1.0, 0.0).rotate_kicad(-90.0),
            Point::new(0.0, 1.0)
        ));
        let p = Point::new(1.0, 0.0).rotate_kicad(45.0);
        assert!((p.x - p.y.abs()).abs() < 1e-12 && p.y < 0.0);
    }

    #[test]
    fn normalize() {
        assert_eq!(normalize_deg(270.0), -90.0);
        assert_eq!(normalize_deg(-180.0), 180.0);
        assert_eq!(normalize_deg(360.0), 0.0);
        assert_eq!(normalize_deg(90.0), 90.0);
    }

    #[test]
    fn circle_through_three_points() {
        let (c, r) = circle_through(
            Point::new(1.0, 0.0),
            Point::new(0.0, 1.0),
            Point::new(-1.0, 0.0),
        )
        .unwrap();
        assert!(close(c, Point::ORIGIN));
        assert!((r - 1.0).abs() < 1e-12);
        assert!(
            circle_through(Point::ORIGIN, Point::new(1.0, 1.0), Point::new(2.0, 2.0)).is_none()
        );
    }

    #[test]
    fn arc_bbox_includes_crossed_extremes_only() {
        // Upper half circle in Y-down coordinates: through (0,-1).
        let b = arc_bbox(
            Point::new(1.0, 0.0),
            Point::new(0.0, -1.0),
            Point::new(-1.0, 0.0),
        );
        assert!(b.approx_eq(
            &BBox {
                min: Point::new(-1.0, -1.0),
                max: Point::new(1.0, 0.0)
            },
            1e-12
        ));
        // Same endpoints, other way round: lower half.
        let b = arc_bbox(
            Point::new(1.0, 0.0),
            Point::new(0.0, 1.0),
            Point::new(-1.0, 0.0),
        );
        assert!(b.approx_eq(
            &BBox {
                min: Point::new(-1.0, 0.0),
                max: Point::new(1.0, 1.0)
            },
            1e-12
        ));
        // Small quarter arc crossing no extreme.
        let s = std::f64::consts::FRAC_1_SQRT_2;
        let m = (22.5f64).to_radians();
        let b = arc_bbox(
            Point::new(1.0, 0.0),
            Point::new(m.cos(), m.sin()),
            Point::new(s, s),
        );
        assert!(b.approx_eq(
            &BBox {
                min: Point::new(s, 0.0),
                max: Point::new(1.0, s)
            },
            1e-12
        ));
        // Three-quarter arc.
        let b = arc_bbox(
            Point::new(1.0, 0.0),
            Point::new(-1.0, 0.0),
            Point::new(0.0, 1.0),
        );
        assert!(b.approx_eq(
            &BBox {
                min: Point::new(-1.0, -1.0),
                max: Point::new(1.0, 1.0)
            },
            1e-12
        ));
    }

    #[test]
    fn affine_compose_invert_and_orientation() {
        let t = Affine::translate(3.0, -2.0);
        let s = Affine::scale(2.0, -1.0);
        let r = Affine::rotate_deg(90.0);
        let m = t.then(&s).then(&r);
        let p = Point::new(1.5, 4.0);
        let step = r.apply(s.apply(t.apply(p)));
        assert!(close(m.apply(p), step));
        let inv = m.inverse().unwrap();
        assert!(close(inv.apply(m.apply(p)), p));
        assert!(m.is_mirroring());
        assert!((m.mean_scale() - 2f64.sqrt()).abs() < 1e-12);
        // SVG rotate(90): +X goes to +Y.
        assert!(close(r.apply(Point::new(1.0, 0.0)), Point::new(0.0, 1.0)));
        assert!(Affine::scale(0.0, 1.0).inverse().is_none());
        let b = BBox {
            min: Point::new(0.0, 0.0),
            max: Point::new(2.0, 1.0),
        };
        let mb = r.apply_bbox(&b);
        assert!(mb.approx_eq(
            &BBox {
                min: Point::new(-1.0, 0.0),
                max: Point::new(0.0, 2.0)
            },
            1e-12
        ));
    }
}
