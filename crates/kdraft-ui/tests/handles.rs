//! Headless tests of edit handles: line endpoints, rectangle corners,
//! polyline vertices (drag, insert, delete), text rotation, dimension
//! re-anchoring; each drag one undo step.

use std::path::PathBuf;

use eframe::egui;
use egui_kittest::Harness;
use kdraft_board::geom::Point;
use kdraft_doc::{GraphicShape, Id, Item};
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

fn drag(h: &mut Harness<'_, App>, a: Point, b: Point) {
    let (from, to) = (screen(h.state(), a), screen(h.state(), b));
    h.hover_at(from);
    h.step();
    h.drag_at(from);
    h.step();
    for k in 1..=6 {
        h.hover_at(from + (to - from) * (k as f32 / 6.0));
        h.step();
    }
    h.drop_at(to);
    h.run_steps(2);
}

fn shape(app: &App, id: &str) -> GraphicShape {
    match app.s.find_item(&Id(id.into())).map(|(_, i)| i.clone()) {
        Some(Item::Graphic(g)) => g.shape,
        o => panic!("{o:?}"),
    }
}

fn add(h: &mut Harness<'_, App>, cmd: &str, args: serde_json::Value) -> String {
    assert!(h.state_mut().cmd(cmd, args));
    h.run_steps(2);
    h.state().s.selection.items[0].0.clone()
}

#[test]
fn line_end_rect_corner_and_polyline_vertices() {
    let mut h = harness();
    h.run_steps(3);
    // Grid 0.5 mm, snap on (anchors are on the view, far away).
    let line = add(
        &mut h,
        "item.add_line",
        json!({ "start": [20, 160], "end": [60, 160] }),
    );
    drag(&mut h, Point::new(60.0, 160.0), Point::new(70.2, 171.1));
    match shape(h.state(), &line) {
        GraphicShape::Line { start, end } => {
            assert_eq!(start, Point::new(20.0, 160.0));
            assert_eq!(end, Point::new(70.0, 171.0), "grid snapped");
        }
        o => panic!("{o:?}"),
    }
    assert_eq!(h.state().s.undo_title(), Some("Point 2"));
    h.state_mut().s.undo();
    assert!(
        matches!(shape(h.state(), &line), GraphicShape::Line { end, .. } if end == Point::new(60.0, 160.0))
    );

    let rect = add(
        &mut h,
        "item.add_rect",
        json!({ "start": [100, 160], "end": [120, 170] }),
    );
    drag(&mut h, Point::new(120.0, 170.0), Point::new(130.0, 180.0));
    let b = shape(h.state(), &rect).bbox().unwrap();
    assert_eq!(
        (b.min, b.max),
        (Point::new(100.0, 160.0), Point::new(130.0, 180.0))
    );
    // Edge handle: the left edge.
    drag(&mut h, Point::new(100.0, 170.0), Point::new(90.0, 150.0));
    let b = shape(h.state(), &rect).bbox().unwrap();
    assert_eq!(
        (b.min, b.max),
        (Point::new(90.0, 160.0), Point::new(130.0, 180.0))
    );

    let poly = add(
        &mut h,
        "item.add_polyline",
        json!({ "points": [[150, 160], [170, 160], [170, 180]] }),
    );
    drag(&mut h, Point::new(170.0, 180.0), Point::new(180.0, 185.0));
    assert_eq!(
        shape(h.state(), &poly).points()[2],
        Point::new(180.0, 185.0)
    );
    // Double-click on the first segment inserts a vertex there.
    let p = screen(h.state(), Point::new(160.0, 160.0));
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
    h.run_steps(2);
    assert_eq!(shape(h.state(), &poly).points().len(), 4);
    assert_eq!(h.state().sel_vertex.as_ref().map(|v| v.1), Some(1));
    // Delete removes the selected vertex, not the polyline.
    h.key_press(egui::Key::Delete);
    h.run_steps(2);
    assert_eq!(shape(h.state(), &poly).points().len(), 3);
}

#[test]
fn text_rotation_handle_and_dimension_reanchor() {
    let mut h = harness();
    h.run_steps(3);
    let t = add(
        &mut h,
        "item.add_text",
        json!({ "text": "ROTATE ME", "at": [60, 180], "height": 4 }),
    );
    h.run_steps(2);
    let hs = h.state().edit_handles();
    let rot = hs
        .iter()
        .find(|x| x.kind == kdraft_ui::handles::HandleKind::Rotate)
        .unwrap()
        .at;
    // Drag the handle a quarter turn around the anchor (CCW on the sheet).
    let at = Point::new(60.0, 180.0);
    let v = Point::new(rot.x - at.x, rot.y - at.y);
    let target = Point::new(at.x + v.y, at.y - v.x);
    drag(&mut h, rot, target);
    match shape(h.state(), &t) {
        GraphicShape::Text { rotation, .. } => assert_eq!(rotation, 90.0),
        o => panic!("{o:?}"),
    }
    assert_eq!(h.state().s.undo_title(), Some("Rotation"));

    // A board-size dimension: drag its first anchor onto another anchor.
    let dim = h.state().s.doc().pages[0]
        .items
        .iter()
        .find_map(|i| match i {
            Item::Dimension(d) if d.to.is_some() => Some(d.clone()),
            _ => None,
        })
        .expect("doc has a linear dimension");
    h.state_mut()
        .cmd("select.set", json!({ "ids": [dim.id.0] }));
    h.run_steps(2);
    let hs = h.state().edit_handles();
    let a0 = hs
        .iter()
        .find(|x| x.kind == kdraft_ui::handles::HandleKind::DimAnchor(0))
        .unwrap()
        .at;
    // A hole anchor of the same view.
    let board = h.state().s.board_arc().unwrap();
    let view = h.state().s.doc().pages[0]
        .views()
        .find(|v| Some(&v.id) == dim.view.as_ref())
        .unwrap()
        .clone();
    let map = kdraft_render::view::ViewMap::new(&view, h.state().s.board_box());
    let hole = map.point(board.holes()[0].position);
    drag(&mut h, a0, hole);
    let d2 = match h.state().s.find_item(&dim.id).map(|(_, i)| i.clone()) {
        Some(Item::Dimension(d)) => d,
        o => panic!("{o:?}"),
    };
    assert_eq!(d2.from, Some(kdraft_doc::Anchor::Hole(0)), "re-anchored");
    assert_eq!(d2.to, dim.to);
    assert_eq!(h.state().s.undo_title(), Some("Dimension start anchor"));
}
