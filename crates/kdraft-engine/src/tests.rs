use std::path::PathBuf;

use proptest::prelude::*;
use serde_json::{json, Value};

use crate::registry::ArgType;
use crate::{Registry, Session};
use kdraft_doc::{Id, Item};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/kicad9-minimal.kicad_pcb")
}

fn session() -> Session {
    Session::new_for_board(&fixture(), None).unwrap()
}

fn sample(t: &ArgType) -> Value {
    match t {
        ArgType::String | ArgType::Id => json!("x"),
        ArgType::Number => json!(1.5),
        ArgType::Integer => json!(1),
        ArgType::Bool => json!(true),
        ArgType::IdList | ArgType::StringList => json!(["x"]),
        ArgType::Point => json!([1.0, 2.0]),
        ArgType::PointList => json!([[1.0, 2.0], [3.0, 4.0]]),
        ArgType::Enum { values } => json!(values[0]),
    }
}

#[test]
fn every_command_spec_matches_its_argument_type() {
    let r = Registry::new();
    let mut names = std::collections::HashSet::new();
    for c in r.specs() {
        assert!(names.insert(c.name), "duplicate {}", c.name);
        assert!(
            !c.title.is_empty() && !c.description.is_empty(),
            "{}",
            c.name
        );
        let all: serde_json::Map<String, Value> = c
            .args
            .iter()
            .map(|a| (a.name.to_string(), sample(&a.ty)))
            .collect();
        r.check_args(c.name, &Value::Object(all.clone()))
            .unwrap_or_else(|e| panic!("{}: all args: {e}", c.name));
        let required: serde_json::Map<String, Value> = c
            .args
            .iter()
            .filter(|a| a.required)
            .map(|a| (a.name.to_string(), sample(&a.ty)))
            .collect();
        r.check_args(c.name, &Value::Object(required.clone()))
            .unwrap_or_else(|e| panic!("{}: required args: {e}", c.name));
        for a in c.args.iter().filter(|a| a.required) {
            let mut m = required.clone();
            m.remove(a.name);
            assert!(
                r.check_args(c.name, &Value::Object(m)).is_err(),
                "{}: {} is not required by the type",
                c.name,
                a.name
            );
        }
        let mut extra = all;
        extra.insert("no_such_arg".into(), json!(1));
        assert!(
            r.check_args(c.name, &Value::Object(extra)).is_err(),
            "{}: unknown args accepted",
            c.name
        );
    }
    assert!(names.len() >= 40, "{}", names.len());
    let j = r.to_json();
    assert!(j.as_array().is_some_and(|a| a.len() == names.len()));
}

fn ids_on_page(s: &Session, pi: usize) -> Vec<String> {
    s.doc().pages[pi]
        .items
        .iter()
        .filter_map(|i| i.id().map(|x| x.0.clone()))
        .collect()
}

#[test]
fn align_with_reference_and_drawing_area() {
    let r = Registry::new();
    let mut s = session();
    // The drill page has a view and two tables.
    let di = s
        .doc()
        .pages
        .iter()
        .position(|p| p.title.as_deref() == Some("Drill and stackup"))
        .unwrap();
    let page = s.doc().pages[di].id.0.clone();
    r.execute(&mut s, "page.select", json!({ "page": page }))
        .unwrap();
    let ids = ids_on_page(&s, di);
    // Reference = first selected (the drill table).
    r.execute(&mut s, "select.set", json!({ "ids": [ids[1], ids[0]] }))
        .unwrap();
    let before = s
        .item_bounds(s.doc().pages[di].item(&Id(ids[1].clone())).unwrap())
        .unwrap();
    r.execute(&mut s, "layout.align", json!({ "mode": "left" }))
        .unwrap();
    let b0 = s
        .item_bounds(s.doc().pages[di].item(&Id(ids[0].clone())).unwrap())
        .unwrap();
    let b1 = s
        .item_bounds(s.doc().pages[di].item(&Id(ids[1].clone())).unwrap())
        .unwrap();
    assert_eq!(b1, before, "reference must not move");
    assert!((b0.min.x - b1.min.x).abs() < 1e-9);
    // One item: to the drawing area.
    r.execute(&mut s, "select.set", json!({ "ids": [ids[0]] }))
        .unwrap();
    r.execute(&mut s, "layout.align", json!({ "mode": "right" }))
        .unwrap();
    let area = s.drawing_area(2).unwrap();
    let b0 = s
        .item_bounds(s.doc().pages[di].item(&Id(ids[0].clone())).unwrap())
        .unwrap();
    assert!((b0.max.x - area.max.x).abs() < 1e-9);
    assert_eq!(s.undo_title(), Some("Align"));
    // Undo restores both document and selection.
    s.undo();
    assert_eq!(s.selection.items, vec![Id(ids[0].clone())]);
    s.undo();
    assert_eq!(s.selection.items.len(), 2);
}

#[test]
fn reorder_then_undo_restores_exactly() {
    let r = Registry::new();
    let mut s = session();
    let original = s.doc().clone();
    r.execute(&mut s, "page.move", json!({ "from": 0, "to": 2 }))
        .unwrap();
    assert_eq!(s.doc().pages[2].id, original.pages[0].id);
    assert!(s.dirty());
    s.undo();
    assert_eq!(*s.doc(), original);
    s.redo();
    assert_eq!(s.doc().pages[2].id, original.pages[0].id);
    assert!(r
        .execute(&mut s, "page.move", json!({ "from": 0, "to": 9 }))
        .is_err());
}

