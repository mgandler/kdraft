//! The egui layer, top level: top strip with menus, status bar, bottom
//! dock (issues, log), panels, start screen, toasts and keyboard
//! dispatch. Every change goes through [`App::run`], [`App::cmd`] or the
//! action catalogue; geometry and state logic live in plain modules.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, Vec2};
use serde_json::json;

use crate::actions::{AppAct, Run};
use crate::app::{App, Dock, Level, UiAction};
use crate::icons::{self, Icon};
use crate::strings as S;
use crate::theme::{self, text, Tokens};
use crate::tools::ToolKind;
use crate::widgets as w;

/// Height of the top strip and the status bar.
const TOP_H: f32 = 38.0;
const STATUS_H: f32 = 26.0;

impl App {
    /// One frame of the whole editor.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.pixels_per_point = f64::from(ctx.pixels_per_point());
        self.max_texture_side = ctx.input(|i| i.max_texture_side) as f64;
        self.tree_hover = None;
        self.apply_theme(&ctx);
        let t = self.tokens();
        self.poll(&ctx);
        self.refresh_inputs();
        self.dropped_files(&ctx);
        self.keyboard(&ctx);
        self.title(&ctx);
        self.track_window(&ctx);
        let has_doc = self.s.current_page_index().is_some();
        // The settings page takes the place of the panels and the canvas.
        let prefs = self.prefs.is_some();

