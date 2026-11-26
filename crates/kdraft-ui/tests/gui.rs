//! Headless interaction tests: egui runs without a window or GPU through
//! egui_kittest (no rendering, AccessKit for finding widgets). They check
//! that toolbar buttons and canvas drags reach the engine; they do not
//! check what anything looks like.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use kdraft_doc::Item;
use kdraft_engine::Session;
use kdraft_ui::App;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/kicad9-minimal.kicad_pcb")
}

fn harness(s: Session) -> Harness<'static, App> {
    let ctx = egui::Context::default();
    let app = App::new(&ctx, s, false);
    Harness::builder()
        .with_size(egui::Vec2::new(1600.0, 1000.0))
        .with_context_setup(kdraft_ui::app::install_fonts)
        .build_ui_state(|ui, app: &mut App| app.show(ui), app)
}

/// Steps until `f` holds (background textures, kicad-cli discovery).
fn step_until(h: &mut Harness<'_, App>, f: impl Fn(&App) -> bool) {
    let t = Instant::now();
    while !f(h.state()) && t.elapsed() < Duration::from_secs(20) {
        h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(f(h.state()), "condition not reached");
}

fn session() -> Session {
    Session::new_for_board(&fixture(), None).unwrap()
}

fn view_at(app: &App, page: usize) -> kdraft_board::geom::Point {
    match &app.s.doc().pages[page].items[0] {
        Item::View(v) => v.at,
        _ => panic!("no view"),
    }
}

#[test]
fn toolbar_align_moves_the_selected_view() {
    let mut h = harness(session());
    h.run_steps(3);
    let id = h.state().s.doc().pages[0].items[0].id().unwrap().0.clone();
    h.state_mut()
        .cmd("select.set", serde_json::json!({ "ids": [id] }));
    h.run_steps(2);
    let before = view_at(h.state(), 0);
    h.get_by_label("Align right").click();
    h.run_steps(2);
    let after = view_at(h.state(), 0);
    assert!(after.x > before.x, "{before:?} -> {after:?}");
    assert_eq!(after.y, before.y);
    // The view's right edge is on the drawing area's right edge.
    let app = h.state_mut();
    let area = app.s.drawing_area(0).unwrap();
    let it = app.s.doc().pages[0].items[0].clone();
    let b = app.s.item_bounds(&it).unwrap();
    assert!((b.max.x - area.max.x).abs() < 1e-9);
    assert_eq!(app.s.undo_title(), Some("Align"));
    // Undo through the toolbar.
    h.get_by_label("Undo").click();
    h.run_steps(2);
    assert_eq!(view_at(h.state(), 0), before);
}

#[test]
fn canvas_drag_moves_with_snapping_as_one_undo_step() {
    let mut h = harness(session());
    // Let the canvas fit the page and the textures arrive.
    step_until(&mut h, |a| {
        a.textures.keys().any(|k| k.starts_with("item:"))
    });
    let app = h.state();
    let at = view_at(app, 0);
    let s0 = app.camera.to_screen(at);
    let o = app.canvas_origin;
    let p0 = egui::pos2((o.x + s0.x) as f32, (o.y + s0.y) as f32);
    h.hover_at(p0);
    h.step();
    h.drag_at(p0);
    h.step();
    for k in 1..=10 {
        h.hover_at(p0 + egui::vec2(6.0 * k as f32, 3.0 * k as f32));
        h.step();
    }
    h.drop_at(p0 + egui::vec2(60.0, 30.0));
    h.run_steps(2);
    let app = h.state();
    let after = view_at(app, 0);
    assert!(
        after.x > at.x + 1.0 && after.y > at.y + 1.0,
        "{at:?} -> {after:?}"
    );
    assert_eq!(app.s.undo_title(), Some("Move"));
    // Snapping on: some edge or centre of the view landed on the grid or
    // on a guide; at least the moved box's top-left is on the 0.5 mm grid
    // or an object target. Check the cheap invariant: one undo restores.
    let raw = kdraft_board::geom::Point::new(60.0 / app.camera.zoom, 30.0 / app.camera.zoom);
    let tol = kdraft_ui::app::SNAP_TOLERANCE_PX / app.camera.zoom;
    let d = after - at;
    assert!(
        (d.x - raw.x).abs() <= tol + 1e-9 && (d.y - raw.y).abs() <= tol + 1e-9,
        "{d:?} vs {raw:?}"
    );
    let selected = app.s.selection.items.clone();
    assert_eq!(selected.len(), 1);
    h.state_mut().s.undo();
    assert_eq!(view_at(h.state(), 0), at);
    assert!(!h.state().s.can_undo());
}

#[test]
fn page_buttons_and_shortcuts() {
    let mut h = harness(session());
    h.run_steps(3);
    let first = h.state().s.doc().pages[0].id.clone();
    let n0 = h.state().s.doc().pages.len();
    // Page 1 is current; Alt+Down moves it to index 1.
    h.key_press_modifiers(egui::Modifiers::ALT, egui::Key::ArrowDown);
    h.run_steps(2);
    assert_eq!(h.state().s.doc().pages[1].id, first);
    h.get_by_label("Add page").click();
    h.run_steps(2);
    assert_eq!(h.state().s.doc().pages.len(), n0 + 1);
    // Ctrl+Z twice restores the original order and count.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert_eq!(h.state().s.doc().pages.len(), n0);
    assert_eq!(h.state().s.doc().pages[0].id, first);
}

#[test]
fn errors_go_to_the_message_area() {
    let mut h = harness(session());
    h.run_steps(2);
    // Nothing selected: the button is disabled; the command itself (as a
    // shortcut or script would run it) reports an error, nothing panics.
    assert!(h.get_by_label("Align left").accesskit_node().is_disabled());
    h.state_mut()
        .cmd("layout.align", serde_json::json!({ "mode": "left" }));
    h.run_steps(2);
    assert!(h
        .state()
        .messages
        .iter()
        .any(|m| m.text.contains("nothing selected")));
}

#[test]
fn board_reload_shows_a_change_banner() {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("b.kicad_pcb");
    let text = std::fs::read_to_string(fixture()).unwrap();
    std::fs::write(&board, &text).unwrap();
    let s = Session::new_for_board(&board, Some(&dir.path().join("b.kdraft"))).unwrap();
    let r1 = s
        .board()
        .unwrap()
        .footprint_by_reference("R1")
        .unwrap()
        .position;
    let mut h = harness(s);
    h.run_steps(2);
    let needle = format!("(at {} {}", r1.x, r1.y);
    std::fs::write(
        &board,
        text.replacen(&needle, &format!("(at {} {}", r1.x + 2.0, r1.y), 1),
    )
    .unwrap();
    h.get_by_label("Reload board").click();
    step_until(&mut h, |a| a.banner.is_some());
    let b = h.state().banner.clone().unwrap();
    assert!(b.contains("1 footprint moved"), "{b}");
    let r1b = h
        .state()
        .s
        .board()
        .unwrap()
        .footprint_by_reference("R1")
        .unwrap()
        .position;
    assert!((r1b.x - r1.x - 2.0).abs() < 1e-9);
}

/// The status bar's cursor readout follows the pointer over the canvas,
/// also over items; the canvas sets arrow, grab and resize cursors.
#[test]
fn cursor_readout_and_icon_over_the_canvas() {
    use kdraft_ui::hit::CursorKind;
    let mut h = harness(session());
    step_until(&mut h, |a| {
        a.textures.keys().any(|k| k.starts_with("item:"))
    });
    let screen = |app: &App, p: kdraft_board::geom::Point| {
        let s = app.camera.to_screen(p);
        let o = app.canvas_origin;
        egui::pos2((o.x + s.x) as f32, (o.y + s.y) as f32)
    };
    // Over the view (an item).
    let at = view_at(h.state(), 0);
    let p0 = screen(h.state(), at);
    h.hover_at(p0);
    h.run_steps(2);
    let app = h.state();
    let c = app.cursor_mm.expect("cursor position over an item");
    assert!(c.dist(at) < 1.0, "{c:?} vs {at:?}");
    let text = app.cursor_text();
    assert!(text.chars().any(|ch| ch.is_ascii_digit()), "{text}");
    // The status bar shows exactly this text (no one-frame lag).
    h.get_by_label(&text);
    assert_eq!(h.state().canvas_cursor, Some(CursorKind::Grab));
    // Moving on: the readout changes with the pointer.
    h.hover_at(p0 + egui::vec2(40.0, 0.0));
    h.run_steps(2);
    let c2 = h.state().cursor_mm.unwrap();
    assert!(c2.x > c.x, "{c:?} -> {c2:?}");
    h.get_by_label(&h.state().cursor_text());
    // Empty sheet corner: arrow.
    let corner = screen(h.state(), kdraft_board::geom::Point::new(2.0, 2.0));
    h.hover_at(corner);
    h.run_steps(2);
    assert_eq!(h.state().canvas_cursor, Some(CursorKind::Default));
    // Selected view: resize cursor on a corner handle.
    let id = h.state().s.doc().pages[0].items[0].id().unwrap().0.clone();
    h.state_mut()
        .cmd("select.set", serde_json::json!({ "ids": [id] }));
    h.run_steps(2);
    let it = h.state().s.doc().pages[0].items[0].clone();
    let b = h.state().s.item_bounds(&it).unwrap();
    h.hover_at(screen(h.state(), b.max));
    h.run_steps(2);
    assert_eq!(h.state().canvas_cursor, Some(CursorKind::ResizeNwSe));
}

fn entry(app: &App, i: usize) -> egui::Rect {
    let r = app.page_entries[i];
    egui::Rect::from_min_max(egui::pos2(r[0], r[1]), egui::pos2(r[2], r[3]))
}

fn click(h: &mut Harness<'_, App>, p: egui::Pos2) {
    h.hover_at(p);
    h.step();
    h.drag_at(p);
    h.step();
    h.drop_at(p);
    h.run_steps(2);
}

fn current(app: &App) -> usize {
    app.s.current_page_index().unwrap()
}

/// Thumbnails, labels and padding select; presses without movement never
/// reorder; real drags reorder and leave the dragged page selected.
#[test]
fn page_entries_click_select_and_drag_reorder() {
    let mut h = harness(session());
    step_until(&mut h, |a| {
        a.textures.keys().any(|k| k.starts_with("thumb:"))
    });
    h.run_steps(3);
    let ids: Vec<_> = h
        .state()
        .s
        .doc()
        .pages
        .iter()
        .map(|p| p.id.clone())
        .collect();
    assert_eq!(current(h.state()), 0);
    // Click on the thumbnail image of page 2 (upper part of the entry).
    let e = entry(h.state(), 1);
    click(&mut h, egui::pos2(e.center().x, e.top() + 0.3 * e.height()));
    assert_eq!(current(h.state()), 1);
    let order = |h: &Harness<'_, App>| -> Vec<_> {
        h.state()
            .s
            .doc()
            .pages
            .iter()
            .map(|p| p.id.clone())
            .collect()
    };
    assert_eq!(order(&h), ids, "a click never reorders");
    // Click on the label of page 1 (bottom of the entry).
    let e = entry(h.state(), 0);
    click(&mut h, egui::pos2(e.center().x, e.bottom() - 4.0));
    assert_eq!(current(h.state()), 0);
    assert_eq!(order(&h), ids);
    // A tiny drag (below egui's threshold) is a click: no reorder.
    let e = entry(h.state(), 1);
    let p = e.center();
    h.hover_at(p);
    h.step();
    h.drag_at(p);
    h.step();
    h.hover_at(p + egui::vec2(1.0, -1.0));
    h.step();
    h.drop_at(p + egui::vec2(1.0, -1.0));
    h.run_steps(2);
    assert_eq!(order(&h), ids);
    assert_eq!(current(h.state()), 1);
    // A real drag of page 2 above page 1.
    let target = entry(h.state(), 0).top() + 2.0;
    h.state_mut()
        .cmd("page.select", serde_json::json!({ "page": ids[0].0 }));
    h.run_steps(2);
    h.hover_at(p);
    h.step();
    h.drag_at(p);
    h.step();
    for k in 1..=8 {
        let y = p.y + (target - p.y) * k as f32 / 8.0;
        h.hover_at(egui::pos2(p.x, y));
        h.step();
    }
    assert_eq!(h.state().page_drag.dragging(), Some(1));
    h.drop_at(egui::pos2(p.x, target));
    h.run_steps(2);
    let now = order(&h);
    assert_eq!(now[0], ids[1]);
    assert_eq!(now[1], ids[0]);
    assert_eq!(h.state().s.doc().pages[current(h.state())].id, ids[1]);
    // PageDown / PageUp select the next / previous page.
    let c = current(h.state());
    h.key_press(egui::Key::PageDown);
    h.run_steps(2);
    assert_eq!(current(h.state()), c + 1);
    h.key_press(egui::Key::PageUp);
    h.run_steps(2);
    assert_eq!(current(h.state()), c);
}

