//! Path geometry: subpaths of lines and cubic Béziers, exact bounds.

use kdraft_board::geom::{Affine, BBox, Point};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "seg", rename_all = "lowercase")]
pub enum Seg {
    Line { to: Point },
    Cubic { c1: Point, c2: Point, to: Point },
}

impl Seg {
    pub fn end(&self) -> Point {
        match *self {
            Seg::Line { to } | Seg::Cubic { to, .. } => to,
        }
    }

    fn map(&self, m: &Affine) -> Seg {
        match *self {
            Seg::Line { to } => Seg::Line { to: m.apply(to) },
            Seg::Cubic { c1, c2, to } => Seg::Cubic {
                c1: m.apply(c1),
                c2: m.apply(c2),
                to: m.apply(to),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SubPath {
    pub start: Point,
    pub segs: Vec<Seg>,
    pub closed: bool,
}

impl SubPath {
    pub fn new(start: Point) -> SubPath {
        SubPath {
            start,
            segs: Vec::new(),
            closed: false,
        }
    }

    pub fn transform(&self, m: &Affine) -> SubPath {
        SubPath {
            start: m.apply(self.start),
            segs: self.segs.iter().map(|s| s.map(m)).collect(),
            closed: self.closed,
        }
    }

    /// Exact bounding box (cubic extremes included).
    pub fn bbox(&self) -> BBox {
        let mut b = BBox::from_point(self.start);
        let mut cur = self.start;
        for s in &self.segs {
            match *s {
                Seg::Line { to } => b.include(to),
                Seg::Cubic { c1, c2, to } => {
                    for p in cubic_extremes(cur, c1, c2, to) {
                        b.include(p);
                    }
                    b.include(to);
                }
            }
            cur = s.end();
        }
        b
    }

    /// Start point and segment end points.
    pub fn vertices(&self) -> impl Iterator<Item = Point> + '_ {
        std::iter::once(self.start).chain(self.segs.iter().map(Seg::end))
    }

    /// True if the subpath encloses an area: closed, or ending where it
    /// started, with at least two segments.
    pub fn is_area(&self) -> bool {
        let ends_at_start = self
            .segs
            .last()
            .is_some_and(|s| s.end().approx_eq(self.start, 1e-9));
        (self.closed || ends_at_start) && self.segs.len() >= 2
    }
}

/// Union of subpath boxes.
pub fn bbox_of(subpaths: &[SubPath]) -> Option<BBox> {
    let mut it = subpaths.iter();
    let mut b = it.next()?.bbox();
    for s in it {
        b = b.union(&s.bbox());
    }
    Some(b)
}

/// Points of the cubic at its interior axis extremes.
fn cubic_extremes(p0: Point, p1: Point, p2: Point, p3: Point) -> Vec<Point> {
    let mut ts = Vec::new();
    for (a0, a1, a2, a3) in [(p0.x, p1.x, p2.x, p3.x), (p0.y, p1.y, p2.y, p3.y)] {
        // Derivative / 3: a t^2 + b t + c.
        let a = -a0 + 3.0 * a1 - 3.0 * a2 + a3;
        let b = 2.0 * (a0 - 2.0 * a1 + a2);
        let c = a1 - a0;
        if a.abs() < 1e-12 {
            if b.abs() > 1e-12 {
                ts.push(-c / b);
            }
        } else {
            let disc = b * b - 4.0 * a * c;
            if disc >= 0.0 {
                let r = disc.sqrt();
                ts.push((-b + r) / (2.0 * a));
                ts.push((-b - r) / (2.0 * a));
            }
        }
    }
    ts.into_iter()
        .filter(|t| *t > 0.0 && *t < 1.0)
        .map(|t| cubic_at(p0, p1, p2, p3, t))
        .collect()
}

pub fn cubic_at(p0: Point, p1: Point, p2: Point, p3: Point, t: f64) -> Point {
    let u = 1.0 - t;
    let w0 = u * u * u;
    let w1 = 3.0 * u * u * t;
    let w2 = 3.0 * u * t * t;
    let w3 = t * t * t;
    Point::new(
        w0 * p0.x + w1 * p1.x + w2 * p2.x + w3 * p3.x,
        w0 * p0.y + w1 * p1.y + w2 * p2.y + w3 * p3.y,
    )
}

/// Cubic segments approximating the SVG elliptical arc from `p0` to `p`
/// (SVG 1.1 implementation notes F.6.5/F.6.6). A degenerate arc is a line.
#[allow(clippy::too_many_arguments)]
pub fn arc_to_cubics(
    p0: Point,
    rx: f64,
    ry: f64,
    x_axis_rotation_deg: f64,
    large_arc: bool,
    sweep: bool,
    p: Point,
) -> Vec<Seg> {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if p0.approx_eq(p, 0.0) {
        return Vec::new();
    }
    if rx == 0.0 || ry == 0.0 {
        return vec![Seg::Line { to: p }];
    }
    let phi = x_axis_rotation_deg.to_radians();
    let (sphi, cphi) = phi.sin_cos();
    let dx = (p0.x - p.x) / 2.0;
    let dy = (p0.y - p.y) / 2.0;
    let x1 = cphi * dx + sphi * dy;
    let y1 = -sphi * dx + cphi * dy;
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }
    let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut coef = if den == 0.0 {
        0.0
    } else {
        (num / den).max(0.0).sqrt()
    };
    if large_arc == sweep {
        coef = -coef;
    }
    let cx1 = coef * rx * y1 / ry;
    let cy1 = -coef * ry * x1 / rx;
    let cx = cphi * cx1 - sphi * cy1 + (p0.x + p.x) / 2.0;
    let cy = sphi * cx1 + cphi * cy1 + (p0.y + p.y) / 2.0;
    let angle = |ux: f64, uy: f64, vx: f64, vy: f64| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    let ux = (x1 - cx1) / rx;
    let uy = (y1 - cy1) / ry;
    let vx = (-x1 - cx1) / rx;
    let vy = (-y1 - cy1) / ry;
    let theta1 = angle(1.0, 0.0, ux, uy);
    let mut dtheta = angle(ux, uy, vx, vy);
    let tau = std::f64::consts::TAU;
    if !sweep && dtheta > 0.0 {
        dtheta -= tau;
    } else if sweep && dtheta < 0.0 {
        dtheta += tau;
    }
    let n = (dtheta.abs() / std::f64::consts::FRAC_PI_2 - 1e-9)
        .ceil()
        .max(1.0) as usize;
    let step = dtheta / n as f64;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let pt = |t: f64| {
        let (s, c) = t.sin_cos();
        Point::new(
            cx + rx * c * cphi - ry * s * sphi,
            cy + rx * c * sphi + ry * s * cphi,
        )
    };
    let der = |t: f64| {
        let (s, c) = t.sin_cos();
        Point::new(
            -rx * s * cphi - ry * c * sphi,
            -rx * s * sphi + ry * c * cphi,
        )
    };
    let mut out = Vec::with_capacity(n);
    let mut t = theta1;
    for i in 0..n {
        let t2 = t + step;
        let a = pt(t);
        let b = if i + 1 == n { p } else { pt(t2) };
        let da = der(t);
        let db = der(t2);
        out.push(Seg::Cubic {
            c1: Point::new(a.x + k * da.x, a.y + k * da.y),
            c2: Point::new(b.x - k * db.x, b.y - k * db.y),
            to: b,
        });
        t = t2;
    }
    out
}

/// Closed circle as four cubics, starting at the rightmost point.
pub fn circle(center: Point, r: f64) -> SubPath {
    ellipse(center, r, r)
}

pub fn ellipse(c: Point, rx: f64, ry: f64) -> SubPath {
    let k = 0.552_284_749_830_793_6;
    let p = |x: f64, y: f64| Point::new(c.x + x, c.y + y);
    SubPath {
        start: p(rx, 0.0),
        segs: vec![
            Seg::Cubic {
                c1: p(rx, k * ry),
                c2: p(k * rx, ry),
                to: p(0.0, ry),
            },
            Seg::Cubic {
                c1: p(-k * rx, ry),
                c2: p(-rx, k * ry),
                to: p(-rx, 0.0),
            },
            Seg::Cubic {
                c1: p(-rx, -k * ry),
                c2: p(-k * rx, -ry),
                to: p(0.0, -ry),
            },
            Seg::Cubic {
                c1: p(k * rx, -ry),
                c2: p(rx, -k * ry),
                to: p(rx, 0.0),
            },
        ],
        closed: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circle_bbox_is_exact() {
        let b = circle(Point::new(1.0, 2.0), 3.0).bbox();
        assert!(b.approx_eq(
            &BBox {
                min: Point::new(-2.0, -1.0),
                max: Point::new(4.0, 5.0)
            },
            1e-9
        ));
    }

    #[test]
    fn arc_half_circle_reaches_extreme() {
        // From (1,0) to (-1,0), radius 1, sweep=1: in SVG (Y down) the
        // positive-angle direction passes through (0,1).
        let segs = arc_to_cubics(
            Point::new(1.0, 0.0),
            1.0,
            1.0,
            0.0,
            false,
            true,
            Point::new(-1.0, 0.0),
        );
        let sp = SubPath {
            start: Point::new(1.0, 0.0),
            segs,
            closed: false,
        };
        let b = sp.bbox();
        assert!((b.max.y - 1.0).abs() < 1e-3, "{b:?}");
        assert!(b.min.y.abs() < 1e-9);
        assert!(sp
            .segs
            .last()
            .unwrap()
            .end()
            .approx_eq(Point::new(-1.0, 0.0), 1e-12));
    }

    #[test]
    fn arc_radius_scaled_up_when_too_small() {
        let segs = arc_to_cubics(
            Point::new(0.0, 0.0),
            0.1,
            0.1,
            0.0,
            false,
            true,
            Point::new(2.0, 0.0),
        );
        let sp = SubPath {
            start: Point::ORIGIN,
            segs,
            closed: false,
        };
        let b = sp.bbox();
        // Centre (1,0), from angle 180 increasing: through (1,-1).
        assert!((b.min.y + 1.0).abs() < 1e-3, "{b:?}");
    }
}
