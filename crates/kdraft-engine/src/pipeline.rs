//! From a session to rendered pages: kicad-cli layer plots (through
//! [`FileBoardSource`] and its cache), stub-board sheets, sheet context and
//! the display lists the back ends draw. Used by `kdraft export` and the
//! GUI.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kdraft_board::{Board, FabTextShow};
use kdraft_doc::{Document, DrillMarks, Page, View};
use kdraft_plot::fit::{fit_board, PlotTransform};
use kdraft_plot::svg::{self, SvgDoc};
use kdraft_render::dl::{DocList, PageList};
use kdraft_render::{Fonts, PageSheet, PlotContent, SheetContext};
use kdraft_sheet::{SheetChoice, WksFile};
use kdraft_source::jobs::{DrillShapeOpt, PageSizeMode, SvgExport};
use kdraft_source::stub::{write_stub, SheetStub};
use kdraft_source::{run_cached, BoardSource, Cache, CommandRunner, FileBoardSource, KicadCli};

use crate::error::{EngineError, R};
use crate::session::Session;

/// Page size mode used for layer exports: the full page (mode 0). On
/// kicad-cli 10.0.8 mode 2 cut half the outline stroke off at the page edge
/// and is board-relative (S1, `docs/STATUS.md`); mode 0 clips nothing. The
/// transform is still fitted per export.
pub const EXPORT_MODE: PageSizeMode = PageSizeMode::Sheet;

/// `--drill-shape-opt` for a view's drill marks; `None` for the exports
/// used only to fit the transform (kicad-cli's default).
pub fn drill_opt(m: Option<&DrillMarks>) -> Option<DrillShapeOpt> {
    m.map(|m| match m {
        DrillMarks::None => DrillShapeOpt::None,
        DrillMarks::Small => DrillShapeOpt::Small,
        DrillMarks::Actual | DrillMarks::Other(_) => DrillShapeOpt::Actual,
    })
}

pub fn layer_request(layer: &str, marks: Option<&DrillMarks>) -> SvgExport {
    SvgExport {
        layers: vec![layer.to_string()],
        black_and_white: true,
        page_size_mode: EXPORT_MODE,
        exclude_drawing_sheet: true,
        drill_shape: drill_opt(marks),
        ..Default::default()
    }
}

/// Key of one plotted layer: name, drill marks and, for fab layers, the
/// fab-text switches (texts hidden in the private copy before plotting).
pub type LayerKey = (String, Option<DrillShapeOpt>, Option<FabTextShow>);

/// The fab-text switches that apply to `layer` in view `v` (`None` for
/// non-fab layers or when everything is shown, so those share plots).
pub fn fab_show_for(v: &View, layer: &str) -> Option<FabTextShow> {
    let show = v.assembly.fab_show();
    (layer.ends_with(".Fab") && show.hides_anything()).then_some(show)
}

pub fn read_svg(path: &Path) -> R<SvgDoc> {
    let text = std::fs::read_to_string(path).map_err(|e| EngineError::io("reading", path, e))?;
    svg::parse(&text).map_err(|e| EngineError::invalid(format!("parsing {}: {e}", path.display())))
}

/// Layer SVGs from kicad-cli and the transform fitted for them.
#[derive(Clone)]
pub struct Plots {
    pub fit: PlotTransform,
    pub layers: Vec<(LayerKey, SvgDoc)>,
}

impl Plots {
    /// Plotted content for a view, if all its layers were plotted. Drill
    /// views are always native.
    pub fn for_view(&self, v: &View) -> Option<PlotContent> {
        if !v.wants_plot() {
            return None;
        }
        let marks = drill_opt(Some(&v.drill_marks()));
        let layers: Option<Vec<SvgDoc>> = v
            .layers
            .iter()
            .map(|l| {
                let fab = fab_show_for(v, l);
                self.layers
                    .iter()
                    .find(|(k, _)| k.0 == *l && k.1 == marks && k.2 == fab)
                    .map(|(_, d)| d.clone())
            })
            .collect();
        Some(PlotContent {
            layers: layers?,
            fit: self.fit,
            image: None,
        })
    }
}