#[test]
fn gesture_coalesces_drag_into_one_step() {
    let r = Registry::new();
    let mut s = session();
    let original = s.doc().clone();
    let v = ids_on_page(&s, 0)[0].clone();
    r.execute(&mut s, "select.set", json!({ "ids": [v] }))
        .unwrap();
    s.begin_gesture("Move");
    for k in 1..=10 {
        r.preview(
            &mut s,
            "item.move",
            json!({ "delta": [k as f64, 0.5 * k as f64] }),
        )
        .unwrap();
    }
    assert!(s.commit_gesture());
    let Some(Item::View(view)) = s.doc().pages[0].item(&Id(v.clone())) else {
        panic!()
    };
    let Some(Item::View(orig)) = original.pages[0].item(&Id(v.clone())) else {
        panic!()
    };
    // The last preview wins; previews do not accumulate.
    assert!((view.at.x - orig.at.x - 10.0).abs() < 1e-9);
    assert_eq!(s.undo_title(), Some("Move"));
    s.undo();
    assert_eq!(*s.doc(), original);
    assert!(!s.can_undo());
    // Cancel restores without a step.
    s.begin_gesture("Move");
    r.preview(&mut s, "item.move", json!({ "delta": [3.0, 0.0] }))
        .unwrap();
    s.cancel_gesture();
    assert_eq!(*s.doc(), original);
    assert!(r
        .preview(&mut s, "item.move", json!({ "delta": [1.0, 0.0] }))
        .is_err());
}

#[test]
fn errors_leave_the_session_unchanged() {
    let r = Registry::new();
    let mut s = session();
    let original = s.doc().clone();
    assert!(r
        .execute(&mut s, "item.move", json!({ "delta": [1.0, 1.0] }))
        .is_err());
    assert!(r
        .execute(
            &mut s,
            "item.set_scale",
            json!({ "ids": ["nope"], "scale": 2 })
        )
        .is_err());
    assert!(r.execute(&mut s, "nope", json!({})).is_err());
    assert!(r
        .execute(&mut s, "item.move", json!({ "delta": "x" }))
        .is_err());
    assert_eq!(*s.doc(), original);
    assert!(!s.can_undo());
}

#[test]
fn add_items_distribute_match_center_nudge() {
    let r = Registry::new();
    let mut s = session();
    r.execute(
        &mut s,
        "page.add",
        json!({ "title": "Notes", "paper": "A4" }),
    )
    .unwrap();
    let pi = s.current_page_index().unwrap();
    assert_eq!(s.doc().pages[pi].size.width, 297.0);
    let mut ids = Vec::new();
    for (x, w) in [(20.0, 10.0), (35.0, 20.0), (100.0, 5.0), (60.0, 8.0)] {
        let o = r
            .execute(
                &mut s,
                "item.add_rect",
                json!({ "start": [x, 50.0], "end": [x + w, 60.0] }),
            )
            .unwrap();
        ids.push(o.created[0].clone());
    }
    r.execute(&mut s, "select.set", json!({ "ids": ids }))
        .unwrap();
    r.execute(
        &mut s,
        "layout.distribute",
        json!({ "direction": "horizontal" }),
    )
    .unwrap();
    r.execute(
        &mut s,
        "layout.distribute",
        json!({ "direction": "horizontal", "spacing": "centres" }),
    )
    .unwrap();
    let cx: Vec<f64> = ids
        .iter()
        .map(|i| {
            s.item_bounds(s.doc().pages[pi].item(&Id(i.clone())).unwrap())
                .unwrap()
                .center()
                .x
        })
        .collect();
    let mut sorted = cx.clone();
    sorted.sort_by(f64::total_cmp);
    let step = sorted[1] - sorted[0];
    assert!(
        (sorted[2] - sorted[1] - step).abs() < 1e-9 && (sorted[3] - sorted[2] - step).abs() < 1e-9
    );
    r.execute(&mut s, "layout.match", json!({ "property": "width" }))
        .unwrap();
    for i in &ids {
        let b = s
            .item_bounds(s.doc().pages[pi].item(&Id(i.clone())).unwrap())
            .unwrap();
        assert!((b.width() - 10.0).abs() < 1e-9);
    }
    r.execute(&mut s, "layout.center", json!({ "target": "page" }))
        .unwrap();
    let all = ids
        .iter()
        .map(|i| {
            s.item_bounds(s.doc().pages[pi].item(&Id(i.clone())).unwrap())
                .unwrap()
        })
        .reduce(|a, b| a.union(&b))
        .unwrap();
    assert!((all.center().x - 148.5).abs() < 1e-9 && (all.center().y - 105.0).abs() < 1e-9);
    let b0 = s
        .item_bounds(s.doc().pages[pi].item(&Id(ids[0].clone())).unwrap())
        .unwrap();
    r.execute(&mut s, "layout.nudge", json!({ "dx": 1, "dy": -10 }))
        .unwrap();
    let b1 = s
        .item_bounds(s.doc().pages[pi].item(&Id(ids[0].clone())).unwrap())
        .unwrap();
    assert!((b1.min.x - b0.min.x - 0.5).abs() < 1e-9 && (b1.min.y - b0.min.y + 5.0).abs() < 1e-9);
    // View add and properties.
    let o = r
        .execute(
            &mut s,
            "item.add_view",
            json!({ "kind": "assembly", "side": "back" }),
        )
        .unwrap();
    let v = Id(o.created[0].clone());
    let Some(Item::View(view)) = s.doc().pages[pi].item(&v) else {
        panic!()
    };
    assert_eq!(view.layers[0], "B.Fab");
    assert_eq!(view.drill_marks, Some(kdraft_doc::DrillMarks::None));
    r.execute(&mut s, "item.set_drill_marks", json!({ "marks": "small" }))
        .unwrap();
    r.execute(&mut s, "item.set_rotation", json!({ "rotation": 450 }))
        .unwrap();
    let Some(Item::View(view)) = s.doc().pages[pi].item(&v) else {
        panic!()
    };
    assert_eq!(view.rotation, 90.0);
    assert_eq!(view.drill_marks(), kdraft_doc::DrillMarks::Small);
    // Copy to page 0 and z-order.
    let p0 = s.doc().pages[0].id.0.clone();
    let o = r
        .execute(&mut s, "item.copy_to_page", json!({ "page": p0 }))
        .unwrap();
    assert_eq!(
        s.doc().pages[0]
            .items
            .last()
            .and_then(|i| i.id())
            .map(|i| i.0.clone()),
        Some(o.created[0].clone())
    );
    r.execute(&mut s, "item.lower", json!({ "to_bottom": true }))
        .unwrap();
    assert_eq!(s.doc().pages[pi].items[0].id(), Some(&v));
}