/// M3 upkeep: toolbar actions disabled when they cannot apply (same rule
/// as the engine), new view settings in the properties panel, and the new
/// item kinds (dimensions, notes) get canvas textures.
#[test]
fn m3_toolbar_enablement_properties_and_new_items() {
    let mut h = harness(session());
    h.run_steps(3);
    assert!(h
        .get_by_label("Distribute H")
        .accesskit_node()
        .is_disabled());
    assert!(h.get_by_label("Align left").accesskit_node().is_disabled());
    let fit = |h: &Harness<'_, App>| {
        let e = h.state().entry("app.fit_selection").unwrap();
        h.state().enabled(&e)
    };
    assert!(!fit(&h));
    let id = h.state().s.doc().pages[0].items[0].id().unwrap().0.clone();
    h.state_mut()
        .cmd("select.set", serde_json::json!({ "ids": [id] }));
    h.run_steps(2);
    assert!(!h.get_by_label("Align left").accesskit_node().is_disabled());
    assert!(h
        .get_by_label("Distribute H")
        .accesskit_node()
        .is_disabled());
    let mw = h.state().entry("layout.match:width").unwrap();
    assert!(!h.state().enabled(&mw));
    assert!(fit(&h));
    // Fabrication page: board-size dimensions and notes are drawn.
    let ids: Vec<String> = h.state().s.doc().pages[0]
        .items
        .iter()
        .filter(|i| matches!(i, Item::Dimension(_) | Item::NoteList(_)))
        .map(|i| i.id().unwrap().0.clone())
        .collect();
    assert_eq!(ids.len(), 3);
    // Note lists are textures; dimensions are drawn live as vectors.
    step_until(&mut h, |a| {
        a.s.doc().pages[0].items.iter().all(|it| match it {
            Item::NoteList(n) => a.textures.contains_key(&format!("item:{}", n.id)),
            Item::Dimension(_) => !a.live_items(it).is_empty(),
            _ => true,
        })
    });
    // Assembly view properties: render mode, fab texts, native options.
    let page = h.state().s.doc().pages[1].id.0.clone();
    h.state_mut()
        .cmd("page.select", serde_json::json!({ "page": page }));
    let vid = h.state().s.doc().pages[1].items[0].id().unwrap().0.clone();
    h.state_mut()
        .cmd("select.set", serde_json::json!({ "ids": [vid] }));
    h.run_steps(3);
    h.get_by_label("Show values");
    h.get_by_label("Pad outlines");
    // The inspector is taller than the window: scroll to the checkbox.
    h.get_by_label("Show reference designators").scroll_to_me();
    h.run_steps(3);
    h.get_by_label("Show reference designators").click();
    h.run_steps(2);
    let refs = match &h.state().s.doc().pages[1].items[0] {
        Item::View(v) => v.assembly.references,
        _ => None,
    };
    assert_eq!(refs, Some(false));
}