/// One layer export a document needs.
pub type LayerNeed = (String, DrillMarks, Option<FabTextShow>);

/// The layer exports all views of `doc` need.
pub fn needed_layers(doc: &Document) -> Vec<LayerNeed> {
    let mut out: Vec<LayerNeed> = Vec::new();
    for p in &doc.pages {
        for v in p.views().filter(|v| v.wants_plot()) {
            let marks = v.drill_marks();
            for l in &v.layers {
                let k = (l.clone(), marks.clone(), fab_show_for(v, l));
                if !out.contains(&k) {
                    out.push(k);
                }
            }
        }
    }
    out
}

/// Exports Edge.Cuts and F.Cu, fits the transform, then exports every
/// layer in `layers`. `None` (with a note) if the exports fail or the fit
/// is poor; views are then drawn natively.
pub fn fetch_plots(
    layers: &[LayerNeed],
    src: &FileBoardSource,
    board: &Board,
    notes: &mut Vec<String>,
) -> Option<Plots> {
    let get = |l: &str, m: Option<&DrillMarks>| -> R<SvgDoc> {
        read_svg(&src.layer_svg(&layer_request(l, m))?)
    };
    let get_fab = |l: &str, m: Option<&DrillMarks>, f: Option<FabTextShow>| -> R<SvgDoc> {
        match f {
            Some(show) => read_svg(&src.layer_svg_fab(&layer_request(l, m), show)?),
            None => get(l, m),
        }
    };
    let (edge, cu) = match (get("Edge.Cuts", None), get("F.Cu", None)) {
        (Ok(e), Ok(c)) => (e, c),
        (Err(e), _) | (_, Err(e)) => {
            notes.push(format!(
                "kicad-cli export failed, views drawn natively: {e}"
            ));
            return None;
        }
    };
    let Some(fit) = fit_board(board, &edge, Some((&cu, "F.Cu"))) else {
        notes.push("no plot transform could be fitted; views drawn natively".to_string());
        return None;
    };
    if !fit.is_good() {
        notes.push(format!(
            "plot transform fit too poor ({:.1}% of {} references within 0.05 mm, method {}); views drawn natively",
            fit.residuals.within_0_05 * 100.0,
            fit.residuals.count,
            fit.method
        ));
        return None;
    }
    notes.push(format!(
        "plot transform fitted on {} ({} references, max residual {:.4} mm)",
        fit.method, fit.residuals.count, fit.residuals.max_mm
    ));
    let mut out: Vec<(LayerKey, SvgDoc)> = vec![
        (("Edge.Cuts".to_string(), None, None), edge),
        (("F.Cu".to_string(), None, None), cu),
    ];
    for (l, marks, fab) in layers {
        let key = (l.clone(), drill_opt(Some(marks)), *fab);
        if out.iter().any(|(k, _)| *k == key) {
            continue;
        }
        match get_fab(l, Some(marks), *fab) {
            Ok(d) => out.push((key, d)),
            Err(e) => notes.push(format!("layer {l}: {e}")),
        }
    }
    Some(Plots {
        fit: fit.transform,
        layers: out,
    })
}

fn stub_for(board: &Board, page: &Page, vars: &BTreeMap<String, String>) -> SheetStub {
    let (w, h) = (page.size.width, page.size.height);
    let named = kdraft_sheet::paper_size(&page.size.name).filter(|(pw, ph)| {
        let close = |a: f64, b: f64| (a - b).abs() < 0.5;
        (close(*pw, w) && close(*ph, h)) || (close(*pw, h) && close(*ph, w))
    });
    let tb = &board.title_block;
    SheetStub {
        paper: if named.is_some() {
            page.size.name.clone()
        } else {
            "User".to_string()
        },
        width: w,
        height: h,
        portrait: named.is_some() && h > w,
        title: tb.title.clone(),
        date: tb.date.clone(),
        rev: tb.rev.clone(),
        company: tb.company.clone(),
        comments: tb.comments.clone(),
        text_variables: vars.clone(),
    }
}