#[test]
fn save_open_round_trip_and_dirty_flag() {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("b.kicad_pcb");
    std::fs::copy(fixture(), &board).unwrap();
    let doc = dir.path().join("sub").join("d.kdraft");
    std::fs::create_dir_all(doc.parent().unwrap()).unwrap();
    let r = Registry::new();
    let mut s = Session::empty().unwrap();
    r.execute(
        &mut s,
        "document.new",
        json!({ "board": board.to_string_lossy(), "output": doc.to_string_lossy() }),
    )
    .unwrap();
    assert_eq!(s.doc().source.board, "../b.kicad_pcb");
    assert!(s.dirty());
    r.execute(&mut s, "document.save", json!({})).unwrap();
    assert!(!s.dirty());
    r.execute(&mut s, "page.rename", json!({ "title": "X" }))
        .unwrap();
    assert!(s.dirty());
    s.undo();
    assert!(!s.dirty());
    let mut t = Session::empty().unwrap();
    r.execute(
        &mut t,
        "document.open",
        json!({ "path": doc.to_string_lossy() }),
    )
    .unwrap();
    assert_eq!(t.doc(), s.doc());
    assert!(t.board().is_some(), "{:?}", t.board_state().error);
    // Reload reports no change for the same file.
    assert!(t.reload_board().unwrap().is_some_and(|d| d.is_empty()));
}

/// Abstract operations resolved against the session at run time.
#[derive(Debug, Clone)]
enum Op {
    AddPage,
    RemovePage(usize),
    MovePage(usize, usize),
    DupPage(usize),
    SelectPage(usize),
    AddRect(f64, f64),
    AddText(f64, f64),
    SelectItems(Vec<usize>),
    Move(f64, f64),
    Align(usize),
    Distribute(bool),
    Remove,
    Duplicate,
    Raise,
    Lower,
    Nudge(i8, i8),
    Scale(f64),
    Rename,
    Undo,
    Redo,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        Just(Op::AddPage),
        (0usize..6).prop_map(Op::RemovePage),
        (0usize..6, 0usize..6).prop_map(|(a, b)| Op::MovePage(a, b)),
        (0usize..6).prop_map(Op::DupPage),
        (0usize..6).prop_map(Op::SelectPage),
        (0.0..300.0f64, 0.0..200.0f64).prop_map(|(x, y)| Op::AddRect(x, y)),
        (0.0..300.0f64, 0.0..200.0f64).prop_map(|(x, y)| Op::AddText(x, y)),
        prop::collection::vec(0usize..8, 1..4).prop_map(Op::SelectItems),
        (-20.0..20.0f64, -20.0..20.0f64).prop_map(|(x, y)| Op::Move(x, y)),
        (0usize..6).prop_map(Op::Align),
        any::<bool>().prop_map(Op::Distribute),
        Just(Op::Remove),
        Just(Op::Duplicate),
        Just(Op::Raise),
        Just(Op::Lower),
        (-3i8..3, -3i8..3).prop_map(|(a, b)| Op::Nudge(a, b)),
        (0.2..10.0f64).prop_map(Op::Scale),
        Just(Op::Rename),
        Just(Op::Undo),
        Just(Op::Redo),
    ]
}