        egui::Panel::top("top")
            .exact_size(TOP_H)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(t.chrome)
                    .inner_margin(egui::Margin::symmetric(8, 0)),
            )
            .show(ui, |ui| self.top_strip(ui));
        egui::Panel::bottom("status")
            .exact_size(STATUS_H)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(t.chrome)
                    .inner_margin(egui::Margin::symmetric(10, 0)),
            )
            .show(ui, |ui| self.status_bar(ui));
        if let (Some(d), false) = (self.dock, prefs) {
            let r = egui::Panel::bottom("dock")
                .resizable(true)
                .default_size(self.settings.dock_height.unwrap_or(190.0))
                .size_range(90.0..=480.0)
                .frame(
                    egui::Frame::new()
                        .fill(t.panel)
                        .inner_margin(egui::Margin::symmetric(10, 6)),
                )
                .show(ui, |ui| self.dock_panel(ui, d));
            self.note_panel_size(PanelKind::Dock, r.response.rect.height());
        }
        if has_doc && !prefs {
            let r = egui::Panel::left("left")
                .resizable(true)
                .default_size(self.settings.left_width.unwrap_or(260.0))
                .size_range(230.0..=420.0)
                .frame(
                    egui::Frame::new()
                        .fill(t.panel)
                        .inner_margin(egui::Margin::ZERO),
                )
                .show(ui, |ui| self.left_panel(ui));
            self.note_panel_size(PanelKind::Left, r.response.rect.width());
            let r = egui::Panel::right("inspector")
                .resizable(true)
                .default_size(self.settings.inspector_width.unwrap_or(296.0))
                .size_range(280.0..=440.0)
                .frame(
                    egui::Frame::new()
                        .fill(t.panel)
                        .inner_margin(egui::Margin::ZERO),
                )
                .show(ui, |ui| self.inspector(ui));
            self.note_panel_size(PanelKind::Inspector, r.response.rect.width());
        }
        egui::CentralPanel::no_frame().show(ui, |ui| {
            if prefs {
                self.prefs_page(ui);
            } else if has_doc {
                self.canvas(ui);
            } else {
                self.start_screen(ui);
            }
        });
        self.prompt_window(&ctx);
        self.palette_window(&ctx);
        self.shortcuts_window(&ctx);
        self.doc_styles_window(&ctx);
        self.layer_pages_window(&ctx);
        self.renumber_window(&ctx);
        self.about_window(&ctx);
        self.toasts_overlay(&ctx);
        let busy = self.render.busy() + self.io.busy() + self.cli.busy() + self.checker.busy() > 0;
        if busy || !self.zoom_settled() || !self.toasts.is_empty() || self.settings_due.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn title(&self, ctx: &egui::Context) {
        let t = format!(
            "{}{} - kdraft",
            if self.s.dirty() { "*" } else { "" },
            self.doc_name()
        );
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(t));
    }

    pub fn doc_name(&self) -> String {
        self.s
            .doc_path()
            .and_then(|p| p.file_name())
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| S::UNTITLED.to_string())
    }

    fn dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<_> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        if let Some(p) = dropped.into_iter().next() {
            self.run(UiAction::OpenPath(p));
        }
    }

    // ------------------------------------------------------------ keys

    fn keyboard(&mut self, ctx: &egui::Context) {
        // The in-place text editor handles its own keys.
        if self.text_edit.is_some() || self.label_edit.is_some() {
            return;
        }
        let typing = ctx.egui_wants_keyboard_input();
        let modal = self.prompt.is_some() || self.palette.is_some() || self.renaming.is_some();
        let mut presses: Vec<(egui::Key, egui::Modifiers)> = Vec::new();
        let mut nudge: Option<(f64, f64)> = None;
        ctx.input(|i| {
            for e in &i.events {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = e
                {
                    presses.push((*key, *modifiers));
                }
            }
            let step = if i.modifiers.shift { 10.0 } else { 1.0 };
            if !i.modifiers.alt && !i.modifiers.command {
                for (k, dx, dy) in [
                    (egui::Key::ArrowLeft, -step, 0.0),
                    (egui::Key::ArrowRight, step, 0.0),
                    (egui::Key::ArrowUp, 0.0, -step),
                    (egui::Key::ArrowDown, 0.0, step),
                ] {
                    if i.key_pressed(k) {
                        nudge = Some((dx, dy));
                    }
                }
            }
        });
        // Escape: cancel the innermost thing first.
        if presses.iter().any(|(k, _)| *k == egui::Key::Escape) {
            if self.prompt.is_none() && self.prefs.take().is_some() {
                return;
            }
            if self.link_pick.take().is_some() {
                self.info("Linking cancelled");
                return;
            }
            if self.palette.is_some() {
                self.palette = None;
                return;
            }
            if self.show_shortcuts || self.show_about {
                self.show_shortcuts = false;
                self.show_about = false;
                return;
            }
            if typing || modal {
                return;
            }
            if !self.tool.kind.is_select() {
                if self.tool.escape() == crate::tools::Out::Exit {
                    self.set_tool(ToolKind::Select);
                }
                return;
            }
            if !self.drag.is_idle() {
                for a in self.drag.cancel() {
                    self.drag_action(a, None);
                }
                return;
            }
            self.cmd("select.clear", json!({}));
            return;
        }
        // The palette's own shortcut works while it is open.
        for (k, m) in &presses {
            if let Some(e) = self.catalogue.for_key(*k, *m) {
                if e.run == Run::App(AppAct::Palette) {
                    let e = e.clone();
                    self.run_entry(&e);
                    return;
                }
            }
        }
        if typing || modal {
            return;
        }
        // Delete with a selected vertex removes the vertex.
        if presses
            .iter()
            .any(|(k, m)| matches!(k, egui::Key::Delete | egui::Key::Backspace) && m.is_none())
            && self.sel_vertex.is_some()
            && self.delete_selected_vertex()
        {
            return;
        }
        // Enter finishes a polyline.
        if self.tool.kind == ToolKind::Polyline
            && presses
                .iter()
                .any(|(k, m)| *k == egui::Key::Enter && m.is_none())
        {
            match self.tool.finish() {
                crate::tools::Out::Place(pl) => self.place(pl),
                crate::tools::Out::Hint(h) => self.info(h),
                _ => {}
            }
            return;
        }
        // Enter edits the selected text in place.
        if presses
            .iter()
            .any(|(k, m)| *k == egui::Key::Enter && m.is_none())
        {
            if let Some(id) = self.selected_text() {
                self.begin_text_edit(&id, false);
                return;
            }
            if let [c] = self.selected_callouts().as_slice() {
                let id = c.id.clone();
                self.begin_label_edit(&id);
                return;
            }
        }
        if let Some((dx, dy)) = nudge {
            if !self.s.selection.items.is_empty() {
                self.cmd("layout.nudge", json!({ "dx": dx, "dy": dy }));
            }
        }
        // Plus and minus change the numbers of selected callouts.
        if !typing && !modal && self.callout_keys(&presses) {
            return;
        }
        for (k, m) in presses {
            if matches!(
                k,
                egui::Key::ArrowLeft
                    | egui::Key::ArrowRight
                    | egui::Key::ArrowUp
                    | egui::Key::ArrowDown
            ) && !m.alt
            {
                continue;
            }
            if let Some(e) = self.catalogue.for_key(k, m).cloned() {
                self.run_entry(&e);
            }
        }
    }

    // ------------------------------------------------------------ top strip

    fn top_strip(&mut self, ui: &mut egui::Ui) {
        let t = self.tokens();
        let full = ui.max_rect();
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            // Wordmark: plain text, no logo.
            ui.label(
                egui::RichText::new(S::APP)
                    .font(text::title())
                    .color(t.text),
            );
            ui.add_space(10.0);
            self.menu_bar(ui);
            ui.add_space(8.0);
            let undo_tip = self.s.undo_title().map(|x| format!("Undo {x}"));
            let r = w::icon_button(
                ui,
                Icon::Undo,
                "Undo",
                Some("Ctrl+Z"),
                self.s.can_undo(),
                false,
            );
            let r = match undo_tip {
                Some(t) => r.on_hover_text(t),
                None => r,
            };
            if r.clicked() {
                self.app_act(AppAct::Undo);
            }
            if w::icon_button(
                ui,
                Icon::Redo,
                "Redo",
                Some("Ctrl+Y"),
                self.s.can_redo(),
                false,
            )
            .clicked()
            {
                self.app_act(AppAct::Redo);
            }
        });
        // Document name centred over the whole strip.
        if self.s.current_page_index().is_some() {
            let name = self.doc_name();
            let g = ui.painter().layout_no_wrap(name, text::title(), t.text);
            let c = full.center();
            let r = Rect::from_center_size(c, g.size());
            ui.painter().galley(r.min, g, t.text);
            if self.s.dirty() {
                ui.painter()
                    .circle_filled(Pos2::new(r.max.x + 9.0, c.y + 1.0), 3.5, t.accent);
                let dot = Rect::from_center_size(Pos2::new(r.max.x + 9.0, c.y), Vec2::splat(10.0));
                ui.interact(dot, egui::Id::unique("modified-dot"), Sense::hover())
                    .on_hover_text(S::MODIFIED);
            }
            if let Some(b) = self
                .s
                .board_state()
                .path
                .as_deref()
                .and_then(|p| p.file_name())
            {
                let txt = b.to_string_lossy();
                ui.painter().text(
                    Pos2::new(r.max.x + 20.0, c.y),
                    Align2::LEFT_CENTER,
                    format!("· {txt}"),
                    text::small(),
                    t.text_secondary,
                );
            }
        }
        // Right: settings, palette and theme.
        let mut right = Rect::from_min_max(Pos2::new(full.max.x - 100.0, full.min.y), full.max);
        right.min.y = full.center().y - 12.0;
        right.max.y = full.center().y + 12.0;
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(right)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        let dark = self.applied_dark.unwrap_or(false);
        let (icon, tip) = if dark {
            (Icon::Sun, S::THEME_LIGHT)
        } else {
            (Icon::Moon, S::THEME_DARK)
        };
        if w::icon_button(&mut child, icon, tip, None, true, false).clicked() {
            self.app_act(AppAct::ToggleTheme);
        }
        if w::icon_button(
            &mut child,
            Icon::Command,
            "Command palette",
            Some("Ctrl+K"),
            true,
            false,
        )
        .clicked()
        {
            self.app_act(AppAct::Palette);
        }
        if w::icon_button(
            &mut child,
            Icon::Settings,
            "Settings",
            Some("Ctrl+Comma"),
            true,
            self.prefs.is_some(),
        )
        .clicked()
        {
            self.app_act(AppAct::Settings);
        }
        ui.painter()
            .hline(full.x_range(), full.max.y - 0.5, Stroke::new(1.0, t.border));
    }

    // ------------------------------------------------------------ status

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let t = self.tokens();
        let r = ui.max_rect();
        ui.painter()
            .hline(r.x_range(), r.min.y + 0.5, Stroke::new(1.0, t.border));
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            let small = |s: String| {
                egui::RichText::new(s)
                    .font(text::small())
                    .color(t.text_secondary)
            };
            ui.label(
                egui::RichText::new(self.cursor_text())
                    .font(text::mono())
                    .color(t.text_secondary),
            );
            if self.s.current_page_index().is_some() {
                ui.label(small(format!("{:.0} %", self.zoom_percent())));
            }
            if let Some(h) = self.tool_hint() {
                ui.label(
                    egui::RichText::new(h)
                        .font(text::small())
                        .color(t.accent_text),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                if w::icon_button_sized(
                    ui,
                    Icon::Log,
                    "Message log",
                    Some("Ctrl+Shift+M"),
                    true,
                    self.dock == Some(Dock::Log),
                    20.0,
                )
                .clicked()
                {
                    self.app_act(AppAct::ToggleLog);
                }
                // Issues badge.
                let (n_err, n_warn) = self.issue_counts();
                let label = if self.issues.is_none() {
                    "checks pending".to_string()
                } else if n_err + n_warn == 0 {
                    "no issues".to_string()
                } else {
                    format!(
                        "{} issue{}",
                        n_err + n_warn,
                        if n_err + n_warn == 1 { "" } else { "s" }
                    )
                };
                let col = if n_err > 0 {
                    t.danger
                } else if n_warn > 0 {
                    t.warning
                } else {
                    t.text_secondary
                };
                if self.s.current_page_index().is_some() {
                    let resp = ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(label).font(text::small()).color(col),
                            )
                            .frame(false),
                        )
                        .on_hover_text("Drawing checks (Ctrl+Shift+I)");
                    if resp.clicked() {
                        self.app_act(AppAct::ToggleIssues);
                    }
                }
                let jobs = self.jobs_status();
                if jobs != "idle" {
                    ui.add(egui::Spinner::new().size(12.0).color(t.accent));
                }
                ui.label(small(jobs));
                // kicad-cli: opens Settings > KiCad.
                if ui
                    .add(egui::Button::new(small(self.kicad_status())).frame(false))
                    .on_hover_text("kicad-cli in use; click for Settings > KiCad")
                    .clicked()
                {
                    self.open_prefs(crate::ui_prefs::PrefsSection::Kicad);
                }
                ui.label(small(self.board_status()));
            });
        });
    }

    pub fn issue_counts(&self) -> (usize, usize) {
        use kdraft_engine::check::Severity;
        self.issues.as_ref().map_or((0, 0), |(_, r)| {
            (
                r.findings
                    .iter()
                    .filter(|f| f.severity == Severity::Error)
                    .count(),
                r.findings
                    .iter()
                    .filter(|f| f.severity == Severity::Warning)
                    .count(),
            )
        })
    }

    pub fn tool_hint(&self) -> Option<String> {
        if self.tool.kind.is_select() {
            return None;
        }
        let mut h = self.tool.stage_hint().to_string();
        if let Some(p) = &self.snapped {
            h = format!("{h} · {}", p.label);
        }
        Some(h)
    }

    // ------------------------------------------------------------ dock

    fn dock_panel(&mut self, ui: &mut egui::Ui, d: Dock) {
        let t = self.tokens();
        ui.horizontal(|ui| {
            let (ne, nw) = self.issue_counts();
            let tabs = [format!("Issues ({})", ne + nw), "Log".to_string()];
            let cur = if d == Dock::Issues { 0 } else { 1 };
            let segs = [w::Seg::text(&tabs[0]), w::Seg::text(&tabs[1])];
            if let Some(k) = w::segmented(ui, "dock-tabs", &segs, Some(cur), 180.0) {
                self.dock = Some(if k == 0 { Dock::Issues } else { Dock::Log });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if w::icon_button(ui, Icon::Close, "Close panel", None, true, false).clicked() {
                    self.dock = None;
                }
                if d == Dock::Log {
                    if w::icon_button(ui, Icon::Trash, "Clear messages", None, true, false)
                        .clicked()
                    {
                        self.messages.clear();
                    }
                } else if w::icon_button(ui, Icon::Reload, "Check again", Some("F7"), true, false)
                    .clicked()
                {
                    self.issues = None;
                }
            });
        });
        ui.add_space(4.0);
        match d {
            Dock::Log => {
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for m in &self.messages {
                            let (c, i) = match m.level {
                                Level::Info => (t.text_secondary, Icon::Info),
                                Level::Warning => (t.warning, Icon::Warning),
                                Level::Error => (t.danger, Icon::Error),
                            };
                            ui.horizontal(|ui| {
                                let (r, _) =
                                    ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                                icons::paint(ui.painter(), r, i, c);
                                ui.label(
                                    egui::RichText::new(&m.text)
                                        .font(text::small())
                                        .color(if m.level == Level::Info { t.text } else { c }),
                                );
                            });
                        }
                    });
            }
            Dock::Issues => self.issues_list(ui),
        }
    }

    fn issues_list(&mut self, ui: &mut egui::Ui) {
        use kdraft_engine::check::Severity;
        let t = self.tokens();
        let Some((_, report)) = self.issues.clone() else {
            ui.label(egui::RichText::new("Checking…").color(t.text_secondary));
            return;
        };
        let mut findings = report.findings.clone();
        findings.sort_by(|a, b| b.severity.cmp(&a.severity).then(a.page.cmp(&b.page)));
        if findings.is_empty() {
            ui.label(egui::RichText::new("No issues found.").color(t.text_secondary));
            return;
        }
        let mut go: Option<(usize, Option<String>)> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (k, f) in findings.iter().enumerate() {
                    let (c, icon) = match f.severity {
                        Severity::Error => (t.danger, Icon::Error),
                        Severity::Warning => (t.warning, Icon::Warning),
                        Severity::Info => (t.text_secondary, Icon::Info),
                    };
                    let w_ = ui.available_width();
                    let (rect, r) = ui.allocate_exact_size(Vec2::new(w_, 22.0), Sense::click());
                    let label = format!("{} {}", f.code, f.message);
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
                    if r.hovered() {
                        ui.painter()
                            .rect_filled(rect, CornerRadius::same(3), t.hover);
                    }
                    icons::paint(
                        ui.painter(),
                        Rect::from_min_size(rect.min + Vec2::new(4.0, 3.0), Vec2::splat(16.0)),
                        icon,
                        c,
                    );
                    let page = if f.page > 0 {
                        format!("p{}", f.page)
                    } else {
                        "doc".into()
                    };
                    ui.painter().text(
                        rect.min + Vec2::new(26.0, 11.0),
                        Align2::LEFT_CENTER,
                        &page,
                        text::small(),
                        t.text_tertiary,
                    );
                    let msg = w::elide(ui, &f.message, text::body(), w_ - 160.0);
                    ui.painter().text(
                        rect.min + Vec2::new(56.0, 11.0),
                        Align2::LEFT_CENTER,
                        msg,
                        text::body(),
                        t.text,
                    );
                    ui.painter().text(
                        Pos2::new(rect.max.x - 6.0, rect.center().y),
                        Align2::RIGHT_CENTER,
                        f.code,
                        text::small(),
                        t.text_tertiary,
                    );
                    if r.clicked() {
                        go = Some((f.page, f.item.clone()));
                    }
                    let _ = k;
                }
            });
        if let Some((page, item)) = go {
            self.go_to_issue(page, item);
        }
    }

    /// Selects the page and item of an issue and zooms to it.
    pub fn go_to_issue(&mut self, page: usize, item: Option<String>) {
        if page >= 1 && page <= self.s.doc().pages.len() {
            let pid = self.s.doc().pages[page - 1].id.0.clone();
            self.cmd("page.select", json!({ "page": pid }));
        }
        if let Some(id) = item {
            if self.cmd("select.set", json!({ "ids": [id] })) {
                self.fit_pending = false;
                self.camera_page = self.s.current_page().map(|p| p.id.clone());
                self.fit_selection();
            }
        }
    }

    // ------------------------------------------------------------ start

    fn start_screen(&mut self, ui: &mut egui::Ui) {
        let t = self.tokens();
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, t.canvas);
        let card = Rect::from_center_size(rect.center(), Vec2::new(520.0, 380.0));
        ui.painter().rect(
            card,
            CornerRadius::same(10),
            t.panel,
            Stroke::new(1.0, t.border),
            egui::StrokeKind::Inside,
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(card.shrink(32.0)));
        let ui = &mut child;
        ui.label(
            egui::RichText::new(S::START_TITLE)
                .font(egui::FontId::new(20.0, theme::bold()))
                .color(t.text),
        );
        ui.add_space(2.0);
        ui.label(egui::RichText::new(S::START_SUB).color(t.text_secondary));
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            if w::button(ui, Some(Icon::Open), S::OPEN_DOTS, true, true).clicked() {
                self.app_act(AppAct::Open);
            }
            if w::button(ui, Some(Icon::Board), S::NEW_FROM_BOARD, false, true).clicked() {
                self.app_act(AppAct::OpenBoard);
            }
            if w::button(ui, Some(Icon::Settings), "Settings…", false, true).clicked() {
                self.open_prefs(crate::ui_prefs::PrefsSection::General);
            }
        });
        ui.add_space(20.0);
        ui.label(
            egui::RichText::new(S::RECENT)
                .font(text::heading())
                .color(t.text),
        );
        ui.add_space(4.0);
        let recent = self.settings.recent.clone();
        if recent.is_empty() {
            ui.label(egui::RichText::new(S::NO_RECENT).color(t.text_tertiary));
        }
        let mut open = None;
        for p in recent.iter().take(7) {
            let name = p
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default();
            let dir = p
                .parent()
                .map(|d| d.display().to_string())
                .unwrap_or_default();
            let wd = ui.available_width();
            let (r, resp) = ui.allocate_exact_size(Vec2::new(wd, 34.0), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
            if resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::same(4), t.hover);
            }
            let icon = if p.extension().is_some_and(|e| e == "kicad_pcb") {
                Icon::Board
            } else {
                Icon::Page
            };
            icons::paint(
                ui.painter(),
                Rect::from_min_size(r.min + Vec2::new(6.0, 9.0), Vec2::splat(16.0)),
                icon,
                t.icon_secondary,
            );
            ui.painter().text(
                r.min + Vec2::new(30.0, 9.0),
                Align2::LEFT_CENTER,
                &name,
                text::body(),
                t.text,
            );
            let dir = w::elide(ui, &dir, text::small(), wd - 40.0);
            ui.painter().text(
                r.min + Vec2::new(30.0, 25.0),
                Align2::LEFT_CENTER,
                dir,
                text::small(),
                t.text_tertiary,
            );
            if resp.clicked() {
                open = Some(p.clone());
            }
        }
        if let Some(p) = open {
            self.run(UiAction::OpenPath(p));
        }
        if self.busy_open {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0).color(t.accent));
                ui.label(egui::RichText::new(S::OPENING).color(t.text_secondary));
            });
        }
    }

    // ------------------------------------------------------------ windows

    fn prompt_window(&mut self, ctx: &egui::Context) {
        let Some((kind, mut text)) = self.prompt.clone() else {
            return;
        };
        let mut close = false;
        let mut go = None;
        egui::Window::new(kind.title())
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 120.0))
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(S::PATH_PROMPT).color(self.tokens().text_secondary));
                let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(440.0));
                if !r.has_focus() && text.is_empty() {
                    r.request_focus();
                }
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if w::button(ui, None, "OK", true, true).clicked() || enter {
                        go = Some(text.clone());
                    }
                    if w::button(ui, None, "Cancel", false, true).clicked() {
                        close = true;
                    }
                });
            });
        if let Some(t) = go {
            let p = std::path::PathBuf::from(t.trim().trim_matches('"'));
            self.prompt = None;
            if !p.as_os_str().is_empty() {
                self.dialog_done(kind, p);
            }
        } else if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.prompt = None;
        } else {
            self.prompt = Some((kind, text));
        }
    }

    fn about_window(&mut self, ctx: &egui::Context) {
        if !self.show_about {
            return;
        }
        let mut open = true;
        egui::Window::new("About kdraft")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new("kdraft").font(text::title()));
                ui.label(format!("version {}", env!("CARGO_PKG_VERSION")));
                ui.label(S::ABOUT);
            });
        self.show_about = open;
    }

    fn toasts_overlay(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        self.toasts.retain(|x| now.duration_since(x.born) < x.ttl());
        if self.toasts.is_empty() {
            return;
        }
        let t: Tokens = self.tokens();
        let screen = ctx.content_rect();
        let mut y = screen.max.y - STATUS_H - 12.0;
        let x = screen.max.x - 304.0 - 16.0;
        let mut dismiss = None;
        for (k, toast) in self.toasts.iter().enumerate().rev() {
            let (c, icon) = match toast.level {
                Level::Info => (t.accent, Icon::Check),
                Level::Warning => (t.warning, Icon::Warning),
                Level::Error => (t.danger, Icon::Error),
            };
            let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique(("toast", k)));
            let p = ctx.layer_painter(layer);
            let g = p.layout(toast.text.clone(), text::body(), t.text, 250.0);
            let h = g.size().y.max(16.0) + 18.0;
            let r = Rect::from_min_size(Pos2::new(x, y - h), Vec2::new(304.0, h));
            p.add(egui::Shape::rect_filled(
                r.translate(Vec2::new(0.0, 3.0)).expand(1.0),
                CornerRadius::same(7),
                t.shadow,
            ));
            p.rect(
                r,
                CornerRadius::same(6),
                t.float,
                Stroke::new(1.0, t.border_strong),
                egui::StrokeKind::Inside,
            );
            p.rect_filled(
                Rect::from_min_size(r.min, Vec2::new(3.0, h)),
                CornerRadius {
                    nw: 6,
                    sw: 6,
                    ne: 0,
                    se: 0,
                },
                c,
            );
            icons::paint(
                &p,
                Rect::from_min_size(r.min + Vec2::new(12.0, 9.0), Vec2::splat(16.0)),
                icon,
                c,
            );
            p.galley(r.min + Vec2::new(36.0, 9.0), g, t.text);
            let resp = ctx.read_response(egui::Id::unique(("toast-r", k)));
            let _ = resp;
            if ctx.input(|i| {
                i.pointer.primary_clicked()
                    && i.pointer.interact_pos().is_some_and(|q| r.contains(q))
            }) {
                dismiss = Some(k);
            }
            y -= h + 8.0;
        }
        if let Some(k) = dismiss {
            self.toasts.remove(k);
        }
    }

    pub fn move_current_page(&mut self, d: i64) {
        let n = self.s.doc().pages.len() as i64;
        let Some(i) = self.s.current_page_index() else {
            return;
        };
        let j = i as i64 + d;
        if j >= 0 && j < n {
            self.cmd("page.move", json!({ "from": i, "to": j }));
        }
    }

    /// The cursor readout of the status bar.
    pub fn cursor_text(&self) -> String {
        match self.cursor_mm {
            Some(p) => format!("x {:8.2}  y {:8.2} mm", p.x, p.y),
            None => "x        -  y        - mm".to_string(),
        }
    }
}

