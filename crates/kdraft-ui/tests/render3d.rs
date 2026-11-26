//! Headless test of 3D views in the editor: two views on one page both
//! go through both render passes (one background slot per picture) and
//! end up trimmed; a fresh 3D view raises no issue.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;
use egui_kittest::Harness;
use kdraft_engine::Session;
use kdraft_render::render3d::State;
use kdraft_ui::App;
use serde_json::json;

#[test]
fn two_views_render_both_passes() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    kdraft_ui::jobs::set_runner(std::sync::Arc::new(kdraft_source::fixture::fixture_runner(
        fixtures.join("kicad-cli-svg"),
        Box::new(kdraft_render::render3d::fake_board_render),
    )));
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("b.kicad_pcb");
    std::fs::copy(fixtures.join("kicad9-minimal.kicad_pcb"), &board).unwrap();
    let ctx = egui::Context::default();
    let s = Session::new_for_board(&board, None).unwrap();
    let mut h = Harness::builder()
        .with_size(egui::Vec2::new(1400.0, 900.0))
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(kdraft_ui::app::install_fonts)
        .build_ui_state(|ui, app: &mut App| app.show(ui), App::new(&ctx, s, false));
    h.run_steps(3);
    h.state_mut().cmd("page.add", json!({}));
    let page = h.state().s.doc().pages.last().unwrap().id.0.clone();
    h.state_mut().cmd(
        "item.add_view",
        json!({ "page": page, "kind": "render3d", "at": [150, 140], "scale": 2 }),
    );
    h.state_mut().cmd(
        "item.add_view",
        json!({ "page": page, "kind": "render3d", "at": [300, 140], "scale": 2 }),
    );
    let iso = h.state().selected_items()[0].id().unwrap().0.clone();
    h.state_mut().cmd(
        "view.set_render3d",
        json!({ "ids": [iso], "preset": "isometric" }),
    );
    let t = Instant::now();
    let ready = |app: &App| {
        app.s.renders.len() >= 2
            && app
                .s
                .renders
                .values()
                .all(|st| matches!(st, State::Ready(_) | State::Failed(_)))
    };
    while !ready(h.state()) && t.elapsed() < Duration::from_secs(60) {
        h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    let renders = &h.state().s.renders;
    assert_eq!(renders.len(), 2, "{:?}", renders.keys().collect::<Vec<_>>());
    let pics: Vec<_> = renders
        .values()
        .map(|st| match st {
            State::Ready(p) => p.clone(),
            other => panic!("{other:?}"),
        })
        .collect();
    // One to scale (outline found), one not.
    assert_eq!(pics.iter().filter(|p| p.outline_px.is_some()).count(), 1);
    for p in &pics {
        assert!(p.note.contains("zoom"), "{}", p.note);
    }
    let bad: Vec<_> = h
        .state()
        .messages
        .iter()
        .filter(|m| !matches!(m.level, kdraft_ui::app::Level::Info))
        .map(|m| m.text.clone())
        .collect();
    assert!(bad.is_empty(), "{bad:?}");
    // The check knows kicad-cli is there: no "needs kicad-cli" finding.
    let mut s = h.state().s.fork();
    let r = kdraft_engine::check::check(&mut s);
    assert!(!r.text().contains("needs kicad-cli"), "{}", r.text());
}