fn apply(r: &Registry, s: &mut Session, op: &Op) {
    let n = s.doc().pages.len().max(1);
    let page_id = |s: &Session, i: usize| s.doc().pages.get(i % n).map(|p| p.id.0.clone());
    let (name, args) = match op {
        Op::AddPage => ("page.add", json!({})),
        Op::RemovePage(i) => ("page.remove", json!({ "page": page_id(s, *i) })),
        Op::MovePage(a, b) => ("page.move", json!({ "from": a % n, "to": b % n })),
        Op::DupPage(i) => ("page.duplicate", json!({ "page": page_id(s, *i) })),
        Op::SelectPage(i) => ("page.select", json!({ "page": page_id(s, *i) })),
        Op::AddRect(x, y) => (
            "item.add_rect",
            json!({ "start": [x, y], "end": [x + 10.0, y + 5.0] }),
        ),
        Op::AddText(x, y) => ("item.add_text", json!({ "text": "T", "at": [x, y] })),
        Op::SelectItems(k) => {
            let ids = s
                .current_page_index()
                .map(|pi| ids_on_page(s, pi))
                .unwrap_or_default();
            if ids.is_empty() {
                return;
            }
            let pick: Vec<&String> = k.iter().map(|i| &ids[i % ids.len()]).collect();
            ("select.set", json!({ "ids": pick }))
        }
        Op::Move(x, y) => ("item.move", json!({ "delta": [x, y] })),
        Op::Align(m) => ("layout.align", {
            let mode = ["left", "hcenter", "right", "top", "vcenter", "bottom"][*m];
            json!({ "mode": mode })
        }),
        Op::Distribute(h) => (
            "layout.distribute",
            json!({ "direction": if *h { "horizontal" } else { "vertical" } }),
        ),
        Op::Remove => ("item.remove", json!({})),
        Op::Duplicate => ("item.duplicate", json!({})),
        Op::Raise => ("item.raise", json!({})),
        Op::Lower => ("item.lower", json!({})),
        Op::Nudge(a, b) => ("layout.nudge", json!({ "dx": a, "dy": b })),
        Op::Scale(v) => ("item.set_scale", json!({ "scale": v })),
        Op::Rename => ("page.rename", json!({ "title": "renamed" })),
        Op::Undo => {
            s.undo();
            return;
        }
        Op::Redo => {
            s.redo();
            return;
        }
    };
    // Errors are allowed (empty selection etc.); they must not change
    // anything, which the undo check below covers.
    let before = s.doc().clone();
    if r.execute(s, name, args).is_err() {
        assert_eq!(*s.doc(), before, "{name} failed but changed the document");
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    #[test]
    fn full_undo_restores_original_and_redo_restores_final(ops in prop::collection::vec(op(), 1..25)) {
        let r = Registry::new();
        let mut s = session();
        let original = s.doc().clone();
        for o in &ops {
            apply(&r, &mut s, o);
        }
        // The tip of the history: everything redone.
        while s.redo().is_some() {}
        let last = s.doc().clone();
        let mut guard = 0;
        while s.undo().is_some() {
            guard += 1;
            prop_assert!(guard < 1000);
        }
        prop_assert_eq!(s.doc(), &original);
        while s.redo().is_some() {}
        prop_assert_eq!(s.doc(), &last);
    }
}

#[test]
fn render_mode_and_assembly_options_round_trip() {
    let r = Registry::new();
    let mut s = session();
    let page = s.doc().pages[1].id.0.clone();
    r.execute(&mut s, "page.select", json!({ "page": page }))
        .unwrap();
    let ids = ids_on_page(&s, 1);
    r.execute(&mut s, "select.set", json!({ "ids": [ids[0]] }))
        .unwrap();
    r.execute(&mut s, "item.set_render", json!({ "mode": "plot" }))
        .unwrap();
    r.execute(
        &mut s,
        "item.set_assembly",
        json!({ "values": true, "dnp": "greyed", "text_min": 1.2, "refs": "all" }),
    )
    .unwrap();
    let v = s.doc().pages[1].views().next().unwrap().clone();
    assert_eq!(v.render, Some(kdraft_doc::RenderMode::Plot));
    assert!(v.wants_plot());
    assert!(v.assembly.values());
    assert_eq!(v.assembly.dnp(), kdraft_doc::DnpStyle::Greyed);
    assert_eq!(v.assembly.text_min(), 1.2);
    let text = s.doc().to_text().unwrap();
    assert!(text.contains("(render plot)"), "{text}");
    assert!(text.contains("(assembly"), "{text}");
    assert_eq!(&kdraft_doc::Document::parse(&text).unwrap(), s.doc());
    // null resets, absent keeps.
    r.execute(&mut s, "item.set_assembly", json!({ "values": null }))
        .unwrap();
    let v = s.doc().pages[1].views().next().unwrap().clone();
    assert_eq!(v.assembly.values, None);
    assert_eq!(v.assembly.text_min, Some(1.2));
    assert!(r
        .execute(&mut s, "item.set_assembly", json!({ "dnp": "x" }))
        .is_err());
    r.execute(&mut s, "item.set_render", json!({ "mode": "default" }))
        .unwrap();
    assert!(!s.doc().pages[1].views().next().unwrap().wants_plot());
}

#[test]
fn dimension_commands() {
    let r = Registry::new();
    let mut s = session();
    let page = s.doc().pages[0].id.0.clone();
    r.execute(&mut s, "page.select", json!({ "page": page }))
        .unwrap();
    let n0 = s.doc().pages[0].items.len();
    r.execute(
        &mut s,
        "dimension.add_linear",
        json!({ "from": "(outline-vertex 0)", "to": "(hole 2)", "kind": "aligned", "offset": 5 }),
    )
    .unwrap();
    r.execute(
        &mut s,
        "dimension.add_radius",
        json!({ "anchor": "(hole 2)", "diameter": true }),
    )
    .unwrap();
    r.execute(
        &mut s,
        "dimension.add_ordinate",
        json!({ "to": "(hole 2)", "axis": "x" }),
    )
    .unwrap();
    r.execute(&mut s, "dimension.board_size", json!({}))
        .unwrap();
    assert_eq!(s.doc().pages[0].items.len(), n0 + 5);
    r.execute(&mut s, "dimension.set_offset", json!({ "offset": -12 }))
        .unwrap();
    r.execute(
        &mut s,
        "dimension.set_style",
        json!({ "decimals": 1, "trim_zeros": true }),
    )
    .unwrap();
    assert_eq!(s.doc().styles.dimension.decimals, 1);
    assert!(r
        .execute(
            &mut s,
            "dimension.add_linear",
            json!({ "from": "(hole 999)", "to": "(hole 1)" })
        )
        .is_err());
    assert!(r
        .execute(
            &mut s,
            "dimension.add_radius",
            json!({ "anchor": "(outline-vertex 0)" })
        )
        .is_err());
    for it in &s.doc().pages[0].items {
        if let Item::Dimension(d) = it {
            assert_eq!(d.last.len(), 2);
        }
    }
}

#[test]
fn drawing_checks() {
    use crate::check::{check, Severity};
    let r = Registry::new();
    let mut s = session();
    let rep = check(&mut s);
    assert_eq!(rep.exit_code(), 0, "{}", rep.text());
    assert!(rep.findings.iter().any(|f| f.code == "designators"));
    // Stale board hash.
    s.doc_mut().source.board_hash = Some("00".into());
    let rep = check(&mut s);
    assert!(rep.findings.iter().any(|f| f.code == "stale"));
    assert_eq!(rep.exit_code(), 1);
    s.doc_mut().source.board_hash = None;
    // Small text and two overlapping rectangles on page 1.
    let page = s.doc().pages[0].id.0.clone();
    r.execute(&mut s, "page.select", json!({ "page": page }))
        .unwrap();
    r.execute(
        &mut s,
        "item.add_text",
        json!({ "text": "tiny", "at": [30, 30], "height": 0.5 }),
    )
    .unwrap();
    r.execute(
        &mut s,
        "item.add_rect",
        json!({ "start": [300, 200], "end": [320, 220] }),
    )
    .unwrap();
    r.execute(
        &mut s,
        "item.add_rect",
        json!({ "start": [310, 210], "end": [330, 230] }),
    )
    .unwrap();
    let rep = check(&mut s);
    let codes: Vec<&str> = rep.findings.iter().map(|f| f.code).collect();
    assert!(codes.contains(&"small-text"), "{codes:?}");
    assert!(codes.contains(&"overlap"), "{codes:?}");
    // A broken dimension anchor and a table moved off the area: errors.
    r.execute(
        &mut s,
        "dimension.add_linear",
        json!({ "from": "(outline-vertex 0)", "to": "(outline-vertex 1)" }),
    )
    .unwrap();
    for it in &mut s.doc_mut().pages[0].items {
        if let Item::Dimension(d) = it {
            d.to = Some(kdraft_doc::Anchor::OutlineVertex(99));
        }
    }
    let di = s
        .doc()
        .pages
        .iter()
        .position(|p| p.title.as_deref() == Some("Drill and stackup"))
        .unwrap();
    for it in &mut s.doc_mut().pages[di].items {
        if let Item::Table(t) = it {
            t.at = kdraft_board::geom::Point::new(400.0, 280.0);
        }
    }
    let rep = check(&mut s);
    let codes: Vec<&str> = rep.findings.iter().map(|f| f.code).collect();
    assert!(codes.contains(&"broken-anchor"), "{codes:?}");
    assert!(codes.contains(&"table-overflow"), "{codes:?}");
    assert_eq!(rep.worst(), Some(Severity::Error));
    assert_eq!(rep.exit_code(), 2);
    // Stable order and output.
    assert_eq!(check(&mut s).text(), rep.text());
    let pages: Vec<usize> = rep.findings.iter().map(|f| f.page).collect();
    assert!(pages.windows(2).all(|w| w[0] <= w[1]));
}

#[test]
fn notes_and_callouts_renumber() {
    let r = Registry::new();
    let mut s = session();
    let page = s.doc().pages[1].id.0.clone();
    r.execute(&mut s, "page.select", json!({ "page": page }))
        .unwrap();
    r.execute(
        &mut s,
        "item.add_notes",
        json!({ "at": [20, 200], "width": 100, "notes": ["First ${LAYER_COUNT} layers", "Second ${NOPE}"] }),
    )
    .unwrap();
    let list_id = s.selection.items[0].0.clone();
    r.execute(
        &mut s,
        "notes.add",
        json!({ "id": list_id, "text": "Third" }),
    )
    .unwrap();
    r.execute(
        &mut s,
        "item.add_callout",
        json!({ "anchor": "(fp-origin \"R1\")", "target": "note:N2", "at": [100, 60] }),
    )
    .unwrap();
    let label = |s: &Session| {
        let c = s.doc().pages[1]
            .items
            .iter()
            .find_map(|i| match i {
                Item::Callout(c) => Some(c.clone()),
                _ => None,
            })
            .unwrap();
        kdraft_render::notes::callout_label(s.doc(), &c, &|_| None)
    };
    assert_eq!(label(&s).as_deref(), Some("2"));
    r.execute(
        &mut s,
        "notes.move",
        json!({ "id": list_id, "note": "N2", "to": 0 }),
    )
    .unwrap();
    assert_eq!(label(&s).as_deref(), Some("1"));
    r.execute(
        &mut s,
        "notes.remove",
        json!({ "id": list_id, "note": "N1" }),
    )
    .unwrap();
    assert_eq!(label(&s).as_deref(), Some("1"));
    let rep = crate::check::check(&mut s);
    assert!(
        rep.findings
            .iter()
            .any(|f| f.code == "unknown-variable" && f.message.contains("NOPE")),
        "{}",
        rep.text()
    );
    assert!(r
        .execute(
            &mut s,
            "item.add_callout",
            json!({ "anchor": "(fp-origin \"ZZ9\")", "target": "note:N1", "at": [1, 1] })
        )
        .is_err());
}

#[test]
fn bom_from_fake_kicad_cli_and_checks() {
    use kdraft_source::{CommandOutput, FakeRunner, KicadCli, Version};
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("b.kicad_pcb");
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/kicad9-minimal.kicad_pcb"),
        &board,
    )
    .unwrap();
    std::fs::write(dir.path().join("b.kicad_sch"), "(kicad_sch)").unwrap();
    let runner = std::sync::Arc::new(FakeRunner::new(|_, args| {
        let out = kdraft_source::arg_value(args, "-o").unwrap_or_default();
        let csv = "\"Refs\",\"Value\",\"Footprint\",\"Qty\",\"DNP\"\n\
                   \"R1\",\"10k\",\"Resistor_SMD:R_0603_1608Metric\",\"1\",\"\"\n\
                   \"C1,C2,C3,C4\",\"100n\",\"Capacitor_SMD:C_0402_1005Metric\",\"4\",\"\"\n\
                   \"U9\",\"LM358\",\"Package_SO:SOIC-8\",\"1\",\"DNP\"\n";
        match std::fs::write(out, csv) {
            Ok(()) => CommandOutput::ok(""),
            Err(e) => CommandOutput::failed(1, e.to_string()),
        }
    }));
    let k = crate::pipeline::Kicad {
        cli: KicadCli {
            path: "/fake/kicad-cli".into(),
            version: Version::new(10, 0, 8),
            version_string: "10.0.8".into(),
            vars: Vec::new(),
        },
        runner: runner.clone(),
    };
    let cache = dir.path().join("cache");
    let mut notes = Vec::new();
    let bom = crate::pipeline::fetch_bom(&board, &k, &cache, &mut notes).unwrap();
    let bom_arc = bom.clone();
    assert_eq!(bom.lines.len(), 3, "{notes:?}");
    assert_eq!(bom.lines[1].refs.len(), 4);
    // Ran on a private copy, and nothing new in the project folder.
    let input = PathBuf::from(runner.calls()[0].args.last().unwrap());
    assert!(input.starts_with(&cache));
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        !names.iter().any(|n| n.ends_with(".kicad_prl")),
        "{names:?}"
    );

    // Rendered: schematic title, ranges compressed, item numbers.
    let mut s = Session::new_for_board(&board, None).unwrap();
    let (mut inputs, _) = crate::pipeline::RenderInputs::from_session(&mut s, None).unwrap();
    inputs.ctx.bom = Some(bom);
    let bp = s
        .doc()
        .pages
        .iter()
        .position(|p| p.title.as_deref() == Some("Bill of materials"))
        .unwrap();
    let (list, _) = inputs.render_page(bp).unwrap();
    let texts: Vec<String> = kdraft_render::dl::texts(&list.items)
        .iter()
        .map(|t| t.text.clone())
        .collect();
    assert!(
        texts.contains(&"BILL OF MATERIALS".to_string()),
        "{texts:?}"
    );
    assert!(texts.contains(&"C1-C4".to_string()), "{texts:?}");
    // DNP in its own column, the value untouched.
    assert!(texts.contains(&"LM358".to_string()), "{texts:?}");
    assert!(texts.contains(&"DNP".to_string()), "{texts:?}");
    // Numbered 1..n in table order for the schematic source, although the
    // document stored PCB numbers at creation.
    for n in ["1", "2", "3"] {
        assert!(texts.contains(&n.to_string()), "{texts:?}");
    }
    // A document made with the schematic BOM stores schematic numbers.
    let s2 = Session::new_for_board_bom(&board, None, Some(bom_arc.clone())).unwrap();
    assert!(s2.doc().pages.iter().flat_map(|p| &p.items).any(
        |i| matches!(i, Item::Table(t) if t.bom_source.as_deref() == Some("schematic") && t.bom_items.len() == 3)
    ));

    // A chunk that shows only part of the BOM: check reports the rest.
    for it in &mut s.doc_mut().pages[bp].items {
        if let Item::Table(t) = it {
            t.rows = Some((0, 1));
        }
    }
    let rep = crate::check::check(&mut s);
    assert!(
        rep.findings.iter().any(|f| f.code == "bom-overflow"),
        "{}",
        rep.text()
    );
    assert_eq!(rep.exit_code(), 2);
}

