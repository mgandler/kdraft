//! `kdraft-gui [file.kdraft | board.kicad_pcb]`
//!
//! Hidden dev command (no window, software rendering):
//! `kdraft-gui --screenshot out.png [--size 1400x900] [--ppp 1]
//! [--theme dark|light] [--script steps.json] [file]`
#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::time::Duration;

use eframe::egui;
use kdraft_ui::App;

struct GuiApp(App);

impl eframe::App for GuiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.show(ui);
    }

    /// Saves the settings (layout, view state) before the window closes.
    #[cfg(feature = "glow")]
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.0.save_settings_now();
    }

    #[cfg(not(feature = "glow"))]
    fn on_exit(&mut self) {
        self.0.save_settings_now();
    }
}

#[derive(Default)]
struct Args {
    file: Option<PathBuf>,
    screenshot: Option<PathBuf>,
    size: (f32, f32),
    ppp: f32,
    theme: Option<String>,
    script: Option<PathBuf>,
    gallery: Option<PathBuf>,
    settings: Option<PathBuf>,
    bench: Option<usize>,
    /// Recorded kicad-cli outputs to answer from (screenshots, tests).
    fake_kicad: Option<PathBuf>,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut a = Args {
        size: (1400.0, 900.0),
        ppp: 1.0,
        ..Default::default()
    };
    let mut it = std::env::args_os().skip(1);
    while let Some(arg) = it.next() {
        let s = arg.to_string_lossy().into_owned();
        let mut val = || {
            it.next()
                .ok_or_else(|| anyhow::anyhow!("{s} needs a value"))
        };
        match s.as_str() {
            "--screenshot" => a.screenshot = Some(PathBuf::from(val()?)),
            "--bench" => a.bench = Some(val()?.to_string_lossy().parse()?),
            "--fake-kicad" => a.fake_kicad = Some(PathBuf::from(val()?)),
            "--settings" => a.settings = Some(PathBuf::from(val()?)),
            "--gallery" => a.gallery = Some(PathBuf::from(val()?)),
            "--script" => a.script = Some(PathBuf::from(val()?)),
            "--theme" => a.theme = Some(val()?.to_string_lossy().into_owned()),
            "--ppp" => a.ppp = val()?.to_string_lossy().parse()?,
            "--size" => {
                let v = val()?.to_string_lossy().into_owned();
                let (w, h) = v
                    .split_once('x')
                    .ok_or_else(|| anyhow::anyhow!("--size WxH"))?;
                a.size = (w.parse()?, h.parse()?);
            }
            _ => a.file = Some(PathBuf::from(arg)),
        }
    }
    Ok(a)
}

/// Installs the fixture runner: recorded plots from `dir`, and for `pcb
/// render` a generated board picture.
fn use_fake_kicad(dir: &std::path::Path) -> anyhow::Result<()> {
    kdraft_ui::jobs::set_runner(std::sync::Arc::new(kdraft_source::fixture::fixture_runner(
        dir.to_path_buf(),
        Box::new(kdraft_render::render3d::fake_board_render),
    )));
    Ok(())
}

