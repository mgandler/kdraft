//! Headless tests of the user settings in the editor: panel sizes and
//! the dock survive a restart; the settings page (navigation, a custom
//! kicad-cli, importing a KiCad theme through the typed-path prompt, the
//! default palette for new documents); per-document page and zoom.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use kdraft_engine::Session;
use kdraft_ui::App;
use serde_json::json;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn harness_with(s: Session, settings: &Path) -> Harness<'static, App> {
    let ctx = egui::Context::default();
    let app = App::new(&ctx, s, false).with_settings(settings.to_path_buf());
    Harness::builder()
        .with_size(egui::Vec2::new(1600.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(kdraft_ui::app::install_fonts)
        .build_ui_state(|ui, app: &mut App| app.show(ui), app)
}

fn board_session() -> Session {
    Session::new_for_board(&fixtures().join("kicad9-minimal.kicad_pcb"), None).unwrap()
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

/// The fixture kicad-cli for this test binary (any path answers).
fn fake_kicad() {
    kdraft_ui::jobs::set_runner(std::sync::Arc::new(kdraft_source::fixture::fixture_runner(
        fixtures().join("kicad-cli-svg"),
        Box::new(kdraft_render::render3d::fake_board_render),
    )));
}

fn wait(h: &mut Harness<'_, App>, f: impl Fn(&App) -> bool) {
    let t = Instant::now();
    while !f(h.state()) && t.elapsed() < Duration::from_secs(20) {
        h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(f(h.state()), "condition not reached");
}

#[test]
fn panels_and_dock_survive_a_restart() {
    fake_kicad();
    let dir = tempfile::tempdir().unwrap();
    let set = dir.path().join("settings.json");
    let mut h = harness_with(board_session(), &set);
    h.run_steps(5);
    // The dock (message log) open, then the three edges dragged.
    h.state_mut().run_id("app.log");
    h.run_steps(5);
    let left0 = h.state().settings.left_width.unwrap();
    let insp0 = h.state().settings.inspector_width.unwrap();
    let dock0 = h.state().settings.dock_height.unwrap();
    let x = h.state().canvas_origin.x as f32 - 20.0;
    drag(
        &mut h,
        egui::pos2(x - 1.0, 400.0),
        egui::pos2(x + 59.0, 400.0),
    );
    let r = (h.state().canvas_origin.x + h.state().viewport.0) as f32;
    drag(
        &mut h,
        egui::pos2(r + 1.0, 400.0),
        egui::pos2(r - 49.0, 400.0),
    );
    let top = (h.state().canvas_origin.y + h.state().viewport.1) as f32;
    drag(
        &mut h,
        egui::pos2(800.0, top + 1.0),
        egui::pos2(800.0, top - 69.0),
    );
    let s = h.state().settings.clone();
    let (l1, i1, d1) = (
        s.left_width.unwrap(),
        s.inspector_width.unwrap(),
        s.dock_height.unwrap(),
    );
    assert!((l1 - left0 - 60.0).abs() < 4.0, "{left0} -> {l1}");
    assert!((i1 - insp0 - 50.0).abs() < 4.0, "{insp0} -> {i1}");
    assert!((d1 - dock0 - 70.0).abs() < 4.0, "{dock0} -> {d1}");
    // A tool variant used.
    h.state_mut().run_id("tool.callout_bom");
    h.run_steps(2);
    h.state_mut().run_id("tool.select");
    // Exit: written synchronously.
    h.state_mut().save_settings_now();
    let canvas = (
        h.state().canvas_origin.x,
        h.state().canvas_origin.x + h.state().viewport.0,
        h.state().viewport.1,
    );
    drop(h);
    // Restart from the same file.
    let mut h = harness_with(board_session(), &set);
    h.run_steps(6);
    let s = h.state().settings.clone();
    assert!(
        (s.left_width.unwrap() - l1).abs() < 1.0,
        "{:?} vs {l1}",
        s.left_width
    );
    assert!((s.inspector_width.unwrap() - i1).abs() < 1.0);
    assert!((s.dock_height.unwrap() - d1).abs() < 1.0);
    assert!(h.state().dock.is_some(), "dock open again");
    let now = (
        h.state().canvas_origin.x,
        h.state().canvas_origin.x + h.state().viewport.0,
        h.state().viewport.1,
    );
    assert!((now.0 - canvas.0).abs() < 1.5, "{now:?} vs {canvas:?}");
    assert!((now.1 - canvas.1).abs() < 1.5, "{now:?} vs {canvas:?}");
    assert!((now.2 - canvas.2).abs() < 1.5, "{now:?} vs {canvas:?}");
    assert_eq!(
        h.state().settings.tools.get("note").map(String::as_str),
        Some("tool.callout_bom")
    );
}

#[test]
fn settings_page_custom_kicad_cli_and_theme_import() {
    fake_kicad();
    let dir = tempfile::tempdir().unwrap();
    let set = dir.path().join("settings.json");
    let mut h = harness_with(board_session(), &set);
    h.state_mut().use_native_dialogs(false);
    wait(&mut h, |a| a.kicad_found());
    // Ctrl+comma opens the page; the sections; Escape closes it.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Comma);
    h.run_steps(3);
    assert!(h.state().prefs.is_some());
    for s in ["KiCad", "Documents", "Layer colours", "Cache", "General"] {
        h.get_by_label(s).click();
        h.run_steps(2);
    }
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().prefs.is_none());
    // The status bar entry opens the KiCad section (once the plots are
    // done, so the text does not change under the pointer).
    wait(&mut h, |a| a.kicad_status().contains("plots ready"));
    h.run_steps(3);
    let status = h.state().kicad_status();
    h.get_by_label(&status).click();
    h.run_steps(3);
    assert_eq!(
        h.state().prefs,
        Some(kdraft_ui::ui_prefs::PrefsSection::Kicad)
    );
    // A custom kicad-cli through the typed-path prompt.
    h.get_by_label("Browse…").click();
    h.run_steps(3);
    let custom = dir.path().join("my-kicad").join("kicad-cli");
    h.event(egui::Event::Text(custom.display().to_string()));
    h.run_steps(1);
    h.get_by_label("OK").click();
    h.run_steps(3);
    wait(&mut h, |a| {
        a.kicad_source == Some(kdraft_source::CliSource::Setting)
    });
    assert!(
        h.state().kicad_status().contains("(settings)"),
        "{}",
        h.state().kicad_status()
    );
    let saved = |set: &Path| kdraft_source::settings::SettingsFile::load(set).settings;
    wait(&mut h, |_| {
        saved(&set).kicad().cli.text() == custom.display().to_string()
    });
    // Layer colours: import a KiCad theme through the prompt.
    h.get_by_label("Layer colours").click();
    h.run_steps(3);
    h.get_by_label("Browse…").click();
    h.run_steps(3);
    let theme = fixtures().join("kicad-themes").join("partial.json");
    h.event(egui::Event::Text(theme.display().to_string()));
    h.run_steps(1);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert_eq!(h.state().prefs_state.palette, "Partial");
    let imported = h.state().user.palette("Partial").unwrap();
    // Adapted: the white silkscreen is dark enough for paper.
    let silk = imported.layer("F.SilkS").unwrap().0;
    assert!(kdraft_doc::style::contrast(silk, kdraft_doc::style::Rgba::WHITE) >= 3.0);
    // The before/after columns are there.
    h.get_by_label("F.SilkS in the theme");
    // Default for new documents, then a new document from the board.
    h.get_by_label("Use for new documents").click();
    h.run_steps(3);
    assert_eq!(h.state().user.documents().layer_palette, "Partial");
    h.get_by_label("Close").click();
    h.run_steps(2);
    let copy = dir.path().join("b.kicad_pcb");
    std::fs::copy(fixtures().join("kicad9-minimal.kicad_pcb"), &copy).unwrap();
    h.state_mut().open_at_start(copy);
    wait(&mut h, |a| {
        a.s.doc()
            .styles
            .layer_palette
            .as_ref()
            .map(|p| p.name.as_str())
            == Some("Partial")
    });
    // A view inserted now takes the palette's colours.
    h.state_mut().cmd(
        "item.add_view",
        json!({ "kind": "layer", "layers": ["F.SilkS", "F.Cu"], "at": [100, 100] }),
    );
    let v = match h.state().selected_items()[0] {
        kdraft_doc::Item::View(v) => v.clone(),
        _ => unreachable!(),
    };
    let st = h.state().s.doc().styles.clone();
    assert_eq!(v.layer_color(&st, "F.SilkS").0, silk);
    assert_eq!(v.layer_color(&st, "F.Cu"), imported.layer("F.Cu").unwrap());
}

#[test]
fn page_and_zoom_come_back_when_a_document_is_reopened() {
    let dir = tempfile::tempdir().unwrap();
    let set = dir.path().join("settings.json");
    let board = dir.path().join("b.kicad_pcb");
    std::fs::copy(fixtures().join("kicad9-minimal.kicad_pcb"), &board).unwrap();
    let doc = dir.path().join("b.kdraft");
    let mut s = Session::new_for_board(&board, Some(&doc)).unwrap();
    s.save(Some(&doc)).unwrap();
    let open = |set: &Path| {
        let mut h = harness_with(Session::empty().unwrap(), set);
        h.state_mut().open_at_start(doc.clone());
        wait(&mut h, |a| a.s.doc_path().is_some());
        h.run_steps(4);
        h
    };
    let mut h = open(&set);
    // Page 3, zoomed in.
    let p3 = h.state().s.doc().pages[2].id.0.clone();
    h.state_mut().cmd("page.select", json!({ "page": p3 }));
    h.run_steps(3);
    h.state_mut().run_named("zoom_in");
    h.state_mut().run_named("zoom_in");
    h.run_steps(3);
    let zoom = h.state().camera.zoom;
    h.state_mut().save_settings_now();
    drop(h);
    // Reopened: the same page at the same zoom.
    let mut h = open(&set);
    assert_eq!(h.state().s.current_page().unwrap().id.0, p3);
    assert!(
        (h.state().camera.zoom - zoom).abs() < 1e-6,
        "{} vs {zoom}",
        h.state().camera.zoom
    );
    // The settings file does not leak into the document.
    assert!(!std::fs::read_to_string(&doc).unwrap().contains("zoom"));
    // A camera that no longer fits (page size changed) falls back to fit.
    h.state_mut()
        .cmd("page.set_paper", json!({ "paper": "A4" }));
    h.state_mut().camera_page = None;
    h.run_steps(3);
    assert!((h.state().camera.zoom - zoom).abs() > 1e-3);
}
