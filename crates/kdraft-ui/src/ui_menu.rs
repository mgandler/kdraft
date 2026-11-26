//! Menus generated from the action catalogue: the menu bar, context
//! menus, the command palette and the shortcut overview.

use eframe::egui::{self, Align2, CornerRadius, Pos2, Rect, Sense, Vec2};
use kdraft_engine::meta::Category;
use serde_json::json;

use crate::actions::{AppAct, Entry};
use crate::app::{App, Palette, RenameTarget};
use crate::icons::{self, Icon};
use crate::theme::text;
use crate::widgets as w;

/// Width of menu rows.
const MENU_W: f32 = 250.0;

impl App {
    /// One menu row: icon, title, shortcut; greyed when disabled.
    /// Returns true when clicked (and closes the menu).
    pub(crate) fn menu_row(&mut self, ui: &mut egui::Ui, e: &Entry) -> bool {
        let enabled = self.enabled(e);
        let t = self.tokens();
        let (rect, r) = ui.allocate_exact_size(
            Vec2::new(MENU_W, 24.0),
            if enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::MenuItem, enabled, &e.title));
        if enabled && r.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(4), t.hover);
        }
        let fg = if enabled { t.text } else { t.text_tertiary };
        if let Some(i) = e.icon {
            icons::paint(
                ui.painter(),
                Rect::from_min_size(rect.min + Vec2::new(6.0, 4.0), Vec2::splat(16.0)),
                i,
                if enabled { t.icon } else { t.text_tertiary },
            );
        }
        // Toggle state for the view options.
        let checked = match &e.run {
            crate::actions::Run::App(AppAct::ToggleGrid) => Some(self.settings.show_grid),
            crate::actions::Run::App(AppAct::ToggleRulers) => Some(self.settings.show_rulers),
            crate::actions::Run::App(AppAct::ToggleSnap) => Some(self.snap),
            _ => None,
        };
        if checked == Some(true) && e.icon.is_none() {
            icons::paint(
                ui.painter(),
                Rect::from_min_size(rect.min + Vec2::new(6.0, 4.0), Vec2::splat(16.0)),
                Icon::Check,
                fg,
            );
        }
        let title = if checked == Some(true) && e.icon.is_some() {
            format!("{}  ✓", e.title)
        } else {
            e.title.clone()
        };
        ui.painter().text(
            Pos2::new(rect.min.x + 30.0, rect.center().y),
            Align2::LEFT_CENTER,
            title,
            text::body(),
            fg,
        );
        if let Some(sc) = &e.shortcut_text {
            ui.painter().text(
                Pos2::new(rect.max.x - 8.0, rect.center().y),
                Align2::RIGHT_CENTER,
                sc,
                text::small(),
                t.text_tertiary,
            );
        }
        if enabled && r.clicked() {
            ui.close();
            return true;
        }
        false
    }

    fn menu_rows(&mut self, ui: &mut egui::Ui, ids: &[&str]) {
        let mut run = None;
        for id in ids {
            if *id == "-" {
                menu_sep(ui);
                continue;
            }
            if let Some(e) = self.entry(id) {
                if self.menu_row(ui, &e) {
                    run = Some(e);
                }
            }
        }
        if let Some(e) = run {
            self.run_entry(&e);
        }
    }

    pub(crate) fn menu_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().button_padding = Vec2::new(6.0, 3.0);
            ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
            for cat in Category::ALL {
                ui.menu_button(cat.title(), |ui| {
                    ui.set_min_width(MENU_W);
                    let rows: Vec<Option<Entry>> = self
                        .catalogue
                        .menu(cat)
                        .into_iter()
                        .map(|o| o.cloned())
                        .collect();
                    let mut run = None;
                    for r in rows {
                        match r {
                            None => menu_sep(ui),
                            Some(e) => {
                                if self.menu_row(ui, &e) {
                                    run = Some(e);
                                }
                            }
                        }
                    }
                    if cat == Category::File && !self.settings.recent.is_empty() {
                        menu_sep(ui);
                        ui.menu_button("Open recent", |ui| {
                            for p in self.settings.recent.clone() {
                                let name = p.display().to_string();
                                if ui
                                    .button(w::elide(ui, &name, text::body(), 420.0))
                                    .clicked()
                                {
                                    ui.close();
                                    self.run(crate::app::UiAction::OpenPath(p));
                                }
                            }
                        });
                    }
                    if let Some(e) = run {
                        self.run_entry(&e);
                    }
                });
            }
        });
    }

    /// Context menu of a canvas item or an item-tree row (the selection).
    pub(crate) fn item_context_menu(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(MENU_W);
        self.menu_rows(
            ui,
            &[
                "item.duplicate",
                "item.remove",
                "-",
                "app.rename",
                "app.toggle_hidden",
                "app.toggle_locked",
                "-",
                "item.raise:front",
                "item.raise:forward",
                "item.lower:backward",
                "item.lower:back",
                "-",
                "app.fit_selection",
            ],
        );
        if !self.selected_callouts().is_empty() {
            self.menu_rows(ui, &["callout.duplicate_next", "app.renumber_callouts"]);
        }
        sub_indent(ui);
        self.link_menu(ui);
        menu_sep(ui);
        ui.menu_button("Align", |ui| {
            self.menu_rows(
                ui,
                &[
                    "layout.align:left",
                    "layout.align:hcenter",
                    "layout.align:right",
                    "layout.align:top",
                    "layout.align:vcenter",
                    "layout.align:bottom",
                    "-",
                    "layout.distribute:horizontal",
                    "layout.distribute:vertical",
                    "-",
                    "layout.center",
                ],
            );
        });
        let pages: Vec<(String, String)> = self
            .s
            .doc()
            .pages
            .iter()
            .enumerate()
            .map(|(i, p)| {
                (
                    p.id.0.clone(),
                    format!("{}. {}", i + 1, p.title.clone().unwrap_or_default()),
                )
            })
            .collect();
        let cur = self.s.current_page().map(|p| p.id.0.clone());
        self.copy_linked_menu(ui, &pages);
        for (label, cmd) in [
            ("Move to page", "item.move_to_page"),
            ("Copy to page", "item.copy_to_page"),
        ] {
            ui.menu_button(label, |ui| {
                for (pid, title) in &pages {
                    if Some(pid) == cur.as_ref() {
                        continue;
                    }
                    if ui.button(title).clicked() {
                        ui.close();
                        self.cmd(cmd, json!({ "page": pid }));
                    }
                }
            });
        }
    }

    /// Context menu of the empty canvas.
    pub(crate) fn canvas_context_menu(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(MENU_W);
        sub_indent(ui);
        ui.menu_button("Insert", |ui| {
            let rows: Vec<Option<Entry>> = self
                .catalogue
                .menu(Category::Insert)
                .into_iter()
                .map(|o| o.cloned())
                .collect();
            let mut run = None;
            for r in rows {
                match r {
                    None => menu_sep(ui),
                    Some(e) => {
                        if self.menu_row(ui, &e) {
                            run = Some(e);
                        }
                    }
                }
            }
            if let Some(e) = run {
                self.run_entry(&e);
            }
        });
        self.menu_rows(
            ui,
            &[
                "select.all",
                "-",
                "app.fit_page",
                "app.zoom_100",
                "-",
                "app.toggle_grid",
                "app.toggle_rulers",
                "app.toggle_snap",
            ],
        );
    }

    /// Context menu of a page entry (the page is made current first).
    pub(crate) fn page_context_menu(&mut self, ui: &mut egui::Ui, index: usize) {
        ui.set_min_width(MENU_W);
        if self.s.current_page_index() != Some(index) {
            let id = self.s.doc().pages[index].id.0.clone();
            self.cmd("page.select", json!({ "page": id }));
        }
        if let Some(e) = self.entry("app.rename_page") {
            if self.menu_row(ui, &e) {
                self.renaming = Some((RenameTarget::Page(index), true));
            }
        }
        self.menu_rows(
            ui,
            &[
                "page.duplicate",
                "page.remove",
                "-",
                "app.move_page_up",
                "app.move_page_down",
                "-",
                "app.choose_sheet",
                "-",
                "page.add",
            ],
        );
    }

    // ------------------------------------------------------------ palette

    pub(crate) fn palette_window(&mut self, ctx: &egui::Context) {
        let Some(mut pal) = self.palette.clone() else {
            return;
        };
        let t = self.tokens();
        let results: Vec<Entry> = self
            .catalogue
            .search(&pal.query)
            .into_iter()
            .cloned()
            .collect();
        let n = results.len();
        let (up, down, enter) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::Enter),
            )
        });
        if down && n > 0 {
            pal.selected = (pal.selected + 1).min(n - 1);
        }
        if up {
            pal.selected = pal.selected.saturating_sub(1);
        }
        pal.selected = pal.selected.min(n.saturating_sub(1));
        let mut run: Option<Entry> = None;
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::unique("palette"))
            .order(egui::Order::Foreground)
            .fixed_pos(Pos2::new(screen.center().x - 280.0, screen.min.y + 70.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(t.float)
                    .stroke(egui::Stroke::new(1.0, t.border_strong))
                    .corner_radius(CornerRadius::same(8))
                    .shadow(egui::Shadow {
                        offset: [0, 6],
                        blur: 24,
                        spread: 0,
                        color: t.shadow,
                    })
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        ui.set_width(560.0);
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                            icons::paint(
                                ui.painter(),
                                r.shrink(2.0),
                                Icon::Search,
                                t.icon_secondary,
                            );
                            let te = ui.add(
                                egui::TextEdit::singleline(&mut pal.query)
                                    .id(egui::Id::unique("palette-query"))
                                    .hint_text("Search commands")
                                    .frame(egui::Frame::NONE)
                                    .font(egui::FontId::new(14.0, egui::FontFamily::Proportional))
                                    .desired_width(520.0),
                            );
                            if !pal.focused {
                                te.request_focus();
                                pal.focused = true;
                            }
                            if te.changed() {
                                pal.selected = 0;
                            }
                        });
                        menu_sep(ui);
                        egui::ScrollArea::vertical()
                            .max_height(360.0)
                            .show(ui, |ui| {
                                if results.is_empty() {
                                    ui.label(
                                        egui::RichText::new("No matching command")
                                            .color(t.text_tertiary),
                                    );
                                }
                                for (k, e) in results.iter().enumerate().take(60) {
                                    let enabled = self.enabled(e);
                                    let (rect, r) = ui.allocate_exact_size(
                                        Vec2::new(556.0, 28.0),
                                        Sense::click(),
                                    );
                                    r.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::Role::ListItem,
                                            enabled,
                                            &e.title,
                                        )
                                    });
                                    if k == pal.selected {
                                        ui.painter().rect_filled(
                                            rect,
                                            CornerRadius::same(4),
                                            t.selected_row,
                                        );
                                        ui.scroll_to_rect(rect, None);
                                    } else if r.hovered() {
                                        ui.painter().rect_filled(
                                            rect,
                                            CornerRadius::same(4),
                                            t.hover,
                                        );
                                    }
                                    let fg = if enabled { t.text } else { t.text_tertiary };
                                    if let Some(i) = e.icon {
                                        icons::paint(
                                            ui.painter(),
                                            Rect::from_min_size(
                                                rect.min + Vec2::new(8.0, 6.0),
                                                Vec2::splat(16.0),
                                            ),
                                            i,
                                            fg,
                                        );
                                    }
                                    ui.painter().text(
                                        rect.min + Vec2::new(34.0, 14.0),
                                        Align2::LEFT_CENTER,
                                        &e.title,
                                        text::body(),
                                        fg,
                                    );
                                    let right = match &e.shortcut_text {
                                        Some(s) => format!("{}    {s}", e.category.title()),
                                        None => e.category.title().to_string(),
                                    };
                                    ui.painter().text(
                                        Pos2::new(rect.max.x - 8.0, rect.center().y),
                                        Align2::RIGHT_CENTER,
                                        right,
                                        text::small(),
                                        t.text_tertiary,
                                    );
                                    if r.clicked() && enabled {
                                        run = Some(e.clone());
                                    }
                                }
                            });
                    });
            });
        if enter {
            if let Some(e) = results.get(pal.selected) {
                if self.enabled(e) {
                    run = Some(e.clone());
                }
            }
        }
        if let Some(e) = run {
            self.palette = None;
            self.run_entry(&e);
        } else if self.palette.is_some() {
            self.palette = Some(pal);
        }
    }

    // ------------------------------------------------------------ shortcuts

    pub(crate) fn shortcuts_window(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts {
            return;
        }
        let t = self.tokens();
        let mut open = true;
        egui::Window::new("Keyboard shortcuts")
            .open(&mut open)
            .collapsible(false)
            .default_width(640.0)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(560.0)
                    .show(ui, |ui| {
                        ui.columns(2, |cols| {
                            for (k, cat) in Category::ALL.iter().enumerate() {
                                let ui = &mut cols[if k < 3 { 0 } else { 1 }];
                                let mut any = false;
                                for e in
                                    self.catalogue.entries.iter().filter(|e| e.category == *cat)
                                {
                                    let Some(sc) = &e.shortcut_text else { continue };
                                    if !any {
                                        ui.add_space(6.0);
                                        ui.label(
                                            egui::RichText::new(cat.title()).font(text::heading()),
                                        );
                                        any = true;
                                    }
                                    ui.horizontal(|ui| {
                                        ui.label(&e.title);
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                ui.label(
                                                    egui::RichText::new(sc)
                                                        .font(text::mono())
                                                        .color(t.text_secondary),
                                                );
                                            },
                                        );
                                    });
                                }
                            }
                            let ui = &mut cols[1];
                            ui.add_space(6.0);
                            ui.label(egui::RichText::new("Canvas").font(text::heading()));
                            for (a, b) in CANVAS_GESTURES {
                                ui.horizontal(|ui| {
                                    ui.label(*a);
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                egui::RichText::new(*b)
                                                    .font(text::mono())
                                                    .color(t.text_secondary),
                                            );
                                        },
                                    );
                                });
                            }
                        });
                    });
            });
        self.show_shortcuts = open;
    }
}

/// Submenu buttons line up with the titles of the rows (after the icon
/// column).
fn sub_indent(ui: &mut egui::Ui) {
    ui.spacing_mut().button_padding = Vec2::new(30.0, 4.0);
    ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
}

/// A separator line exactly as wide as the menu rows.
fn menu_sep(ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(Vec2::new(MENU_W, 7.0), Sense::hover());
    ui.painter()
        .hline(r.x_range(), r.center().y, egui::Stroke::new(1.0, t.border));
}

/// Mouse gestures and fixed keys of the canvas (not catalogue entries).
pub const CANVAS_GESTURES: &[(&str, &str)] = &[
    ("Zoom at the cursor", "Wheel"),
    ("Pan", "Middle drag, Space+drag"),
    ("Add to / remove from selection", "Shift+click"),
    ("Marquee selection", "Drag on empty sheet"),
    ("Nudge by the snap grid", "Arrows (Shift: 10x)"),
    ("Free scaling of a view", "Alt+drag handle"),
    ("Measure gaps to the selection", "Alt+hover"),
    ("Cancel, back to Select", "Escape"),
    ("Zoom to an item", "Double-click"),
];

#[allow(dead_code)]
fn _unused(_: Palette) {}