#[test]
fn rename_hide_lock_and_reorder_items() {
    let reg = Registry::new();
    let mut s = Session::new_for_board(&fixture(), None).unwrap();
    let ids: Vec<String> = s.doc().pages[0]
        .items
        .iter()
        .filter_map(|i| i.id().map(|x| x.0.clone()))
        .collect();
    assert!(ids.len() >= 2);
    reg.execute(
        &mut s,
        "item.rename",
        json!({"id": ids[0], "name": "Top view"}),
    )
    .unwrap();
    reg.execute(
        &mut s,
        "item.set_hidden",
        json!({"ids": [ids[1]], "hidden": true}),
    )
    .unwrap();
    reg.execute(
        &mut s,
        "item.set_locked",
        json!({"ids": [ids[0]], "locked": true}),
    )
    .unwrap();
    let p = &s.doc().pages[0];
    assert_eq!(p.items[0].name(), Some("Top view"));
    assert!(p.items[0].locked() && !p.items[0].hidden());
    assert!(p.items[1].hidden());
    // Round trip through the file format.
    let text = s.doc().to_text().unwrap();
    assert!(text.contains("(name \"Top view\")"), "{text}");
    let back = kdraft_doc::Document::parse(&text).unwrap();
    assert_eq!(back.pages[0].items[0].name(), Some("Top view"));
    assert!(back.pages[0].items[1].hidden());
    // Reorder: first item to the top.
    let n = s.doc().pages[0].items.len() as i64;
    reg.execute(&mut s, "item.reorder", json!({"id": ids[0], "to": n + 5}))
        .unwrap();
    assert_eq!(
        s.doc().pages[0].items.last().unwrap().id().unwrap().0,
        ids[0]
    );
    // Each is one undo step.
    for _ in 0..4 {
        assert!(s.undo().is_some());
    }
    assert_eq!(s.doc().pages[0].items[0].name(), None);
    assert!(!s.doc().pages[0].items[1].hidden());
    // Clearing a name.
    reg.execute(&mut s, "item.rename", json!({"id": ids[0], "name": " "}))
        .unwrap();
    assert_eq!(s.doc().pages[0].items[0].name(), None);
}

