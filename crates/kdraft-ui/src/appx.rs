//! App actions behind the catalogue: editor actions, enablement, file
//! dialogs, tool placements, background checks and the schematic BOM.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use kdraft_doc::{Item, TableKind};
use kdraft_engine::pipeline::{self, Format};
use serde_json::json;

use crate::actions::{AppAct, Entry, Run};
use crate::app::{App, Dock, KicadState, Palette, RenameTarget, UiAction};
use crate::dialogs::DialogKind;
use crate::jobs::{CliJob, IoJob};
use crate::theme::ThemePref;
use crate::tools::{Placement, Tool, ToolKind};

/// Wait after the last change before the drawing checks run.
pub const CHECK_DELAY: Duration = Duration::from_millis(600);

impl App {
    /// Native file dialogs on or off (off: typed-path prompt).
    pub fn use_native_dialogs(&mut self, on: bool) {
        self.dialogs.native = on;
    }

    /// Whether a catalogue entry can run now.
    pub fn enabled(&self, e: &Entry) -> bool {
        let has_doc = self.s.current_page_index().is_some();
        let n_sel = self.s.selection.items.len();
        match &e.run {
            Run::Cmd { name, .. } => {
                if !has_doc {
                    return false;
                }
                if name.starts_with("item.") || name.starts_with("layout.") {
                    if matches!(*name, "item.add_view" | "item.add_table") {
                        return self.s.board().is_some();
                    }
                    if n_sel == 0 && kdraft_engine::enable::min_items(name).is_none() {
                        return false;
                    }
                }
                match *name {
                    "page.remove" => self.s.doc().pages.len() > 1,
                    "dimension.board_size" => self.s.board().is_some(),
                    _ => kdraft_engine::enable::can_run(&self.s, name).is_ok(),
                }
            }
            Run::App(a) => {
                use AppAct as A;
                match a {
                    A::Open
                    | A::Settings
                    | A::OpenBoard
                    | A::Palette
                    | A::Shortcuts
                    | A::About
                    | A::DocStyles
                    | A::ToggleTheme => true,
                    // Without a selection the pick asks for the item first.
                    A::LinkPick => has_doc,
                    A::LayerPages => self.s.board().is_some(),
                    A::RenumberCallouts => self
                        .s
                        .current_page()
                        .is_some_and(|p| p.items.iter().any(|i| matches!(i, Item::Callout(_)))),
                    A::ToggleGrid
                    | A::ToggleRulers
                    | A::ToggleSnap
                    | A::ToggleLog
                    | A::ToggleIssues => true,
                    A::Undo => self.s.can_undo(),
                    A::Redo => self.s.can_redo(),
                    A::FitSelection | A::Rename => n_sel > 0,
                    A::ToggleHidden | A::ToggleLocked => n_sel > 0,
                    A::ReloadBoard => self.s.board_state().path.is_some(),
                    A::MovePageUp => self.s.current_page_index().is_some_and(|i| i > 0),
                    A::MovePageDown => self
                        .s
                        .current_page_index()
                        .is_some_and(|i| i + 1 < self.s.doc().pages.len()),
                    A::Tool(ToolKind::Select | ToolKind::Pan) => has_doc,
                    // Drawing tools need no board (note lists are drawn
                    // through the render pipeline, which needs one).
                    A::Tool(
                        ToolKind::Text
                        | ToolKind::Line
                        | ToolKind::Rect
                        | ToolKind::Polyline
                        | ToolKind::Circle,
                    ) => has_doc,
                    A::Tool(_) => has_doc && self.s.board().is_some(),
                    _ => has_doc,
                }
            }
        }
    }

    pub fn entry(&self, id: &str) -> Option<Entry> {
        self.catalogue.get(id).cloned()
    }

    /// Runs a catalogue entry (if enabled).
    pub fn run_entry(&mut self, e: &Entry) {
        if !self.enabled(e) {
            return;
        }
        match &e.run {
            Run::Cmd { name, args } => {
                // Insert menu: views and tables are placed with a tool.
                self.cmd(name, args.clone());
            }
            Run::App(a) => self.app_act(*a),
        }
    }

    pub fn run_id(&mut self, id: &str) {
        if let Some(e) = self.entry(id) {
            self.run_entry(&e);
        }
    }

