//! Document operations as pure functions: each takes a document and returns
//! a new one, leaving the input untouched (the engine's undo log can keep
//! the old value).

use std::collections::HashMap;

use kdraft_board::geom::{sin_cos_deg, BBox, Point};

use crate::error::OpError;
use crate::model::*;

type R<T> = Result<T, OpError>;

fn page_index(doc: &Document, id: &Id) -> R<usize> {
    doc.pages
        .iter()
        .position(|p| &p.id == id)
        .ok_or_else(|| OpError::NoSuchPage(id.to_string()))
}

/// Inserts `page` at `index` (end if `None`).
pub fn add_page(doc: &Document, page: Page, index: Option<usize>) -> R<Document> {
    let mut d = doc.clone();
    let i = index.unwrap_or(d.pages.len());
    if i > d.pages.len() {
        return Err(OpError::PageIndex {
            index: i,
            len: d.pages.len(),
        });
    }
    d.pages.insert(i, page);
    Ok(d)
}

pub fn remove_page(doc: &Document, id: &Id) -> R<Document> {
    let mut d = doc.clone();
    let i = page_index(&d, id)?;
    d.pages.remove(i);
    Ok(d)
}

/// Moves the page at `from` so that it ends up at index `to`.
pub fn move_page(doc: &Document, from: usize, to: usize) -> R<Document> {
    let len = doc.pages.len();
    for index in [from, to] {
        if index >= len {
            return Err(OpError::PageIndex { index, len });
        }
    }
    let mut d = doc.clone();
    let p = d.pages.remove(from);
    d.pages.insert(to, p);
    Ok(d)
}

/// Copies a page with fresh ids for the page and its items, inserted right
/// after the original. References between items of the page (dimension and
/// callout `view`, view `parent`) are rewired to the copies. Returns the new
/// document and the new page id.
pub fn duplicate_page(
    doc: &Document,
    id: &Id,
    new_id: &mut dyn FnMut() -> Id,
) -> R<(Document, Id)> {
    let i = page_index(doc, id)?;
    let mut page = doc.pages[i].clone();
    page.id = new_id();
    let mut map: HashMap<Id, Id> = HashMap::new();
    for item in &mut page.items {
        if let Some(id) = item.id_mut() {
            let fresh = new_id();
            map.insert(id.clone(), fresh.clone());
            *id = fresh;
        }
    }
    let remap = |r: &mut Option<Id>| {
        if let Some(new) = r.as_ref().and_then(|old| map.get(old)) {
            *r = Some(new.clone());
        }
    };
    for item in &mut page.items {
        match item {
            Item::View(v) => remap(&mut v.parent),
            Item::Dimension(d) => remap(&mut d.view),
            Item::Callout(c) => remap(&mut c.view),
            _ => {}
        }
    }
    let new = page.id.clone();
    let mut d = doc.clone();
    d.pages.insert(i + 1, page);
    Ok((d, new))
}