#[test]
fn dimension_bounds_and_precise_hit() {
    use kdraft_board::geom::Point;
    use kdraft_doc::DimensionKind;
    let s = Session::new_for_board(&fixture(), None).unwrap();
    let page = &s.doc().pages[0];
    let view = page.views().next().unwrap().clone();
    let vb = s.item_bounds(&Item::View(view.clone())).unwrap();
    let dims: Vec<&kdraft_doc::Dimension> = page
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Dimension(d) => Some(d),
            _ => None,
        })
        .collect();
    assert_eq!(dims.len(), 2, "board-size dimensions");
    for d in dims {
        let it = Item::Dimension(d.clone());
        let b = s.item_bounds(&it).expect("dimension has a box");
        let g = s.dimension_geometry(d).unwrap();
        assert!(!g.broken);
        assert_eq!(g.bbox, b);
        // On the dimension line and on the text: hit.
        let (a, c) = g.dim_line;
        let mid = Point::new((a.x + c.x) / 2.0, (a.y + c.y) / 2.0);
        assert!(s.item_hit(&it, mid, 0.2));
        assert!(s.item_hit(&it, g.text_at, 0.0));
        // The box spans the view's width or height, but its empty middle
        // (the view's centre) is no hit.
        let horizontal = (a.y - c.y).abs() < 1e-9;
        let vertical = (a.x - c.x).abs() < 1e-9;
        assert!(horizontal || vertical);
        if horizontal {
            assert!(b.width() >= vb.width() * 0.9, "{b:?} vs {vb:?}");
            assert!(
                b.max.y <= vb.min.y + 1e-6
                    || b.min.y >= vb.max.y - 1e-6
                    || b.height() < vb.height()
            );
        } else {
            assert!(b.height() >= vb.height() * 0.9);
        }
        assert!(!s.item_hit(&it, vb.center(), 0.5));
        // Dragging the dimension line away by 3 mm grows the offset by 3.
        let away = if horizontal {
            Point::new(0.0, if mid.y < vb.center().y { -3.0 } else { 3.0 })
        } else {
            Point::new(if mid.x < vb.center().x { -3.0 } else { 3.0 }, 0.0)
        };
        let o = crate::bounds::dragged_offset(d, &g, d.offset, away).unwrap();
        assert!(
            (o.abs() - (d.offset.abs() + 3.0)).abs() < 1e-9,
            "{o} from {}",
            d.offset
        );
        assert!(matches!(
            d.kind,
            DimensionKind::Horizontal | DimensionKind::Vertical | DimensionKind::Linear
        ));
    }
}

