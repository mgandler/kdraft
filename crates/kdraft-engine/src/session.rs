//! An editing session: document, board, resolved sheets, selection, undo
//! log and paths.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use kdraft_board::geom::{BBox, Point};
use kdraft_board::{Board, BoardDiff};
use kdraft_doc::{Document, Id, Item, Page};
use kdraft_render::view::ViewMap;
use kdraft_render::Fonts;
use kdraft_sheet::{Project, SheetChoice, SheetOrigin, WksFile};

use crate::bounds::{self, BoundsCtx, BoundsMode};
use crate::error::{EngineError, R};

/// The selection: one page and items on it, in selection order (the first
/// item is the reference for align and match).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Selection {
    pub page: Option<Id>,
    pub items: Vec<Id>,
}

impl Selection {
    pub fn contains(&self, id: &Id) -> bool {
        self.items.contains(id)
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "page": self.page.as_ref().map(|p| p.as_str()),
            "items": self.items.iter().map(|i| i.as_str()).collect::<Vec<_>>(),
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    pub doc: Arc<Document>,
    pub selection: Selection,
    pub rev: u64,
}

#[derive(Debug, Clone)]
struct Entry {
    title: String,
    before: Snapshot,
    after: Snapshot,
}

/// Maximum undo depth.
pub const UNDO_LIMIT: usize = 500;

#[derive(Debug, Clone)]
struct Gesture {
    title: String,
    base: Snapshot,
}

/// The board the document refers to.
#[derive(Debug, Clone, Default)]
pub struct BoardState {
    /// Resolved board file path.
    pub path: Option<PathBuf>,
    pub board: Option<Arc<Board>>,
    /// Why the board could not be loaded.
    pub error: Option<String>,
}

pub struct Session {
    doc: Arc<Document>,
    rev: u64,
    next_rev: u64,
    saved_rev: u64,
    pub selection: Selection,
    doc_path: Option<PathBuf>,
    board: BoardState,
    project: Option<(Project, PathBuf)>,
    sheets: Vec<(SheetChoice, Arc<WksFile>)>,
    fonts: Arc<Fonts>,
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    gesture: Option<Gesture>,
    /// The schematic BOM, once fetched through kicad-cli (`None`: the
    /// PCB fallback is used).
    bom: Option<Arc<kdraft_board::bom::Bom>>,
    /// Style copied with `style.copy` (not part of the document).
    pub style_clip: Option<kdraft_doc::style::ItemStyle>,
    /// 3D view pictures and their progress (not part of the document).
    pub renders: Arc<kdraft_render::render3d::Renders>,
    /// kicad-cli is known to be available (3D views wait for pictures
    /// rather than reporting that kicad-cli is missing).
    pub kicad_available: bool,
    /// User defaults for new pages and items (paper, 3D options); from
    /// the user settings.
    pub defaults: kdraft_source::settings::DocDefaults,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("doc_path", &self.doc_path)
            .field("board", &self.board.path)
            .field("pages", &self.doc.pages.len())
            .field("rev", &self.rev)
            .finish()
    }
}