fn to_screen(app: &App, p: kdraft_board::geom::Point) -> egui::Pos2 {
    let s = app.camera.to_screen(p);
    let o = app.canvas_origin;
    egui::pos2((o.x + s.x) as f32, (o.y + s.y) as f32)
}

/// A horizontal dimension between the first pads of two footprints that
/// lie apart in x, with its line 6 mm above them: its box covers part of
/// the view.
fn add_inner_dimension(h: &mut Harness<'_, App>) -> (String, kdraft_engine::bounds::DimGeom) {
    let app = h.state_mut();
    let b = app.s.board().unwrap().clone();
    let mut fps: Vec<_> = b
        .footprints
        .iter()
        .filter(|f| f.reference.is_some() && !f.pads.is_empty())
        .collect();
    fps.sort_by(|a, c| a.pads[0].position.x.total_cmp(&c.pads[0].position.x));
    let (f1, f2) = (fps[0], fps[fps.len() - 1]);
    let a = |f: &kdraft_board::Footprint| {
        format!(
            "(pad \"{}\" \"{}\")",
            f.reference.clone().unwrap(),
            f.pads[0].number
        )
    };
    let vid = app.s.doc().pages[0].views().next().unwrap().id.0.clone();
    assert!(app.cmd(
        "dimension.add_linear",
        serde_json::json!({"view": vid, "from": a(f1), "to": a(f2), "kind": "horizontal", "offset": 6.0}),
    ));
    let d = match app.s.doc().pages[0].items.last().unwrap() {
        Item::Dimension(d) => d.clone(),
        _ => panic!("no dimension"),
    };
    let g = app.s.dimension_geometry(&d).unwrap();
    app.cmd("select.clear", serde_json::json!({}));
    (d.id.0.clone(), g)
}