#[test]
fn broken_dimension_still_has_a_box() {
    let reg = Registry::new();
    let mut s = Session::new_for_board(&fixture(), None).unwrap();
    let vid = s.doc().pages[0].views().next().unwrap().id.0.clone();
    reg.execute(
        &mut s,
        "dimension.add_linear",
        json!({"view": vid, "from": "(outline-vertex 0)", "to": "(outline-vertex 1)"}),
    )
    .unwrap();
    let mut d = match s.doc().pages[0].items.last().unwrap() {
        Item::Dimension(d) => d.clone(),
        _ => panic!(),
    };
    d.from = Some(kdraft_doc::Anchor::Pad {
        reference: "NOPE".into(),
        number: "1".into(),
    });
    d.last.clear();
    let g = s.dimension_geometry(&d).unwrap();
    assert!(g.broken);
    assert!(s.item_bounds(&Item::Dimension(d)).is_some());
}

fn first_view_id(s: &Session) -> String {
    s.doc().pages[0].views().next().unwrap().id.0.clone()
}

fn dims_of(s: &Session, view: &str) -> Vec<kdraft_doc::Dimension> {
    s.doc()
        .pages
        .iter()
        .flat_map(|p| p.items.iter())
        .filter_map(|i| match i {
            Item::Dimension(d) if d.view.as_ref().map(|v| v.0.as_str()) == Some(view) => {
                Some(d.clone())
            }
            _ => None,
        })
        .collect()
}

fn add_callout(reg: &Registry, s: &mut Session, view: &str) -> String {
    let fp = s.board().unwrap().footprints[0].reference.clone().unwrap();
    reg.execute(
        s,
        "item.add_callout",
        json!({"view": view, "anchor": format!("(fp-origin \"{fp}\")"), "target": "note:N1", "at": [60.0, 60.0]}),
    )
    .unwrap();
    s.doc().pages[0]
        .items
        .last()
        .unwrap()
        .id()
        .unwrap()
        .0
        .clone()
}

/// Anchored items go with their view: moved (live, also in a gesture
/// preview, with a new revision per preview), deleted, duplicated and
/// moved to another page together with it.
#[test]
fn anchored_items_follow_their_view() {
    use kdraft_board::geom::Point;
    let reg = Registry::new();
    let mut s = Session::new_for_board(&fixture(), None).unwrap();
    let vid = first_view_id(&s);
    let cid = add_callout(&reg, &mut s, &vid);
    let d0 = dims_of(&s, &vid);
    assert_eq!(d0.len(), 2);
    let g0: Vec<_> = d0
        .iter()
        .map(|d| s.dimension_geometry(d).unwrap())
        .collect();
    let callout_at = |s: &Session| match s.find_item(&Id(cid.clone())).unwrap().1 {
        Item::Callout(c) => c.at,
        _ => panic!(),
    };
    let c0 = callout_at(&s);
    // Move the view alone.
    reg.execute(
        &mut s,
        "item.move",
        json!({"ids": [vid], "delta": [7.0, -3.0]}),
    )
    .unwrap();
    for (d, g) in dims_of(&s, &vid).iter().zip(&g0) {
        let g1 = s.dimension_geometry(d).unwrap();
        assert!((g1.text_at.x - g.text_at.x - 7.0).abs() < 1e-9);
        assert!((g1.text_at.y - g.text_at.y + 3.0).abs() < 1e-9);
    }
    assert!(callout_at(&s).approx_eq(Point::new(c0.x + 7.0, c0.y - 3.0), 1e-9));
    // View and callout selected together: the callout moves once.
    reg.execute(
        &mut s,
        "item.move",
        json!({"ids": [vid, cid], "delta": [1.0, 0.0]}),
    )
    .unwrap();
    assert!(callout_at(&s).approx_eq(Point::new(c0.x + 8.0, c0.y - 3.0), 1e-9));
    // A gesture preview gets a new revision each time; geometry follows.
    s.begin_gesture("Move");
    let r0 = s.revision();
    reg.preview(
        &mut s,
        "item.move",
        json!({"ids": [vid], "delta": [2.0, 0.0]}),
    )
    .unwrap();
    let r1 = s.revision();
    reg.preview(
        &mut s,
        "item.move",
        json!({"ids": [vid], "delta": [4.0, 0.0]}),
    )
    .unwrap();
    let r2 = s.revision();
    assert!(r0 != r1 && r1 != r2);
    let g2 = s.dimension_geometry(&dims_of(&s, &vid)[0]).unwrap();
    assert!((g2.text_at.x - g0[0].text_at.x - 12.0).abs() < 1e-9);
    assert!(s.commit_gesture());
    assert_eq!(s.undo_title(), Some("Move"));

    // Duplicate: the copy gets copies of its dimensions and callout,
    // remapped; only the copy of the view is selected.
    let n0 = s.doc().pages[0].items.len();
    reg.execute(&mut s, "item.duplicate", json!({"ids": [vid]}))
        .unwrap();
    assert_eq!(s.doc().pages[0].items.len(), n0 + 4);
    let new_view = s.selection.items[0].0.clone();
    assert_eq!(s.selection.items.len(), 1);
    assert_eq!(dims_of(&s, &new_view).len(), 2);
    s.undo();

    // Move to another page: along with it.
    let p2 = s.doc().pages[2].id.0.clone();
    reg.execute(
        &mut s,
        "item.move_to_page",
        json!({"ids": [vid], "page": p2}),
    )
    .unwrap();
    assert!(
        s.doc().pages[2]
            .items
            .iter()
            .filter(|i| matches!(i, Item::Dimension(_) | Item::Callout(_)))
            .count()
            >= 3
    );
    s.undo();

    // Delete the view: its dimensions and callout go too, one step.
    reg.execute(&mut s, "item.remove", json!({"ids": [vid]}))
        .unwrap();
    assert!(dims_of(&s, &vid).is_empty());
    assert!(s.find_item(&Id(cid.clone())).is_none());
    s.undo();
    assert_eq!(dims_of(&s, &vid).len(), 2);
    assert!(s.find_item(&Id(cid)).is_some());

    // Align with a dimension in the selection: the dimension is skipped.
    let all: Vec<String> = s.doc().pages[0]
        .items
        .iter()
        .filter_map(|i| i.id().map(|x| x.0.clone()))
        .collect();
    reg.execute(&mut s, "layout.align", json!({"ids": all, "mode": "left"}))
        .unwrap();
}

