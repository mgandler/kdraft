//! Placement links in the engine: after every edit (and every gesture
//! preview) a link group behaves as one placement. A change a command made
//! to a follower's followed properties moves the reference instead (the
//! follower's offset stays); then every follower is put at its
//! reference's placement. References that were deleted hand over to
//! their first follower.

use std::collections::HashSet;

use kdraft_board::geom::Point;
use kdraft_doc::link::{self, Follow, Link};
use kdraft_doc::{Document, GraphicShape, Id, Item};

fn placement_changed(a: &Item, b: &Item) -> bool {
    if a.position() != b.position() {
        return true;
    }
    match (a, b) {
        (Item::View(x), Item::View(y)) => {
            x.scale != y.scale || x.rotation != y.rotation || x.side != y.side
        }
        (Item::Graphic(x), Item::Graphic(y)) => match (&x.shape, &y.shape) {
            (
                GraphicShape::Rect {
                    start: s0, end: e0, ..
                },
                GraphicShape::Rect {
                    start: s1, end: e1, ..
                },
            ) => (e0.x - s0.x, e0.y - s0.y) != (e1.x - s1.x, e1.y - s1.y),
            (GraphicShape::Text { wrap: w0, .. }, GraphicShape::Text { wrap: w1, .. }) => w0 != w1,
            _ => false,
        },
        _ => false,
    }
}

/// Transfers what a command did to follower `now` (was `was`) onto the
/// reference `r`.
fn transfer(r: &mut Item, was: &Item, now: &Item, l: &Link) {
    if let (Some(a), Some(b)) = (was.position(), now.position()) {
        let dx = if l.follows_x() { b.x - a.x } else { 0.0 };
        let dy = if l.follows_y() { b.y - a.y } else { 0.0 };
        if dx != 0.0 || dy != 0.0 {
            r.translate(dx, dy);
        }
    }
    match (&mut *r, was, now) {
        (Item::View(rv), Item::View(w), Item::View(n)) => {
            if l.follows(Follow::Scale) && w.scale != n.scale {
                rv.scale = n.scale;
            }
            if l.follows(Follow::Rotation) && w.rotation != n.rotation {
                rv.rotation = n.rotation;
            }
            if l.follows(Follow::Side) && w.side != n.side {
                rv.side = n.side;
            }
        }
        (Item::Graphic(rg), Item::Graphic(_), Item::Graphic(n)) if l.follows(Follow::Size) => {
            match (&mut rg.shape, &n.shape) {
                (
                    GraphicShape::Rect { start, end, .. },
                    GraphicShape::Rect {
                        start: ns, end: ne, ..
                    },
                ) => {
                    *end = Point::new(start.x + (ne.x - ns.x), start.y + (ne.y - ns.y));
                }
                (GraphicShape::Text { wrap, .. }, GraphicShape::Text { wrap: nw, .. }) => {
                    *wrap = *nw;
                }
                _ => {}
            }
        }
        _ => {}
    }
}

/// Makes `doc` consistent after a command that started from `before`:
/// promotes followers of deleted references, moves references whose
/// followers the command moved, and resolves all links. Returns whether
/// anything changed.
pub fn settle(before: &Document, doc: &mut Document) -> bool {
    let mut changed = false;
    // Deleted references.
    let now = link::index(doc);
    let existed: HashSet<Id> = link::index(before)
        .into_keys()
        .filter(|id| !now.contains_key(id))
        .collect();
    if !existed.is_empty() && !link::promote_orphans(doc, &existed).is_empty() {
        changed = true;
    }
    // Group moves.
    let bix = link::index(before);
    let ix = link::index(doc);
    let mut roots: Vec<Id> = Vec::new();
    for p in &doc.pages {
        for it in &p.items {
            if let Some(l) = it.link() {
                if !roots.contains(&l.to) {
                    roots.push(l.to.clone());
                }
            }
        }
    }
    for root in roots {
        let (Some(r_now), Some(r_was)) =
            (link::get(doc, &ix, &root), link::get(before, &bix, &root))
        else {
            continue;
        };
        if placement_changed(r_was, r_now) {
            continue;
        }
        let mut moved: Option<(Item, Item, Link)> = None;
        for fid in link::followers(doc, &root) {
            let (Some(f_now), Some(f_was)) =
                (link::get(doc, &ix, &fid), link::get(before, &bix, &fid))
            else {
                continue;
            };
            let (Some(l_now), Some(l_was)) = (f_now.link(), f_was.link()) else {
                continue;
            };
            // A changed link (offset, followed properties) is not a move.
            if l_now != l_was {
                continue;
            }
            if placement_changed(f_was, f_now) {
                moved = Some((f_was.clone(), f_now.clone(), l_now));
                break;
            }
        }
        if let Some((was, now_f, l)) = moved {
            if let Some((p, z)) = ix.get(&root) {
                if let Some(r) = doc.pages.get_mut(*p).and_then(|pg| pg.items.get_mut(*z)) {
                    transfer(r, &was, &now_f, &l);
                    changed = true;
                }
            }
        }
    }
    if !link::resolve(doc).is_empty() {
        changed = true;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use kdraft_doc::{Graphic, Page, PageSize};

    fn rect(id: &str, x: f64) -> Item {
        Item::Graphic(Graphic {
            id: Id(id.into()),
            shape: GraphicShape::Rect {
                start: Point::new(x, 10.0),
                end: Point::new(x + 5.0, 15.0),
                radius: 0.0,
            },
            style: None,
            extra: vec![],
        })
    }

    #[test]
    fn follower_move_moves_the_group() {
        let mut d = Document::new("b.kicad_pcb");
        let mut p1 = Page::new("a", PageSize::a4_landscape());
        p1.items.push(rect("r", 10.0));
        let mut p2 = Page::new("b", PageSize::a4_landscape());
        p2.items.push(rect("f", 30.0));
        d.pages.push(p1);
        d.pages.push(p2);
        link::link(&mut d, &Id("f".into()), &Id("r".into()), None, true).unwrap();
        let before = d.clone();
        d.pages[1].items[0].translate(2.0, 3.0);
        assert!(settle(&before, &mut d));
        assert_eq!(d.pages[0].items[0].position(), Some(Point::new(12.0, 13.0)));
        assert_eq!(d.pages[1].items[0].position(), Some(Point::new(32.0, 13.0)));
    }
}
