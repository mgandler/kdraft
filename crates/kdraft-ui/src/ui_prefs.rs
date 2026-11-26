//! The settings page: a full page in place of the canvas (File >
//! Settings, Ctrl+Comma, the gear in the top strip, the kicad-cli entry in
//! the status bar, the start screen), with sections General, KiCad,
//! Documents, Layer colours and Cache. Changes apply at once and are
//! saved; Close or Escape goes back.

use std::path::{Path, PathBuf};

use eframe::egui::{self, CornerRadius, Stroke, Vec2};
use kdraft_doc::palette::{self as pal, Palette};
use kdraft_doc::style::Rgba;
use kdraft_source::settings::{self as us, CliChoice};
use serde_json::json;

use crate::app::{App, KicadState};
use crate::dialogs::DialogKind;
use crate::jobs::{CliJob, IoJob};
use crate::theme::{text, ThemePref};
use crate::widgets::{self as w, Seg};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefsSection {
    General,
    Kicad,
    Documents,
    LayerColours,
    Cache,
}

impl PrefsSection {
    pub const ALL: [PrefsSection; 5] = [
        PrefsSection::General,
        PrefsSection::Kicad,
        PrefsSection::Documents,
        PrefsSection::LayerColours,
        PrefsSection::Cache,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PrefsSection::General => "General",
            PrefsSection::Kicad => "KiCad",
            PrefsSection::Documents => "Documents",
            PrefsSection::LayerColours => "Layer colours",
            PrefsSection::Cache => "Cache",
        }
    }
}

/// Editing state of the settings page.
#[derive(Debug, Clone)]
pub struct PrefsState {
    /// Palette selected in the Layer colours section.
    pub palette: String,
    /// New variable row.
    pub new_var: (String, String),
    /// Custom kicad-cli path being typed.
    pub custom_cli: String,
    /// Darken imported KiCad colours for white paper.
    pub adapt: bool,
    /// The last KiCad theme imported (for the before/after columns).
    pub last_import: Option<kdraft_source::kicad_theme::Imported>,
    /// KiCad colour themes found (listed when the section opens).
    pub themes: Vec<kdraft_source::kicad_theme::ThemeFile>,
    /// Inner copper rows shown in the palette editor.
    pub inner_rows: usize,
    /// Cache folder being typed.
    pub cache_text: String,
}

impl Default for PrefsState {
    fn default() -> Self {
        PrefsState {
            palette: pal::BLACK.to_string(),
            new_var: Default::default(),
            custom_cli: String::new(),
            adapt: true,
            last_import: None,
            themes: Vec::new(),
            inner_rows: 4,
            cache_text: String::new(),
        }
    }
}

const NAV_W: f32 = 190.0;
const CONTENT_W: f32 = 660.0;

fn heading(ui: &mut egui::Ui, s: &str) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(s).font(text::title()));
    ui.add_space(4.0);
}

fn note(ui: &mut egui::Ui, s: &str) {
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.label(
        egui::RichText::new(s)
            .font(text::small())
            .color(t.text_secondary),
    );
}

impl App {
    /// Opens the settings page at `section`.
    pub fn open_prefs(&mut self, section: PrefsSection) {
        self.prefs = Some(section);
        match section {
            PrefsSection::Kicad => {
                self.cli.submit("installs", CliJob::ListInstalls);
                if let CliChoice::Path(p) = self.user.kicad().cli {
                    self.prefs_state.custom_cli = p.display().to_string();
                }
            }
            PrefsSection::LayerColours => {
                self.prefs_state.themes =
                    kdraft_source::kicad_theme::list_themes(&kdraft_source::SystemEnvironment);
            }
            PrefsSection::Cache => {
                self.prefs_state.cache_text = self
                    .user
                    .kicad()
                    .cache_dir
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                self.measure_cache();
            }
            _ => {}
        }
    }

    /// The cache folder of the current document (or the shared one).
    pub fn cache_folder(&self) -> PathBuf {
        let doc = self
            .s
            .doc_path()
            .map(Path::to_path_buf)
            .or_else(|| self.s.board_state().path.clone());
        kdraft_engine::pipeline::cache_dir(doc.as_deref())
    }