fn created(o: &crate::Outcome) -> String {
    o.created[0].clone()
}

#[test]
fn style_commands_and_all_back_ends() {
    use kdraft_doc::style::Rgba;
    let r = Registry::new();
    let mut s = session();
    let original = s.doc().clone();
    let line = created(
        &r.execute(
            &mut s,
            "item.add_line",
            json!({ "start": [20, 20], "end": [80, 20] }),
        )
        .unwrap(),
    );
    let rect = created(
        &r.execute(
            &mut s,
            "item.add_rect",
            json!({ "start": [20, 30], "end": [60, 50] }),
        )
        .unwrap(),
    );
    let text = created(
        &r.execute(
            &mut s,
            "item.add_text",
            json!({ "text": "Zeile 1\nµ ° ± Ø Über", "at": [20, 70] }),
        )
        .unwrap(),
    );
    r.execute(
        &mut s,
        "item.set_style",
        json!({ "ids": [line], "stroke": "#C81E1E", "width": 0.5, "pattern": "dashed", "arrows": "both" }),
    )
    .unwrap();
    // Text attributes do not apply to a line: ignored there, error alone.
    assert!(r
        .execute(
            &mut s,
            "item.set_style",
            json!({ "ids": [line], "bold": true })
        )
        .is_err());
    // A bad colour changes nothing.
    let before = s.doc().clone();
    assert!(r
        .execute(
            &mut s,
            "item.set_style",
            json!({ "ids": [line], "stroke": "#XYZ" })
        )
        .is_err());
    assert_eq!(*s.doc(), before);
    // Unknown args are rejected.
    assert!(r
        .execute(
            &mut s,
            "item.set_style",
            json!({ "ids": [line], "glow": 1 })
        )
        .is_err());
    let find = |s: &Session, id: &str| {
        s.doc()
            .pages
            .iter()
            .flat_map(|p| p.items.iter())
            .find(|i| i.id().map(|x| x.as_str()) == Some(id))
            .cloned()
            .unwrap()
    };
    let st = find(&s, &line).effective_style(&s.doc().styles);
    assert_eq!(st.stroke, Some(Rgba::rgb(0xC8, 0x1E, 0x1E)));
    assert_eq!(st.width, Some(0.5));

    // Copy and paste: only line attributes reach the rectangle.
    r.execute(&mut s, "style.copy", json!({ "id": line }))
        .unwrap();
    r.execute(&mut s, "style.paste", json!({ "ids": [rect] }))
        .unwrap();
    let rs = find(&s, &rect).style_override();
    assert_eq!(rs.stroke, Some(Rgba::rgb(0xC8, 0x1E, 0x1E)));
    assert_eq!(rs.pattern.map(|p| p.as_str()), Some("dashed"));
    // Values equal to the category default are not stored.
    assert_eq!(rs.opacity, None);

    r.execute(
        &mut s,
        "item.set_style",
        json!({ "ids": [text], "text_color": "#1E50C8", "bold": true, "align": "center", "text_height": 3.5 }),
    )
    .unwrap();
    r.execute(
        &mut s,
        "document.set_styles",
        json!({ "category": "graphic", "fill": "#FFF0C0" }),
    )
    .unwrap();
    assert!(r
        .execute(
            &mut s,
            "document.set_styles",
            json!({ "category": "graphic", "bold": true })
        )
        .is_err());
    assert_eq!(
        find(&s, &rect).effective_style(&s.doc().styles).fill,
        Some(Rgba::rgb(0xFF, 0xF0, 0xC0))
    );

    // Save and reopen: styles survive.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("styled.kdraft");
    let text_out = s.doc().to_text().unwrap();
    std::fs::write(&path, &text_out).unwrap();
    let back = kdraft_doc::Document::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(back, *s.doc());

    // Every back end shows the colours.
    let (inputs, _) = crate::pipeline::RenderInputs::from_session(&mut s, None).unwrap();
    let pi = s.current_page_index().unwrap();
    let (page, _) = inputs.render_page(pi).unwrap();
    let svg = kdraft_render::svg_out::page_svg(&page, &inputs.fonts);
    assert!(svg.contains("stroke=\"#C81E1E\""), "line colour");
    assert!(svg.contains("stroke-dasharray"), "dashes");
    assert!(svg.contains("fill=\"#1E50C8\""), "text colour");
    assert!(svg.contains("fill=\"#FFF0C0\""), "fill");
    assert!(
        svg.contains("Zeile 1") && svg.contains("µ ° ± Ø Über"),
        "both lines"
    );
    let png = kdraft_render::raster::page_pixmap(&page, 4.0, &inputs.fonts).unwrap();
    let px = |x: f64, y: f64| {
        let p = png.pixel((x * 4.0) as u32, (y * 4.0) as u32).unwrap();
        (p.red(), p.green(), p.blue())
    };
    // Inside the filled rectangle.
    assert_eq!(px(40.0, 40.0), (0xFF, 0xF0, 0xC0));
    let reds = png
        .pixels()
        .iter()
        .filter(|p| p.red() > 150 && p.green() < 80 && p.blue() < 80)
        .count();
    assert!(reds > 50, "{reds} red pixels");
    let (dl, _) = inputs.render();
    let pdf = kdraft_render::pdf::write_pdf(&dl, &inputs.fonts).unwrap();
    assert!(pdf.starts_with(b"%PDF"));

    // Reset, then undo everything.
    r.execute(&mut s, "item.reset_style", json!({ "ids": [line] }))
        .unwrap();
    assert!(find(&s, &line).style_override().is_empty());
    while s.can_undo() {
        s.undo();
    }
    assert_eq!(*s.doc(), original);
}