/// The schematic BOM through kicad-cli: the schematic next to the board
/// (same stem), exported from a private copy of it and its sheets.
/// `None` (with a note) when there is no schematic or the export fails;
/// BOM tables then fall back to the PCB.
pub fn fetch_bom(
    board_path: &Path,
    k: &Kicad,
    cache: &Path,
    notes: &mut Vec<String>,
) -> Option<Arc<kdraft_board::bom::Bom>> {
    let Some(sch) = kdraft_source::schematic_for(board_path) else {
        notes.push("no schematic next to the board; BOM derived from the PCB".to_string());
        return None;
    };
    let export = kdraft_source::jobs::BomExport::kdraft(&[]);
    match kdraft_source::run_bom(&sch, &export, &k.cli, k.runner.as_ref(), &Cache::new(cache)) {
        Ok((csv, skipped)) => {
            for f in skipped {
                notes.push(format!(
                    "schematic sheet {f} not found or outside the project; BOM may be incomplete"
                ));
            }
            let text = match std::fs::read_to_string(&csv) {
                Ok(t) => t,
                Err(e) => {
                    notes.push(format!("reading BOM CSV: {e}; BOM derived from the PCB"));
                    return None;
                }
            };
            match kdraft_board::bom::parse_csv(&text) {
                Ok(b) => {
                    notes.push(format!("BOM from the schematic: {} lines", b.lines.len()));
                    Some(Arc::new(b))
                }
                Err(e) => {
                    notes.push(format!(
                        "BOM CSV not understood ({e}); BOM derived from the PCB"
                    ));
                    None
                }
            }
        }
        Err(e) => {
            notes.push(format!(
                "kicad-cli BOM export failed ({e}); BOM derived from the PCB"
            ));
            None
        }
    }
}

static CACHE_ROOT: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);

/// Sets one cache folder for all documents (the `kicad.cache_dir` user
/// setting); `None` restores `.kdraft-cache` next to each document. The
/// cache is content-addressed, so documents can share it.
pub fn set_cache_root(root: Option<PathBuf>) {
    if let Ok(mut g) = CACHE_ROOT.write() {
        *g = root;
    }
}

/// Cache folder for a document: the shared one when set, else
/// `<doc dir>/.kdraft-cache`.
pub fn cache_dir(doc_path: Option<&Path>) -> PathBuf {
    if let Some(root) = CACHE_ROOT.read().ok().and_then(|g| g.clone()) {
        return root;
    }
    doc_path
        .and_then(Path::parent)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .join(".kdraft-cache")
}

/// kicad-cli and the runner to use.
#[derive(Clone)]
pub struct Kicad {
    pub cli: KicadCli,
    pub runner: Arc<dyn CommandRunner>,
}

/// Drawing sheets drawn by kicad-cli from stub boards, per page.
pub fn kicad_sheets(
    s: &mut Session,
    k: &Kicad,
    cache: &Path,
    notes: &mut Vec<String>,
) -> R<Vec<Option<SvgDoc>>> {
    let board = s
        .board_arc()
        .ok_or_else(|| EngineError::invalid("no board loaded"))?;
    let vars = project_vars(s);
    let stub_dir = cache.join("stubs");
    let mut out = Vec::new();
    for i in 0..s.doc().pages.len() {
        let page = s.doc().pages[i].clone();
        let stub = stub_for(&board, &page, &vars);
        let pcb = write_stub(&stub_dir, &format!("page{}", i + 1), &stub)?;
        let doc_dir = s
            .doc_path()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let (choice, _) = kdraft_sheet::select_sheet(
            page.sheet.as_deref(),
            s.doc().sheet.as_deref(),
            &doc_dir,
            s.project().map(|(p, d)| (p, d.as_path())),
        );
        let req = SvgExport {
            layers: vec!["Edge.Cuts".to_string()],
            black_and_white: true,
            page_size_mode: PageSizeMode::Sheet,
            exclude_drawing_sheet: false,
            drawing_sheet: match choice {
                SheetChoice::File(f) => Some(f),
                SheetChoice::Default => None,
            },
            ..Default::default()
        };
        let job = req.job(&pcb, &k.cli)?;
        let bytes = std::fs::read(&pcb).map_err(|e| EngineError::io("reading", &pcb, e))?;
        let hash = kdraft_board::BoardHash::of(&bytes).0;
        match run_cached(&job, &hash, &k.cli, k.runner.as_ref(), &Cache::new(cache))
            .map_err(EngineError::from)
            .and_then(|path| read_svg(&path))
        {
            Ok(d) => out.push(Some(d)),
            Err(e) => {
                notes.push(format!(
                    "page {}: kicad-cli sheet failed, drawn natively: {e}",
                    i + 1
                ));
                out.push(None);
            }
        }
    }
    notes.push("kicad-cli sheets show KiCad's own page number (1/1) on every page".to_string());
    Ok(out)
}