/// Board path as stored in a document: relative to the document's folder
/// if both can be resolved, otherwise as given. Always `/`-separated.
pub fn relative_board_path(board: &Path, doc: &Path) -> String {
    let doc_dir = match doc.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let (Ok(b), Ok(d)) = (
        std::fs::canonicalize(board),
        std::fs::canonicalize(&doc_dir),
    ) else {
        return board.to_string_lossy().replace('\\', "/");
    };
    let b = strip_verbatim(&b);
    let d = strip_verbatim(&d);
    let bc: Vec<Component> = b.components().collect();
    let dc: Vec<Component> = d.components().collect();
    let common = bc.iter().zip(&dc).take_while(|(x, y)| x == y).count();
    if common == 0 {
        // Different roots (e.g. drives on Windows).
        return b.to_string_lossy().replace('\\', "/");
    }
    let mut parts: Vec<String> = vec!["..".to_string(); dc.len() - common];
    parts.extend(
        bc[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

/// `canonicalize` on Windows returns `\\?\C:\...`; written into a document
/// that prefix is noise.
fn strip_verbatim(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => p.to_path_buf(),
    }
}

fn dir_of(path: Option<&Path>) -> PathBuf {
    match path.and_then(Path::parent) {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

fn load_project(board: &Path) -> Option<(Project, PathBuf)> {
    let p = Project::for_board(board)?;
    let proj = Project::load(&p).ok()?;
    Some((proj, p.parent().map(Path::to_path_buf).unwrap_or_default()))
}

impl Session {
    fn with_doc(doc: Document, doc_path: Option<PathBuf>, fonts: Arc<Fonts>) -> Session {
        Session {
            doc: Arc::new(doc),
            rev: 0,
            next_rev: 1,
            saved_rev: 0,
            selection: Selection::default(),
            doc_path,
            board: BoardState::default(),
            project: None,
            sheets: Vec::new(),
            fonts,
            undo: Vec::new(),
            redo: Vec::new(),
            gesture: None,
            bom: None,
            style_clip: None,
            renders: Arc::new(Default::default()),
            kicad_available: false,
            defaults: Default::default(),
        }
    }

    /// Everything that follows a command (and every gesture preview):
    /// placement links settle, then callouts follow their views.
    pub fn after_edit(&mut self, before: &Document) {
        let has_links = |d: &Document| {
            d.pages
                .iter()
                .flat_map(|p| p.items.iter())
                .any(|i| i.link().is_some())
        };
        if has_links(&self.doc) || has_links(before) {
            let mut d = (*self.doc).clone();
            if crate::links::settle(before, &mut d) {
                self.doc = Arc::new(d);
            }
        }
        self.follow_views(before);
    }

    /// Callouts follow their view: when a command moved, scaled, rotated or
    /// mirrored a view, the callouts anchored to it that the command did
    /// not move themselves keep their place relative to the view.
    pub fn follow_views(&mut self, before: &Document) {
        let bb = self.board_box();
        let mut changed: Vec<(Id, ViewMap, ViewMap)> = Vec::new();
        for p in &self.doc.pages {
            for v in p.views() {
                let old = before
                    .pages
                    .iter()
                    .flat_map(|q| q.views())
                    .find(|o| o.id == v.id);
                if let Some(o) = old {
                    if o.at != v.at
                        || o.scale != v.scale
                        || o.rotation != v.rotation
                        || o.side != v.side
                        || o.crop != v.crop
                    {
                        changed.push((v.id.clone(), ViewMap::new(o, bb), ViewMap::new(v, bb)));
                    }
                }
            }
        }
        if changed.is_empty() {
            return;
        }
        let old_at = |id: &Id| {
            before
                .pages
                .iter()
                .flat_map(|p| p.items.iter())
                .find_map(|i| match i {
                    Item::Callout(c) if &c.id == id => Some((c.at, c.leader.clone())),
                    _ => None,
                })
        };
        let mut doc = (*self.doc).clone();
        let mut any = false;
        for p in &mut doc.pages {
            for it in &mut p.items {
                let Item::Callout(c) = it else { continue };
                let Some(vid) = c.view.clone() else { continue };
                let Some((_, om, nm)) = changed.iter().find(|(i, _, _)| *i == vid) else {
                    continue;
                };
                // Moved by the command itself (selected with the view).
                if old_at(&c.id).is_none_or(|(at, leader)| at != c.at || leader != c.leader) {
                    continue;
                }
                let Some(inv) = om.board_to_sheet.inverse() else {
                    continue;
                };
                let f = |q: Point| nm.point(inv.apply(q));
                c.at = f(c.at);
                for l in &mut c.leader {
                    *l = f(*l);
                }
                any = true;
            }
        }
        if any {
            self.set_doc(doc);
        }
    }

    /// Uses the schematic BOM (or, with `None`, the PCB fallback).
    pub fn set_bom(&mut self, bom: Option<Arc<kdraft_board::bom::Bom>>) {
        self.bom = bom;
    }

    /// The schematic BOM if fetched.
    pub fn schematic_bom(&self) -> Option<Arc<kdraft_board::bom::Bom>> {
        self.bom.clone()
    }

    /// The BOM in use: the schematic one, else derived from the board
    /// (empty without a board).
    pub fn effective_bom(&self) -> Arc<kdraft_board::bom::Bom> {
        match (&self.bom, self.board()) {
            (Some(b), _) => Arc::clone(b),
            (None, Some(b)) => Arc::new(kdraft_board::bom::from_board(b)),
            (None, None) => Arc::new(kdraft_board::bom::Bom {
                source: kdraft_board::bom::BomSource::Pcb,
                lines: Vec::new(),
                extra_columns: Vec::new(),
            }),
        }
    }

    /// A copy of the document, board and sheets without undo history,
    /// for work on another thread (drawing checks).
    pub fn fork(&self) -> Session {
        Session {
            doc: Arc::clone(&self.doc),
            rev: self.rev,
            next_rev: self.next_rev,
            saved_rev: self.saved_rev,
            selection: self.selection.clone(),
            doc_path: self.doc_path.clone(),
            board: self.board.clone(),
            project: self.project.clone(),
            sheets: self.sheets.clone(),
            fonts: Arc::clone(&self.fonts),
            undo: Vec::new(),
            redo: Vec::new(),
            gesture: None,
            bom: self.bom.clone(),
            style_clip: self.style_clip.clone(),
            renders: Arc::clone(&self.renders),
            kicad_available: self.kicad_available,
            defaults: self.defaults.clone(),
        }
    }

    /// A session with an empty document and no board (before a document
    /// is opened or created).
    pub fn empty() -> R<Session> {
        let fonts = Arc::new(Fonts::load()?);
        Ok(Session::with_doc(Document::new(""), None, fonts))
    }

    /// A session over an in-memory document; the board is loaded from
    /// `board_path` if given (errors are kept, not returned).
    pub fn from_document(
        doc: Document,
        doc_path: Option<PathBuf>,
        board_path: Option<PathBuf>,
    ) -> R<Session> {
        let fonts = Arc::new(Fonts::load()?);
        let mut s = Session::with_doc(doc, doc_path, fonts);
        let board_path = board_path.or_else(|| s.resolve_board_path());
        s.set_board_path(board_path);
        s.select_first_page();
        Ok(s)
    }

    /// A new document for a board (as `kdraft doc new`): three pages on
    /// A3, sheet chosen from the project or the default. `doc_path` is
    /// where it will be saved (the board reference is made relative to
    /// it); it is not written.
    pub fn new_for_board(board_path: &Path, doc_path: Option<&Path>) -> R<Session> {
        Session::new_for_board_bom(board_path, doc_path, None)
    }

    /// [`Session::new_for_board`] with the schematic BOM (fetched by the
    /// caller through kicad-cli), so `doc new` and export number the BOM
    /// from the same source.
    pub fn new_for_board_bom(
        board_path: &Path,
        doc_path: Option<&Path>,
        bom: Option<Arc<kdraft_board::bom::Bom>>,
    ) -> R<Session> {
        Session::new_for_board_with(board_path, doc_path, bom, &Default::default(), None)
    }

    /// [`Session::new_for_board_bom`] with the user's defaults: paper and
    /// orientation, drawing sheet (when the project sets none), snap grid,
    /// callout numbering, 3D options and the layer `palette` (stored
    /// resolved in the document).
    pub fn new_for_board_with(
        board_path: &Path,
        doc_path: Option<&Path>,
        bom: Option<Arc<kdraft_board::bom::Bom>>,
        defaults: &kdraft_source::settings::DocDefaults,
        palette: Option<kdraft_doc::palette::Palette>,
    ) -> R<Session> {
        let board = Board::load(board_path)?;
        let project = load_project(board_path);
        let (mut choice, origin) = kdraft_sheet::select_sheet(
            None,
            None,
            Path::new("."),
            project.as_ref().map(|(p, d)| (p, d.as_path())),
        );
        // The user's default sheet replaces KiCad's default only.
        let mut doc_sheet = None;
        if let (Some(f), kdraft_sheet::SheetChoice::Default) = (&defaults.sheet, &choice) {
            if f.is_file() {
                choice = kdraft_sheet::SheetChoice::File(f.clone());
                doc_sheet = Some(f.to_string_lossy().replace('\\', "/"));
            }
        }
        let _ = origin;
        let sheet = kdraft_sheet::load(&choice)?;
        let board_ref = match doc_path {
            Some(d) => relative_board_path(board_path, d),
            None => board_path.to_string_lossy().replace('\\', "/"),
        };
        let size = crate::commands::paper(&defaults.paper, defaults.portrait, None, None)
            .unwrap_or_else(|_| kdraft_doc::PageSize::a3_landscape());
        let mut doc =
            Document::new_for_board_sized(&board, &board_ref, &sheet, bom.as_deref(), size);
        doc.sheet = doc_sheet;
        doc.styles.snap_grid = defaults.snap_grid;
        doc.styles.callouts_per_document = defaults.callouts_per_document;
        doc.styles.layer_palette = palette.filter(|p| !p.is_black());
        let fonts = Arc::new(Fonts::load()?);
        let mut s = Session::with_doc(doc, doc_path.map(Path::to_path_buf), fonts);
        s.board = BoardState {
            path: Some(board_path.to_path_buf()),
            board: Some(Arc::new(board)),
            error: None,
        };
        s.project = project;
        s.sheets.push((choice, Arc::new(sheet)));
        s.bom = bom;
        s.defaults = defaults.clone();
        // Never saved: dirty until written.
        s.saved_rev = u64::MAX;
        s.select_first_page();
        Ok(s)
    }

    /// Opens a `.kdraft` file and its board. A board that fails to load is
    /// reported in [`Session::board_state`], not as an error.
    pub fn open(path: &Path) -> R<Session> {
        let text =
            std::fs::read_to_string(path).map_err(|e| EngineError::io("reading", path, e))?;
        let doc = Document::parse(&text)?;
        Session::from_document(doc, Some(path.to_path_buf()), None)
    }

    fn resolve_board_path(&self) -> Option<PathBuf> {
        let b = self.doc.source.board.trim();
        if b.is_empty() {
            return None;
        }
        Some(dir_of(self.doc_path.as_deref()).join(b.replace('\\', "/")))
    }

    fn set_board_path(&mut self, path: Option<PathBuf>) {
        self.board = BoardState {
            path: path.clone(),
            ..Default::default()
        };
        self.project = None;
        self.sheets.clear();
        if let Some(p) = path {
            match Board::load(&p) {
                Ok(b) => self.board.board = Some(Arc::new(b)),
                Err(e) => self.board.error = Some(format!("{}: {e}", p.display())),
            }
            self.project = load_project(&p);
        }
    }

    fn select_first_page(&mut self) {
        self.selection = Selection {
            page: self.doc.pages.first().map(|p| p.id.clone()),
            items: Vec::new(),
        };
    }

    // ------------------------------------------------------------ access

    pub fn doc(&self) -> &Document {
        &self.doc
    }

    pub fn doc_arc(&self) -> Arc<Document> {
        Arc::clone(&self.doc)
    }

    /// Mutable document for commands (copy on write).
    pub fn doc_mut(&mut self) -> &mut Document {
        Arc::make_mut(&mut self.doc)
    }

    pub fn set_doc(&mut self, d: Document) {
        self.doc = Arc::new(d);
    }

    pub fn doc_path(&self) -> Option<&Path> {
        self.doc_path.as_deref()
    }

    pub fn set_doc_path(&mut self, p: Option<PathBuf>) {
        self.doc_path = p;
    }

    pub fn board_state(&self) -> &BoardState {
        &self.board
    }

    pub fn board(&self) -> Option<&Board> {
        self.board.board.as_deref()
    }

    pub fn board_arc(&self) -> Option<Arc<Board>> {
        self.board.board.clone()
    }

    pub fn fonts(&self) -> &Fonts {
        &self.fonts
    }

    pub fn fonts_arc(&self) -> Arc<Fonts> {
        Arc::clone(&self.fonts)
    }

    pub fn project(&self) -> Option<&(Project, PathBuf)> {
        self.project.as_ref()
    }

    /// Unsaved changes.
    pub fn dirty(&self) -> bool {
        self.rev != self.saved_rev
    }

    /// Revision of the current document state; changes with every edit,
    /// undo and redo.
    pub fn revision(&self) -> u64 {
        self.rev
    }

    pub fn page_index(&self, id: &Id) -> Option<usize> {
        self.doc.pages.iter().position(|p| &p.id == id)
    }

    /// Index of the selected page; the first page if none is selected.
    pub fn current_page_index(&self) -> Option<usize> {
        self.selection
            .page
            .as_ref()
            .and_then(|p| self.page_index(p))
            .or(if self.doc.pages.is_empty() {
                None
            } else {
                Some(0)
            })
    }

    pub fn current_page(&self) -> Option<&Page> {
        self.current_page_index().map(|i| &self.doc.pages[i])
    }

    /// Page index and item of an id.
    pub fn find_item(&self, id: &Id) -> Option<(usize, &Item)> {
        self.doc
            .pages
            .iter()
            .enumerate()
            .find_map(|(i, p)| p.item(id).map(|it| (i, it)))
    }

    // ------------------------------------------------------------ geometry

    pub fn bounds_ctx(&self, mode: BoundsMode) -> BoundsCtx<'_> {
        BoundsCtx {
            bom: self.bom.as_deref(),
            doc: Some(&self.doc),
            board: self.board(),
            fonts: &self.fonts,
            styles: &self.doc.styles,
            mode,
            renders: Some(&self.renders),
        }
    }

    pub fn item_bounds(&self, item: &Item) -> Option<BBox> {
        bounds::item_bounds(item, &self.bounds_ctx(BoundsMode::Content))
    }

    /// Precise hit test of an item at sheet point `p` (see
    /// [`bounds::item_hit`]).
    pub fn item_hit(&self, item: &Item, p: kdraft_board::geom::Point, tol: f64) -> bool {
        bounds::item_hit(item, &self.bounds_ctx(BoundsMode::Content), p, tol)
    }

    /// Sheet geometry of a dimension as drawn.
    pub fn dimension_geometry(&self, d: &kdraft_doc::Dimension) -> Option<bounds::DimGeom> {
        bounds::dimension_geometry(d, &self.bounds_ctx(BoundsMode::Content))
    }

    /// The board box views are placed by.
    pub fn board_box(&self) -> BBox {
        bounds::board_box(self.board())
    }

    /// The drawing sheet of a page with where the choice came from; falls
    /// back to the default sheet when the chosen file cannot be loaded
    /// (the message says why).
    pub fn sheet_for_page(&mut self, index: usize) -> (Arc<WksFile>, SheetOrigin, Option<String>) {
        let Some(page) = self.doc.pages.get(index) else {
            return (self.default_sheet(), SheetOrigin::Default, None);
        };
        let doc_dir = dir_of(self.doc_path.as_deref());
        let (choice, origin) = kdraft_sheet::select_sheet(
            page.sheet.as_deref(),
            self.doc.sheet.as_deref(),
            &doc_dir,
            self.project.as_ref().map(|(p, d)| (p, d.as_path())),
        );
        if let Some((_, w)) = self.sheets.iter().find(|(c, _)| *c == choice) {
            return (Arc::clone(w), origin, None);
        }
        match kdraft_sheet::load(&choice) {
            Ok(w) => {
                let w = Arc::new(w);
                self.sheets.push((choice, Arc::clone(&w)));
                (w, origin, None)
            }
            Err(e) => (
                self.default_sheet(),
                SheetOrigin::Default,
                Some(format!(
                    "drawing sheet {choice:?} not loaded ({e}); using the default"
                )),
            ),
        }
    }

    fn default_sheet(&mut self) -> Arc<WksFile> {
        if let Some((_, w)) = self.sheets.iter().find(|(c, _)| *c == SheetChoice::Default) {
            return Arc::clone(w);
        }
        // The built-in sheet is a compile-time asset; parsing it is tested.
        let w = Arc::new(kdraft_sheet::default_sheet().unwrap_or_default());
        self.sheets.push((SheetChoice::Default, Arc::clone(&w)));
        w
    }

    /// Drawing area of a page (inside the frame, clear of the title block).
    pub fn drawing_area(&mut self, index: usize) -> Option<BBox> {
        let size = self.doc.pages.get(index)?.size.clone();
        let (w, _, _) = self.sheet_for_page(index);
        Some(kdraft_doc::drawing_area(&w, &size))
    }

    // ------------------------------------------------------------ files

    /// Writes the document (to `path`, or its own path) and marks it clean.
    pub fn save(&mut self, path: Option<&Path>) -> R<PathBuf> {
        // Keeps the board reference valid when saving to another folder.
        let (target, text, rev) = self.prepare_save(path)?;
        std::fs::write(&target, text).map_err(|e| EngineError::io("writing", &target, e))?;
        self.mark_saved(target.clone(), rev);
        Ok(target)
    }

    /// The text and target of a save, for writing on another thread; call
    /// [`Session::mark_saved`] when the write succeeded. The board
    /// reference is first made relative to the target folder (not an undo
    /// step).
    pub fn prepare_save(&mut self, path: Option<&Path>) -> R<(PathBuf, String, u64)> {
        let target = match path {
            Some(p) => p.to_path_buf(),
            None => self
                .doc_path
                .clone()
                .ok_or_else(|| EngineError::invalid("document has no file name; give a path"))?,
        };
        if let (Some(bp), true) = (
            self.board.path.clone(),
            Some(&target) != self.doc_path.as_ref(),
        ) {
            let r = relative_board_path(&bp, &target);
            if r != self.doc.source.board {
                self.doc_mut().source.board = r;
            }
        }
        // The written copy records the board hash and the current anchor
        // positions of dimensions (drawn there if an anchor breaks later).
        if let Some(b) = self.board_arc() {
            let mut out = (*self.doc).clone();
            out.source.board_hash = Some(b.hash.to_hex());
            kdraft_doc::anchor::refresh_dimension_positions(&mut out, &b);
            // BOM item numbers of the source in use are stored (kept for
            // later updates; numbered from scratch for a new source).
            let bom = self.effective_bom();
            kdraft_doc::store_bom_numbers(&mut out, &bom);
            if out != *self.doc {
                *self.doc_mut() = out;
            }
        }
        Ok((target, self.doc.to_text()?, self.rev))
    }

    /// Marks revision `rev` as saved to `path`.
    pub fn mark_saved(&mut self, path: PathBuf, rev: u64) {
        self.doc_path = Some(path);
        self.saved_rev = rev;
    }

    /// Re-reads the board file. Returns the semantic change against the
    /// previous model (`None` if there was none loaded).
    pub fn reload_board(&mut self) -> R<Option<BoardDiff>> {
        let path = self
            .board
            .path
            .clone()
            .ok_or_else(|| EngineError::invalid("no board path"))?;
        match Board::load(&path) {
            Ok(b) => Ok(self.replace_board(b)),
            Err(e) => {
                self.board.error = Some(format!("{}: {e}", path.display()));
                Err(e.into())
            }
        }
    }

    /// Installs a board model loaded elsewhere (a worker thread) for the
    /// same path; returns the change against the previous model.
    pub fn replace_board(&mut self, new: Board) -> Option<BoardDiff> {
        let diff = self
            .board
            .board
            .as_ref()
            .map(|old| kdraft_board::diff(old, &new));
        self.board.board = Some(Arc::new(new));
        self.board.error = None;
        if let Some(p) = self.board.path.clone() {
            self.project = load_project(&p);
        }
        self.sheets.clear();
        diff
    }

    /// Records that loading the board failed (from a worker thread).
    pub fn set_board_error(&mut self, e: String) {
        self.board.error = Some(e);
    }

    /// Replaces this session's document and paths with another session's
    /// (document open/new); history is cleared.
    pub(crate) fn replace_with(&mut self, other: Session) {
        *self = other;
    }

    // ------------------------------------------------------------ history

    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot {
            doc: Arc::clone(&self.doc),
            selection: self.selection.clone(),
            rev: self.rev,
        }
    }

    pub(crate) fn restore(&mut self, s: &Snapshot) {
        self.doc = Arc::clone(&s.doc);
        self.selection = s.selection.clone();
        self.rev = s.rev;
        self.sanitize_selection();
    }

    /// Records the change from `before` to the current state as one undo
    /// step, if the document changed. Returns whether it did.
    pub(crate) fn record(&mut self, title: &str, before: Snapshot) -> bool {
        if Arc::ptr_eq(&before.doc, &self.doc) || *before.doc == *self.doc {
            self.doc = Arc::clone(&before.doc);
            return false;
        }
        self.rev = self.next_rev;
        self.next_rev += 1;
        let after = self.snapshot();
        self.undo.push(Entry {
            title: title.to_string(),
            before,
            after,
        });
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_title(&self) -> Option<&str> {
        self.undo.last().map(|e| e.title.as_str())
    }

    pub fn redo_title(&self) -> Option<&str> {
        self.redo.last().map(|e| e.title.as_str())
    }

    /// Undoes the last step; restores the document and the selection from
    /// before it. Returns the step's title.
    pub fn undo(&mut self) -> Option<String> {
        self.cancel_gesture();
        let e = self.undo.pop()?;
        self.restore(&e.before);
        let t = e.title.clone();
        self.redo.push(e);
        Some(t)
    }

    /// Redoes the last undone step, restoring the document and selection
    /// from after it.
    pub fn redo(&mut self) -> Option<String> {
        self.cancel_gesture();
        let e = self.redo.pop()?;
        self.restore(&e.after);
        let t = e.title.clone();
        self.undo.push(e);
        Some(t)
    }

    /// Starts an interactive gesture (a drag): commands run with
    /// [`crate::Registry::preview`] replace each other until
    /// [`Session::commit_gesture`] records one undo step.
    pub fn begin_gesture(&mut self, title: &str) {
        self.cancel_gesture();
        self.gesture = Some(Gesture {
            title: title.to_string(),
            base: self.snapshot(),
        });
    }

    pub fn in_gesture(&self) -> bool {
        self.gesture.is_some()
    }

    /// A new revision for a previewed state, so caches keyed by revision
    /// see every frame of a gesture.
    pub(crate) fn touch(&mut self) {
        self.rev = self.next_rev;
        self.next_rev += 1;
    }

    /// Resets the document to the gesture's start (before each preview).
    pub(crate) fn gesture_reset(&mut self) -> R<()> {
        let g = self
            .gesture
            .as_ref()
            .ok_or(EngineError::NoGesture)?
            .base
            .clone();
        self.restore(&g);
        Ok(())
    }

    /// Ends the gesture as one undo step (none if nothing changed).
    pub fn commit_gesture(&mut self) -> bool {
        let Some(g) = self.gesture.take() else {
            return false;
        };
        // The selection after the gesture is kept as is.
        self.record(&g.title, g.base)
    }

    /// Ends the gesture and restores the state from its start.
    pub fn cancel_gesture(&mut self) {
        if let Some(g) = self.gesture.take() {
            self.restore(&g.base);
        }
    }

    /// Drops selected ids that no longer exist; keeps the page if it does.
    pub fn sanitize_selection(&mut self) {
        let page_ok = self
            .selection
            .page
            .as_ref()
            .is_some_and(|p| self.page_index(p).is_some());
        if !page_ok {
            self.selection.page = self.doc.pages.first().map(|p| p.id.clone());
        }
        let pi = self.current_page_index();
        let doc = Arc::clone(&self.doc);
        self.selection
            .items
            .retain(|id| pi.is_some_and(|i| doc.pages[i].item(id).is_some()));
    }
}
