//! View placement: board millimetres to sheet millimetres.

use kdraft_board::geom::{Affine, BBox, Point};
use kdraft_doc::{View, ViewSide};

/// Space between the view content and its frame, sheet mm.
pub const FRAME_MARGIN: f64 = 3.0;

#[derive(Debug, Clone, PartialEq)]
pub struct ViewMap {
    pub board_to_sheet: Affine,
    pub mirrored: bool,
    pub scale: f64,
    /// View rotation, degrees counter-clockwise on the sheet.
    pub rotation: f64,
    /// Board-coordinate content box (crop or outline box).
    pub content: BBox,
    /// Sheet box around the mapped content, with [`FRAME_MARGIN`].
    pub frame: BBox,
}

impl ViewMap {
    /// Places `content` (board mm) so its centre lands on `view.at`.
    /// Back views are mirrored about the vertical axis.
    pub fn new(view: &View, board_box: BBox) -> ViewMap {
        let content = view.crop.unwrap_or(board_box);
        let c = content.center();
        let mirrored = view.side == ViewSide::Back;
        let mut m = Affine::translate(-c.x, -c.y);
        if mirrored {
            m = m.then(&Affine::scale(-1.0, 1.0));
        }
        // Counter-clockwise on a Y-down sheet is a negative SVG angle.
        m = m
            .then(&Affine::rotate_deg(-view.rotation))
            .then(&Affine::scale(view.scale, view.scale))
            .then(&Affine::translate(view.at.x, view.at.y));
        let mapped = m.apply_bbox(&content);
        let frame = BBox {
            min: Point::new(mapped.min.x - FRAME_MARGIN, mapped.min.y - FRAME_MARGIN),
            max: Point::new(mapped.max.x + FRAME_MARGIN, mapped.max.y + FRAME_MARGIN),
        };
        ViewMap {
            board_to_sheet: m,
            mirrored,
            scale: view.scale,
            rotation: view.rotation,
            content,
            frame,
        }
    }

    pub fn point(&self, p: Point) -> Point {
        self.board_to_sheet.apply(p)
    }
}

/// `2:1`, `1:2`, `1:1`.
pub fn ratio(scale: f64) -> String {
    scale_label(scale).trim_start_matches("SCALE ").to_string()
}

/// `SCALE 2:1`, `SCALE 1:2`, `SCALE 1:1`.
pub fn scale_label(scale: f64) -> String {
    let fmt = |v: f64| {
        let r = (v * 1000.0).round() / 1000.0;
        if r.fract() == 0.0 {
            format!("{}", r as i64)
        } else {
            format!("{r}")
        }
    };
    if scale >= 1.0 {
        format!("SCALE {}:1", fmt(scale))
    } else if scale > 0.0 {
        format!("SCALE 1:{}", fmt(1.0 / scale))
    } else {
        "SCALE ?".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kdraft_doc::{Id, ViewKind};

    fn view(side: ViewSide, rotation: f64) -> View {
        View {
            id: Id("v".into()),
            kind: ViewKind::Assembly,
            side,
            scale: 2.0,
            rotation,
            at: Point::new(100.0, 50.0),
            layers: vec!["Edge.Cuts".into()],
            crop: None,
            parent: None,
            style: None,
            frame: false,
            drill_marks: None,
            render: None,
            assembly: Default::default(),
            extra: Vec::new(),
        }
    }

    fn board() -> BBox {
        BBox {
            min: Point::new(10.0, 10.0),
            max: Point::new(30.0, 20.0),
        }
    }

    #[test]
    fn front_back_and_rotation() {
        let f = ViewMap::new(&view(ViewSide::Front, 0.0), board());
        assert!(f
            .point(Point::new(20.0, 15.0))
            .approx_eq(Point::new(100.0, 50.0), 1e-12));
        assert!(f
            .point(Point::new(30.0, 20.0))
            .approx_eq(Point::new(120.0, 60.0), 1e-12));
        assert!(f.frame.min.approx_eq(Point::new(77.0, 37.0), 1e-12));
        let b = ViewMap::new(&view(ViewSide::Back, 0.0), board());
        assert!(b.mirrored);
        // The board's right edge appears on the left in a back view.
        assert!(b
            .point(Point::new(30.0, 20.0))
            .approx_eq(Point::new(80.0, 60.0), 1e-12));
        let r = ViewMap::new(&view(ViewSide::Front, 90.0), board());
        // +X on the board turns to "up" on the sheet (counter-clockwise).
        assert!(r
            .point(Point::new(30.0, 15.0))
            .approx_eq(Point::new(100.0, 30.0), 1e-12));
        assert!((r.frame.width() - 26.0).abs() < 1e-9);
    }

    #[test]
    fn labels() {
        assert_eq!(scale_label(2.0), "SCALE 2:1");
        assert_eq!(scale_label(1.0), "SCALE 1:1");
        assert_eq!(scale_label(0.5), "SCALE 1:2");
        assert_eq!(scale_label(2.5), "SCALE 2.5:1");
    }
}
