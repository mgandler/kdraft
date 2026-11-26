//! Dimension geometry on the sheet (mm): extension lines with gap and
//! overshoot, dimension line, arrowheads, value text kept upright and
//! moved outside when it does not fit between the arrows.
//!
//! Measured values are computed by the caller in board millimetres (or
//! from sheet distances divided by the view scale, which is the same for
//! a similarity transform); the geometry here only places them.

use kdraft_board::geom::{BBox, Point};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DimStyle {
    pub decimals: usize,
    pub trim_zeros: bool,
    pub text_height: f64,
    pub arrow_length: f64,
    pub arrow_width: f64,
    /// Gap between the feature and the start of an extension line.
    pub gap: f64,
    /// Extension line overshoot past the dimension line.
    pub overshoot: f64,
    pub line_width: f64,
}

impl Default for DimStyle {
    fn default() -> Self {
        DimStyle {
            decimals: 2,
            trim_zeros: false,
            text_height: 2.5,
            arrow_length: 2.5,
            arrow_width: 0.9,
            gap: 1.0,
            overshoot: 1.5,
            line_width: 0.18,
        }
    }
}

/// `value` with `decimals` decimals; optionally trailing zeros trimmed.
pub fn format_value(value: f64, decimals: usize, trim: bool) -> String {
    let mut s = format!("{value:.decimals$}");
    if trim && s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" || s.chars().all(|c| c == '-' || c == '0' || c == '.') && s.starts_with('-') {
        s = s.trim_start_matches('-').to_string();
    }
    s
}

/// Linear dimension direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Horizontal,
    Vertical,
    Aligned,
}

/// A laid-out dimension in sheet mm.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// Extension and dimension lines, leaders.
    pub lines: Vec<(Point, Point)>,
    /// Arrowheads: tip and unit direction the arrow points to.
    pub arrows: Vec<(Point, Point)>,
    pub text: String,
    /// Text centre.
    pub text_at: Point,
    /// Degrees counter-clockwise, in (-90, 90].
    pub text_rotation: f64,
    /// Box of the text (in its own rotated frame, as an axis-aligned box
    /// for 0/90 degrees; for other angles the rotated box's hull).
    pub text_box: BBox,
    /// The dimension line (for overlap checks).
    pub dim_line: (Point, Point),
}

fn sub(a: Point, b: Point) -> Point {
    Point::new(a.x - b.x, a.y - b.y)
}
fn add(a: Point, b: Point) -> Point {
    Point::new(a.x + b.x, a.y + b.y)
}
fn mul(a: Point, k: f64) -> Point {
    Point::new(a.x * k, a.y * k)
}
fn dot(a: Point, b: Point) -> f64 {
    a.x * b.x + a.y * b.y
}
fn norm(a: Point) -> Point {
    let l = a.x.hypot(a.y).max(1e-12);
    Point::new(a.x / l, a.y / l)
}

/// Upright angle (degrees CCW on a Y-down sheet) for a direction vector.
fn upright(d: Point) -> f64 {
    // Y down: CCW angle is atan2(-dy, dx).
    let mut a = (-d.y).atan2(d.x).to_degrees();
    if a <= -90.0 + 1e-9 {
        a += 180.0;
    } else if a > 90.0 + 1e-9 {
        a -= 180.0;
    }
    a
}

/// Hull of a `w` x `h` box centred at `c` rotated by `deg`.
fn rotated_box(c: Point, w: f64, h: f64, deg: f64) -> BBox {
    let (s, co) = deg.to_radians().sin_cos();
    let corners = [(-w, -h), (w, -h), (w, h), (-w, h)].map(|(x, y)| {
        let (x, y) = (x / 2.0, y / 2.0);
        // CCW on a Y-down sheet.
        Point::new(c.x + x * co + y * s, c.y - x * s + y * co)
    });
    BBox::from_points(corners).unwrap_or(BBox::from_point(c))
}

