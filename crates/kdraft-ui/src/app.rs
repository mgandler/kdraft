//! Editor state and the actions behind every button and shortcut. All
//! document changes go through engine commands; this file only routes.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use kdraft_board::geom::{BBox, Point};
use kdraft_doc::{Id, Item};
use kdraft_engine::pipeline::{self, Plots, RenderInputs};
use kdraft_engine::snap::Guide;
use kdraft_engine::{Registry, Session};
use kdraft_source::KicadCli;
use serde_json::{json, Value};

use crate::camera::{self, Camera};
use crate::drag::Drag;
use crate::jobs::{self, CliJob, CliOut, IoJob, IoOut, Key, RasterJob, RasterOut, RasterWhat};
use crate::watch::{self, Stamp, Watch};
use crate::worker::Worker;

/// Snap tolerance on screen, pixels (converted to mm at the current zoom).
pub const SNAP_TOLERANCE_PX: f64 = 6.0;
/// Pointer travel before a press becomes a drag, pixels.
pub const DRAG_THRESHOLD_PX: f64 = 4.0;
/// Zoom must rest this long before textures are re-rendered for it.
pub const ZOOM_SETTLE: Duration = Duration::from_millis(250);
/// Thumbnail width, pixels.
pub const THUMB_WIDTH: f64 = 160.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub level: Level,
    pub text: String,
}

/// A texture on the GPU with what it was made from.
pub struct Tex {
    pub key: Key,
    pub bucket: i32,
    pub handle: egui::TextureHandle,
    pub region: BBox,
    pub at: Option<Point>,
}

#[derive(Debug, Clone)]
pub enum KicadState {
    Searching,
    Found(KicadCli),
    Missing(String),
}

/// A typed-path prompt (when native file dialogs are off).
pub type Prompt = crate::dialogs::DialogKind;