/// Moves an item to another page (or within its page), inserting at
/// `index` in the target's z-order (top if `None`).
pub fn move_item(doc: &Document, item: &Id, to_page: &Id, index: Option<usize>) -> R<Document> {
    let mut d = doc.clone();
    let target = page_index(&d, to_page)?;
    let (pi, ii) = d
        .pages
        .iter()
        .enumerate()
        .find_map(|(pi, p)| {
            p.items
                .iter()
                .position(|i| i.id() == Some(item))
                .map(|ii| (pi, ii))
        })
        .ok_or_else(|| OpError::NoSuchItem(item.to_string()))?;
    let it = d.pages[pi].items.remove(ii);
    let items = &mut d.pages[target].items;
    let at = index.unwrap_or(items.len());
    if at > items.len() {
        return Err(OpError::PageIndex {
            index: at,
            len: items.len(),
        });
    }
    items.insert(at, it);
    Ok(d)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
    Top,
    Bottom,
    /// Same horizontal centre (centre x).
    HCenter,
    /// Same vertical centre (centre y).
    VCenter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Distribute {
    Horizontal,
    Vertical,
}

/// Sheet bounding box of an item, supplied by the caller (views need the
/// board extents, tables their layout).
pub type Bounds<'a> = dyn Fn(&Item) -> Option<BBox> + 'a;

/// Indices and boxes of the selected items on a page.
fn selection(page: &Page, ids: &[Id], bounds: &Bounds<'_>) -> R<Vec<(usize, BBox)>> {
    ids.iter()
        .map(|id| {
            let i = page
                .items
                .iter()
                .position(|it| it.id() == Some(id))
                .ok_or_else(|| OpError::NoSuchItem(id.to_string()))?;
            let item = &page.items[i];
            if matches!(item, Item::Dimension(_) | Item::Unknown(_)) {
                return Err(OpError::NotMovable(id.to_string()));
            }
            let b = bounds(item).ok_or_else(|| OpError::NoBounds(id.to_string()))?;
            Ok((i, b))
        })
        .collect()
}

/// Aligns the items' bounding boxes. Left/Right/Top/Bottom align to the
/// extreme edge of the selection; HCenter/VCenter to the centre of the
/// selection's overall box.
pub fn align(
    doc: &Document,
    page: &Id,
    ids: &[Id],
    mode: Align,
    bounds: &Bounds<'_>,
) -> R<Document> {
    let pi = page_index(doc, page)?;
    let sel = selection(&doc.pages[pi], ids, bounds)?;
    let Some(all) = sel.iter().map(|(_, b)| *b).reduce(|a, b| a.union(&b)) else {
        return Ok(doc.clone());
    };
    let mut d = doc.clone();
    for (i, b) in sel {
        let (dx, dy) = match mode {
            Align::Left => (all.min.x - b.min.x, 0.0),
            Align::Right => (all.max.x - b.max.x, 0.0),
            Align::Top => (0.0, all.min.y - b.min.y),
            Align::Bottom => (0.0, all.max.y - b.max.y),
            Align::HCenter => (all.center().x - b.center().x, 0.0),
            Align::VCenter => (0.0, all.center().y - b.center().y),
        };
        if dx != 0.0 || dy != 0.0 {
            d.pages[pi].items[i].translate(dx, dy);
        }
    }
    Ok(d)
}

/// Spaces items so the gaps between neighbouring boxes are equal. Items are
/// ordered by their left (top) edge; the first and last stay in place.
/// Fewer than three items are left unchanged.
pub fn distribute(
    doc: &Document,
    page: &Id,
    ids: &[Id],
    dir: Distribute,
    bounds: &Bounds<'_>,
) -> R<Document> {
    let pi = page_index(doc, page)?;
    let mut sel = selection(&doc.pages[pi], ids, bounds)?;
    if sel.len() < 3 {
        return Ok(doc.clone());
    }
    let lo = |b: &BBox| match dir {
        Distribute::Horizontal => b.min.x,
        Distribute::Vertical => b.min.y,
    };
    let size = |b: &BBox| match dir {
        Distribute::Horizontal => b.width(),
        Distribute::Vertical => b.height(),
    };
    sel.sort_by(|a, b| lo(&a.1).total_cmp(&lo(&b.1)));
    let first = sel[0].1;
    let last = sel[sel.len() - 1].1;
    let span = lo(&last) + size(&last) - lo(&first);
    let total: f64 = sel.iter().map(|(_, b)| size(b)).sum();
    let gap = (span - total) / (sel.len() - 1) as f64;
    let mut d = doc.clone();
    let mut cursor = lo(&first) + size(&first) + gap;
    let n = sel.len();
    for (i, b) in sel.into_iter().take(n - 1).skip(1) {
        let delta = cursor - lo(&b);
        let (dx, dy) = match dir {
            Distribute::Horizontal => (delta, 0.0),
            Distribute::Vertical => (0.0, delta),
        };
        if delta != 0.0 {
            d.pages[pi].items[i].translate(dx, dy);
        }
        cursor += size(&b) + gap;
    }
    Ok(d)
}

/// Aligns each item's box to `reference` (the first selected item's box, or
/// the drawing area). Items whose box already matches are left alone.
pub fn align_to(
    doc: &Document,
    page: &Id,
    ids: &[Id],
    mode: Align,
    reference: BBox,
    bounds: &Bounds<'_>,
) -> R<Document> {
    let pi = page_index(doc, page)?;
    let sel = selection(&doc.pages[pi], ids, bounds)?;
    let mut d = doc.clone();
    for (i, b) in sel {
        let (dx, dy) = align_delta(mode, &b, &reference);
        if dx != 0.0 || dy != 0.0 {
            d.pages[pi].items[i].translate(dx, dy);
        }
    }
    Ok(d)
}

/// Translation that aligns `b` to `r` in mode `m`.
pub fn align_delta(m: Align, b: &BBox, r: &BBox) -> (f64, f64) {
    match m {
        Align::Left => (r.min.x - b.min.x, 0.0),
        Align::Right => (r.max.x - b.max.x, 0.0),
        Align::Top => (0.0, r.min.y - b.min.y),
        Align::Bottom => (0.0, r.max.y - b.max.y),
        Align::HCenter => (r.center().x - b.center().x, 0.0),
        Align::VCenter => (0.0, r.center().y - b.center().y),
    }
}

/// Spaces items so their centres are equally spaced; items are ordered by
/// centre, the first and last stay in place. Fewer than three items are
/// left unchanged.
pub fn distribute_centres(
    doc: &Document,
    page: &Id,
    ids: &[Id],
    dir: Distribute,
    bounds: &Bounds<'_>,
) -> R<Document> {
    let pi = page_index(doc, page)?;
    let mut sel = selection(&doc.pages[pi], ids, bounds)?;
    if sel.len() < 3 {
        return Ok(doc.clone());
    }
    let c = |b: &BBox| match dir {
        Distribute::Horizontal => b.center().x,
        Distribute::Vertical => b.center().y,
    };
    sel.sort_by(|a, b| c(&a.1).total_cmp(&c(&b.1)));
    let n = sel.len();
    let (c0, c1) = (c(&sel[0].1), c(&sel[n - 1].1));
    let step = (c1 - c0) / (n - 1) as f64;
    let mut d = doc.clone();
    for (k, (i, b)) in sel.into_iter().enumerate().take(n - 1).skip(1) {
        let delta = c0 + step * k as f64 - c(&b);
        let (dx, dy) = match dir {
            Distribute::Horizontal => (delta, 0.0),
            Distribute::Vertical => (0.0, delta),
        };
        if delta != 0.0 {
            d.pages[pi].items[i].translate(dx, dy);
        }
    }
    Ok(d)
}

/// Moves items one step up (towards the top of the z-order) or, with
/// `to_end`, to the top. Relative order of the moved items is kept.
pub fn raise(doc: &Document, page: &Id, ids: &[Id], to_end: bool) -> R<Document> {
    restack(doc, page, ids, to_end, true)
}

/// Moves items one step down or, with `to_end`, to the bottom.
pub fn lower(doc: &Document, page: &Id, ids: &[Id], to_end: bool) -> R<Document> {
    restack(doc, page, ids, to_end, false)
}

fn restack(doc: &Document, page: &Id, ids: &[Id], to_end: bool, up: bool) -> R<Document> {
    let pi = page_index(doc, page)?;
    for id in ids {
        if doc.pages[pi].item(id).is_none() {
            return Err(OpError::NoSuchItem(id.to_string()));
        }
    }
    let mut d = doc.clone();
    let items = &mut d.pages[pi].items;
    let sel = |it: &Item| it.id().is_some_and(|i| ids.contains(i));
    if to_end {
        let (mut picked, rest): (Vec<Item>, Vec<Item>) =
            std::mem::take(items).into_iter().partition(|i| sel(i));
        *items = if up {
            let mut r = rest;
            r.append(&mut picked);
            r
        } else {
            picked.extend(rest);
            picked
        };
        return Ok(d);
    }
    let n = items.len();
    if up {
        for i in (0..n.saturating_sub(1)).rev() {
            if sel(&items[i]) && !sel(&items[i + 1]) {
                items.swap(i, i + 1);
            }
        }
    } else {
        for i in 1..n {
            if sel(&items[i]) && !sel(&items[i - 1]) {
                items.swap(i, i - 1);
            }
        }
    }
    Ok(d)
}

/// Copies items to `to_page` (may be their own page) with fresh ids,
/// translated by `offset`, on top of the target's z-order. Returns the new
/// document and the new ids in input order.
pub fn copy_items(
    doc: &Document,
    ids: &[Id],
    to_page: &Id,
    offset: Point,
    new_id: &mut dyn FnMut() -> Id,
) -> R<(Document, Vec<Id>)> {
    let target = page_index(doc, to_page)?;
    let mut copies = Vec::new();
    for id in ids {
        let item = doc
            .pages
            .iter()
            .find_map(|p| p.item(id))
            .ok_or_else(|| OpError::NoSuchItem(id.to_string()))?;
        let mut c = item.clone();
        if let Some(i) = c.id_mut() {
            *i = new_id();
        }
        c.translate(offset.x, offset.y);
        copies.push(c);
    }
    let new: Vec<Id> = copies.iter().filter_map(|c| c.id().cloned()).collect();
    // Dimensions and callouts copied with their view refer to the copy.
    let map: Vec<(Id, Id)> = ids.iter().cloned().zip(new.iter().cloned()).collect();
    for c in &mut copies {
        let v = match c {
            Item::Dimension(x) => &mut x.view,
            Item::Callout(x) => &mut x.view,
            _ => continue,
        };
        if let Some(old) = v.clone() {
            if let Some((_, n)) = map.iter().find(|(o, _)| *o == old) {
                *v = Some(n.clone());
            }
        }
    }
    let mut d = doc.clone();
    d.pages[target].items.extend(copies);
    Ok((d, new))
}

/// Removes items by id from whatever page holds them.
pub fn remove_items(doc: &Document, ids: &[Id]) -> R<Document> {
    let mut d = doc.clone();
    for id in ids {
        let mut found = false;
        for p in &mut d.pages {
            let before = p.items.len();
            p.items.retain(|i| i.id() != Some(id));
            found |= p.items.len() != before;
        }
        if !found {
            return Err(OpError::NoSuchItem(id.to_string()));
        }
    }
    Ok(d)
}

/// Sheet box of a view whose content (board outline box, or the view's
/// crop) is `board_box`, honouring scale and rotation.
pub fn view_bounds(view: &View, board_box: BBox) -> BBox {
    let content = view.crop.unwrap_or(board_box);
    let (s, c) = sin_cos_deg(view.rotation);
    let (w, h) = (content.width() * view.scale, content.height() * view.scale);
    let rw = (w * c).abs() + (h * s).abs();
    let rh = (w * s).abs() + (h * c).abs();
    BBox {
        min: Point::new(view.at.x - rw / 2.0, view.at.y - rh / 2.0),
        max: Point::new(view.at.x + rw / 2.0, view.at.y + rh / 2.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn rect(id: &str, x: f64, y: f64, w: f64, h: f64) -> Item {
        Item::Graphic(Graphic {
            id: Id(id.to_string()),
            shape: GraphicShape::Rect {
                start: Point::new(x, y),
                end: Point::new(x + w, y + h),
                radius: 0.0,
            },
            style: None,
            extra: Vec::new(),
        })
    }

    fn doc_with(items: Vec<Item>) -> Document {
        let mut p = Page::new("p", PageSize::a3_landscape());
        p.id = Id("p1".into());
        p.items = items;
        let mut q = Page::new("q", PageSize::a3_landscape());
        q.id = Id("p2".into());
        Document {
            version: CURRENT_VERSION,
            source: Source {
                board: "b.kicad_pcb".into(),
                ..Default::default()
            },
            styles: Styles::default(),
            sheet: None,
            pages: vec![p, q],
            extra: Vec::new(),
        }
    }

    fn graphic_bounds(item: &Item) -> Option<BBox> {
        match item {
            Item::Graphic(g) => g.shape.bbox(),
            _ => None,
        }
    }

    fn boxes(d: &Document) -> Vec<BBox> {
        d.pages[0].items.iter().filter_map(graphic_bounds).collect()
    }

    fn ids(n: usize) -> Vec<Id> {
        (0..n).map(|i| Id(format!("g{i}"))).collect()
    }

    #[test]
    fn align_each_mode() {
        let d = doc_with(vec![
            rect("g0", 0.0, 0.0, 10.0, 5.0),
            rect("g1", 20.0, 10.0, 4.0, 2.0),
        ]);
        let p = Id("p1".into());
        let check = |mode, f: &dyn Fn(&[BBox]) -> bool| {
            let out = align(&d, &p, &ids(2), mode, &graphic_bounds).unwrap();
            assert!(f(&boxes(&out)), "{mode:?}: {:?}", boxes(&out));
        };
        check(Align::Left, &|b| {
            b[0].min.x == 0.0 && b[1].min.x == 0.0 && b[1].min.y == 10.0
        });
        check(Align::Right, &|b| b[0].max.x == 24.0 && b[1].max.x == 24.0);
        check(Align::Top, &|b| {
            b[0].min.y == 0.0 && b[1].min.y == 0.0 && b[1].min.x == 20.0
        });
        check(Align::Bottom, &|b| b[0].max.y == 12.0 && b[1].max.y == 12.0);
        check(Align::HCenter, &|b| {
            b[0].center().x == 12.0 && b[1].center().x == 12.0
        });
        check(Align::VCenter, &|b| {
            b[0].center().y == 6.0 && b[1].center().y == 6.0
        });
        // Input untouched.
        assert_eq!(boxes(&d)[1].min.x, 20.0);
    }

    #[test]
    fn align_rejects_dimensions_and_unknown_ids() {
        let mut d = doc_with(vec![rect("g0", 0.0, 0.0, 1.0, 1.0)]);
        d.pages[0].items.push(Item::Dimension(Dimension {
            id: Id("d".into()),
            kind: DimensionKind::Linear,
            view: None,
            from: None,
            to: None,
            offset: 1.0,
            style: None,
            tolerance: None,
            axis: None,
            angle: None,
            last: Vec::new(),
            extra: Vec::new(),
        }));
        let p = Id("p1".into());
        assert_eq!(
            align(&d, &p, &[Id("d".into())], Align::Left, &graphic_bounds),
            Err(OpError::NotMovable("d".into()))
        );
        assert_eq!(
            align(&d, &p, &[Id("zz".into())], Align::Left, &graphic_bounds),
            Err(OpError::NoSuchItem("zz".into()))
        );
        assert!(matches!(
            align(&d, &Id("nope".into()), &[], Align::Left, &graphic_bounds),
            Err(OpError::NoSuchPage(_))
        ));
    }

    #[test]
    fn distribute_equal_gaps() {
        let d = doc_with(vec![
            rect("g0", 0.0, 0.0, 10.0, 1.0),
            rect("g1", 12.0, 0.0, 2.0, 1.0),
            rect("g2", 50.0, 0.0, 10.0, 1.0),
            rect("g3", 15.0, 0.0, 4.0, 1.0),
        ]);
        let out = distribute(
            &d,
            &Id("p1".into()),
            &ids(4),
            Distribute::Horizontal,
            &graphic_bounds,
        )
        .unwrap();
        let b = boxes(&out);
        // span 60, widths 26, gap 34/3
        let gap = 34.0 / 3.0;
        assert_eq!(b[0].min.x, 0.0);
        assert_eq!(b[2].min.x, 50.0);
        assert!((b[1].min.x - (10.0 + gap)).abs() < 1e-9);
        assert!((b[3].min.x - (12.0 + 2.0 * gap)).abs() < 1e-9);
    }

    #[test]
    fn page_operations() {
        let d = doc_with(vec![rect("g0", 0.0, 0.0, 1.0, 1.0)]);
        let moved = move_page(&d, 0, 1).unwrap();
        assert_eq!(moved.pages[0].id.as_str(), "p2");
        assert!(move_page(&d, 0, 2).is_err());

        let extra = Page::new("x", PageSize::a4_landscape());
        let xid = extra.id.clone();
        let added = add_page(&d, extra, Some(1)).unwrap();
        assert_eq!(added.pages[1].id, xid);
        assert!(add_page(&d, Page::new("y", PageSize::a4_landscape()), Some(9)).is_err());
        let removed = remove_page(&added, &xid).unwrap();
        assert_eq!(removed, d);
        assert!(remove_page(&d, &xid).is_err());

        let it = move_item(&d, &Id("g0".into()), &Id("p2".into()), None).unwrap();
        assert!(it.pages[0].items.is_empty());
        assert_eq!(it.pages[1].items.len(), 1);
        assert!(move_item(&d, &Id("nope".into()), &Id("p2".into()), None).is_err());
    }

    #[test]
    fn duplicate_page_rewires_internal_references() {
        let mut d = doc_with(vec![]);
        let view = View {
            id: Id("v1".into()),
            kind: ViewKind::Assembly,
            side: ViewSide::Front,
            scale: 1.0,
            rotation: 0.0,
            at: Point::new(100.0, 100.0),
            layers: vec!["F.Fab".into()],
            crop: None,
            parent: None,
            style: None,
            frame: false,
            drill_marks: None,
            render: None,
            assembly: Default::default(),
            extra: Vec::new(),
        };
        let mut detail = view.clone();
        detail.id = Id("v2".into());
        detail.parent = Some(Id("v1".into()));
        d.pages[0].items = vec![
            Item::View(view),
            Item::View(detail),
            Item::Dimension(Dimension {
                id: Id("d1".into()),
                kind: DimensionKind::Linear,
                view: Some(Id("v1".into())),
                from: Some(Anchor::OutlineVertex(0)),
                to: Some(Anchor::OutlineVertex(1)),
                offset: 8.0,
                style: None,
                tolerance: None,
                axis: None,
                angle: None,
                last: Vec::new(),
                extra: Vec::new(),
            }),
            Item::Dimension(Dimension {
                id: Id("d2".into()),
                kind: DimensionKind::Linear,
                view: Some(Id("elsewhere".into())),
                from: None,
                to: None,
                offset: 0.0,
                style: None,
                tolerance: None,
                axis: None,
                angle: None,
                last: Vec::new(),
                extra: Vec::new(),
            }),
        ];
        let mut n = 0;
        let mut gen = || {
            n += 1;
            Id(format!("new{n}"))
        };
        let (out, pid) = duplicate_page(&d, &Id("p1".into()), &mut gen).unwrap();
        assert_eq!(pid.as_str(), "new1");
        assert_eq!(out.pages.len(), 3);
        assert_eq!(out.pages[1].id, pid);
        assert_eq!(out.pages[0], d.pages[0]);
        let copy = &out.pages[1].items;
        let item_ids: Vec<_> = copy
            .iter()
            .filter_map(|i| i.id())
            .map(|i| i.as_str())
            .collect();
        assert_eq!(item_ids, ["new2", "new3", "new4", "new5"]);
        let Item::View(v2) = &copy[1] else { panic!() };
        assert_eq!(v2.parent.as_ref().map(Id::as_str), Some("new2"));
        let Item::Dimension(d1) = &copy[2] else {
            panic!()
        };
        assert_eq!(d1.view.as_ref().map(Id::as_str), Some("new2"));
        let Item::Dimension(d2) = &copy[3] else {
            panic!()
        };
        assert_eq!(d2.view.as_ref().map(Id::as_str), Some("elsewhere"));
    }

    #[test]
    fn view_bounds_scale_and_rotation() {
        let v = View {
            id: Id("v".into()),
            kind: ViewKind::Fabrication,
            side: ViewSide::Front,
            scale: 2.0,
            rotation: 90.0,
            at: Point::new(100.0, 50.0),
            layers: vec![],
            crop: None,
            parent: None,
            style: None,
            frame: false,
            drill_marks: None,
            render: None,
            assembly: Default::default(),
            extra: vec![],
        };
        let board = BBox {
            min: Point::new(10.0, 10.0),
            max: Point::new(40.0, 20.0),
        };
        let b = view_bounds(&v, board);
        assert_eq!((b.width(), b.height()), (20.0, 60.0));
        assert_eq!(b.center(), Point::new(100.0, 50.0));
    }

    fn rects() -> impl Strategy<Value = Vec<(f64, f64, f64, f64)>> {
        prop::collection::vec(
            (
                -200.0..200.0f64,
                -200.0..200.0f64,
                0.0..50.0f64,
                0.0..50.0f64,
            ),
            1..8,
        )
    }

    fn mode() -> impl Strategy<Value = Align> {
        prop_oneof![
            Just(Align::Left),
            Just(Align::Right),
            Just(Align::Top),
            Just(Align::Bottom),
            Just(Align::HCenter),
            Just(Align::VCenter),
        ]
    }

    fn make(rs: &[(f64, f64, f64, f64)]) -> Document {
        doc_with(
            rs.iter()
                .enumerate()
                .map(|(i, &(x, y, w, h))| rect(&format!("g{i}"), x, y, w, h))
                .collect(),
        )
    }

    fn approx(a: &[BBox], b: &[BBox]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.approx_eq(y, 1e-9))
    }

    proptest! {
        #[test]
        fn align_is_idempotent(rs in rects(), m in mode()) {
            let d = make(&rs);
            let p = Id("p1".into());
            let once = align(&d, &p, &ids(rs.len()), m, &graphic_bounds).unwrap();
            let twice = align(&once, &p, &ids(rs.len()), m, &graphic_bounds).unwrap();
            prop_assert!(approx(&boxes(&once), &boxes(&twice)));
            // Sizes never change.
            for (a, b) in boxes(&d).iter().zip(boxes(&once).iter()) {
                prop_assert!((a.width() - b.width()).abs() < 1e-9);
                prop_assert!((a.height() - b.height()).abs() < 1e-9);
            }
        }

        #[test]
        fn distribute_keeps_outer_items_and_equalises_gaps(rs in rects(), horiz in any::<bool>()) {
            let dir = if horiz { Distribute::Horizontal } else { Distribute::Vertical };
            let d = make(&rs);
            let out = distribute(&d, &Id("p1".into()), &ids(rs.len()), dir, &graphic_bounds).unwrap();
            let (before, after) = (boxes(&d), boxes(&out));
            let lo = |b: &BBox| if horiz { b.min.x } else { b.min.y };
            let hi = |b: &BBox| if horiz { b.max.x } else { b.max.y };
            let mut order: Vec<usize> = (0..rs.len()).collect();
            order.sort_by(|&a, &b| lo(&before[a]).total_cmp(&lo(&before[b])));
            let (f, l) = (order[0], order[order.len() - 1]);
            prop_assert_eq!(before[f], after[f]);
            prop_assert_eq!(before[l], after[l]);
            if rs.len() >= 3 {
                let gaps: Vec<f64> = order.windows(2).map(|w| lo(&after[w[1]]) - hi(&after[w[0]])).collect();
                for g in &gaps {
                    prop_assert!((g - gaps[0]).abs() < 1e-6, "{:?}", gaps);
                }
            } else {
                prop_assert_eq!(before, after);
            }
        }

        #[test]
        fn move_page_is_a_permutation(n in 1usize..8, a in 0usize..8, b in 0usize..8) {
            let mut d = make(&[]);
            d.pages = (0..n).map(|i| {
                let mut p = Page::new("x", PageSize::a4_landscape());
                p.id = Id(format!("p{i}"));
                p
            }).collect();
            let (a, b) = (a % n, b % n);
            let out = move_page(&d, a, b).unwrap();
            let mut x: Vec<_> = d.pages.iter().map(|p| p.id.clone()).collect();
            let mut y: Vec<_> = out.pages.iter().map(|p| p.id.clone()).collect();
            prop_assert_eq!(&out.pages[b].id, &d.pages[a].id);
            x.sort();
            y.sort();
            prop_assert_eq!(x, y);
            // Pages are moved whole.
            prop_assert_eq!(&out.pages[b], &d.pages[a]);
        }
    }
}
