//! Measurements between boxes for the Alt-hover overlay: the gaps between
//! the selection and another item, or to the drawing-area edges.

use kdraft_board::geom::{BBox, Point};

/// One measured gap: a line from `a` to `b` (sheet mm) and its length.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gap {
    pub a: Point,
    pub b: Point,
    pub value: f64,
}

fn mid(lo: f64, hi: f64) -> f64 {
    (lo + hi) / 2.0
}

/// Gaps between box `s` (selection) and box `o` (hovered): horizontal and
/// vertical where they are apart; when one contains the other, the four
/// inner distances.
pub fn between(s: &BBox, o: &BBox) -> Vec<Gap> {
    let mut out = Vec::new();
    let oy0 = s.min.y.max(o.min.y);
    let oy1 = s.max.y.min(o.max.y);
    let y = if oy0 <= oy1 {
        mid(oy0, oy1)
    } else {
        mid(s.min.y, s.max.y)
    };
    let ox0 = s.min.x.max(o.min.x);
    let ox1 = s.max.x.min(o.max.x);
    let x = if ox0 <= ox1 {
        mid(ox0, ox1)
    } else {
        mid(s.min.x, s.max.x)
    };
    let inside = |a: &BBox, b: &BBox| {
        a.min.x >= b.min.x && a.max.x <= b.max.x && a.min.y >= b.min.y && a.max.y <= b.max.y
    };
    if inside(s, o) || inside(o, s) {
        let (i, e) = if inside(s, o) { (s, o) } else { (o, s) };
        let cy = mid(i.min.y, i.max.y);
        let cx = mid(i.min.x, i.max.x);
        for (a, b) in [
            (Point::new(e.min.x, cy), Point::new(i.min.x, cy)),
            (Point::new(i.max.x, cy), Point::new(e.max.x, cy)),
            (Point::new(cx, e.min.y), Point::new(cx, i.min.y)),
            (Point::new(cx, i.max.y), Point::new(cx, e.max.y)),
        ] {
            let v = a.dist(b);
            if v > 1e-9 {
                out.push(Gap { a, b, value: v });
            }
        }
        return out;
    }
    if o.min.x >= s.max.x {
        out.push(Gap {
            a: Point::new(s.max.x, y),
            b: Point::new(o.min.x, y),
            value: o.min.x - s.max.x,
        });
    } else if o.max.x <= s.min.x {
        out.push(Gap {
            a: Point::new(o.max.x, y),
            b: Point::new(s.min.x, y),
            value: s.min.x - o.max.x,
        });
    }
    if o.min.y >= s.max.y {
        out.push(Gap {
            a: Point::new(x, s.max.y),
            b: Point::new(x, o.min.y),
            value: o.min.y - s.max.y,
        });
    } else if o.max.y <= s.min.y {
        out.push(Gap {
            a: Point::new(x, o.max.y),
            b: Point::new(x, s.min.y),
            value: s.min.y - o.max.y,
        });
    }
    out
}

/// Distances from `s` to the four edges of the drawing area `area`.
pub fn to_area(s: &BBox, area: &BBox) -> Vec<Gap> {
    let cy = mid(s.min.y, s.max.y);
    let cx = mid(s.min.x, s.max.x);
    [
        (Point::new(area.min.x, cy), Point::new(s.min.x, cy)),
        (Point::new(s.max.x, cy), Point::new(area.max.x, cy)),
        (Point::new(cx, area.min.y), Point::new(cx, s.min.y)),
        (Point::new(cx, s.max.y), Point::new(cx, area.max.y)),
    ]
    .into_iter()
    .map(|(a, b)| {
        let v = if a.x == b.x { b.y - a.y } else { b.x - a.x };
        Gap { a, b, value: v }
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(x: f64, y: f64, w: f64, h: f64) -> BBox {
        BBox {
            min: Point::new(x, y),
            max: Point::new(x + w, y + h),
        }
    }

    #[test]
    fn gaps() {
        let s = bx(0.0, 0.0, 10.0, 10.0);
        let g = between(&s, &bx(15.0, 2.0, 5.0, 5.0));
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].value, 5.0);
        assert_eq!(g[0].a, Point::new(10.0, 4.5));
        let g = between(&s, &bx(20.0, 30.0, 5.0, 5.0));
        assert_eq!(g.len(), 2);
        assert_eq!(g[1].value, 20.0);
        let g = between(&s, &bx(2.0, 2.0, 2.0, 2.0));
        assert_eq!(g.len(), 4);
        assert_eq!(g[0].value, 2.0);
        assert_eq!(g[1].value, 6.0);
        let a = to_area(&bx(10.0, 20.0, 10.0, 10.0), &bx(0.0, 0.0, 100.0, 50.0));
        assert_eq!(
            a.iter().map(|g| g.value).collect::<Vec<_>>(),
            vec![10.0, 80.0, 20.0, 20.0]
        );
    }
}
