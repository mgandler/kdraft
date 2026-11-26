//! Placement link tests: resolution, group moves, offsets, promotion of
//! followers of a deleted reference, duplicate rules, cycles, file round
//! trip, the layer pages command, and a property test over random edits.

use std::path::PathBuf;

use kdraft_board::geom::Point;
use kdraft_doc::link::{self, Follow};
use kdraft_doc::{Document, Id, Item};
use proptest::prelude::*;
use serde_json::json;

use crate::{Registry, Session};

fn session() -> Session {
    Session::new_for_board(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/kicad9-minimal.kicad_pcb"),
        None,
    )
    .unwrap()
}

fn created(o: &crate::Outcome) -> String {
    o.created[0].clone()
}

fn pos(s: &Session, id: &str) -> Point {
    s.find_item(&Id(id.into()))
        .and_then(|(_, i)| i.position())
        .unwrap()
}

fn item(s: &Session, id: &str) -> Item {
    s.find_item(&Id(id.into())).map(|(_, i)| i.clone()).unwrap()
}

/// Every follower sits exactly where its link puts it; no chains.
fn links_hold(d: &Document) -> Result<(), String> {
    let ix = link::index(d);
    for p in &d.pages {
        for it in &p.items {
            let Some(l) = it.link() else { continue };
            let Some(r) = link::get(d, &ix, &l.to) else {
                return Err(format!("dangling link to {}", l.to));
            };
            if r.link().is_some() {
                return Err(format!("chain through {}", l.to));
            }
            let mut want = it.clone();
            if link::apply_one(r, &mut want, &l) {
                return Err(format!("{:?} not at its link", it.id()));
            }
        }
    }
    Ok(())
}

/// Two pages with a view each; the second view linked to the first.
fn two_views(r: &Registry, s: &mut Session) -> (String, String) {
    let v1 = s.doc().pages[0].views().next().unwrap().id.0.clone();
    r.execute(s, "page.add", json!({})).unwrap();
    let v2 = created(
        &r.execute(
            s,
            "item.add_view",
            json!({ "kind": "layer", "at": [100, 100] }),
        )
        .unwrap(),
    );
    r.execute(
        s,
        "link.set",
        json!({ "ids": [v2], "to": v1, "keep_offset": false }),
    )
    .unwrap();
    (v1, v2)
}

#[test]
fn view_links_follow_position_scale_rotation_and_move_as_a_group() {
    let r = Registry::new();
    let mut s = session();
    let original = s.doc().clone();
    let (v1, v2) = two_views(&r, &mut s);
    assert_eq!(pos(&s, &v1), pos(&s, &v2));
    let Item::View(a) = item(&s, &v1) else {
        panic!()
    };
    let Item::View(b) = item(&s, &v2) else {
        panic!()
    };
    assert_eq!(a.scale, b.scale);
    let l = item(&s, &v2).link().unwrap();
    assert_eq!(
        l.follow,
        vec![Follow::Position, Follow::Scale, Follow::Rotation]
    );
    links_hold(s.doc()).unwrap();

    // Moving the follower (page 2) moves the reference (page 1).
    let p1 = pos(&s, &v1);
    r.execute(
        &mut s,
        "item.move",
        json!({ "ids": [v2], "delta": [5, -3] }),
    )
    .unwrap();
    assert_eq!(pos(&s, &v1), Point::new(p1.x + 5.0, p1.y - 3.0));
    assert_eq!(pos(&s, &v2), pos(&s, &v1));
    // Scale typed on the follower goes to the group.
    r.execute(&mut s, "item.set_scale", json!({ "ids": [v2], "scale": 2 }))
        .unwrap();
    let Item::View(a) = item(&s, &v1) else {
        panic!()
    };
    assert_eq!(a.scale, 2.0);
    links_hold(s.doc()).unwrap();

    // Live preview: the reference moves with every preview of a drag.
    let p2 = s.doc().pages[1].id.0.clone();
    r.execute(&mut s, "page.select", json!({ "page": p2 }))
        .unwrap();
    r.execute(&mut s, "select.set", json!({ "ids": [v2] }))
        .unwrap();
    s.begin_gesture("Move");
    let p1 = pos(&s, &v1);
    for k in 1..=3 {
        r.preview(&mut s, "item.move", json!({ "delta": [k, 0] }))
            .unwrap();
        assert_eq!(pos(&s, &v1), Point::new(p1.x + f64::from(k), p1.y));
        links_hold(s.doc()).unwrap();
    }
    s.commit_gesture();
    assert_eq!(pos(&s, &v1), Point::new(p1.x + 3.0, p1.y));

    // The offset moves the follower alone.
    let p1 = pos(&s, &v1);
    r.execute(
        &mut s,
        "link.set_offset",
        json!({ "ids": [v2], "offset": [10, 0] }),
    )
    .unwrap();
    assert_eq!(pos(&s, &v1), p1);
    assert_eq!(pos(&s, &v2), Point::new(p1.x + 10.0, p1.y));
    r.execute(
        &mut s,
        "link.set_offset",
        json!({ "ids": [v2], "delta": [0, 2] }),
    )
    .unwrap();
    assert_eq!(pos(&s, &v2), Point::new(p1.x + 10.0, p1.y + 2.0));
    // Following x only: y is free.
    r.execute(
        &mut s,
        "link.set_follow",
        json!({ "id": v2, "follow": ["x"] }),
    )
    .unwrap();
    r.execute(&mut s, "item.move", json!({ "ids": [v2], "delta": [0, 7] }))
        .unwrap();
    assert_eq!(pos(&s, &v1), p1);
    assert!(r
        .execute(
            &mut s,
            "link.set_follow",
            json!({ "id": v2, "follow": ["size"] })
        )
        .is_err());

    // Unlink: stays put.
    let here = pos(&s, &v2);
    r.execute(&mut s, "link.clear", json!({ "ids": [v2] }))
        .unwrap();
    assert_eq!(pos(&s, &v2), here);
    assert!(item(&s, &v2).link().is_none());
    while s.can_undo() {
        s.undo();
    }
    assert_eq!(*s.doc(), original);
}