/// Linear dimension between sheet points `p1` and `p2`. `offset` is the
/// distance of the dimension line beyond the outermost point along the
/// normal (positive: up for horizontal, right for vertical, left of
/// `p1->p2` for aligned; negative: the other side). `text_width` gives the
/// width of a string at a cap height.
pub fn linear(
    p1: Point,
    p2: Point,
    dir: Direction,
    offset: f64,
    text: &str,
    st: &DimStyle,
    text_width: &dyn Fn(&str, f64) -> f64,
) -> Layout {
    let (u, n0) = match dir {
        Direction::Horizontal => (Point::new(1.0, 0.0), Point::new(0.0, -1.0)),
        Direction::Vertical => (Point::new(0.0, -1.0), Point::new(1.0, 0.0)),
        Direction::Aligned => {
            let u = norm(sub(p2, p1));
            // Left of p1->p2 on screen (Y down): (u.y, -u.x).
            (u, Point::new(u.y, -u.x))
        }
    };
    let n = if offset >= 0.0 { n0 } else { mul(n0, -1.0) };
    let off = offset.abs();
    let base = dot(p1, n).max(dot(p2, n));
    let level = base + off;
    let foot = |p: Point| add(p, mul(n, level - dot(p, n)));
    let (a, b) = (foot(p1), foot(p2));
    let mut lines = Vec::new();
    for (p, f) in [(p1, a), (p2, b)] {
        let len = dot(sub(f, p), n);
        if len > st.gap {
            lines.push((add(p, mul(n, st.gap)), add(f, mul(n, st.overshoot))));
        } else {
            lines.push((p, add(f, mul(n, st.overshoot))));
        }
    }
    let len = dot(sub(b, a), u).abs();
    let ua = norm(sub(b, a));
    let ua = if len < 1e-9 { u } else { ua };
    let th = st.text_height;
    let tw = text_width(text, th);
    let rot = upright(ua);
    // Text side: the side of the line away from the measured feature.
    let lift = 0.5 * th + 0.6 * th.min(1.0) + st.line_width;
    let fits = len >= tw + 2.0 * st.arrow_length + 1.0;
    let mut arrows = Vec::new();
    let text_at = if fits {
        lines.push((a, b));
        arrows.push((a, mul(ua, -1.0)));
        arrows.push((b, ua));
        add(mul(add(a, b), 0.5), mul(n, lift))
    } else {
        // Arrows outside pointing in, line extended past b for the text.
        let ext = st.arrow_length + 1.0;
        let a_out = add(a, mul(ua, -ext));
        let b_out = add(b, mul(ua, ext + tw + 1.0));
        lines.push((a_out, b_out));
        arrows.push((a, ua));
        arrows.push((b, mul(ua, -1.0)));
        let c = add(b, mul(ua, ext + 0.5 + tw / 2.0));
        add(c, mul(n, lift))
    };
    let text_box = rotated_box(text_at, tw, th, rot);
    Layout {
        lines,
        arrows,
        text: text.to_string(),
        text_at,
        text_rotation: rot,
        text_box,
        dim_line: (a, b),
    }
}

/// Radius (`diameter` false) or diameter dimension of a circle with sheet
/// centre `c` and radius `r`, drawn at `angle` degrees (CCW from +x on the
/// sheet), with the text on a horizontal shelf outside the circle.
pub fn radial(
    c: Point,
    r: f64,
    angle: f64,
    diameter: bool,
    text: &str,
    st: &DimStyle,
    text_width: &dyn Fn(&str, f64) -> f64,
) -> Layout {
    let (s, co) = angle.to_radians().sin_cos();
    let d = Point::new(co, -s);
    let p = add(c, mul(d, r));
    let out = add(p, mul(d, st.arrow_length + 2.0));
    let th = st.text_height;
    let tw = text_width(text, th);
    let right = d.x >= 0.0;
    let shelf_end = Point::new(out.x + if right { tw + 1.0 } else { -(tw + 1.0) }, out.y);
    let mut lines = vec![(out, shelf_end)];
    let mut arrows = Vec::new();
    if diameter {
        let q = add(c, mul(d, -r));
        lines.push((q, out));
        arrows.push((p, d));
        arrows.push((q, mul(d, -1.0)));
    } else {
        lines.push((c, out));
        arrows.push((p, d));
    }
    let text_at = Point::new(
        (out.x + shelf_end.x) / 2.0,
        out.y - (0.5 * th + 0.6 + st.line_width),
    );
    Layout {
        lines,
        arrows,
        text: text.to_string(),
        text_at,
        text_rotation: 0.0,
        text_box: rotated_box(text_at, tw, th, 0.0),
        dim_line: (out, shelf_end),
    }
}

/// Ordinate dimension: a leader from sheet point `p` along `axis` (true:
/// the value is an x coordinate, the leader runs vertically) to `level`
/// (the sheet y or x where all leaders of this axis end), text after it.
pub fn ordinate(
    p: Point,
    x_axis: bool,
    level: f64,
    text: &str,
    st: &DimStyle,
    text_width: &dyn Fn(&str, f64) -> f64,
) -> Layout {
    let th = st.text_height;
    let tw = text_width(text, th);
    let (end, dir) = if x_axis {
        let s = if level >= p.y { 1.0 } else { -1.0 };
        (Point::new(p.x, level), Point::new(0.0, s))
    } else {
        let s = if level >= p.x { 1.0 } else { -1.0 };
        (Point::new(level, p.y), Point::new(s, 0.0))
    };
    let start = add(p, mul(dir, st.gap));
    let text_at = add(end, mul(dir, 0.6 + tw / 2.0));
    let rot = if x_axis { 90.0 } else { 0.0 };
    let (bw, bh) = if x_axis { (th, tw) } else { (tw, th) };
    Layout {
        lines: vec![(start, end)],
        arrows: Vec::new(),
        text: text.to_string(),
        text_at,
        text_rotation: rot,
        text_box: BBox {
            min: Point::new(text_at.x - bw / 2.0, text_at.y - bh / 2.0),
            max: Point::new(text_at.x + bw / 2.0, text_at.y + bh / 2.0),
        },
        dim_line: (start, end),
    }
}