fn project_vars(s: &Session) -> BTreeMap<String, String> {
    s.project()
        .map(|p| p.0.text_variables.clone())
        .unwrap_or_default()
}

/// Sheet text context for a session (`${KICAD_VERSION}` from `kicad`).
pub fn sheet_context(s: &Session, kicad: Option<&KicadCli>) -> SheetContext {
    let bp = s.board_state().path.clone().unwrap_or_default();
    SheetContext {
        file_name: bp.file_name().map(|f| f.to_string_lossy().into_owned()),
        project_name: bp.file_stem().map(|f| f.to_string_lossy().into_owned()),
        kicad_version: Some(match kicad {
            Some(k) => format!("KiCad {}", k.version),
            None => format!("kdraft {}", env!("CARGO_PKG_VERSION")),
        }),
        project_vars: project_vars(s),
        bom: s.schematic_bom(),
    }
}

/// Resolved drawing sheets for every page (with load warnings).
pub fn page_sheets(s: &mut Session) -> (Vec<Arc<WksFile>>, Vec<String>) {
    let mut w = Vec::new();
    let mut notes = Vec::new();
    for i in 0..s.doc().pages.len() {
        let (sheet, _, msg) = s.sheet_for_page(i);
        w.push(sheet);
        notes.extend(msg);
    }
    (w, notes)
}

/// 3D pictures and their progress by [`render_key`].
pub type Renders = kdraft_render::render3d::Renders;

/// Longest side of the probe render, pixels.
pub const PROBE_PX: u32 = 480;
/// Content fills this fraction of the final render (centred).
pub const FINAL_FILL: f64 = 0.92;

/// Board size and picture resolution bucket of a 3D view: the content's
/// long side in pixels for the view's size on paper at its dpi, rounded up
/// to a power of two (256..4096), so moving or resizing a view re-renders
/// only when the size crosses a factor of two.
pub fn render_bucket(v: &View, board_box: kdraft_board::geom::BBox) -> u32 {
    let o = v.render3d();
    let long_mm = board_box.width().max(board_box.height()) * v.scale;
    let px = (long_mm / 25.4 * f64::from(o.dpi)).max(1.0);
    let mut b = 256u32;
    while f64::from(b) < px && b < kdraft_doc::render3d::MAX_PX {
        b *= 2;
    }
    b
}

/// The key of a 3D view's picture: camera, look and resolution bucket
/// (not position, not size within the bucket).
pub fn render_key(v: &View, board_box: kdraft_board::geom::BBox) -> String {
    let o = v.render3d();
    format!(
        "{} {:?} {} {} {:?} {} {} {} {:.3}x{:.3}",
        o.side,
        o.rotate,
        o.perspective,
        o.quality,
        o.appearance,
        o.stackup_colors,
        o.floor,
        render_bucket(v, board_box),
        board_box.width(),
        board_box.height()
    )
}