fn selected(app: &App) -> Vec<String> {
    app.s.selection.items.iter().map(|i| i.0.clone()).collect()
}

/// Dimensions can be picked by their lines and text, not by the empty
/// middle of their box; dragging moves the dimension line (one undo
/// step); Delete removes it; a marquee includes it.
#[test]
fn dimensions_select_drag_delete_and_marquee() {
    let mut h = harness(session());
    step_until(&mut h, |a| {
        a.textures.keys().any(|k| k.starts_with("item:"))
    });
    let (did, g) = add_inner_dimension(&mut h);
    h.run_steps(3);
    let vid = h.state().s.doc().pages[0]
        .views()
        .next()
        .unwrap()
        .id
        .0
        .clone();
    // On the dimension line (a quarter along, away from the text).
    let (a, b) = g.dim_line;
    let q = kdraft_board::geom::Point::new(a.x + (b.x - a.x) * 0.2, a.y);
    {
        let sp = to_screen(h.state(), q);
        click(&mut h, sp);
    }
    assert_eq!(selected(h.state()), vec![did.clone()], "line click");
    h.state_mut().cmd("select.clear", serde_json::json!({}));
    h.run_steps(2);
    // On the text.
    {
        let sp = to_screen(h.state(), g.text_at);
        click(&mut h, sp);
    }
    assert_eq!(selected(h.state()), vec![did.clone()], "text click");
    h.state_mut().cmd("select.clear", serde_json::json!({}));
    h.run_steps(2);
    // Inside the box, between the extension lines, below the dimension
    // line: over the view, so the view is picked.
    let inside = kdraft_board::geom::Point::new((a.x + b.x) / 2.0, a.y + 4.0);
    assert!(
        g.bbox.min.y <= inside.y && inside.y <= g.bbox.max.y,
        "{g:?}"
    );
    {
        let sp = to_screen(h.state(), inside);
        click(&mut h, sp);
    }
    assert_eq!(
        selected(h.state()),
        vec![vid.clone()],
        "empty middle picks the view"
    );
    // Drag the dimension line 3 mm up: offset grows, one undo step.
    h.state_mut()
        .cmd("select.set", serde_json::json!({"ids": [did]}));
    h.run_steps(2);
    let undo_before = h.state().s.undo_title().map(str::to_string);
    let p0 = to_screen(h.state(), q);
    let dy = (3.0 * h.state().camera.zoom) as f32;
    h.hover_at(p0);
    h.step();
    h.drag_at(p0);
    h.step();
    for k in 1..=6 {
        h.hover_at(p0 - egui::vec2(0.0, dy * k as f32 / 6.0));
        h.step();
    }
    h.drop_at(p0 - egui::vec2(0.0, dy));
    h.run_steps(2);
    let off = |app: &App| match app.s.doc().pages[0]
        .items
        .iter()
        .find(|i| i.id().unwrap().0 == did)
    {
        Some(Item::Dimension(d)) => d.offset,
        _ => f64::NAN,
    };
    let o = off(h.state());
    assert!((o - 9.0).abs() < 0.3, "offset {o}");
    assert_eq!(h.state().s.undo_title(), Some("Dimension offset"));
    h.state_mut().s.undo();
    assert_eq!(off(h.state()), 6.0);
    assert_eq!(h.state().s.undo_title().map(str::to_string), undo_before);
    // Marquee around the dimension (from empty sheet top-left of it).
    h.state_mut().cmd("select.clear", serde_json::json!({}));
    h.run_steps(2);
    // Start on empty paper: above and left of everything around the view.
    let all = h
        .state()
        .item_boxes()
        .into_iter()
        .map(|(_, b)| b)
        .reduce(|x, y| x.union(&y))
        .unwrap();
    let vb = h
        .state()
        .s
        .item_bounds(&h.state().s.doc().pages[0].items[0].clone())
        .unwrap();
    let corner = kdraft_board::geom::Point::new(
        vb.min.x.min(g.bbox.min.x) - 8.0,
        vb.min.y.min(g.bbox.min.y) - 8.0,
    );
    assert!(
        h.state().pick_item(corner, 0.5).is_none(),
        "{corner:?} {all:?}"
    );
    let m0 = to_screen(h.state(), corner);
    let m1 = to_screen(
        h.state(),
        kdraft_board::geom::Point::new(g.bbox.max.x + 3.0, g.bbox.max.y + 3.0),
    );
    // Start on empty paper above the dimension.
    let start = egui::pos2(m0.x, m0.y);
    h.hover_at(start);
    h.step();
    h.drag_at(start);
    h.step();
    for k in 1..=6 {
        h.hover_at(start + (m1 - start) * (k as f32 / 6.0));
        h.step();
    }
    h.drop_at(m1);
    h.run_steps(2);
    assert!(selected(h.state()).contains(&did), "marquee");
    // Delete removes it.
    h.state_mut()
        .cmd("select.set", serde_json::json!({"ids": [did]}));
    h.run_steps(2);
    h.key_press(egui::Key::Delete);
    h.run_steps(2);
    assert!(h.state().s.doc().pages[0]
        .items
        .iter()
        .all(|i| i.id().unwrap().0 != did));
}