/// Everything a button or key can do.
#[derive(Debug, Clone, PartialEq)]
pub enum UiAction {
    /// Run an engine command.
    Cmd(&'static str, Value),
    Undo,
    Redo,
    Save,
    Prompt(Prompt),
    /// Open a .kdraft or create a document for a .kicad_pcb.
    OpenPath(PathBuf),
    SaveAs(PathBuf),
    ExportPdf(PathBuf),
    FitPage,
    FitSelection,
    ZoomBy(f64),
    ToggleSnap,
    SelectPageOffset(i32),
    /// Re-read the board file (also done automatically on change).
    ReloadBoard,
}

type BoxKey = (u64, Option<[u8; 32]>, Option<usize>);
type LiveCache = (
    Option<(u64, Option<[u8; 32]>, Key)>,
    HashMap<Id, Vec<kdraft_render::dl::Item>>,
);

/// Items drawn as live vectors on the canvas (no cached texture):
/// dimensions, callouts and free graphics.
pub fn is_live(it: &Item) -> bool {
    matches!(it, Item::Dimension(_) | Item::Callout(_) | Item::Graphic(_))
}
type RankedBoxes = Vec<(Id, BBox, u8)>;
type DimCache = (
    Option<(u64, Option<[u8; 32]>)>,
    HashMap<Id, Option<kdraft_engine::bounds::DimGeom>>,
);

/// Board hash and layer set the current plots were made for.
type PlotsFor = ([u8; 32], Vec<kdraft_engine::pipeline::LayerNeed>);

pub struct App {
    pub reg: Registry,
    pub s: Session,
    pub camera: Camera,
    /// Page the camera was last fitted for.
    pub camera_page: Option<Id>,
    pub fit_pending: bool,
    pub viewport: (f64, f64),
    pub drag: Drag,
    pub guides: Vec<Guide>,
    pub marquee: Option<BBox>,
    pub snap: bool,
    pub cursor_mm: Option<Point>,
    /// Cursor shape set over the canvas in the last frame.
    pub canvas_cursor: Option<crate::hit::CursorKind>,
    /// Alt held: corner-handle scaling does not snap to standard scales.
    pub free_scale: bool,
    /// Pages panel: click/drag state and entry rectangles of the last
    /// frame (screen points, `[x0, y0, x1, y1]`).
    pub page_drag: crate::pages::PageDrag,
    pub page_entries: Vec<[f32; 4]>,
    /// The visible part of the page list in the last frame, same form.
    pub page_list: [f32; 4],
    pub messages: Vec<Message>,
    pub banner: Option<String>,
    pub prompt: Option<(Prompt, String)>,
    pub busy_open: bool,
    pub(crate) render: Worker<RasterJob, RasterOut>,
    pub(crate) io: Worker<IoJob, IoOut>,
    pub(crate) cli: Worker<CliJob, CliOut>,
    /// Drawing checks on a forked session.
    pub(crate) checker: Worker<Session, (u64, kdraft_engine::check::Report)>,
    pub textures: HashMap<String, Tex>,
    pub inputs: Option<Arc<RenderInputs>>,
    inputs_for: Option<(u64, Key)>,
    pub inputs_gen: Key,
    pub kicad: KicadState,
    pub plots: Option<Arc<Plots>>,
    /// Board the session's 3D pictures belong to.
    renders_board: Option<[u8; 32]>,
    /// 3D render passes running, by picture key.
    pub renders_running: std::collections::HashSet<String>,
    pub plots_gen: u64,
    plots_for: Option<PlotsFor>,
    pub plots_running: bool,
    sheet_gen: u64,
    last_zoom_change: Instant,
    pub pixels_per_point: f64,
    watched: Arc<Mutex<Option<PathBuf>>>,
    stamps: Option<Receiver<(PathBuf, Option<Stamp>)>>,
    watch: Watch,
    watch_path: Option<PathBuf>,
    /// Gesture being dragged in the properties panel.
    pub prop_drag: Option<&'static str>,
    /// Document styles window open.
    pub show_doc_styles: bool,
    /// Handle drag in progress.
    pub edit_drag: Option<crate::ui_handles::EditDrag>,
    /// Selected vertex of a polyline or polygon (Delete removes it).
    pub sel_vertex: Option<(Id, usize)>,
    /// Shift held this frame (canvas).
    pub shift_held: bool,
    /// Items waiting for a reference pick ("Link placement to...").
    /// Picking for "Link placement to...": the items to link; empty
    /// while the user still picks the item to link.
    pub link_pick: Option<Vec<Id>>,
    /// Tool to continue with after placing a view (an anchor tool chosen
    /// on a page without a board view).
    pub then_tool: Option<crate::tools::ToolKind>,
    /// The layer pages window, when open.
    pub layer_pages: Option<crate::ui_link::LayerPagesDialog>,
    /// The in-place text editor, when open.
    pub text_edit: Option<crate::ui_textedit::TextEdit>,
    /// In-place editor of a callout's label.
    pub label_edit: Option<crate::ui_callout::LabelEdit>,
    /// The "Renumber callouts" window.
    pub renumber: Option<crate::ui_callout::RenumberDialog>,
    pub doc_style_kind: kdraft_doc::style::StyleKind,
    /// Colour attribute being previewed from the full picker.
    pub color_live: Option<(crate::ui_style::Target, &'static str)>,
    pub color_pending: Option<kdraft_doc::style::Rgba>,
    pub edit_title: Option<String>,
    pub edit_sheet: Option<String>,
    pub dragging_page: Option<usize>,
    pub move_ctx: Option<crate::movectx::MoveCtx>,
    /// Screen position of the canvas origin (top-left), points.
    pub canvas_origin: Point,
    boxes_cache: Mutex<Option<(BoxKey, RankedBoxes)>>,
    dim_cache: Mutex<DimCache>,
    live_cache: Mutex<LiveCache>,
    /// Largest texture side of the renderer (from egui's input).
    pub max_texture_side: f64,
    /// Dimension offset drag: id, offset at the start, geometry.
    pub dim_drag: Option<(Id, f64, kdraft_engine::bounds::DimGeom)>,
    pub settings: crate::settings::Settings,
    /// Where settings are saved; `None` keeps them in memory (tests).
    pub settings_path: Option<PathBuf>,
    /// The shared settings (kicad, documents, palettes) as last loaded or
    /// edited in the settings page.
    pub user: kdraft_source::settings::SettingsFile,
    /// `user` changed and must be written with the next save.
    pub shared_dirty: bool,
    /// A debounced settings save is due at this time.
    pub settings_due: Option<Instant>,
    /// Where the kicad-cli in use came from.
    pub kicad_source: Option<kdraft_source::CliSource>,
    /// Installs found (Settings > KiCad), when listed.
    pub kicad_installs: Option<Vec<kdraft_source::Install>>,
    /// Result of the Test button.
    pub kicad_test: Option<(PathBuf, Result<String, String>)>,
    /// The settings page, when open, and its section.
    pub prefs: Option<crate::ui_prefs::PrefsSection>,
    /// Settings page state (palette selection, buffers, last import).
    pub prefs_state: crate::ui_prefs::PrefsState,
    /// Cache folder size, when measured.
    pub cache_size: Option<(PathBuf, u64)>,
    /// Cameras of the current document's pages (page id), restored when
    /// a page is shown again and saved with the settings.
    pub page_cams: HashMap<String, crate::settings::PageCam>,
    /// The saved window placement was checked against the monitor.
    pub window_checked: bool,
    /// Theme currently applied (`None` before the first frame).
    pub applied_dark: Option<bool>,
    pub catalogue: crate::actions::Catalogue,
    pub tool: crate::tools::Tool,
    /// Last tool used per tool-bar group (dropdown carets).
    pub group_tool: HashMap<&'static str, crate::tools::ToolKind>,
    /// Anchor under the pointer while a tool picks anchors.
    pub snapped: Option<crate::tools::Pick>,
    pub anchors: crate::anchors::AnchorCache,
    pub palette: Option<Palette>,
    pub show_shortcuts: bool,
    pub show_about: bool,
    pub dock: Option<Dock>,
    pub toasts: Vec<Toast>,
    /// Inline rename in progress, and whether it starts this frame.
    pub renaming: Option<(RenameTarget, bool)>,
    /// Item tree rows of the current page, by document revision.
    pub tree_cache: Option<(u64, Option<Id>, Vec<crate::tree::Row>)>,
    pub tree_anchor: Option<usize>,
    pub tree_drag: Option<usize>,
    /// Item tree rows of the last frame (screen rects), for tests.
    pub tree_rects: Vec<(Id, [f32; 4])>,
    /// Inspector sections closed by the user.
    pub closed_sections: std::collections::HashSet<&'static str>,
    pub layer_filter: String,
    pub layers_expanded: bool,
    /// Item under the pointer on the canvas.
    pub hover_item: Option<Id>,
    /// Item-tree row under the pointer (highlighted on the canvas).
    pub tree_hover: Option<Id>,
    /// Alt held: measurement overlay.
    pub measure_on: bool,
    /// Layer names of the board, by board hash.
    pub layer_cache: Option<([u8; 32], Vec<String>)>,
    /// Drawing checks of a document revision.
    pub issues: Option<(u64, kdraft_engine::check::Report)>,
    pub(crate) issues_running: Option<u64>,
    pub(crate) issues_due: Option<Instant>,
    /// Schematic BOM for a board hash (`None` inside: PCB fallback).
    pub bom: Option<([u8; 32], Option<Arc<kdraft_board::bom::Bom>>)>,
    pub(crate) bom_running: bool,
    pub bom_gen: u64,
    /// File dialog results.
    pub(crate) dialogs: crate::dialogs::Dialogs,
}

/// Command palette state.
#[derive(Debug, Clone, Default)]
pub struct Palette {
    pub query: String,
    pub selected: usize,
    pub focused: bool,
}

/// The bottom dock: drawing checks or the message log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dock {
    Issues,
    Log,
}

/// A transient message.
#[derive(Debug, Clone)]
pub struct Toast {
    pub level: Level,
    pub text: String,
    pub born: Instant,
}

impl Toast {
    pub fn ttl(&self) -> Duration {
        match self.level {
            Level::Info => Duration::from_millis(3500),
            Level::Warning => Duration::from_millis(6000),
            Level::Error => Duration::from_millis(8000),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RenameTarget {
    Item(Id),
    Page(usize),
}

/// UI fonts: the bundled IBM Plex Sans (OFL) instead of egui's default
/// fonts, whose Ubuntu Font Licence part is not clearly GPL-compatible.
pub fn install_fonts(ctx: &egui::Context) {
    let mut f = egui::FontDefinitions::empty();
    for (name, data) in [
        ("plex", kdraft_render::font::REGULAR),
        ("plex-bold", kdraft_render::font::BOLD),
    ] {
        f.font_data.insert(
            name.to_string(),
            Arc::new(egui::FontData::from_static(data)),
        );
    }
    f.families
        .insert(egui::FontFamily::Proportional, vec!["plex".to_string()]);
    f.families
        .insert(egui::FontFamily::Monospace, vec!["plex".to_string()]);
    f.families.insert(
        crate::theme::bold(),
        vec!["plex-bold".to_string(), "plex".to_string()],
    );
    ctx.set_fonts(f);
}

fn waker(ctx: &egui::Context) -> Arc<dyn Fn() + Send + Sync> {
    let c = ctx.clone();
    Arc::new(move || c.request_repaint())
}

impl App {
    /// The editor over a session; workers start immediately. `watch`
    /// starts the board polling thread (off in tests).
    pub fn new(ctx: &egui::Context, s: Session, watch_files: bool) -> App {
        install_fonts(ctx);
        let w = waker(ctx);
        let watched = Arc::new(Mutex::new(None::<PathBuf>));
        let stamps = watch_files.then(|| {
            let (tx, rx) = channel();
            let watched = Arc::clone(&watched);
            let w = Arc::clone(&w);
            let _ = std::thread::Builder::new()
                .name("kdraft-watch".into())
                .spawn(move || loop {
                    std::thread::sleep(Duration::from_millis(watch::POLL_MS));
                    let p = watched.lock().map(|g| g.clone()).unwrap_or(None);
                    if let Some(p) = p {
                        let st = watch::stamp(&p);
                        if tx.send((p, st)).is_err() {
                            return;
                        }
                        w();
                    }
                });
            rx
        });
        let cli = Worker::new("kdraft-cli", 1, jobs::run_cli, Arc::clone(&w));
        cli.submit(
            "discover",
            CliJob::Discover {
                choice: Default::default(),
                vars: Vec::new(),
            },
        );
        let mut app = App {
            reg: Registry::new(),
            s,
            camera: Camera::default(),
            camera_page: None,
            fit_pending: true,
            viewport: (800.0, 600.0),
            drag: Drag::default(),
            guides: Vec::new(),
            marquee: None,
            snap: true,
            cursor_mm: None,
            canvas_cursor: None,
            free_scale: false,
            page_drag: Default::default(),
            page_entries: Vec::new(),
            page_list: [0.0; 4],
            messages: Vec::new(),
            banner: None,
            prompt: None,
            busy_open: false,
            render: Worker::new("kdraft-render", 1, jobs::run_raster, Arc::clone(&w)),
            io: Worker::new("kdraft-io", 1, jobs::run_io, Arc::clone(&w)),
            cli,
            checker: Worker::new(
                "kdraft-check",
                1,
                |mut s: Session| (s.revision(), kdraft_engine::check::check(&mut s)),
                Arc::clone(&w),
            ),
            textures: HashMap::new(),
            inputs: None,
            inputs_for: None,
            inputs_gen: [0; 32],
            kicad: KicadState::Searching,
            plots: None,
            plots_gen: 0,
            plots_for: None,
            plots_running: false,
            renders_board: None,
            renders_running: Default::default(),
            sheet_gen: 0,
            last_zoom_change: Instant::now(),
            pixels_per_point: 1.0,
            watched,
            stamps,
            watch: Watch::default(),
            watch_path: None,
            prop_drag: None,
            show_doc_styles: false,
            edit_drag: None,
            sel_vertex: None,
            shift_held: false,
            link_pick: None,
            then_tool: None,
            layer_pages: None,
            text_edit: None,
            label_edit: None,
            renumber: None,
            doc_style_kind: kdraft_doc::style::StyleKind::Graphic,
            color_live: None,
            color_pending: None,
            edit_title: None,
            edit_sheet: None,
            dragging_page: None,
            move_ctx: None,
            canvas_origin: Point::ORIGIN,
            boxes_cache: Mutex::new(None),
            dim_cache: Mutex::new((None, HashMap::new())),
            dim_drag: None,
            max_texture_side: 2048.0,
            live_cache: Mutex::new((None, HashMap::new())),
            settings: crate::settings::Settings::defaults(),
            settings_path: None,
            user: Default::default(),
            shared_dirty: false,
            settings_due: None,
            kicad_source: None,
            kicad_installs: None,
            kicad_test: None,
            prefs: None,
            prefs_state: Default::default(),
            cache_size: None,
            page_cams: HashMap::new(),
            window_checked: false,
            applied_dark: None,
            catalogue: crate::actions::Catalogue::default(),
            tool: crate::tools::Tool::default(),
            group_tool: HashMap::new(),
            snapped: None,
            anchors: Default::default(),
            palette: None,
            show_shortcuts: false,
            show_about: false,
            dock: None,
            toasts: Vec::new(),
            renaming: None,
            tree_cache: None,
            tree_anchor: None,
            tree_drag: None,
            tree_rects: Vec::new(),
            closed_sections: Default::default(),
            layer_filter: String::new(),
            layers_expanded: false,
            hover_item: None,
            tree_hover: None,
            measure_on: false,
            layer_cache: None,
            issues: None,
            issues_running: None,
            issues_due: None,
            bom: None,
            bom_running: false,
            bom_gen: 0,
            dialogs: crate::dialogs::Dialogs::new(Arc::clone(&w)),
        };
        app.catalogue = crate::actions::Catalogue::build(&app.reg);
        app.session_changed();
        app
    }

    /// Opens a file at start-up (in the background).
    pub fn open_at_start(&mut self, path: PathBuf) {
        self.run(UiAction::OpenPath(path));
    }

    pub fn info(&mut self, t: impl Into<String>) {
        self.push(Level::Info, t.into());
    }

    pub fn warn(&mut self, t: impl Into<String>) {
        self.push(Level::Warning, t.into());
    }

    pub fn error(&mut self, t: impl Into<String>) {
        self.push(Level::Error, t.into());
    }

    /// An info message that is also shown as a toast.
    pub fn notify(&mut self, t: impl Into<String>) {
        let text = t.into();
        self.toast(Level::Info, text.clone());
        self.push_log(Level::Info, text);
    }

    fn toast(&mut self, level: Level, text: String) {
        self.toasts.push(Toast {
            level,
            text,
            born: Instant::now(),
        });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    fn push(&mut self, level: Level, text: String) {
        if level != Level::Info {
            self.toast(level, text.clone());
        }
        self.push_log(level, text);
    }

    fn push_log(&mut self, level: Level, text: String) {
        self.messages.push(Message { level, text });
        if self.messages.len() > 200 {
            self.messages.remove(0);
        }
    }

    /// After a new session: reset view state, caches and the watcher.
    pub(crate) fn session_changed(&mut self) {
        self.textures.clear();
        self.render.clear();
        self.inputs = None;
        self.inputs_for = None;
        self.plots = None;
        self.plots_for = None;
        // The page and camera last used for this document, else fitted.
        self.fit_pending = false;
        self.camera_page = None;
        self.restore_doc_view();
        self.s.defaults = self.user.documents();
        self.drag = Drag::default();
        self.guides.clear();
        self.marquee = None;
        self.edit_title = None;
        self.edit_sheet = None;
        self.tool = crate::tools::Tool::default();
        self.snapped = None;
        self.anchors.invalidate();
        self.tree_cache = None;
        self.renaming = None;
        self.issues = None;
        self.issues_due = Some(Instant::now());
        self.bom = None;
        if let Some(p) = self.s.doc_path().map(Path::to_path_buf) {
            self.settings.add_recent(&p);
            self.save_settings();
        }
        let bp = self.s.board_state().path.clone();
        self.watch = Watch::new(bp.as_deref().and_then(watch::stamp));
        self.watch_path = bp.clone();
        if let Ok(mut g) = self.watched.lock() {
            *g = bp;
        }
        if let Some(e) = self.s.board_state().error.clone() {
            self.error(format!("board not loaded: {e}"));
        }
    }

    // ------------------------------------------------------------ actions

    /// Runs an engine command and reports errors in the message area.
    pub fn cmd(&mut self, name: &str, args: Value) -> bool {
        match self.reg.execute(&mut self.s, name, args) {
            Ok(o) => {
                for m in o.messages {
                    self.info(m);
                }
                true
            }
            Err(e) => {
                self.error(format!("{name}: {e}"));
                false
            }
        }
    }

    pub fn run(&mut self, a: UiAction) {
        match a {
            UiAction::Cmd(name, args) => {
                self.cmd(name, args);
            }
            UiAction::Undo => {
                if let Some(t) = self.s.undo() {
                    self.info(format!("undo: {t}"));
                }
            }
            UiAction::Redo => {
                if let Some(t) = self.s.redo() {
                    self.info(format!("redo: {t}"));
                }
            }
            UiAction::Save => {
                if self.s.doc_path().is_none() {
                    self.ask(crate::dialogs::DialogKind::SaveAs);
                } else {
                    self.save(None);
                }
            }
            UiAction::Prompt(p) => self.ask(p),
            UiAction::OpenPath(p) => self.open(p),
            UiAction::SaveAs(p) => self.save(Some(p)),
            UiAction::ExportPdf(p) => self.export_pdf(p),
            UiAction::FitPage => self.fit_pending = true,
            UiAction::FitSelection => self.fit_selection(),
            UiAction::ZoomBy(f) => {
                let c = Point::new(self.viewport.0 / 2.0, self.viewport.1 / 2.0);
                self.camera.zoom_at(c, f);
                self.last_zoom_change = Instant::now();
            }
            UiAction::ToggleSnap => {
                self.snap = !self.snap;
                self.save_settings();
            }
            UiAction::ReloadBoard => match self.s.board_state().path.clone() {
                Some(p) => self.io.submit("board", IoJob::LoadBoard(p)),
                None => self.error("no board to reload"),
            },
            UiAction::SelectPageOffset(d) => {
                let n = self.s.doc().pages.len() as i64;
                if n > 0 {
                    let i = self.s.current_page_index().unwrap_or(0) as i64;
                    let j = (i + i64::from(d)).clamp(0, n - 1) as usize;
                    let id = self.s.doc().pages[j].id.0.clone();
                    self.cmd("page.select", json!({ "page": id }));
                }
            }
        }
    }

    fn open(&mut self, p: PathBuf) {
        let is_board = p
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("kicad_pcb"));
        self.busy_open = true;
        self.info(format!("opening {}", p.display()));
        self.io.submit(
            "open",
            if is_board {
                IoJob::NewForBoard {
                    board: p,
                    kicad: match &self.kicad {
                        KicadState::Found(c) => Some(c.clone()),
                        _ => None,
                    },
                    defaults: Box::new(self.user.documents()),
                    palette: self.user.default_palette(),
                }
            } else {
                IoJob::Open(p)
            },
        );
    }

    fn save(&mut self, path: Option<PathBuf>) {
        match self.s.prepare_save(path.as_deref()) {
            Ok((path, text, rev)) => self.io.submit("save", IoJob::Save { path, text, rev }),
            Err(e) => self.error(format!("save: {e}")),
        }
    }

    fn export_pdf(&mut self, path: PathBuf) {
        self.export(path, pipeline::Format::Pdf);
    }

    pub fn fit_selection(&mut self) {
        let Some(pi) = self.s.current_page_index() else {
            return;
        };
        let page = &self.s.doc().pages[pi];
        let b = self
            .s
            .selection
            .items
            .iter()
            .filter_map(|id| page.item(id))
            .filter_map(|it| self.s.item_bounds(it))
            .reduce(|a, b| a.union(&b));
        if let Some(b) = b {
            self.camera = Camera::fit(&b, self.viewport.0, self.viewport.1, 40.0);
            self.last_zoom_change = Instant::now();
        }
    }

    pub fn page_rect(&self) -> Option<BBox> {
        let p = self.s.current_page()?;
        Some(BBox {
            min: Point::ORIGIN,
            max: Point::new(p.size.width, p.size.height),
        })
    }

    // ------------------------------------------------------------ background

    /// Applies finished background work. Called once per frame.
    pub fn poll(&mut self, ctx: &egui::Context) {
        if self.settings_due.is_some_and(|d| Instant::now() >= d) {
            self.save_settings();
        }
        for (slot, r) in self.io.results() {
            match r {
                IoOut::Opened(Ok(s)) => {
                    self.busy_open = false;
                    let name = s
                        .doc_path()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();
                    self.s = *s;
                    self.session_changed();
                    self.notify(format!("Opened {name}"));
                }
                IoOut::Opened(Err(e)) => {
                    self.busy_open = false;
                    self.error(format!("open: {e}"));
                }
                IoOut::Saved(Ok((p, rev))) => {
                    self.notify(format!("Saved {}", p.display()));
                    self.settings.add_recent(&p);
                    self.save_settings();
                    self.s.mark_saved(p, rev);
                }
                IoOut::Saved(Err(e)) => self.error(format!("save: {e}")),
                IoOut::Exported(Ok((p, notes))) => {
                    for n in notes {
                        self.warn(n);
                    }
                    self.notify(format!("Exported {}", p.display()));
                }
                IoOut::Exported(Err(e)) => self.error(e),
                IoOut::SettingsWritten(Err(e)) => self.warn(format!("settings: {e}")),
                IoOut::SettingsWritten(Ok(())) => {}
                IoOut::CacheSize(p, n) => self.cache_size = Some((p, n)),
                IoOut::CacheCleared(Ok(p)) => {
                    self.notify(format!("Cache cleared: {}", p.display()));
                    self.cache_size = Some((p, 0));
                }
                IoOut::CacheCleared(Err(e)) => self.error(format!("clear cache: {e}")),
                IoOut::Board(p, r) => {
                    if Some(&p) != self.s.board_state().path.as_ref() {
                        continue;
                    }
                    match r {
                        Ok(b) => {
                            let diff = self.s.replace_board(*b);
                            let text = diff
                                .as_ref()
                                .map_or("board loaded".to_string(), crate::summary::diff_summary);
                            self.banner = Some(format!("Board changed on disk: {text}"));
                            self.info(format!("board reloaded: {text}"));
                        }
                        Err(e) => {
                            self.s.set_board_error(e.clone());
                            self.error(format!("board reload failed: {e}"));
                        }
                    }
                }
            }
            let _ = slot;
        }
        for (_, r) in self.cli.results() {
            match r {
                CliOut::Found(Ok((c, src))) => {
                    self.info(format!(
                        "kicad-cli {} at {} ({})",
                        c.version,
                        c.path.display(),
                        src.label()
                    ));
                    // Another install or other variables: plots and 3D
                    // pictures again (the cache keys include the version).
                    let changed = !matches!(&self.kicad, KicadState::Found(o) if *o == c);
                    self.kicad = KicadState::Found(c);
                    self.kicad_source = Some(src);
                    self.s.kicad_available = true;
                    if changed {
                        self.plots_for = None;
                        self.s.renders = Arc::new(Default::default());
                        self.renders_board = None;
                        self.bom = None;
                    }
                }
                CliOut::Installs(v) => self.kicad_installs = Some(v),
                CliOut::Tested(p, r) => self.kicad_test = Some((p, r)),
                CliOut::Found(Err(e)) => {
                    self.kicad_source = None;
                    self.kicad = KicadState::Missing(e);
                }
                CliOut::Bom {
                    board_hash,
                    bom,
                    notes,
                } => {
                    self.bom_running = false;
                    if self.s.board().map(|b| b.hash.0) == Some(board_hash) {
                        for n in notes {
                            self.info(n);
                        }
                        self.s.set_bom(bom.clone());
                        self.bom = Some((board_hash, bom));
                        self.bom_gen += 1;
                    }
                }
                CliOut::Render3d {
                    board_hash,
                    key,
                    view,
                    board_box,
                    result,
                } => {
                    self.renders_running.remove(&key);
                    if self.s.board().map(|b| b.hash.0) == Some(board_hash) {
                        use kdraft_render::render3d::State;
                        let st = match result {
                            Ok(jobs::Render3dPass::Probe(png)) => {
                                self.submit_render3d(&key, &view, board_box, Some(png));
                                State::Pending("Rendering 3D view, pass 2 of 2...".into())
                            }
                            Ok(jobs::Render3dPass::Done(p)) => State::Ready(p),
                            Err(e) => {
                                self.warn(format!("3D render failed: {e}"));
                                State::Failed(e)
                            }
                        };
                        self.set_render_state(&key, st);
                    }
                }
                CliOut::Plots {
                    board_hash,
                    layers,
                    plots,
                    notes,
                } => {
                    self.plots_running = false;
                    let current = self.s.board().map(|b| b.hash.0);
                    if current == Some(board_hash) {
                        for n in notes
                            .into_iter()
                            .filter(|n| !n.starts_with("plot transform fitted"))
                        {
                            self.warn(n);
                        }
                        self.plots = plots;
                        self.plots_gen += 1;
                        self.plots_for = Some((board_hash, layers));
                    }
                }
            }
        }
        if let Some(rx) = &self.stamps {
            let stamps: Vec<_> = rx.try_iter().collect();
            for (p, st) in stamps {
                if Some(&p) == self.watch_path.as_ref() && self.watch.observe(st) {
                    self.io.submit("board", IoJob::LoadBoard(p));
                }
            }
        }
        for (_, r) in self.checker.results() {
            let (rev, report) = r;
            self.issues_running = None;
            self.issues = Some((rev, report));
        }
        self.poll_dialogs();
        self.request_plots();
        self.request_renders();
        self.request_bom();
        self.request_check();
        self.apply_rasters(ctx);
    }

    fn request_plots(&mut self) {
        let KicadState::Found(cli) = &self.kicad else {
            return;
        };
        let (Some(board), Some(bp)) = (self.s.board_arc(), self.s.board_state().path.clone())
        else {
            return;
        };
        let layers = pipeline::needed_layers(self.s.doc());
        let want = (board.hash.0, layers.clone());
        if self.plots_for.as_ref() == Some(&want) || self.plots_running {
            return;
        }
        if layers.is_empty() {
            return;
        }
        self.plots_running = true;
        let cache = pipeline::cache_dir(Some(self.s.doc_path().unwrap_or(&bp)));
        self.cli.submit(
            "plots",
            CliJob::Plots {
                cli: cli.clone(),
                board_path: bp,
                board,
                layers,
                cache,
            },
        );
    }

    /// Whether kicad-cli was found.
    pub fn kicad_found(&self) -> bool {
        matches!(self.kicad, KicadState::Found(_))
    }

    fn set_render_state(&mut self, key: &str, st: kdraft_render::render3d::State) {
        let mut m = (*self.s.renders).clone();
        m.insert(key.to_string(), st);
        self.s.renders = Arc::new(m);
        self.plots_gen += 1;
    }

    fn submit_render3d(
        &mut self,
        key: &str,
        view: &kdraft_doc::View,
        board_box: kdraft_board::geom::BBox,
        probe: Option<Vec<u8>>,
    ) {
        let KicadState::Found(cli) = &self.kicad else {
            return;
        };
        let (Some(board), Some(bp)) = (self.s.board_arc(), self.s.board_state().path.clone())
        else {
            return;
        };
        let cache = pipeline::cache_dir(Some(self.s.doc_path().unwrap_or(&bp)));
        self.renders_running.insert(key.to_string());
        // One slot per picture: "latest wins" must not drop another
        // view's render.
        self.cli.submit(
            &format!("render3d:{key}"),
            CliJob::Render3d {
                cli: cli.clone(),
                board_path: bp,
                board_hash: board.hash.0,
                key: key.to_string(),
                view: Box::new(view.clone()),
                board_box,
                probe,
                cache,
            },
        );
    }

    /// Starts pass 1 for every 3D view whose picture is not known or
    /// being made (pictures are kept per board).
    fn request_renders(&mut self) {
        if !matches!(self.kicad, KicadState::Found(_)) {
            return;
        }
        let Some(board) = self.s.board_arc() else {
            return;
        };
        if self.renders_board != Some(board.hash.0) {
            self.s.renders = Arc::new(Default::default());
            self.renders_board = Some(board.hash.0);
        }
        let bb = self.s.board_box();
        let views: Vec<kdraft_doc::View> = self
            .s
            .doc()
            .pages
            .iter()
            .flat_map(|p| p.views())
            .filter(|v| v.kind == kdraft_doc::ViewKind::Render3d)
            .cloned()
            .collect();
        for v in views {
            let key = pipeline::render_key(&v, bb);
            if self.s.renders.contains_key(&key) || self.renders_running.contains(&key) {
                continue;
            }
            self.set_render_state(
                &key,
                kdraft_render::render3d::State::Pending("Rendering 3D view, pass 1 of 2...".into()),
            );
            self.submit_render3d(&key, &v, bb, None);
        }
    }

    /// Rebuilds the render inputs when the document, board or plots changed.
    pub fn refresh_inputs(&mut self) {
        let gen = jobs::inputs_key(
            self.s.board(),
            self.plots_gen,
            self.sheet_gen + 1000 * self.bom_gen,
        );
        let want = (self.s.revision(), gen);
        if self.inputs_for == Some(want) && self.inputs.is_some() {
            return;
        }
        if self.inputs_for.map(|(r, _)| r) != Some(self.s.revision()) {
            self.prune_vanished();
        }
        if self.s.board().is_none() {
            self.inputs = None;
            return;
        }
        let kicad = match &self.kicad {
            KicadState::Found(c) => Some(c.clone()),
            _ => None,
        };
        match RenderInputs::from_session(&mut self.s, kicad.as_ref()) {
            Ok((mut i, notes)) => {
                for n in notes {
                    self.warn(n);
                }
                i.plots = self.plots.clone();
                if let Some((h, Some(b))) = &self.bom {
                    if Some(*h) == self.s.board().map(|b| b.hash.0) {
                        i.ctx.bom = Some(Arc::clone(b));
                    }
                }
                self.inputs = Some(Arc::new(i));
                self.inputs_for = Some(want);
                self.inputs_gen = gen;
            }
            Err(e) => {
                self.inputs = None;
                self.error(format!("render: {e}"));
            }
        }
    }

    /// Drops waiting render jobs and textures of pages and items that are
    /// no longer in the document (deleted, undone).
    fn prune_vanished(&mut self) {
        let ids = self.live_ids();
        let gone = |slot: &str| slot_vanished(slot, &ids);
        self.render.cancel_where(gone);
        self.textures.retain(|slot, _| !gone(slot));
    }

    /// The ids of every page and item the document has now.
    fn live_ids(&self) -> HashSet<String> {
        let mut ids = HashSet::new();
        for p in &self.s.doc().pages {
            ids.insert(p.id.0.clone());
            for it in &p.items {
                if let Some(id) = it.id() {
                    ids.insert(id.0.clone());
                }
            }
        }
        ids
    }

    fn apply_rasters(&mut self, ctx: &egui::Context) {
        let results = self.render.results();
        if results.is_empty() {
            return;
        }
        let ids = self.live_ids();
        for (slot, r) in results {
            // A job that was already running when its page or item was
            // deleted renders from the document it started with, so it
            // does not know. Its result is dropped here; otherwise the
            // texture would stay until the next change of the document.
            if slot_vanished(&slot, &ids) {
                self.textures.remove(&slot);
                continue;
            }
            if r.gone {
                // Deleted while the job waited: nothing to show or say.
                self.textures.remove(&slot);
                continue;
            }
            if let Some(e) = &r.error {
                self.error(format!("render {slot}: {e}"));
                continue;
            }
            if r.size[0] == 0 || r.size[1] == 0 {
                self.textures.remove(&slot);
                continue;
            }
            let max = ctx.input(|i| i.max_texture_side);
            if r.size[0] > max || r.size[1] > max {
                // Rendered for a larger limit (cannot happen with the cap
                // in the job); dropped instead of panicking in egui.
                continue;
            }
            let img = egui::ColorImage::from_rgba_premultiplied(r.size, &r.rgba);
            let handle = ctx.load_texture(&slot, img, egui::TextureOptions::LINEAR);
            self.textures.insert(
                slot,
                Tex {
                    key: r.key,
                    bucket: r.bucket,
                    handle,
                    region: r.region,
                    at: r.at,
                },
            );
        }
    }

    /// Asks the render worker for a texture if the cached one is stale.
    pub fn want_texture(&mut self, slot: &str, key: Key, what: RasterWhat, bucket: i32) {
        let Some(inputs) = self.inputs.clone() else {
            return;
        };
        let settled = self.last_zoom_change.elapsed() >= ZOOM_SETTLE;
        let stale = match self.textures.get(slot) {
            None => true,
            Some(t) if t.key != key => true,
            Some(t) => t.bucket != bucket && settled,
        };
        if !stale || self.render.is_pending(slot) {
            return;
        }
        self.render.submit(
            slot,
            RasterJob {
                inputs,
                what,
                key,
                bucket,
                pixels_per_point: self.pixels_per_point,
                thumb_width: THUMB_WIDTH,
                max_side: self.max_texture_side,
            },
        );
    }

    pub fn note_zoom_change(&mut self) {
        self.last_zoom_change = Instant::now();
    }

    pub fn zoom_settled(&self) -> bool {
        self.last_zoom_change.elapsed() >= ZOOM_SETTLE
    }

    /// Boxes of the current page's visible items in z-order, with their
    /// pick rank; cached per document revision and board.
    pub fn item_boxes_ranked(&self) -> Vec<(Id, BBox, u8)> {
        let key = (
            self.s.revision(),
            self.s.board().map(|b| b.hash.0),
            self.s.current_page_index(),
        );
        if let Ok(c) = self.boxes_cache.lock() {
            if let Some((k, v)) = c.as_ref() {
                if *k == key {
                    return v.clone();
                }
            }
        }
        let v: Vec<(Id, BBox, u8)> = match self.s.current_page() {
            None => Vec::new(),
            Some(p) => p
                .items
                .iter()
                .filter(|it| !it.hidden())
                .filter_map(|it| {
                    Some((
                        it.id()?.clone(),
                        self.s.item_bounds(it)?,
                        crate::hit::rank(it),
                    ))
                })
                .collect(),
        };
        if let Ok(mut c) = self.boxes_cache.lock() {
            *c = Some((key, v.clone()));
        }
        v
    }

    /// Boxes of the current page's visible items in z-order.
    pub fn item_boxes(&self) -> Vec<(Id, BBox)> {
        self.item_boxes_ranked()
            .into_iter()
            .map(|(i, b, _)| (i, b))
            .collect()
    }

    /// The item to pick at `p` on the canvas: visible, unlocked, by rank
    /// (annotations, tables, views) and the precise shape test.
    pub fn pick_item(&self, p: Point, tol: f64) -> Option<Id> {
        let page = self.s.current_page()?;
        let cands: Vec<(Id, BBox, u8)> = self
            .item_boxes_ranked()
            .into_iter()
            .filter(|(id, _, _)| page.item(id).is_some_and(|it| !it.locked()))
            .collect();
        crate::hit::hit_ranked(&cands, p, tol, &|id| {
            page.item(id).is_some_and(|it| self.dim_hit(it, p, tol))
        })
    }

    /// Precise hit, with dimension geometry from the cache.
    fn dim_hit(&self, it: &Item, p: Point, tol: f64) -> bool {
        match it {
            Item::Dimension(d) => self.dim_geometry(d).is_some_and(|g| g.hit(p, tol)),
            _ => self.s.item_hit(it, p, tol),
        }
    }

    /// Display list of a live item from the current document, cached per
    /// revision, board and render inputs.
    pub fn live_items(&self, it: &Item) -> Vec<kdraft_render::dl::Item> {
        let (Some(board), Some(id)) = (self.s.board(), it.id()) else {
            return Vec::new();
        };
        let key = (self.s.revision(), Some(board.hash.0), self.inputs_gen);
        if let Ok(c) = self.live_cache.lock() {
            if c.0 == Some(key) {
                if let Some(v) = c.1.get(id) {
                    return v.clone();
                }
            }
        }
        let ctx = self
            .inputs
            .as_ref()
            .map(|i| i.ctx.clone())
            .unwrap_or_else(|| kdraft_engine::pipeline::sheet_context(&self.s, None));
        let (v, _) = kdraft_render::page::render_item(
            self.s.doc(),
            it,
            board,
            &ctx,
            &|_| None,
            self.s.fonts(),
        );
        if let Ok(mut c) = self.live_cache.lock() {
            if c.0 != Some(key) {
                *c = (Some(key), HashMap::new());
            }
            c.1.insert(id.clone(), v.clone());
        }
        v
    }

    /// Sheet geometry of a dimension, cached per revision and board.
    pub fn dim_geometry(
        &self,
        d: &kdraft_doc::Dimension,
    ) -> Option<kdraft_engine::bounds::DimGeom> {
        let key = (self.s.revision(), self.s.board().map(|b| b.hash.0));
        if let Ok(mut c) = self.dim_cache.lock() {
            if c.0 != Some(key) {
                *c = (Some(key), HashMap::new());
            }
            if let Some(g) = c.1.get(&d.id) {
                return g.clone();
            }
            let g = self.s.dimension_geometry(d);
            c.1.insert(d.id.clone(), g.clone());
            return g;
        }
        self.s.dimension_geometry(d)
    }

    pub fn selected_items(&self) -> Vec<&Item> {
        let Some(p) = self.s.current_page() else {
            return Vec::new();
        };
        self.s
            .selection
            .items
            .iter()
            .filter_map(|id| p.item(id))
            .collect()
    }

    pub fn board_status(&self) -> String {
        let st = self.s.board_state();
        match (&st.board, &st.error) {
            (_, Some(e)) => format!("board error: {e}"),
            (Some(b), None) => {
                let h: String = b
                    .hash
                    .0
                    .iter()
                    .take(4)
                    .map(|x| format!("{x:02x}"))
                    .collect();
                let name = st
                    .path
                    .as_deref()
                    .and_then(Path::file_name)
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default();
                format!("board {name} #{h}")
            }
            (None, None) => "no board".to_string(),
        }
    }

    pub fn kicad_status(&self) -> String {
        match &self.kicad {
            KicadState::Searching => "kicad-cli: searching".to_string(),
            KicadState::Found(c) => {
                let p = if self.plots_running {
                    ", plotting"
                } else if self.plots.is_some() {
                    ", plots ready"
                } else {
                    ""
                };
                let src = self.kicad_source.map_or("", |s| match s {
                    kdraft_source::CliSource::Auto => " (auto)",
                    kdraft_source::CliSource::Setting => " (settings)",
                    kdraft_source::CliSource::Env => " (KDRAFT_KICAD_CLI)",
                    kdraft_source::CliSource::Flag => "",
                });
                format!("kicad-cli {}{src}{p}", c.version)
            }
            KicadState::Missing(_) => "kicad-cli: not found (native preview)".to_string(),
        }
    }

    pub fn jobs_status(&self) -> String {
        let n = self.render.busy() + self.io.busy() + self.cli.busy() + self.checker.busy();
        if n == 0 {
            "idle".to_string()
        } else {
            format!("{n} background job(s)")
        }
    }

    /// No background work pending and the zoom settled (headless
    /// screenshots wait for this).
    pub fn idle(&self) -> bool {
        self.render.busy() + self.io.busy() + self.cli.busy() == 0
            && self.zoom_settled()
            && !self.busy_open
    }

    /// Theme by name (`dark`, `light`, `system`).
    pub fn set_theme_name(&mut self, name: &str) {
        if let Some(t) = crate::theme::ThemePref::parse(name) {
            self.set_theme(t);
        }
    }

    /// Changes the theme preference and saves the settings.
    pub fn set_theme(&mut self, t: crate::theme::ThemePref) {
        if self.settings.theme != t {
            self.settings.theme = t;
            self.save_settings();
        }
    }

    /// Uses settings loaded from `path` and saves changes there.
    pub fn with_settings(mut self, path: PathBuf) -> App {
        let l = kdraft_source::settings::SettingsFile::load(&path);
        if let Some(w) = l.warning {
            self.warn(w);
        }
        self.settings = crate::settings::Settings::from_file(&l.settings);
        self.user = l.settings;
        self.snap = self.settings.snap;
        self.settings_path = Some(path);
        self.apply_user_settings();
        self.restore_layout();
        self
    }

    /// Applies the shared settings: kicad-cli choice and variables (found
    /// again in the background), cache folder, document defaults.
    pub fn apply_user_settings(&mut self) {
        let k = self.user.kicad();
        kdraft_engine::pipeline::set_cache_root(k.cache_dir.clone());
        self.s.defaults = self.user.documents();
        self.kicad = KicadState::Searching;
        self.cli.submit(
            "discover",
            CliJob::Discover {
                choice: k.cli,
                vars: k.vars,
            },
        );
    }

    /// Tool variants and dock from the settings.
    fn restore_layout(&mut self) {
        for (_g, id) in self.settings.tools.clone() {
            if let Some(k) = crate::actions::TOOLS
                .iter()
                .find(|t| t.1 == id)
                .map(|t| t.0)
            {
                if let Some(g) = crate::ui_canvas::tool_group(k) {
                    self.group_tool.insert(g, k);
                }
            }
        }
        self.dock = match self.settings.dock.as_deref() {
            Some("issues") => Some(Dock::Issues),
            Some("log") => Some(Dock::Log),
            _ => None,
        };
    }

    /// The layout and view state into `settings`.
    fn capture_settings(&mut self) {
        self.settings.snap = self.snap;
        self.settings.dock = self.dock.map(|d| match d {
            Dock::Issues => "issues".to_string(),
            Dock::Log => "log".to_string(),
        });
        self.settings.tools = self
            .group_tool
            .iter()
            .filter_map(|(g, k)| {
                crate::actions::TOOLS
                    .iter()
                    .find(|t| t.0 == *k)
                    .map(|t| (g.to_string(), t.1.to_string()))
            })
            .collect();
        self.capture_doc_view();
    }

    /// Saves soon (debounced; for layout changes while dragging).
    pub fn save_settings_soon(&mut self) {
        let due = Instant::now() + Duration::from_millis(800);
        self.settings_due = Some(self.settings_due.map_or(due, |d| d.min(due)));
    }

    /// Writes the settings in the background (no-op without a path).
    pub fn save_settings(&mut self) {
        self.settings_due = None;
        self.capture_settings();
        if let Some(p) = &self.settings_path {
            let shared = self.shared_dirty.then(|| self.shared_sections());
            self.shared_dirty = false;
            self.io.submit(
                "settings",
                IoJob::WriteSettings {
                    path: p.clone(),
                    text: self.settings.text(),
                    shared,
                },
            );
        }
    }

    /// Writes the settings now, on this thread (on exit).
    pub fn save_settings_now(&mut self) {
        self.capture_settings();
        if let Some(p) = self.settings_path.clone() {
            let shared = self.shared_dirty.then(|| self.shared_sections());
            self.shared_dirty = false;
            if let Err(e) = crate::settings::write(&p, &self.settings.text(), shared.as_ref()) {
                self.warn(format!("settings: {e}"));
            }
        }
    }

    fn shared_sections(&self) -> serde_json::Map<String, serde_json::Value> {
        ["kicad", "documents", "palettes"]
            .iter()
            .filter_map(|k| self.user.get(k).map(|v| (k.to_string(), v.clone())))
            .collect()
    }

    /// The shared settings changed (settings page): apply and save.
    pub fn user_changed(&mut self) {
        self.shared_dirty = true;
        self.s.defaults = self.user.documents();
        kdraft_engine::pipeline::set_cache_root(self.user.kicad().cache_dir);
        self.save_settings();
    }

    /// Applies the theme for the preference and the system setting when
    /// it changed. Called once per frame.
    pub fn apply_theme(&mut self, ctx: &egui::Context) {
        let sys = ctx.system_theme().map(|t| t == egui::Theme::Dark);
        let dark = self.settings.theme.is_dark(sys);
        if self.applied_dark != Some(dark) {
            crate::theme::apply(ctx, crate::theme::Tokens::for_dark(dark));
            self.applied_dark = Some(dark);
        }
    }

    pub fn tokens(&self) -> crate::theme::Tokens {
        crate::theme::Tokens::for_dark(self.applied_dark.unwrap_or(false))
    }

    /// Runs a UI action by name (screenshot scripts); false if unknown.
    pub fn run_named(&mut self, name: &str) -> bool {
        let a = match name {
            "fit_page" => UiAction::FitPage,
            "fit_selection" => UiAction::FitSelection,
            "undo" => UiAction::Undo,
            "redo" => UiAction::Redo,
            "zoom_in" => UiAction::ZoomBy(1.25),
            "zoom_out" => UiAction::ZoomBy(0.8),
            "clear_toasts" => {
                self.toasts.clear();
                return true;
            }
            "dock_log" => {
                self.dock = Some(Dock::Log);
                return true;
            }
            "dock_issues" => {
                self.dock = Some(Dock::Issues);
                return true;
            }
            "shortcuts" => {
                self.show_shortcuts = true;
                return true;
            }
            "theme_light" | "theme_dark" => {
                self.set_theme_name(&name[6..]);
                return true;
            }
            // Settings page sections and palette helpers for scripts.
            n if n.starts_with("prefs:") => {
                let sec = crate::ui_prefs::PrefsSection::ALL
                    .into_iter()
                    .find(|s| s.label().eq_ignore_ascii_case(&n[6..]));
                return match sec {
                    Some(s) => {
                        self.open_prefs(s);
                        true
                    }
                    None => false,
                };
            }
            n if n.starts_with("import_theme:") => {
                self.import_theme(std::path::Path::new(&n[13..]));
                return true;
            }
            n if n.starts_with("doc_palette:") => {
                let Some(p) = self.user.palette(&n[12..]) else {
                    return false;
                };
                self.cmd(
                    "document.set_layer_palette",
                    serde_json::json!({ "palette": kdraft_source::settings::palette_to_json(&p) }),
                );
                return true;
            }
            // Any menu entry by id ("app.layer_pages").
            _ => match self.entry(name) {
                Some(e) => {
                    self.run_entry(&e);
                    return true;
                }
                None => return false,
            },
        };
        self.run(a);
        true
    }

    /// The bucket the canvas renders at now.
    pub fn bucket(&self) -> i32 {
        camera::zoom_bucket(self.camera.zoom)
    }
}

/// Whether a texture slot belongs to a page or item that is not in `ids`.
fn slot_vanished(slot: &str, ids: &HashSet<String>) -> bool {
    slot.split_once(':')
        .is_some_and(|(kind, id)| matches!(kind, "thumb" | "sheet" | "item") && !ids.contains(id))
}
