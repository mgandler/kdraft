//! The side panels keep their width and can be resized by dragging their
//! inner edge (they once collapsed to the minimum width every frame,
//! because their contents live in child uis that did not grow the panel).

use std::path::PathBuf;

use eframe::egui;
use egui_kittest::Harness;
use kdraft_engine::Session;
use kdraft_ui::App;

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

fn drag(h: &mut Harness<'_, App>, from: egui::Pos2, to: egui::Pos2) {
    h.hover_at(from);
    h.step();
    h.drag_at(from);
    h.step();
    for k in 1..=6 {
        h.hover_at(from + (to - from) * (k as f32 / 6.0));
        h.step();
    }
    h.drop_at(to);
    for _ in 0..5 {
        h.step();
    }
}

/// Ruler width; the rulers are on by default and sit between the left
/// panel and the canvas.
const RULER: f32 = 20.0;

/// Right edge of the left panel.
fn canvas_left(h: &Harness<'_, App>) -> f32 {
    h.state().canvas_origin.x as f32 - RULER
}

/// Left edge of the inspector.
fn canvas_right(h: &Harness<'_, App>) -> f32 {
    (h.state().canvas_origin.x + h.state().viewport.0) as f32
}

#[test]
fn side_panels_resize_and_keep_their_width() {
    let mut h = harness();
    for _ in 0..5 {
        h.step();
    }
    let (l0, r0) = (canvas_left(&h), canvas_right(&h));
    // The panels start at their default widths, not at their minimum.
    // The panels start at their default widths (260 and 296 plus the
    // panel's own spacing), not collapsed to their minimum.
    assert!(
        (258.0..=272.0).contains(&l0),
        "left panel default width, got {l0}"
    );
    let insp = 1600.0 - r0;
    assert!(
        (294.0..=308.0).contains(&insp),
        "inspector default width, got {insp}"
    );

    // Wider left panel.
    drag(
        &mut h,
        egui::pos2(l0 - 1.0, 600.0),
        egui::pos2(l0 + 79.0, 600.0),
    );
    let l1 = canvas_left(&h);
    assert!(
        (l1 - l0 - 80.0).abs() < 4.0,
        "left panel grew by the drag: {l0} -> {l1}"
    );

    // Wider inspector.
    drag(
        &mut h,
        egui::pos2(r0 + 1.0, 600.0),
        egui::pos2(r0 - 79.0, 600.0),
    );
    let r1 = canvas_right(&h);
    assert!(
        (r0 - r1 - 80.0).abs() < 4.0,
        "inspector grew by the drag: {r0} -> {r1}"
    );

    // Both keep their width on later frames.
    for _ in 0..30 {
        h.step();
    }
    assert!((canvas_left(&h) - l1).abs() < 0.5);
    assert!((canvas_right(&h) - r1).abs() < 0.5);

    // Narrower again, but never below the minimum.
    drag(&mut h, egui::pos2(l1 - 1.0, 600.0), egui::pos2(20.0, 600.0));
    let l2 = canvas_left(&h);
    assert!(
        (228.0..=242.0).contains(&l2),
        "left panel stops at its minimum, got {l2}"
    );
}