#[test]
fn deleting_a_reference_promotes_the_first_follower() {
    let r = Registry::new();
    let mut s = session();
    let a = created(
        &r.execute(
            &mut s,
            "item.add_rect",
            json!({ "start": [10, 10], "end": [20, 20] }),
        )
        .unwrap(),
    );
    let b = created(
        &r.execute(
            &mut s,
            "item.add_rect",
            json!({ "start": [40, 10], "end": [50, 20] }),
        )
        .unwrap(),
    );
    let c = created(
        &r.execute(
            &mut s,
            "item.add_rect",
            json!({ "start": [70, 10], "end": [80, 20] }),
        )
        .unwrap(),
    );
    r.execute(&mut s, "link.set", json!({ "ids": [b, c], "to": a }))
        .unwrap();
    let before = s.doc().clone();
    r.execute(&mut s, "item.remove", json!({ "ids": [a] }))
        .unwrap();
    assert!(item(&s, &b).link().is_none());
    assert_eq!(item(&s, &c).link().unwrap().to, Id(b.clone()));
    assert_eq!(pos(&s, &b), Point::new(40.0, 10.0));
    assert_eq!(pos(&s, &c), Point::new(70.0, 10.0));
    links_hold(s.doc()).unwrap();
    // One undo step restores everything.
    s.undo();
    assert_eq!(*s.doc(), before);
    // Moving the new group keeps the old distance.
    s.redo();
    r.execute(&mut s, "item.move", json!({ "ids": [c], "delta": [1, 1] }))
        .unwrap();
    assert_eq!(pos(&s, &b), Point::new(41.0, 11.0));
}

