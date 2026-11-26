//! Left panel: the pages list (thumbnails, select on click, reorder by
//! drag, inline rename, context menu) and the item tree of the current
//! page (select, multi-select, rename, show/hide, lock, z-order drag).

use eframe::egui::{self, Align2, CornerRadius, Pos2, Rect, Sense, Stroke, Vec2};
use serde_json::json;

use crate::app::{App, RenameTarget};
use crate::icons::{self, Icon};
use crate::jobs::{self, RasterWhat};
use crate::pages::PageAction;
use crate::strings as S;
use crate::theme::text;
use crate::widgets::{self as w, Rename};

/// Page entry height and thumbnail width, points.
const PAGE_ROW: f32 = 66.0;
const THUMB_W: f32 = 64.0;
/// Item tree row height.
const TREE_ROW: f32 = 24.0;

impl App {
    pub(crate) fn left_panel(&mut self, ui: &mut egui::Ui) {
        let full = ui.max_rect();
        // The contents live in child uis, which do not grow this ui. Claim
        // the whole rect, or the panel collapses to its minimum width
        // every frame and cannot be resized.
        ui.allocate_rect(full, Sense::hover());
        // Whole page rows only: no half-cut entry at the bottom; no more
        // room than the pages need.
        let header = 34.0;
        let row = crate::ui_left::PAGE_ROW + 4.0;
        let max_h = (full.height() * 0.42).clamp(140.0, 420.0);
        let n = self.s.doc().pages.len().max(1) as f32;
        let fit_rows = ((max_h - header) / row).floor().max(1.0);
        let pages_h = header + row * n.min(fit_rows) + 8.0;
        let top = Rect::from_min_size(full.min, Vec2::new(full.width(), pages_h));
        let bottom = Rect::from_min_max(Pos2::new(full.min.x, top.max.y), full.max);
        let t = self.tokens();
        let mut a = ui.new_child(egui::UiBuilder::new().max_rect(top.shrink2(Vec2::new(8.0, 4.0))));
        self.pages_list(&mut a);
        ui.painter()
            .hline(full.x_range(), top.max.y, Stroke::new(1.0, t.border));
        let mut b =
            ui.new_child(egui::UiBuilder::new().max_rect(bottom.shrink2(Vec2::new(8.0, 4.0))));
        self.item_tree(&mut b);
        // Right border.
        ui.painter()
            .vline(full.max.x - 0.5, full.y_range(), Stroke::new(1.0, t.border));
    }

    // ------------------------------------------------------------ pages

