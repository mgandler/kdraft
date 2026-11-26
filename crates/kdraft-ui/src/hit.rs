//! Hit testing and selection handles in sheet millimetres.

use kdraft_board::geom::{BBox, Point};
use kdraft_doc::Id;

/// Box grown by `d` on all sides.
pub fn grow(b: &BBox, d: f64) -> BBox {
    BBox {
        min: Point::new(b.min.x - d, b.min.y - d),
        max: Point::new(b.max.x + d, b.max.y + d),
    }
}

pub fn contains(b: &BBox, p: Point) -> bool {
    p.x >= b.min.x && p.x <= b.max.x && p.y >= b.min.y && p.y <= b.max.y
}

/// The top-most item whose box (grown by `tol`) contains `p`. `boxes` are in
/// z-order, bottom first. Among overlapping boxes the top-most wins, except
/// that a smaller box wholly inside a larger one wins over it (so a table on
/// a large view can be picked).
pub fn hit_test(boxes: &[(Id, BBox)], p: Point, tol: f64) -> Option<Id> {
    let hits: Vec<&(Id, BBox)> = boxes
        .iter()
        .filter(|(_, b)| contains(&grow(b, tol), p))
        .collect();
    let area = |b: &BBox| b.width() * b.height();
    let top = hits.last()?;
    let mut best = *top;
    for h in hits.iter().rev() {
        let inside = h.1.min.x >= best.1.min.x
            && h.1.min.y >= best.1.min.y
            && h.1.max.x <= best.1.max.x
            && h.1.max.y <= best.1.max.y;
        if inside && area(&h.1) < area(&best.1) {
            best = h;
        }
    }
    Some(best.0.clone())
}

/// Pick priority of an item kind: small annotation items (dimensions,
/// callouts, notes, graphics) before tables before views.
pub fn rank(item: &kdraft_doc::Item) -> u8 {
    use kdraft_doc::Item as I;
    match item {
        I::View(_) => 2,
        I::Table(_) => 1,
        _ => 0,
    }
}

/// Hit test with priorities: of the items whose box (grown by `tol`)
/// contains `p` and that pass the `precise` test, the lowest rank wins;
/// among equal ranks, [`hit_test`]'s rule. `items` are in z-order.
pub fn hit_ranked(
    items: &[(Id, BBox, u8)],
    p: Point,
    tol: f64,
    precise: &dyn Fn(&Id) -> bool,
) -> Option<Id> {
    let hits: Vec<&(Id, BBox, u8)> = items
        .iter()
        .filter(|(id, b, _)| contains(&grow(b, tol), p) && precise(id))
        .collect();
    let best = hits.iter().map(|h| h.2).min()?;
    let same: Vec<(Id, BBox)> = hits
        .iter()
        .filter(|h| h.2 == best)
        .map(|h| (h.0.clone(), h.1))
        .collect();
    hit_test(&same, p, tol)
}

/// Items whose boxes lie wholly inside `rect`, in z-order.
pub fn marquee(boxes: &[(Id, BBox)], rect: &BBox) -> Vec<Id> {
    boxes
        .iter()
        .filter(|(_, b)| {
            b.min.x >= rect.min.x
                && b.max.x <= rect.max.x
                && b.min.y >= rect.min.y
                && b.max.y <= rect.max.y
        })
        .map(|(i, _)| i.clone())
        .collect()
}

