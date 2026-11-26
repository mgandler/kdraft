//! Headless tests of numbered callouts in the editor: placing with the
//! tool numbers 1, 2, 3; editing a label in the inspector continues the
//! sequence; in-place editing; the plus key; renumbering by position;
//! switching the content to a note and back.

use std::path::PathBuf;

use eframe::egui;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use kdraft_board::geom::Point;
use kdraft_doc::{CalloutKind, Id, Item};
use kdraft_engine::Session;
use kdraft_ui::App;
use serde_json::json;

fn harness() -> Harness<'static, App> {
    let ctx = egui::Context::default();
    let s = Session::new_for_board(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/kicad9-minimal.kicad_pcb"),
        None,
    )
    .unwrap();
    Harness::builder()
        .with_size(egui::Vec2::new(1600.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(kdraft_ui::app::install_fonts)
        .build_ui_state(|ui, app: &mut App| app.show(ui), App::new(&ctx, s, false))
}

fn screen(app: &App, p: Point) -> egui::Pos2 {
    let s = app.camera.to_screen(p);
    let o = app.canvas_origin;
    egui::pos2((o.x + s.x) as f32, (o.y + s.y) as f32)
}

fn click(h: &mut Harness<'_, App>, p: Point) {
    let pos = screen(h.state(), p);
    h.hover_at(pos);
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
    }
    h.run_steps(2);
}

/// Sheet position of the first footprint's origin in the page's view.
fn anchor_point(app: &App) -> Point {
    let s = &app.s;
    let b = s.board().unwrap();
    let r = b.footprints[0].reference.clone().unwrap();
    let a = kdraft_doc::Anchor::FpOrigin(r);
    let p = kdraft_doc::anchor::resolve(&a, b, &b.holes()).unwrap();
    let v = s.current_page().unwrap().views().next().unwrap();
    kdraft_render::view::ViewMap::new(v, s.board_box()).point(p)
}

/// Label callouts of the current page in placement order.
fn callouts(app: &App) -> Vec<(Id, String)> {
    app.s
        .current_page()
        .unwrap()
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Callout(c) => Some((c.id.clone(), app.callout_shown(c))),
            _ => None,
        })
        .collect()
}

fn place(h: &mut Harness<'_, App>, at: Point) {
    let a = anchor_point(h.state());
    click(h, a);
    click(h, at);
}

#[test]
fn numbered_callouts_place_edit_bump_renumber() {
    let mut h = harness();
    h.run_steps(4);
    h.state_mut().run_id("tool.callout");
    h.run_steps(2);
    let spots = [
        Point::new(370.0, 120.0),
        Point::new(350.0, 60.0),
        Point::new(390.0, 60.0),
    ];
    for p in spots {
        place(&mut h, p);
    }
    let got: Vec<String> = callouts(h.state()).into_iter().map(|x| x.1).collect();
    assert_eq!(got, ["1", "2", "3"]);
    // The tool stays active; Escape twice back to Select.
    h.key_press(egui::Key::Escape);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    // Edit the second to 7 in the inspector.
    let second = callouts(h.state())[1].0.clone();
    h.state_mut()
        .cmd("select.set", json!({ "ids": [second.0] }));
    h.run_steps(3);
    h.get_by_label("Callout label").click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("7".into()));
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(callouts(h.state())[1].1, "7");
    // The next one placed gets 8.
    h.state_mut().cmd("select.set", json!({ "ids": [] }));
    h.state_mut().run_id("tool.callout");
    h.run_steps(2);
    place(&mut h, Point::new(390.0, 120.0));
    assert_eq!(callouts(h.state())[3].1, "8");
    h.key_press(egui::Key::Escape);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    // In-place edit: Enter on the selected balloon, type, Enter.
    // Click the balloon (also takes the focus from the inspector).
    click(&mut h, Point::new(390.0, 120.0));
    let fourth = callouts(h.state())[3].0.clone();
    assert_eq!(h.state().s.selection.items, vec![fourth]);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(
        h.state().label_edit.is_some(),
        "typing {} sel {}",
        h.ctx.egui_wants_keyboard_input(),
        h.state().selected_callouts().len()
    );
    h.event(egui::Event::Text("12".into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.state().label_edit.is_none());
    assert_eq!(callouts(h.state())[3].1, "12");
    // Plus key bumps, minus key goes back.
    h.key_press(egui::Key::Plus);
    h.run_steps(2);
    assert_eq!(callouts(h.state())[3].1, "13");
    h.key_press(egui::Key::Minus);
    h.run_steps(2);
    assert_eq!(callouts(h.state())[3].1, "12");
    // Renumber all by position (rows): y 60 row (x 350, 390), then
    // y 120 row (x 370, 390).
    let all: Vec<String> = callouts(h.state()).iter().map(|c| c.0 .0.clone()).collect();
    h.state_mut().cmd("select.set", json!({ "ids": all }));
    h.run_steps(2);
    h.state_mut().run_id("app.renumber_callouts");
    h.run_steps(3);
    h.get_by_label("Renumber").click();
    h.run_steps(3);
    let got: Vec<String> = callouts(h.state()).into_iter().map(|x| x.1).collect();
    assert_eq!(got, ["3", "1", "2", "4"]);
    // One undo step.
    h.state_mut().s.undo();
    let got: Vec<String> = callouts(h.state()).into_iter().map(|x| x.1).collect();
    assert_eq!(got, ["1", "7", "3", "12"]);
}

#[test]
fn switch_content_to_note_and_back() {
    let mut h = harness();
    h.run_steps(4);
    h.state_mut().run_id("tool.callout");
    h.run_steps(2);
    place(&mut h, Point::new(370.0, 60.0));
    h.key_press(egui::Key::Escape);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    let id = callouts(h.state())[0].0.clone();
    h.state_mut().cmd("select.set", json!({ "ids": [id.0] }));
    h.run_steps(3);
    assert_eq!(h.state().s.selection.items, vec![id.clone()]);
    h.get_by_label("Note").click();
    h.run_steps(3);
    let kind = |h: &Harness<'_, App>| match h.state().s.find_item(&id).unwrap().1 {
        Item::Callout(c) => c.kind(),
        _ => unreachable!(),
    };
    assert_eq!(kind(&h), CalloutKind::Note);
    let shown_note = callouts(h.state())[0].1.clone();
    h.get_by_label("Number").click();
    h.run_steps(3);
    assert_eq!(kind(&h), CalloutKind::Label);
    // Switching back keeps the number that was shown.
    assert_eq!(callouts(h.state())[0].1, shown_note);
    // The tool's variants.
    for id in ["tool.callout", "tool.callout_bom", "tool.callout_note"] {
        assert!(h.state().entry(id).is_some(), "{id}");
    }
}
