//! Uniform-grid spatial index over boxes and segments (sheet mm).

use std::collections::HashMap;

use kdraft_board::geom::{BBox, Point};

/// Strict overlap of two boxes (touching edges do not overlap).
pub fn overlaps(a: &BBox, b: &BBox) -> bool {
    a.min.x < b.max.x && b.min.x < a.max.x && a.min.y < b.max.y && b.min.y < a.max.y
}

/// `inner` lies inside `outer` (edges may touch).
pub fn inside(inner: &BBox, outer: &BBox) -> bool {
    inner.min.x >= outer.min.x
        && inner.max.x <= outer.max.x
        && inner.min.y >= outer.min.y
        && inner.max.y <= outer.max.y
}

/// The segment `a`-`b` intersects the box (Liang-Barsky clipping).
pub fn segment_hits_box(a: Point, b: Point, r: &BBox) -> bool {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let mut t0: f64 = 0.0;
    let mut t1: f64 = 1.0;
    for (p, q) in [
        (-dx, a.x - r.min.x),
        (dx, r.max.x - a.x),
        (-dy, a.y - r.min.y),
        (dy, r.max.y - a.y),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return false;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}

/// Boxes bucketed into square cells.
#[derive(Debug, Clone)]
pub struct BoxGrid {
    cell: f64,
    cells: HashMap<(i64, i64), Vec<usize>>,
    pub boxes: Vec<BBox>,
}

impl BoxGrid {
    pub fn new(cell: f64) -> BoxGrid {
        BoxGrid {
            cell: cell.max(1e-3),
            cells: HashMap::new(),
            boxes: Vec::new(),
        }
    }

    fn range(&self, b: &BBox) -> (i64, i64, i64, i64) {
        let f = |v: f64| (v / self.cell).floor() as i64;
        (f(b.min.x), f(b.min.y), f(b.max.x), f(b.max.y))
    }

    /// Adds a box; returns its index.
    pub fn insert(&mut self, b: BBox) -> usize {
        let i = self.boxes.len();
        let (x0, y0, x1, y1) = self.range(&b);
        // Very large boxes would touch many cells; cap the work.
        if (x1 - x0 + 1) * (y1 - y0 + 1) <= 40_000 {
            for x in x0..=x1 {
                for y in y0..=y1 {
                    self.cells.entry((x, y)).or_default().push(i);
                }
            }
        } else {
            self.cells.entry((i64::MIN, i64::MIN)).or_default().push(i);
        }
        self.boxes.push(b);
        i
    }

    /// Indices of boxes overlapping `b` (strictly), ascending, except
    /// `skip`.
    pub fn query(&self, b: &BBox, skip: Option<usize>) -> Vec<usize> {
        let (x0, y0, x1, y1) = self.range(b);
        let mut out = Vec::new();
        let mut visit = |v: &Vec<usize>| {
            for &i in v {
                if Some(i) != skip && overlaps(&self.boxes[i], b) {
                    out.push(i);
                }
            }
        };
        if (x1 - x0 + 1) * (y1 - y0 + 1) <= 40_000 {
            for x in x0..=x1 {
                for y in y0..=y1 {
                    if let Some(v) = self.cells.get(&(x, y)) {
                        visit(v);
                    }
                }
            }
        } else {
            for v in self.cells.values() {
                visit(v);
            }
        }
        if let Some(v) = self.cells.get(&(i64::MIN, i64::MIN)) {
            visit(v);
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    pub fn any(&self, b: &BBox, skip: Option<usize>) -> bool {
        !self.query(b, skip).is_empty()
    }
}

/// Segments bucketed by their boxes.
#[derive(Debug, Clone)]
pub struct SegmentGrid {
    grid: BoxGrid,
    segs: Vec<(Point, Point)>,
}

impl SegmentGrid {
    pub fn new(cell: f64, segs: Vec<(Point, Point)>) -> SegmentGrid {
        let mut grid = BoxGrid::new(cell);
        for (a, b) in &segs {
            let mut bb = BBox::from_point(*a);
            bb.include(*b);
            // Axis-aligned segments have zero-area boxes; grow a little so
            // the strict overlap query finds them.
            let e = 1e-6;
            grid.insert(BBox {
                min: Point::new(bb.min.x - e, bb.min.y - e),
                max: Point::new(bb.max.x + e, bb.max.y + e),
            });
        }
        SegmentGrid { grid, segs }
    }

    /// Any segment crosses or lies inside `r`.
    pub fn hits(&self, r: &BBox) -> bool {
        self.grid
            .query(r, None)
            .into_iter()
            .any(|i| segment_hits_box(self.segs[i].0, self.segs[i].1, r))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(x0: f64, y0: f64, x1: f64, y1: f64) -> BBox {
        BBox {
            min: Point::new(x0, y0),
            max: Point::new(x1, y1),
        }
    }

    #[test]
    fn grid_queries() {
        let mut g = BoxGrid::new(2.0);
        g.insert(bx(0.0, 0.0, 1.0, 1.0));
        g.insert(bx(5.0, 5.0, 9.0, 6.0));
        g.insert(bx(-1000.0, -1000.0, 1000.0, 1000.0));
        assert_eq!(g.query(&bx(0.5, 0.5, 0.6, 0.6), None), vec![0, 2]);
        assert_eq!(g.query(&bx(8.0, 5.5, 8.5, 5.6), Some(2)), vec![1]);
        assert!(
            !g.any(&bx(1.0, 0.0, 2.0, 1.0), Some(2)),
            "touching is no overlap"
        );
    }

    #[test]
    fn segments() {
        let r = bx(0.0, 0.0, 1.0, 1.0);
        assert!(segment_hits_box(
            Point::new(-1.0, 0.5),
            Point::new(2.0, 0.5),
            &r
        ));
        assert!(segment_hits_box(
            Point::new(0.2, 0.2),
            Point::new(0.3, 0.3),
            &r
        ));
        assert!(!segment_hits_box(
            Point::new(-1.0, 2.0),
            Point::new(2.0, 2.0),
            &r
        ));
        assert!(!segment_hits_box(
            Point::new(-1.0, 0.9),
            Point::new(0.9, -1.0),
            &r
        ));
        let sg = SegmentGrid::new(1.0, vec![(Point::new(0.5, -5.0), Point::new(0.5, 5.0))]);
        assert!(sg.hits(&r));
        assert!(!sg.hits(&bx(2.0, 0.0, 3.0, 1.0)));
    }
}
