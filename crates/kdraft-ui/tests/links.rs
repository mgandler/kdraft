//! Headless tests of placement links in the editor: linking through the
//! context menu and a canvas pick, dragging a follower on another page,
//! unlinking, and the layer pages window.

use std::path::PathBuf;

use eframe::egui;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use kdraft_board::geom::Point;
use kdraft_doc::{Id, Item};
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
    let app = App::new(&ctx, s, false);
    Harness::builder()
        .with_size(egui::Vec2::new(1600.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(kdraft_ui::app::install_fonts)
        .build_ui_state(|ui, app: &mut App| app.show(ui), app)
}

fn screen(app: &App, p: Point) -> egui::Pos2 {
    let s = app.camera.to_screen(p);
    let o = app.canvas_origin;
    egui::pos2((o.x + s.x) as f32, (o.y + s.y) as f32)
}

fn pos(app: &App, id: &str) -> Point {
    app.s
        .find_item(&Id(id.into()))
        .and_then(|(_, i)| i.position())
        .unwrap()
}

fn click(h: &mut Harness<'_, App>, p: egui::Pos2) {
    h.hover_at(p);
    h.step();
    h.drag_at(p);
    h.step();
    h.drop_at(p);
    h.run_steps(2);
}

#[test]
fn link_by_menu_and_canvas_pick_then_drag_follower_and_unlink() {
    let mut h = harness();
    h.run_steps(3);
    let app = h.state_mut();
    let page1 = app.s.doc().pages[0].id.0.clone();
    let r = app.s.doc().pages[0].views().next().unwrap().id.0.clone();
    // Page 3 with a rectangle and a text.
    app.cmd("page.add", json!({}));
    app.cmd("page.add", json!({}));
    assert!(app.cmd(
        "item.add_rect",
        json!({ "start": [30, 30], "end": [60, 50] })
    ));
    let rect = app.s.selection.items[0].0.clone();
    assert!(app.cmd("item.add_text", json!({ "text": "T", "at": [150, 150] })));
    let text = app.s.selection.items[0].0.clone();
    h.run_steps(3);

    // Context menu > Link placement to > page 1 > the view.
    h.state_mut().cmd("select.set", json!({ "ids": [rect] }));
    h.run_steps(2);
    let p = screen(h.state(), Point::new(45.0, 40.0));
    h.hover_at(p);
    h.step();
    h.event(egui::Event::PointerButton {
        pos: p,
        button: egui::PointerButton::Secondary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.event(egui::Event::PointerButton {
        pos: p,
        button: egui::PointerButton::Secondary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(3);
    h.get_by_label("Link placement to").hover();
    h.run_steps(3);
    h.get_by_label_contains("1. ").hover();
    h.run_steps(3);
    h.get_by_label("Fabrication view").click();
    h.run_steps(3);
    let l = h
        .state()
        .s
        .find_item(&Id(rect.clone()))
        .and_then(|(_, i)| i.link())
        .expect("linked through the menu");
    assert_eq!(l.to.0, r);

    // Canvas pick: the text picks the rectangle (on this page), which
    // links it to the rectangle's reference.
    h.state_mut().cmd("select.set", json!({ "ids": [text] }));
    h.state_mut().start_link_pick();
    h.run_steps(2);
    let at = screen(h.state(), Point::new(45.0, 40.0));
    click(&mut h, at);
    let l = h
        .state()
        .s
        .find_item(&Id(text.clone()))
        .and_then(|(_, i)| i.link())
        .expect("linked through the pick");
    assert_eq!(l.to.0, r, "linked to the root reference");
    assert!(h.state().link_pick.is_none());

    // Drag the follower on page 3: the view on page 1 moves.
    let v0 = pos(h.state(), &r);
    let from = screen(h.state(), Point::new(45.0, 40.0));
    let to = screen(h.state(), Point::new(55.0, 45.0));
    h.state_mut().cmd("select.set", json!({ "ids": [rect] }));
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
    let v1 = pos(h.state(), &r);
    assert!((v1.x - v0.x - 10.0).abs() < 1.0, "{v0:?} -> {v1:?}");
    assert!((v1.y - v0.y - 5.0).abs() < 1.0, "{v0:?} -> {v1:?}");
    assert_eq!(h.state().s.undo_title(), Some("Move"));

    // Inspector: Unlink.
    h.run_steps(2);
    h.get_by_label("Unlink").click();
    h.run_steps(2);
    assert!(h
        .state()
        .s
        .find_item(&Id(rect.clone()))
        .and_then(|(_, i)| i.link())
        .is_none());
    let _ = page1;
}

#[test]
fn layer_pages_window_creates_one_page_per_layer() {
    let mut h = harness();
    h.run_steps(3);
    let v = h.state().s.doc().pages[0]
        .views()
        .next()
        .unwrap()
        .id
        .0
        .clone();
    h.state_mut().cmd("select.set", json!({ "ids": [v] }));
    h.state_mut().open_layer_pages();
    h.run_steps(3);
    let n0 = h.state().s.doc().pages.len();
    h.get_by_label("Create 2 page(s)").click();
    h.run_steps(3);
    let d = h.state().s.doc();
    assert_eq!(d.pages.len(), n0 + 2);
    let views: Vec<_> = d.pages[1..3]
        .iter()
        .map(|p| p.views().next().unwrap().clone())
        .collect();
    assert_eq!(views[0].layers[0], "F.Cu");
    assert_eq!(views[1].layers[0], "B.Cu");
    for w in &views {
        assert_eq!(w.at, pos(h.state(), &v));
        let Some((_, Item::View(src))) = h.state().s.find_item(&Id(v.clone())) else {
            panic!()
        };
        assert_eq!(w.scale, src.scale);
    }
}

#[test]
fn layer_pages_window_from_an_empty_page() {
    let mut h = harness();
    h.run_steps(3);
    h.state_mut().cmd("page.add", json!({}));
    h.run_steps(2);
    let n0 = h.state().s.doc().pages.len();
    let cur = h.state().s.current_page_index().unwrap();
    let empty = h.state().s.doc().pages[cur].id.clone();
    // Through the menu entry: enabled without a selection or a view here.
    let e = h.state().entry("app.layer_pages").unwrap();
    assert!(h.state().enabled(&e));
    h.state_mut().run_entry(&e);
    h.run_steps(3);
    let d = h.state().layer_pages.clone().unwrap();
    assert_eq!(d.view, None);
    assert!(d.use_current);
    // The reference list offers a new placement and the existing views.
    h.get_by_label("New placement (centred, scale fitted to the sheet)");
    let summary = format!(
        "Page {} becomes the F.Cu page, 1 more page(s) after it (B.Cu); new placement",
        cur + 1
    );
    assert!(
        h.query_all_by(|n| {
            let t = n.label().or_else(|| n.value()).unwrap_or_default();
            t.starts_with(&summary)
        })
        .next()
        .is_some(),
        "no summary line: {:?}",
        h.query_all_by(|n| n.label().is_some_and(|l| l.contains("page")))
            .map(|n| format!("{n:?}"))
            .collect::<Vec<_>>()
    );
    h.get_by_label("Create 2 page(s)").click();
    h.run_steps(3);
    let s = &h.state().s;
    assert_eq!(s.doc().pages.len(), n0 + 1);
    let p = &s.doc().pages[cur];
    assert_eq!(p.id, empty);
    assert_eq!(p.title.as_deref(), Some("F.Cu"));
    let first = p.views().next().unwrap().id.clone();
    let second = s.doc().pages[cur + 1].views().next().unwrap();
    assert_eq!(second.layers[0], "B.Cu");
    let it = Item::View(second.clone());
    assert_eq!(it.link().unwrap().to, first);
    assert!(h.state().messages.iter().all(|m| !matches!(
        m.level,
        kdraft_ui::app::Level::Warning | kdraft_ui::app::Level::Error
    )));
}

/// "Link placement to..." with nothing selected asks for the item first,
/// then for the reference.
#[test]
fn link_pick_without_selection_asks_for_the_item() {
    let mut h = harness();
    h.run_steps(3);
    let d = h.state().s.doc();
    let a = d.pages[2].views().next().unwrap().id.clone();
    let b = d.pages[3].views().next().unwrap().id.clone();
    let p2 = d.pages[2].id.0.clone();
    h.state_mut()
        .cmd("select.set", json!({ "page": p2, "ids": [] }));
    let e = h.state().entry("app.link_pick").unwrap();
    assert!(h.state().enabled(&e));
    h.state_mut().run_entry(&e);
    assert_eq!(h.state().link_pick, Some(Vec::new()));
    h.state_mut().finish_link_pick(&a);
    assert_eq!(h.state().link_pick, Some(vec![a.clone()]));
    assert_eq!(h.state().s.selection.items, vec![a.clone()]);
    h.state_mut().finish_link_pick(&b);
    assert_eq!(h.state().link_pick, None);
    let it = h.state().s.find_item(&a).unwrap().1.clone();
    assert_eq!(it.link().unwrap().to, b);
}

/// A dimension tool on a page without a board view places a fabrication
/// view first and then continues with the dimension tool; drawing tools
/// work without a board.
#[test]
fn anchor_tool_on_a_page_without_view_places_a_view_first() {
    use kdraft_ui::tools::{LinearKind, Placement, ToolKind, ViewTool};
    let mut h = harness();
    h.run_steps(3);
    h.state_mut().cmd("page.add", json!({}));
    h.run_steps(2);
    let e = h.state().entry("tool.dim_horizontal").unwrap();
    assert!(h.state().enabled(&e));
    h.state_mut().run_entry(&e);
    assert_eq!(h.state().tool.kind, ToolKind::View(ViewTool::Fabrication));
    h.state_mut().place(Placement::View {
        kind: ViewTool::Fabrication,
        at: Point::new(200.0, 140.0),
    });
    assert_eq!(
        h.state().tool.kind,
        ToolKind::DimLinear(LinearKind::Horizontal)
    );
    let pi = h.state().s.current_page_index().unwrap();
    assert_eq!(h.state().s.doc().pages[pi].views().count(), 1);

    let ctx = egui::Context::default();
    let mut app = App::new(&ctx, Session::empty().unwrap(), false);
    if app.s.current_page_index().is_none() {
        app.cmd("page.add", json!({}));
    }
    assert!(app.s.board().is_none());
    for id in ["tool.text", "tool.line", "tool.rect", "tool.circle"] {
        let e = app.entry(id).unwrap();
        assert!(app.enabled(&e), "{id}");
    }
    let e = app.entry("tool.dim_horizontal").unwrap();
    assert!(!app.enabled(&e));
}
