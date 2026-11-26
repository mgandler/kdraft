//! Snapping during a drag: the boxes captured when the drag starts and the
//! snapped delta for each pointer position.

use kdraft_board::geom::{BBox, Point};
use kdraft_engine::snap::{self, Guide, SnapInput};

#[derive(Debug, Clone, PartialEq)]
pub struct MoveCtx {
    /// Union of the moving items' boxes at the start.
    pub start: BBox,
    /// Boxes of the items that stay.
    pub others: Vec<BBox>,
    pub page: BBox,
    pub area: Option<BBox>,
    pub grid: f64,
}

impl MoveCtx {
    /// Delta to apply for a raw pointer delta, and the guides to draw.
    /// With snapping off the raw delta is used unchanged.
    pub fn delta(&self, raw: Point, enabled: bool, tolerance_mm: f64) -> (Point, Vec<Guide>) {
        if !enabled {
            return (raw, Vec::new());
        }
        let s = snap::snap(&SnapInput {
            moving: snap::moved(&self.start, raw),
            others: &self.others,
            page: self.page,
            area: self.area,
            grid: Some(self.grid),
            objects: true,
            tolerance: tolerance_mm,
        });
        (raw + s.delta, s.guides)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snaps_to_grid_and_edges() {
        let c = MoveCtx {
            start: BBox {
                min: Point::new(10.0, 10.0),
                max: Point::new(20.0, 20.0),
            },
            others: vec![BBox {
                min: Point::new(50.0, 100.0),
                max: Point::new(60.0, 110.0),
            }],
            page: BBox {
                min: Point::ORIGIN,
                max: Point::new(420.0, 297.0),
            },
            area: None,
            grid: 0.5,
        };
        let (d, g) = c.delta(Point::new(0.2, 0.3), true, 0.3);
        assert!(d.approx_eq(Point::new(0.0, 0.5), 1e-9), "{d:?}");
        assert!(g.is_empty());
        // Left edge near the other's left edge (50): snaps to it.
        let (d, g) = c.delta(Point::new(39.8, 0.0), true, 0.3);
        assert!((d.x - 40.0).abs() < 1e-9);
        assert!(!g.is_empty());
        let (d, g) = c.delta(Point::new(39.8, 0.1), false, 0.3);
        assert_eq!(d, Point::new(39.8, 0.1));
        assert!(g.is_empty());
    }
}