    pub fn app_act(&mut self, a: AppAct) {
        use AppAct as A;
        match a {
            A::Open => self.ask(DialogKind::Open),
            A::OpenBoard => self.ask(DialogKind::OpenBoard),
            A::Save => self.run(UiAction::Save),
            A::SaveAs => self.ask(DialogKind::SaveAs),
            A::ExportPdf => self.ask(DialogKind::ExportPdf),
            A::ExportPng => self.ask(DialogKind::ExportPng),
            A::ExportSvg => self.ask(DialogKind::ExportSvg),
            A::ChooseSheet => self.ask(DialogKind::ChooseSheet),
            A::ReloadBoard => self.run(UiAction::ReloadBoard),
            A::CloseDocument => self.close_document(),
            A::Undo => self.run(UiAction::Undo),
            A::Redo => self.run(UiAction::Redo),
            A::ZoomIn => self.run(UiAction::ZoomBy(1.25)),
            A::ZoomOut => self.run(UiAction::ZoomBy(0.8)),
            A::ZoomActual => self.zoom_to(1.0),
            A::FitPage => self.run(UiAction::FitPage),
            A::FitSelection => self.run(UiAction::FitSelection),
            A::ToggleGrid => {
                self.settings.show_grid = !self.settings.show_grid;
                self.save_settings();
            }
            A::ToggleRulers => {
                self.settings.show_rulers = !self.settings.show_rulers;
                self.save_settings();
            }
            A::ToggleSnap => self.run(UiAction::ToggleSnap),
            A::ToggleTheme => {
                let dark = self.applied_dark.unwrap_or(false);
                self.set_theme(if dark {
                    ThemePref::Light
                } else {
                    ThemePref::Dark
                });
            }
            A::Palette => {
                self.palette = match self.palette {
                    Some(_) => None,
                    None => Some(Palette::default()),
                }
            }
            A::Shortcuts => self.show_shortcuts = !self.show_shortcuts,
            A::DocStyles => self.show_doc_styles = !self.show_doc_styles,
            A::LinkPick => self.start_link_pick(),
            A::LayerPages => self.open_layer_pages(),
            A::Settings => {
                if self.prefs.is_some() {
                    self.prefs = None;
                } else {
                    self.open_prefs(crate::ui_prefs::PrefsSection::General);
                }
            }
            A::RenumberCallouts => {
                self.renumber = Some(crate::ui_callout::RenumberDialog::default());
            }
            A::About => self.show_about = true,
            A::ToggleLog => self.toggle_dock(Dock::Log),
            A::ToggleIssues => self.toggle_dock(Dock::Issues),
            A::PrevPage => self.run(UiAction::SelectPageOffset(-1)),
            A::NextPage => self.run(UiAction::SelectPageOffset(1)),
            A::MovePageUp => self.move_current_page(-1),
            A::MovePageDown => self.move_current_page(1),
            A::RenamePage => {
                if let Some(i) = self.s.current_page_index() {
                    self.renaming = Some((RenameTarget::Page(i), true));
                }
            }
            A::Rename => {
                if let Some(id) = self.s.selection.items.first().cloned() {
                    self.renaming = Some((RenameTarget::Item(id), true));
                }
            }
            A::ToggleHidden => {
                let hide = !self.selected_items().iter().all(|i| i.hidden());
                self.cmd("item.set_hidden", json!({ "hidden": hide }));
            }
            A::ToggleLocked => {
                let lock = !self.selected_items().iter().all(|i| i.locked());
                self.cmd("item.set_locked", json!({ "locked": lock }));
            }
            A::Tool(k) => self.set_tool(k),
        }
    }

    fn toggle_dock(&mut self, d: Dock) {
        self.dock = if self.dock == Some(d) { None } else { Some(d) };
    }

    /// Zoom so that 1 mm is `f` times its size on a 96 dpi screen.
    pub fn zoom_to(&mut self, f: f64) {
        let target = f * 96.0 / 25.4;
        let c = kdraft_board::geom::Point::new(self.viewport.0 / 2.0, self.viewport.1 / 2.0);
        self.camera.zoom_at(c, target / self.camera.zoom);
        self.note_zoom_change();
    }

    /// Zoom in percent (100 % = true size on a 96 dpi screen).
    pub fn zoom_percent(&self) -> f64 {
        self.camera.zoom / (96.0 / 25.4) * 100.0
    }