fn base_job(v: &View, project_dir: Option<&Path>) -> kdraft_source::jobs::RenderJob {
    use kdraft_source::jobs::{RenderBackground, RenderJob, RenderQuality, RenderSide};
    let o = v.render3d();
    RenderJob {
        width: 16,
        height: 16,
        side: match o.side.as_str() {
            "bottom" => RenderSide::Bottom,
            "front" => RenderSide::Front,
            "back" => RenderSide::Back,
            "left" => RenderSide::Left,
            "right" => RenderSide::Right,
            _ => RenderSide::Top,
        },
        quality: if o.quality == "high" {
            RenderQuality::High
        } else {
            RenderQuality::Basic
        },
        // Always transparent: the content box comes from alpha; a white
        // background is drawn under the picture.
        background: RenderBackground::Transparent,
        variant: None,
        perspective: o.perspective,
        zoom: None,
        rotate: o.rotate.map(|[x, y, z]| format!("{x},{y},{z}")),
        define_vars: project_dir
            .map(|d| vec![("KIPRJMOD".to_string(), d.to_string_lossy().into_owned())])
            .unwrap_or_default(),
        preset: o.appearance.clone(),
        stackup_colors: o.stackup_colors,
        floor: o.floor,
    }
}

/// Size of the probe: the board's aspect for views to scale, 16:10
/// otherwise; the long side [`PROBE_PX`].
fn probe_size(v: &View, bb: kdraft_board::geom::BBox) -> (u32, u32) {
    let ar = if v.render3d().to_scale() {
        (bb.width() / bb.height().max(1e-9)).clamp(0.1, 10.0)
    } else {
        1.6
    };
    let p = f64::from(PROBE_PX);
    if ar >= 1.0 {
        (PROBE_PX, ((p / ar).round() as u32).max(16))
    } else {
        (((p * ar).round() as u32).max(16), PROBE_PX)
    }
}

/// Pass 1: a small, fast render of the same camera (basic quality).
pub fn probe_job(
    v: &View,
    bb: kdraft_board::geom::BBox,
    project_dir: Option<&Path>,
) -> kdraft_source::jobs::RenderJob {
    let (w, h) = probe_size(v, bb);
    kdraft_source::jobs::RenderJob {
        width: w,
        height: h,
        quality: kdraft_source::jobs::RenderQuality::Basic,
        ..base_job(v, project_dir)
    }
}

/// Pass 2 from the probe: the same picture aspect, enlarged and zoomed so
/// the content (centred) fills [`FINAL_FILL`] of it and its long side gets
/// the view's resolution bucket. Returns the job and the zoom asked for.
/// With a floor (no transparent background to measure) the final render
/// is not zoomed.
pub fn final_job(
    v: &View,
    bb: kdraft_board::geom::BBox,
    project_dir: Option<&Path>,
    probe_png: &[u8],
) -> Result<(kdraft_source::jobs::RenderJob, f64), String> {
    use kdraft_render::render3d as r3;
    let (pw, ph) = probe_size(v, bb);
    let target = f64::from(render_bucket(v, bb));
    let o = v.render3d();
    let (zoom, content_long) = if o.floor {
        (1.0, f64::from(pw.max(ph)))
    } else {
        let (rgba, w, h) = r3::decode(probe_png).ok_or("the probe render is not a PNG")?;
        let c = r3::content_box(&rgba, w, h).ok_or("the probe render is empty")?;
        let fill = r3::centred_fill(c, w, h).max(0.05);
        let z = (FINAL_FILL / fill).max(1.0);
        (z, f64::from((c[2] - c[0]).max(c[3] - c[1])))
    };
    let max = f64::from(kdraft_doc::render3d::MAX_PX);
    let k = (target / (content_long * zoom))
        .min(max / f64::from(pw.max(ph)))
        .max(0.1);
    let job = kdraft_source::jobs::RenderJob {
        width: ((f64::from(pw) * k).round() as u32).max(16),
        height: ((f64::from(ph) * k).round() as u32).max(16),
        zoom: (zoom > 1.0001).then_some((zoom * 1000.0).round() / 1000.0),
        ..base_job(v, project_dir)
    };
    Ok((job, zoom))
}