    fn measure_cache(&mut self) {
        let dir = self.cache_folder();
        self.io.submit("cache-size", IoJob::CacheSize(dir));
    }

    /// The page.
    pub(crate) fn prefs_page(&mut self, ui: &mut egui::Ui) {
        let Some(section) = self.prefs else { return };
        let t = self.tokens();
        let full = ui.max_rect();
        ui.painter().rect_filled(full, 0.0, t.panel);
        // Navigation.
        let nav = egui::Rect::from_min_size(full.min, Vec2::new(NAV_W, full.height()));
        ui.painter().rect_filled(nav, 0.0, t.chrome);
        ui.painter()
            .vline(nav.max.x - 0.5, nav.y_range(), Stroke::new(1.0, t.border));
        let mut nav_ui = ui.new_child(egui::UiBuilder::new().max_rect(nav.shrink(12.0)));
        nav_ui.label(egui::RichText::new("Settings").font(text::title()));
        nav_ui.add_space(8.0);
        let mut go = None;
        for s in PrefsSection::ALL {
            let r = nav_ui.add_sized(
                [NAV_W - 24.0, 26.0],
                egui::Button::selectable(s == section, s.label()),
            );
            if r.clicked() {
                go = Some(s);
            }
        }
        nav_ui.add_space(12.0);
        if w::button(
            &mut nav_ui,
            Some(crate::icons::Icon::Close),
            "Close",
            false,
            true,
        )
        .on_hover_text("Back to the drawing (Escape)")
        .clicked()
        {
            self.prefs = None;
            return;
        }
        if let Some(s) = go {
            self.open_prefs(s);
        }
        // Content.
        let content = egui::Rect::from_min_max(
            egui::pos2(nav.max.x + 24.0, full.min.y + 12.0),
            egui::pos2(full.max.x - 12.0, full.max.y),
        );
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(content));
        egui::ScrollArea::vertical()
            .id_salt(("prefs", section.label()))
            .auto_shrink([false, false])
            .show(&mut c, |ui| {
                ui.set_max_width(CONTENT_W.min(content.width() - 12.0));
                ui.spacing_mut().item_spacing.y = 6.0;
                match section {
                    PrefsSection::General => self.prefs_general(ui),
                    PrefsSection::Kicad => self.prefs_kicad(ui),
                    PrefsSection::Documents => self.prefs_documents(ui),
                    PrefsSection::LayerColours => self.prefs_palettes(ui),
                    PrefsSection::Cache => self.prefs_cache(ui),
                }
                ui.add_space(40.0);
            });
    }

    fn prefs_general(&mut self, ui: &mut egui::Ui) {
        heading(ui, "General");
        w::row(ui, "Theme", |ui| {
            let segs = [Seg::text("System"), Seg::text("Light"), Seg::text("Dark")];
            let cur = match self.settings.theme {
                ThemePref::System => 0,
                ThemePref::Light => 1,
                ThemePref::Dark => 2,
            };
            if let Some(k) = w::segmented(ui, "pref-theme", &segs, Some(cur), 220.0) {
                self.set_theme([ThemePref::System, ThemePref::Light, ThemePref::Dark][k]);
            }
        });
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.add_space(w::LABEL_W);
            changed |= ui
                .checkbox(&mut self.settings.show_rulers, "Rulers")
                .changed();
            changed |= ui.checkbox(&mut self.settings.show_grid, "Grid").changed();
            changed |= ui.checkbox(&mut self.snap, "Snap").changed();
        });
        if changed {
            self.save_settings();
        }
        w::row(ui, "Language", |ui| {
            ui.add_enabled_ui(false, |ui| {
                w::dropdown(ui, "pref-lang", "English", &["English"], 160.0);
            });
            note(ui, "more languages later");
        });
        ui.add_space(8.0);
        note(
            ui,
            "Window size and position, panel widths, the dock, the last used tool variants and each document's last page and zoom are remembered on their own.",
        );
        if let Some(p) = &self.settings_path {
            note(ui, &format!("Settings file: {}", p.display()));
        }
        note(
            ui,
            "The settings file holds local paths (recent files, kicad-cli); nothing is sent anywhere.",
        );
    }

    fn set_kicad(&mut self, k: &us::KicadSettings) {
        self.user.set_kicad(k);
        self.user_changed();
        self.apply_user_settings();
    }

    fn prefs_kicad(&mut self, ui: &mut egui::Ui) {
        let t = self.tokens();
        heading(ui, "KiCad");
        let in_use = match (&self.kicad, self.kicad_source) {
            (KicadState::Found(c), src) => format!(
                "In use: kicad-cli {} at {}{}",
                c.version,
                c.path.display(),
                src.map_or(String::new(), |s| format!(" (from {})", s.label()))
            ),
            (KicadState::Searching, _) => "Looking for kicad-cli…".to_string(),
            (KicadState::Missing(e), _) => format!("No kicad-cli: {e}"),
        };
        ui.label(egui::RichText::new(in_use).font(text::body()));
        if std::env::var_os(kdraft_source::ENV_VAR).is_some() {
            note(
                ui,
                "KDRAFT_KICAD_CLI is set: it wins over the choice below.",
            );
        }
        ui.add_space(4.0);
        let mut k = self.user.kicad();
        let mut new_choice: Option<CliChoice> = None;
        ui.label(egui::RichText::new("kicad-cli").strong());
        if ui
            .radio(k.cli == CliChoice::Auto, "Auto (newest install)")
            .clicked()
        {
            new_choice = Some(CliChoice::Auto);
        }
        match &self.kicad_installs {
            None => note(ui, "Looking for installs…"),
            Some(v) if v.is_empty() => {
                note(ui, "No install found on PATH or in the standard folders.")
            }
            Some(v) => {
                for i in v.clone() {
                    let label = match &i.cli {
                        Some(c) => format!("KiCad {}  ·  {}", c.version, i.path.display()),
                        None => format!(
                            "{}  ({})",
                            i.path.display(),
                            i.error.as_deref().unwrap_or("no version")
                        ),
                    };
                    let on = k.cli == CliChoice::Path(i.path.clone());
                    if ui.radio(on, label).clicked() {
                        new_choice = Some(CliChoice::Path(i.path.clone()));
                    }
                }
            }
        }
        let custom = matches!(&k.cli, CliChoice::Path(p) if !self.kicad_installs.as_ref().is_some_and(|v| v.iter().any(|i| &i.path == p)));
        ui.horizontal(|ui| {
            if ui.radio(custom, "Custom path…").clicked()
                && !self.prefs_state.custom_cli.trim().is_empty()
            {
                new_choice = Some(CliChoice::Path(PathBuf::from(
                    self.prefs_state.custom_cli.trim(),
                )));
            }
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.prefs_state.custom_cli)
                    .hint_text("path to kicad-cli")
                    .desired_width(300.0),
            );
            if r.lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                && !self.prefs_state.custom_cli.trim().is_empty()
            {
                new_choice = Some(CliChoice::Path(PathBuf::from(
                    self.prefs_state.custom_cli.trim(),
                )));
            }
            if w::button(ui, None, "Browse…", false, true).clicked() {
                self.ask(DialogKind::KicadCli);
            }
        });
        ui.horizontal(|ui| {
            let target = match (&k.cli, &self.kicad) {
                (CliChoice::Path(p), _) => Some(p.clone()),
                (CliChoice::Auto, KicadState::Found(c)) => Some(c.path.clone()),
                _ => None,
            };
            if w::button(ui, None, "Test", false, target.is_some())
                .on_hover_text("Runs kicad-cli version")
                .clicked()
            {
                if let Some(p) = target {
                    self.kicad_test = None;
                    self.cli.submit("test", CliJob::TestCli(p));
                }
            }
            match &self.kicad_test {
                Some((p, Ok(v))) => {
                    ui.colored_label(t.success, format!("{}: {v}", p.display()));
                }
                Some((p, Err(e))) => {
                    ui.colored_label(t.danger, format!("{}: {e}", p.display()));
                }
                None => {}
            }
        });
        if let Some(c) = new_choice {
            k.cli = c;
            self.set_kicad(&k);
        }
        ui.add_space(10.0);
        ui.label(egui::RichText::new("Extra variables").strong());
        note(
            ui,
            "Passed as --define-var NAME=VALUE to kicad-cli's SVG export and 3D render (for custom path variables of your libraries).",
        );
        let mut remove = None;
        for (i, (n, v)) in k.vars.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("{n} = {v}")).font(text::mono()));
                if w::icon_button(ui, crate::icons::Icon::Close, "Remove", None, true, false)
                    .clicked()
                {
                    remove = Some(i);
                }
            });
        }
        let mut add = false;
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.prefs_state.new_var.0)
                    .hint_text("NAME")
                    .desired_width(140.0),
            );
            ui.label("=");
            ui.add(
                egui::TextEdit::singleline(&mut self.prefs_state.new_var.1)
                    .hint_text("value")
                    .desired_width(260.0),
            );
            let ok = us::parse_var(&format!(
                "{}={}",
                self.prefs_state.new_var.0, self.prefs_state.new_var.1
            ))
            .is_some();
            if w::button(ui, None, "Add", false, ok).clicked() {
                add = true;
            }
        });
        if let Some(i) = remove {
            k.vars.remove(i);
            self.set_kicad(&k);
        } else if add {
            let (n, v) = std::mem::take(&mut self.prefs_state.new_var);
            k.vars.retain(|(x, _)| x != n.trim());
            k.vars.push((n.trim().to_string(), v));
            self.set_kicad(&k);
        }
    }

    fn set_documents(&mut self, d: &us::DocDefaults) {
        self.user.set_documents(d);
        self.user_changed();
    }

    fn prefs_documents(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Documents");
        note(
            ui,
            "Defaults for new documents and pages; a document keeps what it was made with, and explicit choices win.",
        );
        let mut d = self.user.documents();
        let before = d.clone();
        w::row(ui, "Paper", |ui| {
            if let Some(k) = w::dropdown(ui, "pref-paper", &d.paper, us::PAPERS, 100.0) {
                d.paper = us::PAPERS[k].to_string();
            }
            let segs = [Seg::text("Landscape"), Seg::text("Portrait")];
            if let Some(k) = w::segmented(
                ui,
                "pref-orient",
                &segs,
                Some(usize::from(d.portrait)),
                170.0,
            ) {
                d.portrait = k == 1;
            }
        });
        w::row(ui, "Drawing sheet", |ui| {
            let shown = d.sheet.as_ref().map_or_else(
                || "the project's, else KiCad's default".to_string(),
                |p| p.display().to_string(),
            );
            ui.label(egui::RichText::new(shown).font(text::body()));
            if w::button(ui, None, "Choose…", false, true).clicked() {
                self.ask(DialogKind::DefaultSheet);
            }
            if d.sheet.is_some() && w::button(ui, None, "KiCad default", false, true).clicked() {
                d.sheet = None;
            }
        });
        let names = self.user.palette_names();
        let shown: Vec<String> = names.iter().map(|n| pal::display_name(n)).collect();
        let refs: Vec<&str> = shown.iter().map(String::as_str).collect();
        w::row(ui, "Layer palette", |ui| {
            let cur = pal::display_name(&d.layer_palette);
            if let Some(k) = w::dropdown(ui, "pref-palette", &cur, &refs, 240.0) {
                d.layer_palette = names[k].clone();
            }
        });
        w::row(ui, "3D resolution", |ui| {
            let mut s = d.render_dpi.to_string();
            let r = ui.add(egui::TextEdit::singleline(&mut s).desired_width(60.0));
            if r.changed() {
                if let Ok(v) = s.trim().parse::<u32>() {
                    if (10..=1200).contains(&v) {
                        d.render_dpi = v;
                    }
                }
            }
            ui.label("dpi");
            let segs = [Seg::text("basic"), Seg::text("high")];
            let cur = usize::from(d.render_quality == "high");
            if let Some(k) = w::segmented(ui, "pref-quality", &segs, Some(cur), 120.0) {
                d.render_quality = ["basic", "high"][k].to_string();
            }
        });
        w::row(ui, "Callouts", |ui| {
            let segs = [Seg::text("Per page"), Seg::text("Document")];
            if let Some(k) = w::segmented(
                ui,
                "pref-callouts",
                &segs,
                Some(usize::from(d.callouts_per_document)),
                170.0,
            ) {
                d.callouts_per_document = k == 1;
            }
        });
        w::row(ui, "Snap grid", |ui| {
            let mut s = format!("{}", d.snap_grid);
            let r = ui.add(egui::TextEdit::singleline(&mut s).desired_width(60.0));
            if r.changed() {
                if let Ok(v) = s.trim().parse::<f64>() {
                    if v.is_finite() && (0.0..=50.0).contains(&v) {
                        d.snap_grid = v;
                    }
                }
            }
            ui.label("mm");
        });
        if d != before {
            self.set_documents(&d);
        }
        if self.s.current_page_index().is_some() {
            ui.add_space(12.0);
            ui.label(egui::RichText::new("This document").strong());
            self.document_palette_row(ui);
        }
    }

    /// "Layer palette" of the open document (also in the page inspector).
    pub(crate) fn document_palette_row(&mut self, ui: &mut egui::Ui) {
        let cur = self
            .s
            .doc()
            .styles
            .layer_palette
            .as_ref()
            .map_or(pal::BLACK.to_string(), |p| p.name.clone());
        let cur = pal::display_name(&cur);
        let names = self.user.palette_names();
        let shown: Vec<String> = names.iter().map(|n| pal::display_name(n)).collect();
        let refs: Vec<&str> = shown.iter().map(String::as_str).collect();
        w::row(ui, "Layer palette", |ui| {
            // No wider than the inspector: a fixed width pushed the row and
            // the note below it past the panel edge.
            let width = ui.available_width().min(220.0);
            if let Some(k) = w::dropdown(ui, "doc-palette", &cur, &refs, width) {
                let name = names[k].clone();
                let args = if pal::BUILTIN.contains(&name.as_str()) {
                    json!({ "name": name })
                } else {
                    match self.user.palette(&name) {
                        Some(p) => json!({ "palette": us::palette_to_json(&p) }),
                        None => return,
                    }
                };
                self.cmd("document.set_layer_palette", args);
            }
        });
        note(
            ui,
            "Colours of views without their own; stored in the document. Fabrication and drill views stay black.",
        );
    }

    fn prefs_palettes(&mut self, ui: &mut egui::Ui) {
        let t = self.tokens();
        heading(ui, "Layer colours");
        note(
            ui,
            "Palettes give each layer a colour and opacity. A document stores the colours of its palette, so it looks the same on another PC.",
        );
        let names = self.user.palette_names();
        if !names.contains(&self.prefs_state.palette) {
            self.prefs_state.palette = pal::BLACK.to_string();
        }
        let sel = self.prefs_state.palette.clone();
        let builtin = pal::BUILTIN.contains(&sel.as_str());
        let mut p = self
            .user
            .palette(&sel)
            .unwrap_or_else(|| Palette::new(&sel));
        ui.horizontal_wrapped(|ui| {
            for n in &names {
                let tag = if pal::BUILTIN.contains(&n.as_str()) {
                    format!("{} ·", pal::display_name(n))
                } else {
                    n.clone()
                };
                if ui.selectable_label(*n == sel, tag).clicked() {
                    self.prefs_state.palette = n.clone();
                }
            }
        });
        note(ui, "· built-in");
        let mut action: Option<&str> = None;
        ui.horizontal(|ui| {
            if w::button(ui, None, "Duplicate", false, true).clicked() {
                action = Some("dup");
            }
            if w::button(ui, None, "Delete", false, !builtin).clicked() {
                action = Some("del");
            }
            if w::button(ui, None, "Export…", false, true).clicked() {
                action = Some("export");
            }
            if w::button(ui, None, "Import palette…", false, true).clicked() {
                action = Some("import");
            }
            let is_default = self.user.documents().layer_palette == sel;
            if w::button(ui, None, "Use for new documents", false, !is_default).clicked() {
                action = Some("default");
            }
        });
        match action {
            Some("dup") => {
                let mut q = p.clone();
                let mut k = 2;
                q.name = format!("{sel} copy");
                while names.contains(&q.name) {
                    q.name = format!("{sel} copy {k}");
                    k += 1;
                }
                self.prefs_state.palette = q.name.clone();
                self.store_palette(q);
                return;
            }
            Some("del") => {
                let mut all = self.user.palettes();
                all.retain(|q| q.name != sel);
                self.user.set_palettes(&all);
                let mut d = self.user.documents();
                if d.layer_palette == sel {
                    d.layer_palette = pal::BLACK.into();
                    self.user.set_documents(&d);
                }
                self.prefs_state.palette = pal::BLACK.into();
                self.user_changed();
                return;
            }
            Some("export") => self.ask(DialogKind::PaletteExport),
            Some("import") => self.ask(DialogKind::PaletteImport),
            Some("default") => {
                let mut d = self.user.documents();
                d.layer_palette = sel.clone();
                self.set_documents(&d);
            }
            _ => {}
        }
        // Import of KiCad themes.
        ui.add_space(6.0);
        egui::Frame::new()
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Import KiCad colour theme…").strong());
                ui.checkbox(&mut self.prefs_state.adapt, "Adapt for white paper")
                    .on_hover_text("Darken colours too light to see on white paper (contrast 3:1), keeping their hue");
                let themes = self.prefs_state.themes.clone();
                if themes.is_empty() {
                    note(ui, "No themes found in KiCad's colors folders.");
                }
                ui.horizontal_wrapped(|ui| {
                    for th in themes {
                        if w::button(ui, None, &format!("{} (KiCad {})", th.name, th.version), false, true)
                            .on_hover_text(th.path.display().to_string())
                            .clicked()
                        {
                            self.import_theme(&th.path);
                        }
                    }
                    if w::button(ui, None, "Browse…", false, true).clicked() {
                        self.ask(DialogKind::KicadTheme);
                    }
                });
                note(ui, "kdraft copies the colours; it never writes into KiCad's folders.");
            });
        ui.add_space(6.0);
        // The editor.
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Name").font(text::small()));
            let mut name = p.name.clone();
            let r = ui.add_enabled(
                !builtin,
                egui::TextEdit::singleline(&mut name).desired_width(240.0),
            );
            if r.lost_focus() && name.trim() != p.name && !name.trim().is_empty() {
                let new = name.trim().to_string();
                if names.contains(&new) || pal::BUILTIN.contains(&new.as_str()) {
                    self.warn(format!("a palette {new:?} exists already"));
                } else {
                    let mut all = self.user.palettes();
                    for q in &mut all {
                        if q.name == sel {
                            q.name = new.clone();
                        }
                    }
                    self.user.set_palettes(&all);
                    let mut d = self.user.documents();
                    if d.layer_palette == sel {
                        d.layer_palette = new.clone();
                        self.user.set_documents(&d);
                    }
                    self.prefs_state.palette = new;
                    self.user_changed();
                    return;
                }
            }
            if builtin {
                note(ui, "built-in: duplicate it to edit");
            }
        });
        let before = self
            .prefs_state
            .last_import
            .as_ref()
            .filter(|i| i.palette.name == sel)
            .map(|i| i.original.clone());
        let mut changed = false;
        egui::Grid::new("palette-grid")
            .num_columns(if before.is_some() { 5 } else { 4 })
            .spacing([12.0, 4.0])
            .striped(true)
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Layer").font(text::small()));
                if before.is_some() {
                    ui.label(egui::RichText::new("Theme").font(text::small()));
                }
                ui.label(egui::RichText::new("Colour").font(text::small()));
                ui.label(egui::RichText::new("Opacity").font(text::small()));
                ui.label("");
                ui.end_row();
                for key in pal::editor_keys(self.prefs_state.inner_rows) {
                    let label = pal::key_label(&key);
                    ui.label(egui::RichText::new(&label).font(text::body()));
                    if let Some(b) = &before {
                        let c = b.colors.get(&key).copied();
                        crate::ui_style::swatch(
                            ui,
                            c.map(|c| Rgba { a: 255, ..c }),
                            &format!("{label} in the theme"),
                            false,
                        );
                    }
                    let cur = p.colors.get(&key).copied();
                    let shown = cur.map(|c| Rgba { a: 255, ..c });
                    let resp =
                        crate::ui_style::swatch(ui, shown, &format!("{label} colour"), false);
                    if !builtin {
                        let pid = egui::Id::unique(("pal-color", &sel, &key));
                        let mut edit = None;
                        egui::Popup::menu(&resp).id(pid).show(|ui| {
                            ui.set_min_width(250.0);
                            edit = self.color_popup(ui, ("palette", key.as_str()), shown, false);
                        });
                        if let Some(
                            crate::ui_style::ColorEdit::Set(c)
                            | crate::ui_style::ColorEdit::Live(c),
                        ) = edit
                        {
                            let a = cur.map_or(255, |o| o.a);
                            p.colors.insert(key.clone(), Rgba { a, ..c });
                            changed = true;
                        }
                    }
                    let mut op = cur.map_or(100.0, |c| f64::from(c.a) / 2.55);
                    let r = ui.add_enabled(
                        !builtin && cur.is_some(),
                        egui::DragValue::new(&mut op)
                            .range(0.0..=100.0)
                            .suffix(" %")
                            .speed(1.0)
                            .fixed_decimals(0),
                    );
                    if r.changed() {
                        if let Some(c) = cur {
                            p.colors.insert(key.clone(), pal::join(c, op / 100.0));
                            changed = true;
                        }
                    }
                    if !builtin
                        && cur.is_some()
                        && w::icon_button(
                            ui,
                            crate::icons::Icon::Close,
                            "Reset (no colour: follows the layer)",
                            None,
                            true,
                            false,
                        )
                        .clicked()
                    {
                        p.colors.remove(&key);
                        changed = true;
                    } else if builtin || cur.is_none() {
                        ui.label("");
                    }
                    ui.end_row();
                }
            });
        ui.horizontal(|ui| {
            if w::button(
                ui,
                None,
                "More inner layers",
                false,
                self.prefs_state.inner_rows < 30,
            )
            .clicked()
            {
                self.prefs_state.inner_rows = (self.prefs_state.inner_rows + 4).min(30);
            }
            if before.is_some() {
                note(
                    ui,
                    "Theme: the colour in the KiCad theme; Colour: what kdraft uses on paper.",
                );
            }
        });
        if changed && !builtin {
            self.store_palette(p);
        }
    }

    /// Adds or replaces a user palette and saves.
    fn store_palette(&mut self, p: Palette) {
        if let Err(e) = self.user.upsert_palette(p) {
            self.warn(e);
            return;
        }
        self.user_changed();
    }

    /// Imports a KiCad colour theme as a palette and selects it.
    pub fn import_theme(&mut self, path: &Path) {
        match kdraft_source::kicad_theme::import_file(path, None, self.prefs_state.adapt) {
            Ok(imp) => {
                let changed = imp
                    .original
                    .colors
                    .iter()
                    .filter(|(k, c)| imp.palette.colors.get(*k) != Some(*c))
                    .count();
                let mut imp = imp;
                if pal::BUILTIN.contains(&imp.name.as_str()) {
                    imp.name = format!("{} (imported)", imp.name);
                    imp.palette.name = imp.name.clone();
                }
                self.notify(format!(
                    "Imported palette {:?}: {} colours from the theme, {} darkened for paper",
                    imp.name, imp.from_file, changed
                ));
                self.prefs_state.palette = imp.name.clone();
                self.store_palette(imp.palette.clone());
                self.prefs_state.last_import = Some(imp);
            }
            Err(e) => self.error(e),
        }
    }

    fn prefs_cache(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Cache");
        note(
            ui,
            "kicad-cli results (plots, BOM, 3D pictures) are cached by content; the cache can be deleted at any time.",
        );
        let k = self.user.kicad();
        let mut shared = k.cache_dir.is_some();
        let mut changed = false;
        if ui
            .radio(!shared, "Next to each document (.kdraft-cache)")
            .clicked()
        {
            shared = false;
            changed = true;
        }
        ui.horizontal(|ui| {
            if ui.radio(shared, "One folder for all documents:").clicked() {
                shared = true;
                changed = true;
            }
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.prefs_state.cache_text)
                    .hint_text("folder")
                    .desired_width(320.0),
            );
            if r.lost_focus() && shared {
                changed = true;
            }
        });
        if changed {
            let mut k = k.clone();
            k.cache_dir = (shared && !self.prefs_state.cache_text.trim().is_empty())
                .then(|| PathBuf::from(self.prefs_state.cache_text.trim()));
            self.user.set_kicad(&k);
            self.user_changed();
            self.measure_cache();
        }
        let dir = self.cache_folder();
        w::row(ui, "Folder", |ui| {
            ui.label(egui::RichText::new(dir.display().to_string()).font(text::body()));
        });
        w::row(ui, "Size", |ui| {
            let s = match &self.cache_size {
                Some((p, n)) if *p == dir => us::size_text(*n),
                _ => "measuring…".to_string(),
            };
            ui.label(egui::RichText::new(s).font(text::body()));
            if w::button(ui, None, "Measure", false, true).clicked() {
                self.measure_cache();
            }
            if w::button(ui, None, "Clear cache", false, true).clicked() {
                self.io
                    .submit("cache-clear", IoJob::ClearCache(dir.clone()));
            }
        });
        note(ui, "No size limit: the cache grows until cleared.");
    }

    /// A path chosen for one of the settings page's dialogs.
    pub(crate) fn prefs_dialog_done(&mut self, kind: DialogKind, p: PathBuf) {
        match kind {
            DialogKind::KicadCli => {
                self.prefs_state.custom_cli = p.display().to_string();
                let mut k = self.user.kicad();
                k.cli = CliChoice::Path(p);
                self.set_kicad(&k);
            }
            DialogKind::KicadTheme => self.import_theme(&p),
            DialogKind::PaletteImport => {
                let r = std::fs::read_to_string(&p)
                    .map_err(|e| e.to_string())
                    .and_then(|t| {
                        serde_json::from_str::<serde_json::Value>(&t).map_err(|e| e.to_string())
                    })
                    .and_then(|v| {
                        us::palette_from_json(&v)
                            .ok_or_else(|| "not a kdraft palette (name, colors)".to_string())
                    });
                match r {
                    Ok(q) => {
                        self.prefs_state.palette = q.name.clone();
                        self.store_palette(q);
                    }
                    Err(e) => self.error(format!("{}: {e}", p.display())),
                }
            }
            DialogKind::PaletteExport => {
                let p = if p.extension().is_none() {
                    p.with_extension("json")
                } else {
                    p
                };
                match self.user.palette(&self.prefs_state.palette) {
                    Some(q) => {
                        let text = serde_json::to_string_pretty(&us::palette_to_json(&q))
                            .unwrap_or_default();
                        match us::write_atomic(&p, &text) {
                            Ok(()) => self.notify(format!("Exported {}", p.display())),
                            Err(e) => self.error(e),
                        }
                    }
                    None => self.error("no palette selected"),
                }
            }
            DialogKind::DefaultSheet => {
                let mut d = self.user.documents();
                d.sheet = Some(p);
                self.set_documents(&d);
            }
            _ => {}
        }
    }
}
