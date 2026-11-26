//! Headless tests of the style section, the colour picker, copy/paste
//! style and in-place multi-line text editing.

use std::path::PathBuf;

use eframe::egui;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use kdraft_doc::style::Rgba;
use kdraft_doc::{GraphicShape, Item};
use kdraft_engine::Session;
use kdraft_ui::App;
use serde_json::json;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/kicad9-minimal.kicad_pcb")
}

fn harness() -> Harness<'static, App> {
    let ctx = egui::Context::default();
    let app = App::new(
        &ctx,
        Session::new_for_board(&fixture(), None).unwrap(),
        false,
    );
    Harness::builder()
        .with_size(egui::Vec2::new(1600.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(kdraft_ui::app::install_fonts)
        .build_ui_state(|ui, app: &mut App| app.show(ui), app)
}

fn item(app: &App, id: &str) -> Item {
    app.s
        .find_item(&kdraft_doc::Id(id.into()))
        .map(|(_, i)| i.clone())
        .unwrap()
}

fn screen(app: &App, p: kdraft_board::geom::Point) -> egui::Pos2 {
    let s = app.camera.to_screen(p);
    let o = app.canvas_origin;
    egui::pos2((o.x + s.x) as f32, (o.y + s.y) as f32)
}

#[test]
fn palette_swatch_sets_the_line_colour_and_paste_copies_it() {
    let mut h = harness();
    h.run_steps(3);
    assert!(h.state_mut().cmd(
        "item.add_line",
        json!({ "start": [20, 20], "end": [60, 20] })
    ));
    let line = h.state().s.selection.items[0].0.clone();
    assert!(h.state_mut().cmd(
        "item.add_rect",
        json!({ "start": [20, 30], "end": [60, 50] })
    ));
    let rect = h.state().s.selection.items[0].0.clone();
    h.state_mut().cmd("select.set", json!({ "ids": [line] }));
    h.run_steps(3);
    h.get_by_label("Line colour colour").click();
    h.run_steps(3);
    h.get_by_label("#C81E1E").click();
    h.run_steps(3);
    let st = item(h.state(), &line).style_override();
    assert_eq!(st.stroke, Some(Rgba::rgb(0xC8, 0x1E, 0x1E)));
    // One undo step.
    h.state_mut().s.undo();
    assert!(item(h.state(), &line).style_override().is_empty());
    h.state_mut().s.redo();
    assert_eq!(h.state().settings.recent_colors[0], "#C81E1E");
    // Copy style, select the rectangle, paste with the menu shortcut.
    h.get_by_label("Copy").click();
    h.run_steps(2);
    h.state_mut().cmd("select.set", json!({ "ids": [rect] }));
    h.run_steps(2);
    h.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::ALT,
        egui::Key::V,
    );
    h.run_steps(3);
    assert_eq!(
        item(h.state(), &rect).style_override().stroke,
        Some(Rgba::rgb(0xC8, 0x1E, 0x1E))
    );
    // Reset to the document style.
    h.get_by_label("Reset to document style").click();
    h.run_steps(2);
    assert!(item(h.state(), &rect).style_override().is_empty());
}

#[test]
fn double_click_edits_text_in_place_with_new_lines() {
    let mut h = harness();
    h.run_steps(3);
    assert!(h.state_mut().cmd(
        "item.add_text",
        json!({ "text": "Hello", "at": [100, 100], "height": 5 })
    ));
    let id = h.state().s.selection.items[0].0.clone();
    h.run_steps(3);
    let p = screen(h.state(), kdraft_board::geom::Point::new(103.0, 98.0));
    h.hover_at(p);
    h.step();
    for pressed in [true, false, true, false] {
        h.event(egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
    }
    h.run_steps(4);
    assert!(
        h.state().text_edit.is_some(),
        "editor open; p={p:?} cursor={:?} canvas={:?} zoom={}",
        h.state().cursor_mm,
        h.state().canvas_origin,
        h.state().camera.zoom
    );
    let undo_before = h.state().s.can_undo();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::End);
    h.key_press(egui::Key::Enter);
    h.event(egui::Event::Text("Wörld µ ±0.1".into()));
    h.run_steps(2);
    // Escape cancels.
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().text_edit.is_none());
    let text = |h: &Harness<'_, App>| match item(h.state(), &id) {
        Item::Graphic(g) => match g.shape {
            GraphicShape::Text { text, .. } => text,
            _ => unreachable!(),
        },
        _ => unreachable!(),
    };
    assert_eq!(text(&h), "Hello");
    // Enter on the selected text opens it again; Ctrl+Enter commits.
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.state().text_edit.is_some());
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::End);
    h.key_press(egui::Key::Enter);
    h.event(egui::Event::Text("Wörld µ ±0.1".into()));
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Enter);
    h.run_steps(3);
    assert!(h.state().text_edit.is_none());
    assert_eq!(text(&h), "Hello\nWörld µ ±0.1");
    assert!(undo_before);
    h.state_mut().s.undo();
    assert_eq!(text(&h), "Hello");
}