    pub fn set_tool(&mut self, k: ToolKind) {
        self.then_tool = None;
        // Dimensions and callouts pick anchors of a board view: on a page
        // without one, place a fabrication view first, then continue.
        let has_view = self
            .s
            .current_page()
            .is_some_and(|p| p.views().any(|v| v.kind != kdraft_doc::ViewKind::Render3d));
        let k = if k.picks_anchors() && !has_view && self.s.board().is_some() {
            self.then_tool = Some(k);
            self.notify("This page has no board view: click to place a fabrication view, then the tool continues");
            ToolKind::View(crate::tools::ViewTool::Fabrication)
        } else {
            k
        };
        self.tool = Tool::new(k);
        self.snapped = None;
        if let Some(g) = crate::ui_canvas::tool_group(k) {
            self.group_tool.insert(g, k);
        }
        if !k.is_select() && !matches!(k, ToolKind::Pan) && !self.drag.is_idle() {
            for a in self.drag.cancel() {
                self.drag_action(a, None);
            }
        }
    }

    pub fn close_document(&mut self) {
        match kdraft_engine::Session::empty() {
            Ok(s) => {
                self.s = s;
                self.session_changed();
            }
            Err(e) => self.error(format!("close: {e}")),
        }
    }

    /// Opens a file dialog, or the typed-path prompt without native ones.
    pub fn ask(&mut self, kind: DialogKind) {
        let doc = self.s.doc_path().map(Path::to_path_buf);
        let initial: Option<PathBuf> = match kind {
            DialogKind::Open | DialogKind::OpenBoard | DialogKind::ChooseSheet => doc
                .clone()
                .or_else(|| self.settings.recent.first().cloned()),
            DialogKind::SaveAs => doc.clone(),
            DialogKind::ExportPdf => doc.as_ref().map(|d| d.with_extension("pdf")),
            DialogKind::ExportPng => doc.as_ref().map(|d| d.with_extension("png")),
            DialogKind::ExportSvg => doc.as_ref().map(|d| d.with_extension("svg")),
            DialogKind::PaletteExport => Some(PathBuf::from(format!(
                "{}.json",
                self.prefs_state.palette.replace(['/', '\\', ':'], "_")
            ))),
            _ => None,
        };
        if !self.dialogs.request(kind, initial.clone()) && self.dialogs.open.is_none() {
            let text = match kind {
                DialogKind::Open
                | DialogKind::OpenBoard
                | DialogKind::ChooseSheet
                | DialogKind::KicadCli
                | DialogKind::KicadTheme
                | DialogKind::PaletteImport
                | DialogKind::DefaultSheet => String::new(),
                _ => initial.map(|p| p.display().to_string()).unwrap_or_default(),
            };
            self.prompt = Some((kind, text));
        }
    }

    /// A path chosen in a dialog or typed into the prompt.
    pub fn dialog_done(&mut self, kind: DialogKind, p: PathBuf) {
        match kind {
            DialogKind::Open | DialogKind::OpenBoard => self.run(UiAction::OpenPath(p)),
            DialogKind::SaveAs => {
                let p = if p.extension().is_none() {
                    p.with_extension("kdraft")
                } else {
                    p
                };
                self.run(UiAction::SaveAs(p));
            }
            DialogKind::ExportPdf => self.export(p, Format::Pdf),
            DialogKind::ExportPng => self.export(p, Format::Png { dpi: 150 }),
            DialogKind::ExportSvg => self.export(p, Format::Svg),
            DialogKind::KicadCli
            | DialogKind::KicadTheme
            | DialogKind::PaletteImport
            | DialogKind::PaletteExport
            | DialogKind::DefaultSheet => self.prefs_dialog_done(kind, p),
            DialogKind::ChooseSheet => {
                self.cmd(
                    "page.set_sheet",
                    json!({ "sheet": p.display().to_string() }),
                );
            }
        }
    }

    pub(crate) fn poll_dialogs(&mut self) {
        for (kind, r) in self.dialogs.results() {
            if let Some(p) = r {
                self.dialog_done(kind, p);
            }
        }
    }

    pub fn export(&mut self, path: PathBuf, format: Format) {
        self.refresh_inputs();
        match self.inputs.clone() {
            Some(inputs) => {
                self.info(format!("exporting {}", path.display()));
                self.io.submit(
                    "export",
                    IoJob::Export {
                        inputs,
                        path,
                        format,
                    },
                );
            }
            None => self.error("export: no board loaded"),
        }
    }