/// Normalised box from two corners.
pub fn rect_from(a: Point, b: Point) -> BBox {
    BBox {
        min: Point::new(a.x.min(b.x), a.y.min(b.y)),
        max: Point::new(a.x.max(b.x), a.y.max(b.y)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

impl Handle {
    /// Corner handles scale views; edge handles are drawn only.
    pub fn is_corner(self) -> bool {
        matches!(
            self,
            Handle::TopLeft | Handle::TopRight | Handle::BottomRight | Handle::BottomLeft
        )
    }
}

/// The eight handle positions of a box.
pub fn handles(b: &BBox) -> [(Handle, Point); 8] {
    let c = b.center();
    [
        (Handle::TopLeft, b.min),
        (Handle::Top, Point::new(c.x, b.min.y)),
        (Handle::TopRight, Point::new(b.max.x, b.min.y)),
        (Handle::Right, Point::new(b.max.x, c.y)),
        (Handle::BottomRight, b.max),
        (Handle::Bottom, Point::new(c.x, b.max.y)),
        (Handle::BottomLeft, Point::new(b.min.x, b.max.y)),
        (Handle::Left, Point::new(b.min.x, c.y)),
    ]
}

/// The handle within `tol` of `p`, nearest first.
pub fn hit_handle(b: &BBox, p: Point, tol: f64) -> Option<Handle> {
    handles(b)
        .into_iter()
        .map(|(h, q)| (h, q.dist(p)))
        .filter(|(_, d)| *d <= tol)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(h, _)| h)
}

/// Pointer shape over the canvas (mapped to egui's cursor icons in
/// `ui.rs`; kept here so it can be tested without egui).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorKind {
    Default,
    /// Over a selectable item.
    Grab,
    /// Moving items.
    Grabbing,
    ResizeNwSe,
    ResizeNeSw,
    /// Panning (space or middle button).
    Pan,
}

/// The cursor for a pointer over the canvas. `handle` is the scale handle
/// under the pointer (only corner handles of a single selected view are
/// active), `over_item` whether an item is under it.
pub fn cursor_for(
    handle: Option<Handle>,
    over_item: bool,
    moving: bool,
    panning: bool,
) -> CursorKind {
    if panning {
        return CursorKind::Pan;
    }
    if moving {
        return CursorKind::Grabbing;
    }
    match handle {
        Some(Handle::TopLeft | Handle::BottomRight) => CursorKind::ResizeNwSe,
        Some(Handle::TopRight | Handle::BottomLeft) => CursorKind::ResizeNeSw,
        _ if over_item => CursorKind::Grab,
        _ => CursorKind::Default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_kinds() {
        assert_eq!(cursor_for(None, false, false, false), CursorKind::Default);
        assert_eq!(cursor_for(None, true, false, false), CursorKind::Grab);
        assert_eq!(cursor_for(None, true, true, false), CursorKind::Grabbing);
        assert_eq!(
            cursor_for(Some(Handle::TopLeft), true, false, false),
            CursorKind::ResizeNwSe
        );
        assert_eq!(
            cursor_for(Some(Handle::BottomLeft), true, false, false),
            CursorKind::ResizeNeSw
        );
        // Edge handles are inactive: no resize cursor.
        assert_eq!(
            cursor_for(Some(Handle::Top), true, false, false),
            CursorKind::Grab
        );
        assert_eq!(cursor_for(None, true, true, true), CursorKind::Pan);
    }

    fn bx(x: f64, y: f64, w: f64, h: f64) -> BBox {
        BBox {
            min: Point::new(x, y),
            max: Point::new(x + w, y + h),
        }
    }

    #[test]
    fn top_most_and_nested() {
        let boxes = vec![
            (Id("view".into()), bx(0.0, 0.0, 100.0, 100.0)),
            (Id("over".into()), bx(50.0, 50.0, 100.0, 100.0)),
            (Id("small".into()), bx(10.0, 10.0, 5.0, 5.0)),
        ];
        assert_eq!(
            hit_test(&boxes, Point::new(60.0, 60.0), 0.0).unwrap().0,
            "over"
        );
        assert_eq!(
            hit_test(&boxes, Point::new(12.0, 12.0), 0.0).unwrap().0,
            "small"
        );
        assert_eq!(
            hit_test(&boxes, Point::new(1.0, 1.0), 0.0).unwrap().0,
            "view"
        );
        assert!(hit_test(&boxes, Point::new(-1.0, 1.0), 0.5).is_none());
        assert!(hit_test(&boxes, Point::new(-1.0, 1.0), 1.5).is_some());
        // A small box under a large later one still wins when inside it.
        let boxes = vec![
            (Id("small".into()), bx(10.0, 10.0, 5.0, 5.0)),
            (Id("big".into()), bx(0.0, 0.0, 100.0, 100.0)),
        ];
        assert_eq!(
            hit_test(&boxes, Point::new(12.0, 12.0), 0.0).unwrap().0,
            "small"
        );
    }

    #[test]
    fn ranked_hits_prefer_annotations_and_respect_precise_tests() {
        let items = vec![
            (Id("view".into()), bx(0.0, 0.0, 100.0, 100.0), 2),
            (Id("table".into()), bx(10.0, 10.0, 30.0, 30.0), 1),
            (Id("dim".into()), bx(-5.0, -10.0, 110.0, 12.0), 0),
        ];
        // The dimension's box covers (50, 1) but only its line at y = -8
        // is a precise hit.
        let on_line = |id: &Id| id.0 != "dim";
        assert_eq!(
            hit_ranked(&items, Point::new(50.0, 1.0), 0.5, &on_line)
                .unwrap()
                .0,
            "view"
        );
        let all = |_: &Id| true;
        assert_eq!(
            hit_ranked(&items, Point::new(50.0, 1.0), 0.5, &all)
                .unwrap()
                .0,
            "dim"
        );
        assert_eq!(
            hit_ranked(&items, Point::new(20.0, 20.0), 0.5, &all)
                .unwrap()
                .0,
            "table"
        );
        assert!(hit_ranked(&items, Point::new(500.0, 1.0), 0.5, &all).is_none());
    }

    #[test]
    fn marquee_and_handles() {
        let boxes = vec![
            (Id("a".into()), bx(0.0, 0.0, 10.0, 10.0)),
            (Id("b".into()), bx(20.0, 0.0, 10.0, 10.0)),
        ];
        let r = rect_from(Point::new(25.0, 15.0), Point::new(-1.0, -1.0));
        assert_eq!(marquee(&boxes, &r), vec![Id("a".into())]);
        let b = bx(0.0, 0.0, 10.0, 20.0);
        assert_eq!(
            hit_handle(&b, Point::new(10.2, 19.9), 0.5),
            Some(Handle::BottomRight)
        );
        assert_eq!(hit_handle(&b, Point::new(5.0, 0.1), 0.5), Some(Handle::Top));
        assert_eq!(hit_handle(&b, Point::new(5.0, 5.0), 0.5), None);
        assert!(Handle::TopLeft.is_corner() && !Handle::Left.is_corner());
    }
}