/// Every M3 item kind can be selected on the canvas and shows its
/// properties.
#[test]
fn m3_items_select_and_show_properties() {
    let mut h = harness(session());
    step_until(&mut h, |a| {
        a.textures.keys().any(|k| k.starts_with("item:"))
    });
    let app = h.state_mut();
    let vid = app.s.doc().pages[0].views().next().unwrap().id.0.clone();
    let fp = app.s.board().unwrap().footprints[0]
        .reference
        .clone()
        .unwrap();
    assert!(app.cmd(
        "item.add_text",
        serde_json::json!({"text": "HELLO", "at": [40.0, 40.0]})
    ));
    assert!(app.cmd(
        "item.add_callout",
        serde_json::json!({"view": vid, "anchor": format!("(fp-origin \"{fp}\")"), "target": "note:N1", "at": [60.0, 60.0]})
    ));
    h.run_steps(3);
    let items = h.state().s.doc().pages[0].items.clone();
    for it in items.iter().filter(|i| !matches!(i, Item::View(_))) {
        let id = it.id().unwrap().0.clone();
        let b = h
            .state()
            .s
            .item_bounds(it)
            .unwrap_or_else(|| panic!("{} has no box", it.kind_name()));
        let p = match it {
            Item::Dimension(d) => h.state().s.dimension_geometry(d).unwrap().text_at,
            Item::Graphic(_) => {
                kdraft_board::geom::Point::new(b.min.x + 0.5, (b.min.y + b.max.y) / 2.0)
            }
            _ => b.center(),
        };
        h.state_mut().cmd("select.clear", serde_json::json!({}));
        h.run_steps(2);
        {
            let sp = to_screen(h.state(), p);
            click(&mut h, sp);
        }
        assert_eq!(selected(h.state()), vec![id], "{}", it.kind_name());
    }
    // Properties of a note list and a dimension.
    let notes = items
        .iter()
        .find(|i| matches!(i, Item::NoteList(_)))
        .unwrap()
        .id()
        .unwrap()
        .0
        .clone();
    h.state_mut()
        .cmd("select.set", serde_json::json!({"ids": [notes]}));
    h.run_steps(2);
    h.get_by_label("Add note");
    let dim = items
        .iter()
        .find(|i| matches!(i, Item::Dimension(_)))
        .unwrap()
        .id()
        .unwrap()
        .0
        .clone();
    h.state_mut()
        .cmd("select.set", serde_json::json!({"ids": [dim]}));
    h.run_steps(2);
    h.get_by_label("Offset");
    h.get_by_label("Tolerance");
}