    /// Starts the schematic BOM export when the document has BOM tables
    /// and kicad-cli is available.
    pub(crate) fn request_bom(&mut self) {
        if self.bom_running {
            return;
        }
        let KicadState::Found(cli) = &self.kicad else {
            return;
        };
        let (Some(board), Some(bp)) = (self.s.board(), self.s.board_state().path.clone()) else {
            return;
        };
        let hash = board.hash.0;
        if self.bom.as_ref().is_some_and(|(h, _)| *h == hash) {
            return;
        }
        let wants = self.s.doc().pages.iter().any(|p| {
            p.items.iter().any(|i| {
                matches!(i, Item::Table(t) if t.kind == TableKind::Bom)
                    || matches!(i, Item::Callout(_))
            })
        });
        if !wants {
            return;
        }
        let cache = pipeline::cache_dir(Some(self.s.doc_path().unwrap_or(&bp)));
        self.bom_running = true;
        self.cli.submit(
            "bom",
            CliJob::Bom {
                cli: cli.clone(),
                board_path: bp,
                board_hash: hash,
                cache,
            },
        );
    }

    /// Runs the drawing checks in the background a moment after the last
    /// change.
    pub(crate) fn request_check(&mut self) {
        if self.s.current_page_index().is_none() || self.s.in_gesture() {
            return;
        }
        let rev = self.s.revision();
        let done = self.issues.as_ref().is_some_and(|(r, _)| *r == rev);
        if done || self.issues_running == Some(rev) {
            return;
        }
        match self.issues_due {
            None => self.issues_due = Some(Instant::now() + CHECK_DELAY),
            Some(t) if Instant::now() >= t && self.issues_running.is_none() => {
                self.issues_due = None;
                self.issues_running = Some(rev);
                self.checker.submit("check", self.s.fork());
            }
            _ => {}
        }
    }

    /// Executes a tool placement as one engine command.
    pub fn place(&mut self, pl: Placement) {
        let a = |x: &kdraft_doc::Anchor| kdraft_doc::anchor_text(x).unwrap_or_default();
        let pt = |p: kdraft_board::geom::Point| json!([p.x, p.y]);
        let ok = match pl {
            Placement::View { kind, at } => {
                self.cmd("item.add_view", json!({ "kind": kind.kind(), "at": pt(at) }))
            }
            Placement::Table { kind, at } => {
                self.cmd("item.add_table", json!({ "kind": kind.kind(), "at": pt(at) }))
            }
            Placement::Notes { at } => self.cmd("item.add_notes", json!({ "at": pt(at) })),
            Placement::Text { at } => {
                let ok = self.cmd("item.add_text", json!({ "text": "", "at": pt(at) }));
                if let Some(id) = self.s.selection.items.first().cloned() {
                    self.begin_text_edit(&id, true);
                }
                ok
            }
            Placement::Line { start, end } => {
                self.cmd("item.add_line", json!({ "start": pt(start), "end": pt(end) }))
            }
            Placement::Polyline { points } => {
                let v: Vec<serde_json::Value> = points.iter().map(|q| pt(*q)).collect();
                self.cmd("item.add_polyline", json!({ "points": v }))
            }
            Placement::Circle { center, radius } => self.cmd(
                "item.add_circle",
                json!({ "center": pt(center), "radius": radius }),
            ),
            Placement::Rect { start, end } => {
                self.cmd("item.add_rect", json!({ "start": pt(start), "end": pt(end) }))
            }
            Placement::DimLinear {
                view,
                from,
                to,
                kind,
                offset,
            } => self.cmd(
                "dimension.add_linear",
                json!({ "view": view.0, "from": a(&from), "to": a(&to), "kind": kind.as_str(), "offset": offset }),
            ),
            Placement::DimRadius {
                view,
                anchor,
                diameter,
                angle,
            } => self.cmd(
                "dimension.add_radius",
                json!({ "view": view.0, "anchor": a(&anchor), "diameter": diameter, "angle": angle }),
            ),
            Placement::DimOrdinate {
                view,
                datum,
                to,
                axis,
                offset,
            } => self.cmd(
                "dimension.add_ordinate",
                json!({ "view": view.0, "datum": a(&datum), "to": a(&to), "axis": axis, "offset": offset }),
            ),
            Placement::Callout {
                view,
                anchor,
                at,
                variant,
            } => {
                // Number: the next label; BOM and note variants: the
                // part's line or the first note (else the engine starts a
                // note list).
                self.cmd(
                    "item.add_callout",
                    json!({
                        "view": view.0,
                        "anchor": a(&anchor),
                        "at": pt(at),
                        "kind": variant.kind(),
                    }),
                )
            }
        };
        if ok {
            if let Some(next) = self.then_tool.take() {
                self.set_tool(next);
            } else if !matches!(self.tool.kind, ToolKind::DimOrdinate | ToolKind::Callout(_)) {
                self.set_tool(ToolKind::Select);
            }
        }
    }
}
