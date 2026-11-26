//! Headless interaction tests of the UI pass: menus and palette from the
//! command catalogue, item tree, layer presets, number fields, tools,
//! toasts, issues panel, theme persistence.

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

fn session() -> Session {
    Session::new_for_board(&fixture(), None).unwrap()
}

fn harness_with(app: App) -> Harness<'static, App> {
    Harness::builder()
        .with_size(egui::Vec2::new(1600.0, 1000.0))
        .with_context_setup(kdraft_ui::app::install_fonts)
        .build_ui_state(|ui, app: &mut App| app.show(ui), app)
}

fn harness() -> Harness<'static, App> {
    let ctx = egui::Context::default();
    harness_with(App::new(&ctx, session(), false))
}

fn step_until(h: &mut Harness<'_, App>, f: impl Fn(&App) -> bool) {
    let t = Instant::now();
    while !f(h.state()) && t.elapsed() < Duration::from_secs(20) {
        h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(f(h.state()), "condition not reached");
}

fn screen(app: &App, p: kdraft_board::geom::Point) -> egui::Pos2 {
    let s = app.camera.to_screen(p);
    let o = app.canvas_origin;
    egui::pos2((o.x + s.x) as f32, (o.y + s.y) as f32)
}

fn click(h: &mut Harness<'_, App>, p: egui::Pos2) {
    h.hover_at(p);
    h.step();
    h.drag_at(p);
    h.step();
    h.drop_at(p);
    h.run_steps(2);
}

fn view_id(app: &App) -> String {
    app.s.doc().pages[0].views().next().unwrap().id.0.clone()
}

fn select(h: &mut Harness<'_, App>, id: &str) {
    h.state_mut()
        .cmd("select.set", serde_json::json!({ "ids": [id] }));
    h.run_steps(2);
}

#[test]
fn menus_are_generated_from_the_catalogue() {
    let mut h = harness();
    h.run_steps(3);
    let vid = view_id(h.state());
    select(&mut h, &vid);
    h.get_by_label("Arrange").click();
    h.run_steps(2);
    // An entry that exists only in the menu.
    h.get_by_label("Bring to front").click();
    h.run_steps(2);
    let page = &h.state().s.doc().pages[0];
    assert_eq!(page.items.last().unwrap().id().unwrap().0, vid);
    assert_eq!(h.state().s.undo_title(), Some("Bring forward"));
    // A disabled entry: distribute needs three items.
    h.get_by_label("Arrange").click();
    h.run_steps(2);
    let node = h.get_by_label("Distribute horizontally");
    assert!(node.accesskit_node().is_disabled());
}

#[test]
fn palette_searches_and_runs() {
    let mut h = harness();
    h.run_steps(3);
    let vid = view_id(h.state());
    select(&mut h, &vid);
    let n = h.state().s.doc().pages[0].items.len();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::K);
    h.run_steps(2);
    assert!(h.state().palette.is_some());
    h.event(egui::Event::Text("duplic".into()));
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.state().palette.is_none());
    assert!(h.state().s.doc().pages[0].items.len() > n, "duplicated");
    assert_eq!(h.state().s.undo_title(), Some("Duplicate"));
}