/// Trims the final render (content box plus [`PADDING_MM`] on paper) and,
/// for views to scale, calibrates the board outline box in it.
pub fn finish_render(
    v: &View,
    bb: kdraft_board::geom::BBox,
    png: &[u8],
    zoom: f64,
) -> Result<kdraft_render::render3d::Picture, String> {
    use kdraft_render::render3d as r3;
    let o = v.render3d();
    let (rgba, w, h) = r3::decode(png).ok_or("the render is not a PNG")?;
    if o.floor {
        // Nothing to trim against: the whole picture, not to scale.
        return Ok(r3::Picture {
            png: Arc::new(png.to_vec()),
            width: w,
            height: h,
            outline_px: None,
            fill: 1.0,
            note: "floor on: not trimmed, not to scale".into(),
        });
    }
    let c = r3::content_box(&rgba, w, h).ok_or("the render is empty")?;
    let cut = r3::touches_border(c, w, h);
    let long_mm = bb.width().max(bb.height()) * v.scale;
    let px_per_paper_mm = f64::from((c[2] - c[0]).max(c[3] - c[1])) / long_mm.max(1e-9);
    let board = o.to_scale().then(|| (bb.width(), bb.height()));
    let mut pic = r3::trim(png, r3::PADDING_MM * px_per_paper_mm, board)
        .ok_or("the render could not be trimmed")?;
    let dpi = px_per_paper_mm * 25.4;
    pic.note = format!(
        "{} x {} px, about {:.0} dpi{}{}",
        pic.width,
        pic.height,
        dpi,
        if zoom > 1.0001 {
            format!(", zoom {zoom:.2}")
        } else {
            String::new()
        },
        if cut {
            "; content touches the edge (zoom?)"
        } else {
            ""
        }
    );
    Ok(pic)
}

/// Runs both passes for one view (cached by kicad-cli job), synchronously.
pub fn fetch_render(
    v: &View,
    bb: kdraft_board::geom::BBox,
    project_dir: Option<&Path>,
    src: &FileBoardSource,
) -> Result<kdraft_render::render3d::Picture, String> {
    let run = |j: &kdraft_source::jobs::RenderJob| -> Result<Vec<u8>, String> {
        let job = j.job(src.path(), src.cli()).map_err(|e| e.to_string())?;
        let p = src.run_job(&job).map_err(|e| e.to_string())?;
        std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))
    };
    let probe = run(&probe_job(v, bb, project_dir))?;
    let (job, zoom) = final_job(v, bb, project_dir, &probe)?;
    let png = run(&job)?;
    let pic = finish_render(v, bb, &png, zoom)?;
    // Zoom not honoured as asked (content cut off): render unzoomed at the
    // same size instead.
    if zoom > 1.0001 && pic.note.contains("touches the edge") {
        let job1 = kdraft_source::jobs::RenderJob { zoom: None, ..job };
        let png = run(&job1)?;
        return finish_render(v, bb, &png, 1.0);
    }
    Ok(pic)
}

/// Renders every 3D view of the document (keyed by [`render_key`]).
pub fn fetch_renders(
    doc: &Document,
    bb: kdraft_board::geom::BBox,
    project_dir: Option<&Path>,
    src: &FileBoardSource,
    notes: &mut Vec<String>,
) -> Renders {
    use kdraft_render::render3d::State;
    let mut out = Renders::new();
    for v in doc
        .pages
        .iter()
        .flat_map(|p| p.views())
        .filter(|v| v.kind == kdraft_doc::ViewKind::Render3d)
    {
        let key = render_key(v, bb);
        if out.contains_key(&key) {
            continue;
        }
        let st = match fetch_render(v, bb, project_dir, src) {
            Ok(p) => State::Ready(Arc::new(p)),
            Err(e) => {
                notes.push(format!("3D render failed: {e}"));
                State::Failed(e)
            }
        };
        out.insert(key, st);
    }
    out
}