/// User report: dragging a view left its dimensions behind (cached
/// textures keyed by the view's position), then they jumped. Anchored
/// items are drawn live: during the drag preview, before the commit, the
/// dimension geometry the canvas draws from has moved, and a headless
/// screenshot taken mid-gesture shows the dimension line at its new place.
#[test]
fn dimensions_follow_a_dragged_view_live() {
    use kdraft_ui::shot::Shooter;
    let ctx = egui::Context::default();
    let mut app = App::new(&ctx, session(), false);
    let mut sh = Shooter::new(ctx, egui::vec2(1400.0, 900.0), 1.0);
    sh.settle(&mut app, Duration::from_secs(30));
    let horizontal = |app: &App| {
        app.s.doc().pages[0]
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Dimension(d) => app.dim_geometry(d),
                _ => None,
            })
            .find(|g| (g.dim_line.0.y - g.dim_line.1.y).abs() < 1e-9)
            .unwrap()
    };
    let g0 = horizontal(&app);
    let vid = app.s.doc().pages[0].views().next().unwrap().id.clone();
    let vb = app
        .s
        .item_bounds(app.s.doc().pages[0].item(&vid).unwrap())
        .unwrap();
    let a = to_screen(&app, vb.center());
    sh.drag(&mut app, a, a + egui::vec2(0.0, 60.0), false);
    assert!(app.s.in_gesture(), "still dragging");
    let g1 = horizontal(&app);
    let dy = g1.dim_line.0.y - g0.dim_line.0.y;
    assert!(
        dy > 5.0,
        "dimension moved with the view during the preview: {dy}"
    );
    // Mid-gesture screenshot: dark pixels on the new line, none on the old.
    let f = sh.capture(egui::Color32::BLACK);
    let rgba = f.rgba();
    let dark_near = |p: egui::Pos2| {
        (-2..=2).any(|k| {
            let (x, y) = (p.x.round() as i64, p.y.round() as i64 + k);
            let i = ((y * f.w as i64 + x) * 4) as usize;
            rgba[i] < 120 && rgba[i + 1] < 120 && rgba[i + 2] < 120
        })
    };
    let quarter = |g: &kdraft_engine::bounds::DimGeom| {
        let (p, q) = g.dim_line;
        kdraft_board::geom::Point::new(p.x + (q.x - p.x) * 0.3, p.y)
    };
    assert!(
        dark_near(to_screen(&app, quarter(&g1))),
        "line at its new place"
    );
    assert!(
        !dark_near(to_screen(&app, quarter(&g0))),
        "nothing left at the old place"
    );
}