#[test]
fn duplicates_cycles_chains_and_round_trip() {
    let r = Registry::new();
    let mut s = session();
    let a = created(
        &r.execute(
            &mut s,
            "item.add_rect",
            json!({ "start": [10, 10], "end": [20, 20] }),
        )
        .unwrap(),
    );
    let b = created(
        &r.execute(
            &mut s,
            "item.add_rect",
            json!({ "start": [40, 10], "end": [50, 20] }),
        )
        .unwrap(),
    );
    let c = created(
        &r.execute(
            &mut s,
            "item.add_text",
            json!({ "text": "x", "at": [5, 5] }),
        )
        .unwrap(),
    );
    r.execute(&mut s, "link.set", json!({ "ids": [b], "to": a }))
        .unwrap();
    // Cycles are rejected.
    assert!(r
        .execute(&mut s, "link.set", json!({ "ids": [a], "to": b }))
        .is_err());
    // Linking to a follower links to its root.
    r.execute(&mut s, "link.set", json!({ "ids": [c], "to": b }))
        .unwrap();
    assert_eq!(item(&s, &c).link().unwrap().to, Id(a.clone()));
    // A reference that becomes a follower hands its followers over.
    let d = created(
        &r.execute(
            &mut s,
            "item.add_rect",
            json!({ "start": [90, 90], "end": [95, 95] }),
        )
        .unwrap(),
    );
    r.execute(&mut s, "link.set", json!({ "ids": [a], "to": d }))
        .unwrap();
    for id in [&a, &b, &c] {
        assert_eq!(item(&s, id).link().unwrap().to, Id(d.clone()), "{id}");
    }
    links_hold(s.doc()).unwrap();

    // Duplicating a follower keeps its link; duplicating the reference
    // does not take the followers along.
    let dup = created(
        &r.execute(&mut s, "item.duplicate", json!({ "ids": [b] }))
            .unwrap(),
    );
    assert_eq!(item(&s, &dup).link().unwrap().to, Id(d.clone()));
    let n_before = link::followers(s.doc(), &Id(d.clone())).len();
    let dup_ref = created(
        &r.execute(&mut s, "item.duplicate", json!({ "ids": [d] }))
            .unwrap(),
    );
    assert!(link::followers(s.doc(), &Id(dup_ref)).is_empty());
    assert_eq!(link::followers(s.doc(), &Id(d.clone())).len(), n_before);
    // Duplicating a page keeps links to the original reference.
    let page = s.doc().pages[0].id.0.clone();
    r.execute(&mut s, "page.duplicate", json!({ "page": page }))
        .unwrap();
    assert!(s.doc().pages[1]
        .items
        .iter()
        .filter_map(Item::link)
        .all(|l| l.to == Id(d.clone())));
    links_hold(s.doc()).unwrap();
    // Copy to page, linked.
    let page2 = s.doc().pages[2].id.0.clone();
    let o = r
        .execute(
            &mut s,
            "item.copy_to_page",
            json!({ "ids": [d], "page": page2, "linked": true }),
        )
        .unwrap();
    assert_eq!(item(&s, &o.created[0]).link().unwrap().to, Id(d.clone()));
    assert_eq!(pos(&s, &o.created[0]), pos(&s, &d));

    // Round trip with an unknown node inside the link.
    let text = s.doc().to_text().unwrap();
    assert!(text.contains("(link"));
    let messy = text.replacen(
        "(follow position)",
        "(follow position wobble) (future 1)",
        1,
    );
    let back = Document::parse(&messy).unwrap();
    let out = back.to_text().unwrap();
    assert!(
        out.contains("wobble") && out.contains("(future 1)"),
        "{out}"
    );
    assert_eq!(Document::parse(&out).unwrap(), back);
    assert_eq!(Document::parse(&text).unwrap(), *s.doc());
}

#[test]
fn layer_pages_create_linked_single_layer_views_and_update() {
    let r = Registry::new();
    let mut s = session();
    let v = s.doc().pages[0].views().next().unwrap().id.0.clone();
    let n0 = s.doc().pages.len();
    let o = r
        .execute(
            &mut s,
            "view.pages_per_layer",
            json!({ "view": v, "set": "copper" }),
        )
        .unwrap();
    assert_eq!(o.created.len(), 4, "{o:?}");
    assert_eq!(s.doc().pages.len(), n0 + 2);
    let titles: Vec<String> = s.doc().pages[1..3]
        .iter()
        .map(|p| p.title.clone().unwrap())
        .collect();
    assert_eq!(titles, vec!["F.Cu", "B.Cu"]);
    for p in &s.doc().pages[1..3] {
        let lv = p.views().next().unwrap();
        assert_eq!(lv.layers[0], *p.title.as_ref().unwrap());
        assert!(lv.layers.contains(&"Edge.Cuts".to_string()));
        assert_eq!(p.size, s.doc().pages[0].size);
        assert_eq!(lv.at, pos(&s, &v));
    }
    links_hold(s.doc()).unwrap();
    // Again: nothing to add.
    let o = r
        .execute(
            &mut s,
            "view.pages_per_layer",
            json!({ "view": v, "set": "copper" }),
        )
        .unwrap();
    assert!(o.created.is_empty());
    // Adding a layer keeps the existing pages.
    let o = r
        .execute(
            &mut s,
            "view.pages_per_layer",
            json!({ "view": v, "layers": ["F.Cu", "F.SilkS"] }),
        )
        .unwrap();
    assert_eq!(o.created.len(), 2);
    assert_eq!(s.doc().pages.len(), n0 + 3);
    assert!(r
        .execute(
            &mut s,
            "view.pages_per_layer",
            json!({ "view": v, "layers": ["Nope.Cu"] })
        )
        .is_err());
}