fn screenshot(a: &Args, out: &std::path::Path) -> anyhow::Result<()> {
    if let Some(d) = &a.fake_kicad {
        use_fake_kicad(d)?;
    }
    let ctx = egui::Context::default();
    let session = kdraft_engine::Session::empty()?;
    let mut app = App::new(&ctx, session, false);
    if let Some(p) = &a.settings {
        app = app.with_settings(p.clone());
    }
    if let Some(t) = &a.theme {
        app.set_theme_name(t);
    }
    if let Some(p) = &a.file {
        app.open_at_start(p.clone());
    }
    let mut sh = kdraft_ui::shot::Shooter::new(ctx, egui::vec2(a.size.0, a.size.1), a.ppp);
    sh.settle(&mut app, Duration::from_secs(60));
    if let Some(script) = &a.script {
        let text = std::fs::read_to_string(script)?;
        let base = out.parent().unwrap_or(std::path::Path::new("."));
        let steps = kdraft_ui::shot::parse_script(&text, base).map_err(anyhow::Error::msg)?;
        kdraft_ui::shot::run_script(&mut app, &mut sh, &steps).map_err(anyhow::Error::msg)?;
    }
    if let Some(n) = a.bench {
        // Frame time: egui run plus tessellation, pointer sweeping the
        // canvas (hover, cursor readout), every page shown in turn.
        let mut times = Vec::new();
        let pages = app.s.doc().pages.len().max(1);
        for k in 0..n {
            if k % (n / pages).max(1) == 0 && k > 0 {
                sh.key(&mut app, egui::Key::PageDown, egui::Modifiers::NONE);
            }
            let x = 300.0 + (k % 50) as f32 * 14.0;
            let y = 200.0 + (k % 37) as f32 * 12.0;
            sh.move_to(&mut app, egui::pos2(x, y));
            times.push(sh.last_frame.as_secs_f64() * 1000.0);
        }
        times.sort_by(f64::total_cmp);
        let mean = times.iter().sum::<f64>() / times.len().max(1) as f64;
        let p95 = times[(times.len() * 95 / 100).min(times.len() - 1)];
        eprintln!(
            "frames {}: mean {:.2} ms, median {:.2} ms, p95 {:.2} ms, max {:.2} ms",
            times.len(),
            mean,
            times[times.len() / 2],
            p95,
            times.last().copied().unwrap_or(0.0)
        );
        // Dragging the first page's view (live dimensions, previews).
        sh.key(&mut app, egui::Key::Home, egui::Modifiers::NONE);
        app.cmd(
            "page.select",
            serde_json::json!({ "page": app.s.doc().pages[0].id.0.clone() }),
        );
        sh.settle(&mut app, Duration::from_secs(30));
        let first = app.s.doc().pages[0].views().next().cloned();
        if let Some(v) = first {
            let bounds = app.s.item_bounds(&kdraft_doc::Item::View(v));
            if let Some(b) = bounds {
                let c = app.camera.to_screen(b.center());
                let o = app.canvas_origin;
                let p0 = egui::pos2((o.x + c.x) as f32, (o.y + c.y) as f32);
                sh.move_to(&mut app, p0);
                sh.frame(
                    &mut app,
                    vec![egui::Event::PointerButton {
                        pos: p0,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
                let mut dt = Vec::new();
                for k in 1..=120 {
                    sh.move_to(&mut app, p0 + egui::vec2(k as f32 * 1.5, (k % 20) as f32));
                    dt.push(sh.last_frame.as_secs_f64() * 1000.0);
                }
                dt.sort_by(f64::total_cmp);
                let mean = dt.iter().sum::<f64>() / dt.len() as f64;
                eprintln!(
                    "drag frames {}: mean {:.2} ms, p95 {:.2} ms, max {:.2} ms",
                    dt.len(),
                    mean,
                    dt[dt.len() * 95 / 100],
                    dt[dt.len() - 1]
                );
            }
        }
    }
    sh.frame(&mut app, Vec::new());
    let f = sh.capture(egui::Color32::BLACK);
    std::fs::write(out, f.png().map_err(anyhow::Error::msg)?)?;
    eprintln!(
        "wrote {} ({} x {} px, last frame {:.1} ms)",
        out.display(),
        f.w,
        f.h,
        sh.last_frame.as_secs_f64() * 1000.0
    );
    Ok(())
}

fn main() -> anyhow::Result<()> {
    if std::env::args().any(|a| a == "--shortcuts-md") {
        print!(
            "{}",
            kdraft_ui::actions::Catalogue::build(&kdraft_engine::Registry::new())
                .shortcuts_markdown()
        );
        return Ok(());
    }
    let args = parse_args()?;
    if let Some(out) = &args.gallery {
        let f = kdraft_ui::shot::gallery(args.ppp).map_err(anyhow::Error::msg)?;
        std::fs::write(out, f.png().map_err(anyhow::Error::msg)?)?;
        return Ok(());
    }
    if let Some(out) = args.screenshot.clone() {
        return screenshot(&args, &out);
    }
    let start = args.file.clone();
    // The settings file: --settings, KDRAFT_SETTINGS, else the default.
    let settings_path = args
        .settings
        .clone()
        .or_else(kdraft_ui::settings::default_path);
    // The window where it was (checked against the monitor once it is
    // known), else the default size.
    let saved = settings_path
        .as_deref()
        .map(kdraft_ui::settings::Settings::load)
        .and_then(|s| s.window);
    let plan = kdraft_ui::settings::window_plan(saved, None);
    let mut vp = egui::ViewportBuilder::default()
        .with_title("kdraft")
        .with_inner_size([plan.w, plan.h])
        .with_maximized(plan.maximized)
        .with_drag_and_drop(true);
    if saved.is_some() && plan.x.is_finite() && plan.y.is_finite() {
        vp = vp.with_position([plan.x, plan.y]);
    }
    let options = eframe::NativeOptions {
        viewport: vp,
        ..Default::default()
    };
    eframe::run_native(
        "kdraft",
        options,
        Box::new(move |cc| {
            let session = kdraft_engine::Session::empty()?;
            let mut app = App::new(&cc.egui_ctx, session, true);
            if let Some(p) = settings_path {
                app = app.with_settings(p);
            }
            app.use_native_dialogs(std::env::var_os("KDRAFT_TYPED_PATHS").is_none());
            if let Some(p) = start {
                app.open_at_start(p);
            }
            Ok(Box::new(GuiApp(app)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}