    fn pages_list(&mut self, ui: &mut egui::Ui) {
        let t = self.tokens();
        let (act, _) = w::section_header(ui, S::PAGES, None, &[(Icon::PageAdd, S::ADD_PAGE)]);
        if act == Some(0) {
            self.cmd("page.add", json!({}));
        }
        let cur = self.s.current_page_index();
        let n = self.s.doc().pages.len();
        let mut rows: Vec<(f32, f32)> = Vec::new();
        let mut acts: Vec<PageAction> = Vec::new();
        let mut entries = Vec::new();
        let mut menu_for: Option<usize> = None;
        let mut rename_start: Option<usize> = None;
        let mut list_rect = ui.available_rect_before_wrap();
        egui::ScrollArea::vertical()
            .id_salt("pages-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                list_rect = ui.max_rect();
                for i in 0..n {
                    let page = &self.s.doc().pages[i];
                    let pid = page.id.clone();
                    let title = page
                        .title
                        .clone()
                        .unwrap_or_else(|| format!("Page {}", i + 1));
                    let sub = format!("{} · {} items", page.size.name, page.items.len());
                    let slot = format!("thumb:{pid}");
                    if self.inputs.is_some() {
                        let key = jobs::page_key(self.s.doc(), i, &self.inputs_gen);
                        self.want_texture(&slot, key, RasterWhat::Thumb { page: pid.clone() }, 0);
                    }
                    let tex = self
                        .textures
                        .get(&slot)
                        .map(|x| (x.handle.id(), x.handle.size_vec2()));
                    let wd = ui.available_width();
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(wd, PAGE_ROW), Sense::hover());
                    let r = ui.interact(
                        rect,
                        egui::Id::unique(("page-entry", pid.0.as_str())),
                        Sense::click_and_drag(),
                    );
                    r.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::Role::ListItem,
                            true,
                            cur == Some(i),
                            &title,
                        )
                    });
                    let selected = cur == Some(i);
                    let p = ui.painter();
                    if selected {
                        p.rect_filled(rect, CornerRadius::same(5), t.selected_row);
                    } else if r.hovered() {
                        p.rect_filled(rect, CornerRadius::same(5), t.hover);
                    }
                    // Thumbnail, aspect kept, white paper.
                    let th_h = PAGE_ROW - 12.0;
                    let (tw, th) = match tex {
                        Some((_, s)) if s.x > 0.0 => {
                            let k = (THUMB_W / s.x).min(th_h / s.y.max(1.0));
                            (s.x * k, s.y * k)
                        }
                        _ => (THUMB_W, THUMB_W * 0.707),
                    };
                    let tr = Rect::from_min_size(
                        Pos2::new(
                            rect.min.x + 6.0 + (THUMB_W - tw) / 2.0,
                            rect.center().y - th / 2.0,
                        ),
                        Vec2::new(tw, th),
                    );
                    p.rect_filled(tr, 0.0, t.paper);
                    if let Some((id, _)) = tex {
                        p.image(
                            id,
                            tr,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                    }
                    p.rect_stroke(
                        tr,
                        0.0,
                        if selected {
                            Stroke::new(2.0, t.accent)
                        } else {
                            Stroke::new(1.0, t.border_strong)
                        },
                        egui::StrokeKind::Outside,
                    );
                    let tx = rect.min.x + THUMB_W + 14.0;
                    let avail = rect.max.x - tx - 4.0;
                    p.text(
                        Pos2::new(tx, rect.center().y - 9.0),
                        Align2::LEFT_TOP,
                        format!("{}", i + 1),
                        text::small(),
                        t.text_tertiary,
                    );
                    let title_rect = Rect::from_min_size(
                        Pos2::new(tx + 14.0, rect.center().y - 12.0),
                        Vec2::new(avail - 14.0, 22.0),
                    );
                    let renaming_this =
                        matches!(&self.renaming, Some((RenameTarget::Page(k), _)) if *k == i);
                    if !renaming_this {
                        let s = w::elide(ui, &title, text::heading(), avail - 14.0);
                        ui.painter().text(
                            Pos2::new(tx + 14.0, rect.center().y - 10.0),
                            Align2::LEFT_TOP,
                            s,
                            if selected {
                                text::heading()
                            } else {
                                text::body()
                            },
                            t.text,
                        );
                        let sub = w::elide(ui, &sub, text::small(), avail - 14.0);
                        ui.painter().text(
                            Pos2::new(tx + 14.0, rect.center().y + 6.0),
                            Align2::LEFT_TOP,
                            sub,
                            text::small(),
                            t.text_secondary,
                        );
                    } else {
                        let start = matches!(&self.renaming, Some((_, true)));
                        match w::rename_field(
                            ui,
                            egui::Id::unique(("rename-page", i)),
                            title_rect,
                            &title,
                            start,
                        ) {
                            Rename::Commit(s) => {
                                self.renaming = None;
                                if s != title {
                                    self.cmd("page.rename", json!({ "page": pid.0, "title": s }));
                                }
                            }
                            Rename::Cancel => self.renaming = None,
                            Rename::Editing => {
                                if start {
                                    self.renaming = Some((RenameTarget::Page(i), false));
                                }
                            }
                        }
                    }
                    let r = if w::elide(ui, &title, text::heading(), avail - 14.0) != title {
                        r.on_hover_text(&title)
                    } else {
                        r
                    };
                    if r.double_clicked() {
                        rename_start = Some(i);
                    } else if r.drag_started() {
                        acts.extend(self.page_drag.drag_started(i));
                    } else if r.clicked() {
                        acts.extend(self.page_drag.clicked(i));
                    }
                    if r.secondary_clicked() {
                        menu_for = Some(i);
                    }
                    r.context_menu(|ui| self.page_context_menu(ui, i));
                    if self.page_drag.dragging().is_some() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    } else if r.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    entries.push(rect);
                    rows.push((rect.top(), rect.bottom()));
                    ui.add_space(2.0);
                }
            });
        let _ = menu_for;
        if let Some(i) = rename_start {
            self.renaming = Some((RenameTarget::Page(i), true));
        }
        self.page_entries = entries
            .iter()
            .map(|r| [r.min.x, r.min.y, r.max.x, r.max.y])
            .collect();
        if let Some(from) = self.page_drag.dragging() {
            let p = ui.ctx().pointer_interact_pos();
            if let Some(p) = p {
                let ins = crate::pages::insertion_index(&rows, p.y);
                let y = match rows.get(ins) {
                    Some(r) => r.0 - 1.0,
                    None => rows.last().map_or(0.0, |r| r.1 + 1.0),
                };
                w::insertion_line(
                    ui.painter(),
                    list_rect.left() + 2.0,
                    list_rect.right() - 2.0,
                    y,
                );
                let title = self
                    .s
                    .doc()
                    .pages
                    .get(from)
                    .and_then(|x| x.title.clone())
                    .unwrap_or_default();
                w::drag_ghost(ui.ctx(), p, &title);
            }
            if ui.input(|i| i.pointer.any_released()) {
                let over = p.filter(|p| list_rect.expand(20.0).contains(*p));
                let ins = over.map(|p| crate::pages::insertion_index(&rows, p.y));
                acts.extend(self.page_drag.released(ins, n));
            }
        }
        for a in acts {
            match a {
                PageAction::Select(i) => {
                    let id = self.s.doc().pages[i].id.0.clone();
                    self.cmd("page.select", json!({ "page": id }));
                }
                PageAction::Move { from, to } => {
                    self.cmd("page.move", json!({ "from": from, "to": to }));
                }
            }
        }
    }

    // ------------------------------------------------------------ item tree

    /// Rows of the current page, cached by document revision.
    pub fn tree_rows(&mut self) -> Vec<crate::tree::Row> {
        let page = self.s.current_page().map(|p| p.id.clone());
        let rev = self.s.revision();
        if let Some((r, p, rows)) = &self.tree_cache {
            if *r == rev && *p == page {
                return rows.clone();
            }
        }
        let rows = self
            .s
            .current_page()
            .map(crate::tree::rows)
            .unwrap_or_default();
        self.tree_cache = Some((rev, page, rows.clone()));
        rows
    }

    fn item_tree(&mut self, ui: &mut egui::Ui) {
        let t = self.tokens();
        let rows = self.tree_rows();
        let linked: std::collections::HashSet<kdraft_doc::Id> = self
            .s
            .doc()
            .pages
            .iter()
            .flat_map(|p| p.items.iter())
            .filter_map(|i| i.link().map(|l| (i.id().cloned(), l.to)))
            .flat_map(|(a, b)| a.into_iter().chain(std::iter::once(b)))
            .collect();
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(S::ITEMS)
                    .font(text::heading())
                    .color(t.text),
            );
            ui.label(
                egui::RichText::new(format!("{}", rows.len()))
                    .font(text::small())
                    .color(t.text_tertiary),
            );
        });
        ui.add_space(2.0);
        if rows.is_empty() {
            ui.label(egui::RichText::new(S::NO_ITEMS).color(t.text_tertiary));
            return;
        }
        let mut rects: Vec<(f32, f32)> = Vec::new();
        let mut tree_rects = Vec::new();
        let mut select: Option<(usize, bool, bool)> = None;
        let mut toggles: Vec<(&'static str, String, bool)> = Vec::new();
        let mut rename_start: Option<kdraft_doc::Id> = None;
        let mut link_pick: Option<kdraft_doc::Id> = None;
        let mut menu_row: Option<usize> = None;
        let mut drag_start: Option<usize> = None;
        let mut list_rect = ui.available_rect_before_wrap();
        egui::ScrollArea::vertical()
            .id_salt("tree-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                list_rect = ui.max_rect();
                for (k, row) in rows.iter().enumerate() {
                    let wd = ui.available_width();
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(wd, TREE_ROW), Sense::hover());
                    let r = ui.interact(rect, egui::Id::unique(("tree-row", row.id.0.as_str())), Sense::click_and_drag());
                    let selected = self.s.selection.contains(&row.id);
                    r.widget_info(|| egui::WidgetInfo::selected(egui::Role::TreeItem, true, selected, &row.name));
                    let p = ui.painter();
                    if selected {
                        p.rect_filled(rect, CornerRadius::same(4), t.selected_row);
                    } else if r.hovered() || self.hover_item.as_ref() == Some(&row.id) {
                        p.rect_filled(rect, CornerRadius::same(4), t.hover);
                    }
                    let fg = if row.hidden { t.text_tertiary } else { t.text };
                    icons::paint(p, Rect::from_min_size(rect.min + Vec2::new(6.0, 4.0), Vec2::splat(16.0)), row.icon, if row.hidden { t.text_tertiary } else { t.icon_secondary });
                    // Toggles on the right: lock, eye.
                    let eye = Rect::from_min_size(Pos2::new(rect.max.x - 24.0, rect.min.y + 1.0), Vec2::splat(22.0));
                    let lock = eye.translate(Vec2::new(-24.0, 0.0));
                    let show_toggles = r.hovered() || row.hidden || row.locked || ui.rect_contains_pointer(eye) || ui.rect_contains_pointer(lock);
                    let name_w = rect.width() - 30.0 - 52.0 - if linked.contains(&row.id) { 18.0 } else { 0.0 };
                    let renaming = matches!(&self.renaming, Some((RenameTarget::Item(id), _)) if *id == row.id);
                    if renaming {
                        let start = matches!(&self.renaming, Some((_, true)));
                        let fr = Rect::from_min_size(rect.min + Vec2::new(28.0, 1.0), Vec2::new(name_w, TREE_ROW - 2.0));
                        let initial = self.s.current_page().and_then(|p| p.item(&row.id)).and_then(|i| i.name().map(str::to_string)).unwrap_or_else(|| row.name.clone());
                        match w::rename_field(ui, egui::Id::unique(("rename-item", row.id.0.as_str())), fr, &initial, start) {
                            Rename::Commit(s) => {
                                self.renaming = None;
                                if s != initial {
                                    self.cmd("item.rename", json!({ "id": row.id.0, "name": s }));
                                }
                            }
                            Rename::Cancel => self.renaming = None,
                            Rename::Editing => {
                                if start {
                                    self.renaming = Some((RenameTarget::Item(row.id.clone()), false));
                                }
                            }
                        }
                    } else {
                        let s = w::elide(ui, &row.name, text::body(), name_w);
                        ui.painter().text(Pos2::new(rect.min.x + 28.0, rect.center().y), Align2::LEFT_CENTER, s, text::body(), fg);
                    }
                    if linked.contains(&row.id) {
                        let lr = Rect::from_center_size(Pos2::new(rect.max.x - 60.0, rect.center().y), Vec2::splat(13.0));
                        icons::paint(ui.painter(), lr, Icon::Link, t.accent_text);
                    }
                    if show_toggles {
                        for (br, on, icon_on, icon_off, tip_on, tip_off, what) in [
                            (eye, !row.hidden, Icon::Eye, Icon::EyeOff, S::HIDE, S::SHOW, "hidden"),
                            (lock, row.locked, Icon::Lock, Icon::Unlock, S::UNLOCK, S::LOCK, "locked"),
                        ] {
                            let b = ui.interact(br, egui::Id::unique(("tree-toggle", what, row.id.0.as_str())), Sense::click());
                            let tip = if on { tip_on } else { tip_off };
                            let b = b.on_hover_text(tip);
                            b.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{tip} {}", row.name)));
                            if b.hovered() {
                                ui.painter().rect_filled(br, CornerRadius::same(4), t.pressed);
                            }
                            let dim = (what == "hidden" && on) || (what == "locked" && !on);
                            icons::paint(ui.painter(), Rect::from_center_size(br.center(), Vec2::splat(14.0)), if on { icon_on } else { icon_off }, if dim { t.icon_secondary } else { t.icon });
                            if b.clicked() {
                                let val = if what == "hidden" { on } else { !on };
                                toggles.push((what, row.id.0.clone(), val));
                            }
                        }
                    }
                    if !renaming {
                        if r.double_clicked() {
                            rename_start = Some(row.id.clone());
                        } else if r.drag_started() {
                            drag_start = Some(k);
                        } else if r.clicked() && self.link_pick.is_some() {
                            link_pick = Some(row.id.clone());
                        } else if r.clicked() {
                            let (shift, ctrl) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));
                            select = Some((k, shift, ctrl));
                        }
                        if r.secondary_clicked() {
                            menu_row = Some(k);
                        }
                        r.context_menu(|ui| self.item_context_menu(ui));
                    }
                    if r.hovered() {
                        self.tree_hover = Some(row.id.clone());
                    }
                    rects.push((rect.top(), rect.bottom()));
                    tree_rects.push((row.id.clone(), [rect.min.x, rect.min.y, rect.max.x, rect.max.y]));
                }
            });
        self.tree_rects = tree_rects;
        for (what, id, val) in toggles {
            let cmd = if what == "hidden" {
                "item.set_hidden"
            } else {
                "item.set_locked"
            };
            self.cmd(cmd, json!({ "ids": [id], what: val }));
        }
        if let Some(id) = link_pick {
            self.finish_link_pick(&id);
        }
        if let Some(id) = rename_start {
            self.cmd("select.set", json!({ "ids": [id.0] }));
            self.renaming = Some((RenameTarget::Item(id), true));
        }
        if let Some(k) = menu_row {
            if !self.s.selection.contains(&rows[k].id) {
                self.cmd("select.set", json!({ "ids": [rows[k].id.0] }));
            }
        }
        if let Some((k, shift, ctrl)) = select {
            let ids = crate::tree::click_select(
                &rows,
                &self.s.selection.items,
                k,
                self.tree_anchor,
                shift,
                ctrl,
            );
            if !shift {
                self.tree_anchor = Some(k);
            }
            let ids: Vec<String> = ids.into_iter().map(|i| i.0).collect();
            if ids.is_empty() {
                self.cmd("select.clear", json!({}));
            } else {
                self.cmd("select.set", json!({ "ids": ids }));
            }
        }
        if let Some(k) = drag_start {
            self.tree_drag = Some(k);
            if !self.s.selection.contains(&rows[k].id) {
                self.cmd("select.set", json!({ "ids": [rows[k].id.0] }));
            }
        }
        if let Some(from) = self.tree_drag {
            let p = ui.ctx().pointer_interact_pos();
            if let Some(p) = p {
                let ins = crate::pages::insertion_index(&rects, p.y);
                let y = match rects.get(ins) {
                    Some(r) => r.0,
                    None => rects.last().map_or(0.0, |r| r.1),
                };
                w::insertion_line(
                    ui.painter(),
                    list_rect.left() + 2.0,
                    list_rect.right() - 2.0,
                    y,
                );
                if let Some(row) = rows.get(from) {
                    w::drag_ghost(ui.ctx(), p, &row.name);
                }
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            if ui.input(|i| i.pointer.any_released()) {
                self.tree_drag = None;
                let over = p.filter(|p| list_rect.expand(20.0).contains(*p));
                if let Some(p) = over {
                    let ins = crate::pages::insertion_index(&rects, p.y);
                    if let Some(z) = crate::tree::drop_z(&rows, from, ins) {
                        self.cmd("item.reorder", json!({ "id": rows[from].id.0, "to": z }));
                    }
                }
            }
        }
    }
}