#[derive(Debug, Clone)]
enum Op {
    Add(f64, f64),
    Link(usize, usize),
    Unlink(usize),
    Move(usize, f64, f64),
    Remove(usize),
    Offset(usize, f64),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0.0..200.0f64, 0.0..150.0f64).prop_map(|(x, y)| Op::Add(x, y)),
        (0usize..8, 0usize..8).prop_map(|(a, b)| Op::Link(a, b)),
        (0usize..8).prop_map(Op::Unlink),
        (0usize..8, -10.0..10.0f64, -10.0..10.0f64).prop_map(|(a, x, y)| Op::Move(a, x, y)),
        (0usize..8).prop_map(Op::Remove),
        (0usize..8, -5.0..5.0f64).prop_map(|(a, x)| Op::Offset(a, x)),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 40, ..ProptestConfig::default() })]

    #[test]
    fn random_link_edits_keep_links_exact_and_undo_restores(ops in prop::collection::vec(op(), 1..30)) {
        let r = Registry::new();
        let mut s = session();
        let original = s.doc().clone();
        for o in &ops {
            let ids: Vec<String> = s.doc().pages[0].items.iter().filter(|i| i.linkable()).filter_map(|i| i.id().map(|x| x.0.clone())).collect();
            let pick = |k: usize| ids.get(k % ids.len().max(1)).cloned();
            let (name, args) = match o {
                Op::Add(x, y) => ("item.add_rect", json!({ "page": s.doc().pages[0].id.0, "start": [x, y], "end": [x + 5.0, y + 5.0] })),
                Op::Link(a, b) => match (pick(*a), pick(*b)) {
                    (Some(a), Some(b)) => ("link.set", json!({ "ids": [a], "to": b })),
                    _ => continue,
                },
                Op::Unlink(a) => match pick(*a) { Some(a) => ("link.clear", json!({ "ids": [a] })), None => continue },
                Op::Move(a, x, y) => match pick(*a) { Some(a) => ("item.move", json!({ "ids": [a], "delta": [x, y] })), None => continue },
                Op::Remove(a) => match pick(*a) { Some(a) => ("item.remove", json!({ "ids": [a] })), None => continue },
                Op::Offset(a, x) => match pick(*a) { Some(a) => ("link.set_offset", json!({ "ids": [a], "delta": [x, 0] })), None => continue },
            };
            let before = s.doc().clone();
            if r.execute(&mut s, name, args).is_err() {
                prop_assert_eq!(s.doc(), &before);
            }
            if let Err(e) = links_hold(s.doc()) {
                prop_assert!(false, "after {:?}: {}", o, e);
            }
        }
        while s.undo().is_some() {}
        prop_assert_eq!(s.doc(), &original);
    }
}