/// Finished background jobs request a repaint (nothing waits for the next
/// input event).
#[test]
fn finished_background_jobs_request_a_repaint() {
    let ctx = egui::Context::default();
    let mut app = App::new(&ctx, session(), false);
    let mut sh = kdraft_ui::shot::Shooter::new(ctx.clone(), egui::vec2(1200.0, 800.0), 1.0);
    sh.settle(&mut app, Duration::from_secs(30));
    // Frames until egui stops asking for repaints on its own.
    for _ in 0..10 {
        sh.frame(&mut app, Vec::new());
    }
    assert!(app.idle());
    // A change makes new textures (raster jobs); when they finish, the
    // worker's waker asks for a repaint.
    app.cmd("item.set_scale", serde_json::json!({"ids": [app.s.doc().pages[0].views().next().unwrap().id.0.clone()], "scale": 1.0}));
    sh.frame(&mut app, Vec::new());
    let t = Instant::now();
    while !app.idle() && t.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(5));
        if ctx.has_requested_repaint() {
            break;
        }
    }
    assert!(ctx.has_requested_repaint());
}

/// A page deleted while its thumbnail and sheet are being rendered: the
/// late jobs are dropped without a warning or error in the log.
#[test]
fn deleting_a_page_with_pending_renders_logs_nothing() {
    use kdraft_ui::jobs::RasterWhat;
    let mut h = harness(Session::new_for_board(&fixture(), None).unwrap());
    step_until(&mut h, |a| a.inputs.is_some());
    // A new page: its thumbnail and sheet are requested on the next frame.
    h.state_mut().cmd("page.add", serde_json::json!({}));
    h.step();
    let pid = h.state().s.current_page().unwrap().id.0.clone();
    h.state_mut()
        .cmd("page.remove", serde_json::json!({ "page": pid }));
    // Jobs that reach the worker for a page its document does not have.
    let gen = h.state().inputs_gen;
    let key = kdraft_ui::jobs::page_key(h.state().s.doc(), 0, &gen);
    let gone = kdraft_doc::Id("no-such-page".into());
    h.state_mut().want_texture(
        "thumb:no-such-page",
        key,
        RasterWhat::Thumb { page: gone.clone() },
        0,
    );
    h.state_mut().want_texture(
        "sheet:no-such-page",
        key,
        RasterWhat::Sheet { page: gone },
        0,
    );
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(2) {
        h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    let bad: Vec<String> = h
        .state()
        .messages
        .iter()
        .filter(|m| !matches!(m.level, kdraft_ui::app::Level::Info))
        .map(|m| m.text.clone())
        .collect();
    assert!(bad.is_empty(), "{bad:?}");
    assert!(!h.state().textures.contains_key(&format!("thumb:{pid}")));
    assert!(!h.state().textures.contains_key("thumb:no-such-page"));
    assert!(!h.state().textures.contains_key("sheet:no-such-page"));
}