#[test]
fn item_tree_select_rename_hide_and_reorder() {
    let mut h = harness();
    h.run_steps(4);
    let rect = |app: &App, k: usize| {
        let r = app.tree_rects[k].1;
        egui::Rect::from_min_max(egui::pos2(r[0], r[1]), egui::pos2(r[2], r[3]))
    };
    let ids: Vec<String> = h
        .state()
        .tree_rects
        .iter()
        .map(|x| x.0 .0.clone())
        .collect();
    assert!(ids.len() >= 3);
    // Click selects; Ctrl+click adds.
    let r0 = rect(h.state(), 0);
    click(&mut h, r0.left_center() + egui::vec2(40.0, 0.0));
    assert_eq!(h.state().s.selection.items.len(), 1);
    assert_eq!(h.state().s.selection.items[0].0, ids[0]);
    let names: Vec<String> = kdraft_ui::tree::rows(&h.state().s.doc().pages[0])
        .into_iter()
        .map(|r| r.name)
        .collect();
    h.get_by_label(&names[1])
        .click_modifiers(egui::Modifiers::COMMAND);
    h.run_steps(2);
    assert_eq!(h.state().s.selection.items.len(), 2);
    // Rename through the inline field (F2 on the selection).
    select(&mut h, &ids[0]);
    h.key_press(egui::Key::F2);
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    let id0 = kdraft_doc::Id(ids[0].clone());
    // The rename field has focus: typing replaces the selected name.
    h.event(egui::Event::Text("Board top".into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    let name = |app: &App| app.s.find_item(&id0).unwrap().1.name().map(str::to_string);
    assert_eq!(name(h.state()).as_deref(), Some("Board top"));
    // Hide through the eye toggle (visible on hover).
    let r = rect(h.state(), 0);
    h.hover_at(r.center());
    h.run_steps(2);
    h.get_by_label("Hide Board top").click();
    h.run_steps(2);
    assert!(h.state().s.find_item(&id0).unwrap().1.hidden());
    // Drag the top row to the bottom: z-order changes in one step.
    let rows = h.state().tree_rects.clone();
    let from = rect(h.state(), 0).center();
    let last = rows.len() - 1;
    let to = rect(h.state(), last).center() + egui::vec2(0.0, 10.0);
    h.hover_at(from);
    h.step();
    h.drag_at(from);
    h.step();
    for k in 1..=8 {
        h.hover_at(from + (to - from) * (k as f32 / 8.0));
        h.step();
    }
    h.drop_at(to);
    h.run_steps(3);
    assert_eq!(
        h.state().s.doc().pages[0].items[0].id().unwrap().0,
        ids[0],
        "moved to the bottom"
    );
    assert_eq!(h.state().s.undo_title(), Some("Change stacking order"));
}

#[test]
fn layer_preset_and_number_fields() {
    let mut h = harness();
    h.run_steps(3);
    let vid = view_id(h.state());
    select(&mut h, &vid);
    h.run_steps(2);
    h.get_by_label("Copper top").click();
    h.run_steps(2);
    let layers = |app: &App| match &app.s.doc().pages[0].items[0] {
        Item::View(v) => v.layers.clone(),
        _ => vec![],
    };
    assert_eq!(
        layers(h.state()),
        vec!["F.Cu".to_string(), "Edge.Cuts".to_string()]
    );
    // Typed entry with an expression.
    h.get_by_label("x").click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("100+5".into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    let at = |app: &App| app.s.doc().pages[0].views().next().unwrap().at;
    assert_eq!(at(h.state()).x, 105.0);
    // Scrubbing the lead letter: one undo step.
    let r = h.get_by_label("x").rect();
    let lead = egui::pos2(r.min.x - 10.0, r.center().y);
    h.hover_at(lead);
    h.step();
    h.drag_at(lead);
    h.step();
    for k in 1..=5 {
        h.hover_at(lead + egui::vec2(4.0 * k as f32, 0.0));
        h.step();
    }
    h.drop_at(lead + egui::vec2(20.0, 0.0));
    h.run_steps(2);
    let x = at(h.state()).x;
    assert!((x - 107.0).abs() < 1e-6, "scrubbed to {x}");
    assert_eq!(h.state().s.undo_title(), Some("Set position"));
    h.state_mut().s.undo();
    assert_eq!(at(h.state()).x, 105.0);
}

#[test]
fn tools_place_a_view_and_a_linear_dimension() {
    let mut h = harness();
    step_until(&mut h, |a| {
        a.textures.keys().any(|k| k.starts_with("item:"))
    });
    // A view by click.
    let n = h.state().s.doc().pages[0].items.len();
    h.state_mut().run_id("tool.view_assembly");
    h.run_steps(2);
    let p = screen(h.state(), kdraft_board::geom::Point::new(330.0, 220.0));
    click(&mut h, p);
    assert_eq!(h.state().s.doc().pages[0].items.len(), n + 1);
    assert_eq!(h.state().s.undo_title(), Some("Add view"));
    assert!(h.state().tool.kind.is_select(), "back to select");
    h.state_mut().s.undo();
    h.run_steps(2);
    // A horizontal dimension between two outline vertices.
    let app = h.state();
    let b = app.s.board().unwrap().clone();
    let v = app.s.doc().pages[0].views().next().unwrap().clone();
    let map = kdraft_render::view::ViewMap::new(&v, kdraft_engine::bounds::board_box(Some(&b)));
    let verts = b.outline.outer().unwrap().vertices.clone();
    let (a, c) = (map.point(verts[0]), map.point(verts[1]));
    h.key_press(egui::Key::D);
    h.run_steps(2);
    assert!(!h.state().tool.kind.is_select());
    let n = h.state().s.doc().pages[0].items.len();
    for q in [a, c] {
        let sp = screen(h.state(), q);
        h.hover_at(sp);
        h.run_steps(2);
        assert!(h.state().snapped.is_some(), "anchor snapped at {q:?}");
        click(&mut h, sp);
    }
    let off = screen(
        h.state(),
        kdraft_board::geom::Point::new((a.x + c.x) / 2.0, a.y.min(c.y) - 12.0),
    );
    h.hover_at(off);
    h.run_steps(2);
    click(&mut h, off);
    let items = &h.state().s.doc().pages[0].items;
    assert_eq!(items.len(), n + 1);
    assert!(matches!(items.last(), Some(Item::Dimension(_))));
    assert_eq!(h.state().s.undo_title(), Some("Add linear dimension"));
    // Escape leaves a tool.
    h.key_press(egui::Key::D);
    h.run_steps(1);
    h.key_press(egui::Key::Escape);
    h.run_steps(1);
    assert!(h.state().tool.kind.is_select());
}

#[test]
fn toasts_appear_and_expire() {
    let mut h = harness();
    h.run_steps(2);
    h.state_mut().toasts.clear();
    h.state_mut().warn("something to see");
    h.run_steps(1);
    assert_eq!(h.state().toasts.len(), 1);
    let born = Instant::now() - Duration::from_secs(60);
    h.state_mut().toasts[0].born = born;
    h.run_steps(2);
    assert!(h.state().toasts.is_empty());
    // The log keeps it.
    assert!(h
        .state()
        .messages
        .iter()
        .any(|m| m.text == "something to see"));
}

#[test]
fn issues_panel_click_selects_the_item() {
    let mut h = harness();
    h.run_steps(2);
    let vid = view_id(h.state());
    // Push the view far outside the drawing area.
    h.state_mut().cmd(
        "item.move",
        serde_json::json!({ "ids": [vid], "delta": [300.0, 0.0] }),
    );
    h.state_mut().cmd("select.clear", serde_json::json!({}));
    step_until(&mut h, |a| {
        a.issues.as_ref().is_some_and(|(r, _)| *r == a.s.revision())
    });
    h.state_mut().run_named("dock_issues");
    h.run_steps(2);
    let label = h
        .state()
        .issues
        .as_ref()
        .unwrap()
        .1
        .findings
        .iter()
        .find(|f| f.item.as_deref() == Some(vid.as_str()) && f.code == "outside-area")
        .map(|f| format!("{} {}", f.code, f.message))
        .expect("outside-area finding");
    h.get_by_label(&label).click();
    h.run_steps(2);
    assert_eq!(h.state().s.selection.items.len(), 1);
    assert_eq!(h.state().s.selection.items[0].0, vid);
}

#[test]
fn theme_toggle_persists() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("kdraft").join("settings.json");
    let ctx = egui::Context::default();
    let app = App::new(&ctx, session(), false).with_settings(path.clone());
    let mut h = harness_with(app);
    h.run_steps(2);
    assert_eq!(h.state().applied_dark, Some(false));
    h.get_by_label(kdraft_ui::strings::THEME_DARK).click();
    h.run_steps(2);
    assert_eq!(h.state().applied_dark, Some(true));
    let t = Instant::now();
    while !path.exists() && t.elapsed() < Duration::from_secs(10) {
        h.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    let s = kdraft_ui::settings::Settings::load(&path);
    assert_eq!(s.theme, kdraft_ui::theme::ThemePref::Dark);
}