#[test]
fn layer_pages_reference_paths() {
    use crate::layer_pages::{self, LayerPagesArgs, Placement};
    let r = Registry::new();
    // Explicit id (new argument name).
    let mut s = session();
    let v = s.doc().pages[0].views().next().unwrap().id.0.clone();
    let first_page = s.doc().pages[0].id.0.clone();
    r.execute(
        &mut s,
        "select.set",
        json!({ "page": first_page, "ids": [] }),
    )
    .unwrap();
    let o = r
        .execute(&mut s, "view.pages_per_layer", json!({ "reference": v }))
        .unwrap();
    assert_eq!(o.created.len(), 4, "{o:?}");
    assert!(
        o.messages[0].contains("linked to Fabrication view on page 1"),
        "{:?}",
        o.messages
    );
    let lv = s.doc().pages[1].views().next().unwrap().id.0.clone();
    assert_eq!(item(&s, &lv).link().unwrap().to.0, v);
    links_hold(s.doc()).unwrap();

    // Selected view, no argument.
    let mut s = session();
    let v = s.doc().pages[0].views().next().unwrap().id.0.clone();
    let p0 = s.doc().pages[0].id.0.clone();
    r.execute(&mut s, "select.set", json!({ "page": p0, "ids": [v] }))
        .unwrap();
    assert_eq!(layer_pages::default_reference(&s).unwrap().0, v);
    let o = r
        .execute(
            &mut s,
            "view.pages_per_layer",
            json!({ "layers": ["B.Cu"] }),
        )
        .unwrap();
    assert_eq!(o.created.len(), 2);
    let lv = o.created[1].clone();
    assert_eq!(item(&s, &lv).link().unwrap().to.0, v);
    assert_eq!(s.doc().pages[1].title.as_deref(), Some("B.Cu"));

    // Single view on the current page: offered by default (the dialog);
    // the command itself without a selection makes a new placement.
    let mut s = session();
    let v = s.doc().pages[0].views().next().unwrap().id.0.clone();
    let p0 = s.doc().pages[0].id.0.clone();
    r.execute(&mut s, "select.set", json!({ "page": p0, "ids": [] }))
        .unwrap();
    assert_eq!(s.doc().pages[0].views().count(), 1);
    assert_eq!(layer_pages::default_reference(&s).unwrap().0, v);
    let pl = layer_pages::plan(&mut s, &LayerPagesArgs::default()).unwrap();
    assert!(matches!(pl.placement, Placement::New { .. }), "{pl:?}");

    // A 3D view is not a reference.
    let o3 = r
        .execute(
            &mut s,
            "item.add_view",
            json!({ "page": p0, "kind": "render3d", "at": [300, 60] }),
        )
        .unwrap();
    let e = r
        .execute(
            &mut s,
            "view.pages_per_layer",
            json!({ "reference": created(&o3) }),
        )
        .unwrap_err();
    assert!(e.to_string().contains("board view"), "{e}");
}

#[test]
fn layer_pages_new_placement_on_empty_page() {
    use crate::layer_pages::page_reusable;
    let r = Registry::new();
    let mut s = session();
    let n0 = s.doc().pages.len();
    // An empty untitled page after the first.
    let first_page = s.doc().pages[0].id.0.clone();
    r.execute(
        &mut s,
        "select.set",
        json!({ "page": first_page, "ids": [] }),
    )
    .unwrap();
    r.execute(&mut s, "page.add", json!({})).unwrap();
    let cur = s.current_page_index().unwrap();
    assert_eq!(cur, 1);
    assert!(page_reusable(&s, cur));
    assert!(!page_reusable(&s, 0));
    let empty_id = s.doc().pages[cur].id.clone();
    let o = r
        .execute(
            &mut s,
            "view.pages_per_layer",
            json!({ "reference": "new", "use_current_page": true }),
        )
        .unwrap();
    assert!(
        o.messages[0].starts_with(
            "Page 2 becomes the F.Cu page, 1 more page(s) after it (B.Cu); new placement"
        ),
        "{:?}",
        o.messages
    );
    assert_eq!(s.doc().pages.len(), n0 + 2);
    let p1 = &s.doc().pages[1];
    assert_eq!(p1.id, empty_id);
    assert_eq!(p1.title.as_deref(), Some("F.Cu"));
    let first = p1.views().next().unwrap().clone();
    assert!(first.extra.iter().all(|v| !v.is(link::LINK)));
    let area = s.drawing_area(1).unwrap();
    assert!((first.at.x - area.center().x).abs() < 1e-9);
    assert!((first.at.y - area.center().y).abs() < 1e-9);
    let bb = s.board_box();
    let want =
        kdraft_doc::choose_scale(bb.width(), bb.height(), area.width(), area.height()).unwrap();
    assert_eq!(first.scale, want);
    let second = s.doc().pages[2].views().next().unwrap().id.0.clone();
    assert_eq!(item(&s, &second).link().unwrap().to, first.id);
    links_hold(s.doc()).unwrap();
    // The first page is not empty: asking to use it fails with advice.
    let first_page = s.doc().pages[0].id.0.clone();
    r.execute(
        &mut s,
        "select.set",
        json!({ "page": first_page, "ids": [] }),
    )
    .unwrap();
    let e = r
        .execute(
            &mut s,
            "view.pages_per_layer",
            json!({ "reference": "new", "use_current_page": true, "layers": ["F.SilkS"] }),
        )
        .unwrap_err();
    assert!(e.to_string().contains("not empty"), "{e}");
}