/// Inputs for rendering, owned, so they can move to a worker thread.
#[derive(Clone)]
pub struct RenderInputs {
    pub doc: Arc<Document>,
    pub board: Arc<Board>,
    pub sheets: Vec<Arc<WksFile>>,
    pub sheet_svgs: Vec<Option<SvgDoc>>,
    pub ctx: SheetContext,
    pub plots: Option<Arc<Plots>>,
    pub fonts: Arc<Fonts>,
    /// 3D pictures by [`render_request`] key.
    pub renders: Arc<Renders>,
    /// kicad-cli is available (3D views show "rendering" rather than
    /// "needs kicad-cli" while their picture is missing).
    pub kicad: bool,
}

impl RenderInputs {
    pub fn from_session(
        s: &mut Session,
        kicad: Option<&KicadCli>,
    ) -> R<(RenderInputs, Vec<String>)> {
        let board = s.board_arc().ok_or_else(|| {
            EngineError::invalid(
                s.board_state()
                    .error
                    .clone()
                    .unwrap_or_else(|| "no board loaded".into()),
            )
        })?;
        let (sheets, notes) = page_sheets(s);
        let n = sheets.len();
        Ok((
            RenderInputs {
                doc: s.doc_arc(),
                board,
                sheets,
                sheet_svgs: vec![None; n],
                ctx: sheet_context(s, kicad),
                plots: None,
                fonts: s.fonts_arc(),
                renders: Arc::clone(&s.renders),
                kicad: kicad.is_some() || s.kicad_available,
            },
            notes,
        ))
    }

