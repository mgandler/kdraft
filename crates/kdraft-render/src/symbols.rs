//! Drill symbols: one distinct symbol per drill-table row, drawn at every
//! hole of the row in the drill drawing and in the table's SYM column.
//!
//! Fixed order: plus, cross, circle, square, diamond, triangle, hexagon;
//! then each closed shape (circle, square, diamond, triangle, hexagon)
//! with a plus and with a cross inside; then letters A, B, ... in a
//! circle. Row 0 always gets the plus, and so on.

use kdraft_board::geom::{BBox, Point};

use crate::dl::{Color, HAlign, Item, Path, Stroke, TextRun, VAlign};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Plus,
    Cross,
    Circle,
    Square,
    Diamond,
    Triangle,
    Hexagon,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Symbol {
    Simple(Shape),
    /// A closed shape with a plus (`false`) or cross (`true`) inside.
    Combined(Shape, bool),
    /// A letter in a circle.
    Letter(String),
}

const CLOSED: [Shape; 5] = [
    Shape::Circle,
    Shape::Square,
    Shape::Diamond,
    Shape::Triangle,
    Shape::Hexagon,
];

/// Symbol for drill row `i`.
pub fn symbol(i: usize) -> Symbol {
    const SIMPLE: [Shape; 7] = [
        Shape::Plus,
        Shape::Cross,
        Shape::Circle,
        Shape::Square,
        Shape::Diamond,
        Shape::Triangle,
        Shape::Hexagon,
    ];
    if i < SIMPLE.len() {
        return Symbol::Simple(SIMPLE[i]);
    }
    let j = i - SIMPLE.len();
    if j < 2 * CLOSED.len() {
        return Symbol::Combined(CLOSED[j / 2], j % 2 == 1);
    }
    Symbol::Letter(crate::table::symbol(j - 2 * CLOSED.len()))
}

fn polygon(c: Point, r: f64, n: usize, start_deg: f64) -> Path {
    let pts: Vec<Point> = (0..n)
        .map(|k| {
            let a = (start_deg + 360.0 * k as f64 / n as f64).to_radians();
            Point::new(c.x + r * a.cos(), c.y - r * a.sin())
        })
        .collect();
    Path::polyline(&pts, true)
}

fn shape_path(s: Shape, c: Point, r: f64) -> Path {
    match s {
        Shape::Plus => {
            let mut p = Path::line(Point::new(c.x - r, c.y), Point::new(c.x + r, c.y));
            p.move_to(Point::new(c.x, c.y - r))
                .line_to(Point::new(c.x, c.y + r));
            p
        }
        Shape::Cross => {
            let d = r * std::f64::consts::FRAC_1_SQRT_2;
            let mut p = Path::line(Point::new(c.x - d, c.y - d), Point::new(c.x + d, c.y + d));
            p.move_to(Point::new(c.x - d, c.y + d))
                .line_to(Point::new(c.x + d, c.y - d));
            p
        }
        Shape::Circle => Path::circle(c, r),
        Shape::Square => {
            let d = r * 0.85;
            Path::rect(&BBox {
                min: Point::new(c.x - d, c.y - d),
                max: Point::new(c.x + d, c.y + d),
            })
        }
        Shape::Diamond => polygon(c, r, 4, 90.0),
        Shape::Triangle => polygon(Point::new(c.x, c.y + 0.15 * r), r, 3, 90.0),
        Shape::Hexagon => polygon(c, r, 6, 0.0),
    }
}

/// Items drawing symbol `sym` centred at `c` with overall size `size`
/// (sheet mm) and stroke width `w`.
pub fn draw(sym: &Symbol, c: Point, size: f64, w: f64, color: Color) -> Vec<Item> {
    let r = size / 2.0;
    let st = Stroke::solid(w, color);
    match sym {
        Symbol::Simple(s) => vec![Item::stroked(shape_path(*s, c, r), st)],
        Symbol::Combined(s, cross) => {
            let inner = if *cross { Shape::Cross } else { Shape::Plus };
            vec![
                Item::stroked(shape_path(*s, c, r), st.clone()),
                Item::stroked(shape_path(inner, c, 0.5 * r), st),
            ]
        }
        Symbol::Letter(l) => vec![
            Item::stroked(Path::circle(c, r), st),
            Item::Text(TextRun {
                text: l.clone(),
                at: c,
                size: 0.55 * size / (l.chars().count() as f64).max(1.0).sqrt(),
                h_align: HAlign::Center,
                v_align: VAlign::Center,
                rotation: 0.0,
                bold: false,
                italic: false,
                color,
                invisible: false,
            }),
        ],
    }
}

/// Symbol size per hole: `target`, but at most `max_frac` of the distance
/// to the nearest other hole and at least `min` (sheet mm).
pub fn sizes(centres: &[Point], target: f64, min: f64, max_frac: f64) -> Vec<f64> {
    let cell = target.max(0.5);
    let mut grid: std::collections::HashMap<(i64, i64), Vec<usize>> = Default::default();
    let k = |p: Point| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64);
    for (i, p) in centres.iter().enumerate() {
        grid.entry(k(*p)).or_default().push(i);
    }
    centres
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let (cx, cy) = k(*p);
            let mut near = f64::INFINITY;
            for dx in -1..=1 {
                for dy in -1..=1 {
                    if let Some(v) = grid.get(&(cx + dx, cy + dy)) {
                        for &j in v {
                            if j != i {
                                let d = p.dist(centres[j]);
                                if d > 1e-9 {
                                    near = near.min(d);
                                }
                            }
                        }
                    }
                }
            }
            target.min(max_frac * near).max(min)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_are_distinct_and_ordered() {
        let v: Vec<Symbol> = (0..40).map(symbol).collect();
        assert_eq!(v[0], Symbol::Simple(Shape::Plus));
        assert_eq!(v[1], Symbol::Simple(Shape::Cross));
        assert_eq!(v[2], Symbol::Simple(Shape::Circle));
        for i in 0..v.len() {
            for j in i + 1..v.len() {
                assert_ne!(v[i], v[j], "{i} {j}");
            }
        }
        assert_eq!(v[17], Symbol::Letter("A".into()));
    }

    #[test]
    fn sizes_clamp_to_pitch() {
        let c = [
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(50.0, 50.0),
        ];
        let s = sizes(&c, 2.0, 0.5, 0.7);
        assert!((s[0] - 0.7).abs() < 1e-9);
        assert_eq!(s[2], 2.0);
        let tight = sizes(&[Point::new(0.0, 0.0), Point::new(0.2, 0.0)], 2.0, 0.5, 0.7);
        assert_eq!(tight[0], 0.5);
    }
}
