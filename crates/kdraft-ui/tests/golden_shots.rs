//! Screenshot goldens of a few stable views (software-rasterised, see
//! `kdraft_ui::shot`), compared with a tolerance. Regenerate with
//! `KDRAFT_UPDATE_GOLDEN=1 cargo test -p kdraft-ui --test golden_shots`.

use std::path::PathBuf;
use std::time::Duration;

use eframe::egui;
use kdraft_ui::shot::{self, Shooter};
use kdraft_ui::App;

fn golden(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name)
}

/// Mean channel difference allowed (0..255): small layout shifts fail,
/// anti-aliasing differences between machines do not.
const TOLERANCE: f64 = 1.5;

fn check(name: &str, f: &shot::Frame) {
    let path = golden(name);
    let png = f.png().unwrap();
    if std::env::var_os("KDRAFT_UPDATE_GOLDEN").is_some() || !path.exists() {
        std::fs::write(&path, &png).unwrap();
        return;
    }
    let want = decode(&std::fs::read(&path).unwrap());
    let got = f.rgba();
    let d = shot::mean_abs_diff(&want, &got).unwrap_or(f64::INFINITY);
    assert!(
        d <= TOLERANCE,
        "{name}: mean difference {d:.2} (KDRAFT_UPDATE_GOLDEN=1 to accept)"
    );
}

fn decode(png: &[u8]) -> Vec<u8> {
    let p = tiny_skia::Pixmap::decode_png(png).unwrap();
    p.data().to_vec()
}

fn app_shot(app: &mut App, ctx: egui::Context, dark: bool) -> shot::Frame {
    app.set_theme_name(if dark { "dark" } else { "light" });
    let mut sh = Shooter::new(ctx, egui::vec2(1200.0, 760.0), 1.0);
    sh.settle(app, Duration::from_secs(30));
    app.toasts.clear();
    sh.frame(app, Vec::new());
    sh.frame(app, Vec::new());
    sh.capture(egui::Color32::BLACK)
}

#[test]
fn golden_start_screen_both_themes() {
    for dark in [false, true] {
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, kdraft_engine::Session::empty().unwrap(), false);
        let f = app_shot(&mut app, ctx, dark);
        check(
            if dark {
                "start-dark.png"
            } else {
                "start-light.png"
            },
            &f,
        );
    }
}

#[test]
fn golden_fixture_fabrication_page() {
    let board =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/kicad9-minimal.kicad_pcb");
    let ctx = egui::Context::default();
    let s = kdraft_engine::Session::new_for_board(&board, None).unwrap();
    let mut app = App::new(&ctx, s, false);
    let f = app_shot(&mut app, ctx, false);
    check("fixture-fab-light.png", &f);
}

#[test]
fn golden_icon_gallery() {
    check("icons.png", &shot::gallery(1.0).unwrap());
}