    fn page_sheet(&self, i: usize) -> PageSheet<'_> {
        PageSheet {
            wks: &self.sheets[i],
            svg: self.sheet_svgs.get(i).and_then(|o| o.as_ref()),
        }
    }

    fn plot_for(&self, v: &View) -> Option<PlotContent> {
        if v.kind == kdraft_doc::ViewKind::Render3d {
            let key = render_key(v, kdraft_render::page::board_box_of(&self.board));
            let image = self.renders.get(&key).cloned();
            return (image.is_some() || self.kicad).then_some(PlotContent {
                layers: Vec::new(),
                fit: kdraft_plot::fit::PlotTransform {
                    sx: 1.0,
                    ox: 0.0,
                    sy: 1.0,
                    oy: 0.0,
                },
                image,
            });
        }
        self.plots.as_ref().and_then(|p| p.for_view(v))
    }

    /// All pages.
    pub fn render(&self) -> (DocList, Vec<String>) {
        kdraft_render::render_document(
            &self.doc,
            &self.board,
            &self.ctx,
            &|i, _| self.page_sheet(i),
            &|v| self.plot_for(v),
            &self.fonts,
        )
    }

    /// One page.
    pub fn render_page(&self, i: usize) -> Option<(PageList, Vec<String>)> {
        let p = self.doc.pages.get(i)?;
        Some(kdraft_render::page::render_page(
            &self.doc,
            p,
            i,
            &self.board,
            &self.ctx,
            &self.page_sheet(i),
            &|v| self.plot_for(v),
            &self.fonts,
        ))
    }

    /// The drawing sheet of one page only.
    pub fn render_sheet(&self, i: usize) -> Option<(Vec<kdraft_render::dl::Item>, Vec<String>)> {
        let p = self.doc.pages.get(i)?;
        Some(kdraft_render::page::render_sheet(
            &self.doc,
            p,
            i,
            &self.board,
            &self.ctx,
            &self.page_sheet(i),
            &self.fonts,
        ))
    }

    /// One item of the document.
    pub fn render_item(
        &self,
        item: &kdraft_doc::Item,
    ) -> (Vec<kdraft_render::dl::Item>, Vec<String>) {
        kdraft_render::page::render_item(
            &self.doc,
            item,
            &self.board,
            &self.ctx,
            &|v| self.plot_for(v),
            &self.fonts,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Pdf,
    Svg,
    Png { dpi: u32 },
}

pub struct ExportOptions {
    pub out: PathBuf,
    pub format: Format,
    pub kicad: Option<Kicad>,
    pub sheet_from_kicad: bool,
    pub cache: PathBuf,
}

pub struct Exported {
    pub files: Vec<PathBuf>,
    pub pages: usize,
    pub notes: Vec<String>,
}

/// Renders the session's document with kicad-cli plots where available and
/// writes PDF, one SVG per page, or one PNG per page.
pub fn export(s: &mut Session, o: &ExportOptions) -> R<Exported> {
    let (mut inputs, mut notes) = RenderInputs::from_session(s, o.kicad.as_ref().map(|k| &k.cli))?;
    if let Some(k) = &o.kicad {
        let bp = s
            .board_state()
            .path
            .clone()
            .ok_or_else(|| EngineError::invalid("no board path"))?;
        let src = FileBoardSource::new(
            &bp,
            k.cli.clone(),
            Arc::clone(&k.runner),
            Cache::new(&o.cache),
        );
        inputs.plots =
            fetch_plots(&needed_layers(s.doc()), &src, &inputs.board, &mut notes).map(Arc::new);
        let bb = kdraft_render::page::board_box_of(&inputs.board);
        let r = fetch_renders(s.doc(), bb, bp.parent(), &src, &mut notes);
        if !r.is_empty() {
            let mut all = (*s.renders).clone();
            all.extend(r);
            s.renders = Arc::new(all);
            inputs.renders = Arc::clone(&s.renders);
        }
        let has_bom = s.doc().pages.iter().any(|p| {
            p.items.iter().any(
                |i| matches!(i, kdraft_doc::Item::Table(t) if t.kind == kdraft_doc::TableKind::Bom),
            )
        });
        if has_bom && s.schematic_bom().is_none() {
            let b = fetch_bom(&bp, k, &o.cache, &mut notes);
            s.set_bom(b.clone());
            inputs.ctx.bom = b;
        }
        if o.sheet_from_kicad {
            inputs.sheet_svgs = kicad_sheets(s, k, &o.cache, &mut notes)?;
        }
    }
    let mut r = export_inputs(&inputs, &o.out, o.format)?;
    notes.append(&mut r.notes);
    r.notes = notes;
    Ok(r)
}

/// Renders prepared inputs and writes them (no kicad-cli runs; usable on a
/// worker thread).
pub fn export_inputs(inputs: &RenderInputs, out: &Path, format: Format) -> R<Exported> {
    let (list, notes) = inputs.render();
    let files = write_output(&list, &inputs.fonts, out, format)?;
    Ok(Exported {
        files,
        pages: list.pages.len(),
        notes,
    })
}

fn write_output(list: &DocList, fonts: &Fonts, out: &Path, format: Format) -> R<Vec<PathBuf>> {
    let write = |f: &Path, bytes: &[u8]| {
        std::fs::write(f, bytes).map_err(|e| EngineError::io("writing", f, e))
    };
    let per_page = |ext: &str| {
        let stem = out
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "page".to_string());
        let dir = out.parent().map(Path::to_path_buf).unwrap_or_default();
        (0..list.pages.len())
            .map(|i| dir.join(format!("{stem}-p{}.{ext}", i + 1)))
            .collect::<Vec<_>>()
    };
    match format {
        Format::Pdf => {
            write(out, &kdraft_render::pdf::write_pdf(list, fonts)?)?;
            Ok(vec![out.to_path_buf()])
        }
        Format::Svg => {
            let files = per_page("svg");
            for (p, f) in list.pages.iter().zip(&files) {
                write(f, kdraft_render::svg_out::page_svg(p, fonts).as_bytes())?;
            }
            Ok(files)
        }
        Format::Png { dpi } => {
            let files = per_page("png");
            let ppm = f64::from(dpi) / 25.4;
            for (p, f) in list.pages.iter().zip(&files) {
                let png = kdraft_render::raster::page_png(p, ppm, fonts)?;
                write(f, &png)?;
            }
            Ok(files)
        }
    }
}