/// A painted colour swatch helper for the warning banner.
pub(crate) fn banner(ui: &mut egui::Ui, t: &Tokens, text_: &str) -> bool {
    let w_ = ui.available_width();
    let (r, _) = ui.allocate_exact_size(Vec2::new(w_, 30.0), Sense::hover());
    ui.painter().rect(
        r,
        CornerRadius::same(5),
        t.float,
        Stroke::new(1.0, t.warning),
        egui::StrokeKind::Inside,
    );
    icons::paint(
        ui.painter(),
        Rect::from_min_size(r.min + Vec2::new(8.0, 7.0), Vec2::splat(16.0)),
        Icon::Warning,
        t.warning,
    );
    let s = w::elide(ui, text_, text::body(), w_ - 80.0);
    ui.painter().text(
        r.min + Vec2::new(32.0, 15.0),
        Align2::LEFT_CENTER,
        s,
        text::body(),
        t.text,
    );
    let br = Rect::from_min_size(Pos2::new(r.max.x - 26.0, r.min.y + 4.0), Vec2::splat(22.0));
    let b = ui
        .interact(br, egui::Id::unique("banner-close"), Sense::click())
        .on_hover_text("Dismiss");
    b.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Dismiss"));
    icons::paint(
        ui.painter(),
        Rect::from_center_size(br.center(), Vec2::splat(14.0)),
        Icon::Close,
        t.icon_secondary,
    );
    b.clicked()
}

#[allow(dead_code)]
fn unused(_: Color32) {}

/// A resizable panel whose size is kept in the settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PanelKind {
    Left,
    Inspector,
    Dock,
}

impl App {
    /// Remembers a panel's size (saved debounced).
    pub(crate) fn note_panel_size(&mut self, k: PanelKind, size: f32) {
        if !size.is_finite() || size < 20.0 {
            return;
        }
        let slot = match k {
            PanelKind::Left => &mut self.settings.left_width,
            PanelKind::Inspector => &mut self.settings.inspector_width,
            PanelKind::Dock => &mut self.settings.dock_height,
        };
        if slot.is_none_or(|v| (v - size).abs() > 0.5) {
            *slot = Some(size);
            self.save_settings_soon();
        }
    }
}