#[test]
fn layer_pages_in_a_document_without_views() {
    let r = Registry::new();
    let mut s = session();
    for p in &mut s.doc_mut().pages {
        p.items.clear();
    }
    let n0 = s.doc().pages.len();
    let first_page = s.doc().pages[0].id.0.clone();
    r.execute(
        &mut s,
        "select.set",
        json!({ "page": first_page, "ids": [] }),
    )
    .unwrap();
    let o = r
        .execute(&mut s, "view.pages_per_layer", json!({ "at_end": true }))
        .unwrap();
    assert_eq!(o.created.len(), 4);
    assert_eq!(s.doc().pages.len(), n0 + 2);
    assert_eq!(s.doc().pages[n0].title.as_deref(), Some("F.Cu"));
    assert_eq!(s.doc().pages[n0 + 1].title.as_deref(), Some("B.Cu"));
    // The new first page is shown.
    assert_eq!(s.current_page_index(), Some(n0));
    links_hold(s.doc()).unwrap();
    // Undo removes all of it in one step.
    s.undo();
    assert_eq!(s.doc().pages.len(), n0);
}

/// Commands that miss an input take a sensible default instead of
/// refusing.
#[test]
fn missing_inputs_default() {
    let r = Registry::new();
    let mut s = session();
    let p0 = s.doc().pages[0].id.0.clone();
    r.execute(&mut s, "select.set", json!({ "page": p0, "ids": [] }))
        .unwrap();
    r.execute(&mut s, "page.add", json!({})).unwrap();
    let pi = s.current_page_index().unwrap();
    assert_eq!(s.doc().pages[pi].title.as_deref(), Some("Page 2"));
    // Board size dimensions on a page without a view: a fabrication view
    // comes first.
    let o = r
        .execute(&mut s, "dimension.board_size", json!({}))
        .unwrap();
    assert_eq!(o.created.len(), 3, "{o:?}");
    assert!(o.messages[0].contains("added a fabrication view"));
    let page = &s.doc().pages[pi];
    assert_eq!(page.views().count(), 1);
    assert_eq!(
        page.views().next().unwrap().kind,
        kdraft_doc::ViewKind::Fabrication
    );
    // A note without a note list: one is started (top left of the area).
    let o = r
        .execute(&mut s, "notes.add", json!({ "text": "Lead-free" }))
        .unwrap();
    assert!(o.messages[0].contains("note list"), "{o:?}");
    let area = s.drawing_area(pi).unwrap();
    let nl = s.doc().pages[pi]
        .items
        .iter()
        .find_map(|i| match i {
            Item::NoteList(n) => Some(n.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(nl.notes[0].text, "Lead-free");
    assert_eq!(nl.at, Point::new(area.min.x + 10.0, area.min.y + 10.0));
    // The next note goes to that list even though it is not selected.
    r.execute(&mut s, "select.set", json!({ "ids": [] }))
        .unwrap();
    r.execute(&mut s, "notes.add", json!({ "text": "Second" }))
        .unwrap();
    let n = match s.find_item(&nl.id).unwrap().1 {
        Item::NoteList(n) => n.notes.len(),
        _ => 0,
    };
    assert_eq!(n, 2);
    // A callout without a target: the part's BOM line.
    let reference = s
        .board()
        .unwrap()
        .footprints
        .iter()
        .find_map(|f| f.reference.clone())
        .unwrap();
    let o = r
        .execute(
            &mut s,
            "item.add_callout",
            json!({ "anchor": format!("(fp-origin \"{reference}\")"), "at": [50, 50], "kind": "bom" }),
        )
        .unwrap();
    let c = item(&s, &created(&o));
    let Item::Callout(c) = c else { panic!() };
    assert!(c.target.as_deref().unwrap().starts_with("bom:"), "{c:?}");
    // Not a part: the first note on the page.
    let o = r
        .execute(
            &mut s,
            "item.add_callout",
            json!({ "anchor": "(outline-vertex 0)", "at": [60, 50], "kind": "note" }),
        )
        .unwrap();
    let Item::Callout(c) = item(&s, &created(&o)) else {
        panic!()
    };
    assert_eq!(c.target.as_deref(), Some("note:N1"));
    // A note list without `at`.
    let o = r.execute(&mut s, "item.add_notes", json!({})).unwrap();
    assert!(matches!(item(&s, &created(&o)), Item::NoteList(_)));
    // Renumbering without a BOM table says what to do.
    for p in &mut s.doc_mut().pages {
        p.items
            .retain(|i| !matches!(i, Item::Table(t) if t.kind == kdraft_doc::TableKind::Bom));
    }
    let e = r.execute(&mut s, "bom.renumber", json!({})).unwrap_err();
    assert!(e.to_string().contains("place one first"), "{e}");
}