/// Distance from `p` to segment `a`-`b`.
pub fn point_segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let ab = sub(b, a);
    let l2 = dot(ab, ab);
    if l2 < 1e-18 {
        return p.dist(a);
    }
    let t = (dot(sub(p, a), ab) / l2).clamp(0.0, 1.0);
    p.dist(add(a, mul(ab, t)))
}

/// Value measured by a linear dimension, from sheet points and the view
/// scale.
pub fn measured(p1: Point, p2: Point, dir: Direction, scale: f64) -> f64 {
    let d = sub(p2, p1);
    let v = match dir {
        Direction::Horizontal => d.x.abs(),
        Direction::Vertical => d.y.abs(),
        Direction::Aligned => d.x.hypot(d.y),
    };
    v / scale
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::segment_hits_box;
    use proptest::prelude::*;

    fn tw(s: &str, h: f64) -> f64 {
        0.7 * h * s.chars().count() as f64
    }

    #[test]
    fn formatting() {
        assert_eq!(format_value(12.5, 2, false), "12.50");
        assert_eq!(format_value(12.5, 2, true), "12.5");
        assert_eq!(format_value(12.0, 2, true), "12");
        assert_eq!(format_value(-0.001, 2, false), "0.00");
        assert_eq!(format_value(0.004, 3, false), "0.004");
    }

    #[test]
    fn horizontal_above_with_text_inside() {
        let st = DimStyle::default();
        let l = linear(
            Point::new(10.0, 50.0),
            Point::new(110.0, 40.0),
            Direction::Horizontal,
            8.0,
            "100.00",
            &st,
            &tw,
        );
        // Line 8 mm above the higher point (y 40 -> 32).
        assert!((l.dim_line.0.y - 32.0).abs() < 1e-9);
        assert_eq!(l.arrows.len(), 2);
        assert!(l.text_at.y < 32.0 && (l.text_at.x - 60.0).abs() < 1e-9);
        assert_eq!(l.text_rotation, 0.0);
        // Extension lines start a gap away from the points and overshoot.
        let e1 = l.lines[0];
        assert!((e1.0.y - 49.0).abs() < 1e-9 && (e1.1.y - 30.5).abs() < 1e-9);
    }

    #[test]
    fn short_dimension_puts_text_outside() {
        let st = DimStyle::default();
        let l = linear(
            Point::new(0.0, 0.0),
            Point::new(3.0, 0.0),
            Direction::Horizontal,
            -5.0,
            "3.00",
            &st,
            &tw,
        );
        assert!(l.text_at.x > 3.0 + st.arrow_length);
        assert!(l.text_at.y > 5.0, "below: {:?}", l.text_at);
    }

    #[test]
    fn vertical_text_is_upright() {
        let l = linear(
            Point::new(0.0, 0.0),
            Point::new(0.0, 80.0),
            Direction::Vertical,
            10.0,
            "80.00",
            &DimStyle::default(),
            &tw,
        );
        assert_eq!(l.text_rotation, 90.0);
        assert!(l.text_at.x > 10.0);
    }

    proptest! {
        #[test]
        fn value_independent_of_scale_and_text_clear_of_line(
            x1 in -100.0f64..100.0, y1 in -100.0f64..100.0,
            x2 in -100.0f64..100.0, y2 in -100.0f64..100.0,
            scale in 0.25f64..10.0, off in -30.0f64..30.0, d in 0usize..3,
        ) {
            let dir = [Direction::Horizontal, Direction::Vertical, Direction::Aligned][d];
            let (b1, b2) = (Point::new(x1, y1), Point::new(x2, y2));
            prop_assume!(b1.dist(b2) > 0.1);
            let s1 = mul(b1, scale);
            let s2 = mul(b2, scale);
            let v = measured(s1, s2, dir, scale);
            let want = match dir {
                Direction::Horizontal => (x2 - x1).abs(),
                Direction::Vertical => (y2 - y1).abs(),
                Direction::Aligned => b1.dist(b2),
            };
            prop_assert!((v - want).abs() < 1e-9 * want.max(1.0));
            let st = DimStyle::default();
            let text = format_value(v, 2, false);
            let l = linear(s1, s2, dir, off, &text, &st, &tw);
            // The text never overlaps the dimension line (shrink the box
            // by a hair for touching edges).
            let b = l.text_box;
            let inner = BBox {
                min: Point::new(b.min.x + 1e-6, b.min.y + 1e-6),
                max: Point::new(b.max.x - 1e-6, b.max.y - 1e-6),
            };
            let (a, bb) = l.dim_line;
            if dir != Direction::Aligned {
                prop_assert!(!segment_hits_box(a, bb, &inner), "{:?} {:?}", l.dim_line, b);
            } else {
                // Rotated text: its centre is at least half its height
                // away from the line.
                prop_assert!(point_segment_distance(l.text_at, a, bb) >= 0.5 * st.text_height);
            }
            prop_assert!(l.text_rotation > -90.0 - 1e-9 && l.text_rotation <= 90.0 + 1e-9);
        }
    }
}
