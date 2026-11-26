//! Placement links in the editor: "Link placement to..." (pick the
//! reference on the canvas or in the item tree, or from a list grouped by
//! page), unlink, select reference / linked items, the inspector's
//! "Placement link" section, and the "Layer pages" window
//! (`view.pages_per_layer`).

use eframe::egui;
use kdraft_doc::link::{self, Follow};
use kdraft_doc::{Id, Item};
use serde_json::json;

use crate::app::App;
use crate::theme::text;
use crate::widgets::{self as w, Lead};

/// A page's title and its items that can be a reference.
type PageChoices = (usize, String, Vec<(Id, String)>);

/// State of the layer pages window.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LayerPagesDialog {
    /// Reference view; `None` is a new placement.
    pub view: Option<Id>,
    pub chosen: Vec<String>,
    pub common: bool,
    pub at_end: bool,
    /// Use the (empty, untitled) current page as the first layer page.
    pub use_current: bool,
}

impl LayerPagesDialog {
    /// The command's arguments.
    pub fn args(&self) -> serde_json::Value {
        json!({
            "reference": self.view.as_ref().map_or(kdraft_engine::layer_pages::NEW.to_string(), |v| v.0.clone()),
            "layers": self.chosen,
            "common": if self.common { vec!["Edge.Cuts"] } else { Vec::new() },
            "at_end": self.at_end,
            "use_current_page": self.use_current,
        })
    }
}

impl App {
    /// Starts picking a reference for the selected items; with nothing
    /// selected, first the item to link.
    pub fn start_link_pick(&mut self) {
        let ids = self.s.selection.items.clone();
        if ids.is_empty() {
            self.link_pick = Some(Vec::new());
            self.info("Click the item to link (Esc cancels); then its reference");
            return;
        }
        self.link_pick = Some(ids);
        self.info(
            "Click the reference item on any page, or its row in the item tree (Esc cancels)",
        );
    }

    /// A click while picking: the item to link (first step without a
    /// selection), else the reference.
    pub fn finish_link_pick(&mut self, target: &Id) {
        let Some(ids) = self.link_pick.take() else {
            return;
        };
        if ids.is_empty() {
            let linkable = self
                .s
                .find_item(target)
                .is_some_and(|(_, it)| it.linkable());
            if !linkable {
                self.link_pick = Some(Vec::new());
                self.info("This item cannot be linked; click another one (Esc cancels)");
                return;
            }
            self.cmd("select.set", json!({ "ids": [target.0] }));
            self.link_pick = Some(vec![target.clone()]);
            self.info(
                "Now click the reference item on any page, or its row in the item tree (Esc cancels)",
            );
            return;
        }
        let ids: Vec<String> = ids
            .into_iter()
            .filter(|i| i != target)
            .map(|i| i.0)
            .collect();
        if ids.is_empty() {
            self.link_pick = Some(vec![target.clone()]);
            self.info("That is the item being linked; click its reference (Esc cancels)");
            return;
        }
        self.cmd("link.set", json!({ "ids": ids, "to": target.0 }));
    }

    /// Link entries of the item context menu.
    pub(crate) fn link_menu(&mut self, ui: &mut egui::Ui) {
        let sel: Vec<Item> = self.selected_items().into_iter().cloned().collect();
        let all_linkable = !sel.is_empty() && sel.iter().all(Item::linkable);
        let any_linked = sel.iter().any(|i| i.link().is_some());
        let is_ref = sel.len() == 1
            && sel[0]
                .id()
                .is_some_and(|id| !link::followers(self.s.doc(), id).is_empty());
        ui.add_enabled_ui(all_linkable, |ui| {
            ui.menu_button("Link placement to", |ui| {
                if ui.button("Pick on canvas or in the item tree…").clicked() {
                    ui.close();
                    self.start_link_pick();
                }
                ui.separator();
                let pages: Vec<PageChoices> = self
                    .s
                    .doc()
                    .pages
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        (
                            i,
                            format!("{}. {}", i + 1, p.title.clone().unwrap_or_default()),
                            p.items
                                .iter()
                                .filter(|it| it.linkable())
                                .filter(|it| !sel.iter().any(|s| s.id() == it.id()))
                                .filter_map(|it| {
                                    Some((it.id()?.clone(), crate::tree::display_name(it)))
                                })
                                .collect(),
                        )
                    })
                    .collect();
                for (_, title, items) in pages {
                    if items.is_empty() {
                        continue;
                    }
                    ui.menu_button(title, |ui| {
                        for (id, name) in items {
                            if ui.button(name).clicked() {
                                ui.close();
                                let ids: Vec<String> =
                                    self.s.selection.items.iter().map(|i| i.0.clone()).collect();
                                self.cmd("link.set", json!({ "ids": ids, "to": id.0 }));
                            }
                        }
                    });
                }
            });
        });
        if ui
            .add_enabled(any_linked, egui::Button::new("Unlink"))
            .clicked()
        {
            ui.close();
            self.cmd("link.clear", json!({}));
        }
        if ui
            .add_enabled(
                sel.len() == 1 && any_linked,
                egui::Button::new("Select reference"),
            )
            .clicked()
        {
            ui.close();
            self.cmd("link.select_reference", json!({}));
        }
        if ui
            .add_enabled(
                any_linked || is_ref,
                egui::Button::new("Select linked items"),
            )
            .clicked()
        {
            ui.close();
            self.cmd("link.select_followers", json!({}));
        }
        let views = sel.iter().any(|i| matches!(i, Item::View(_)));
        if ui
            .add_enabled(
                views && (any_linked || is_ref),
                egui::Button::new("Match layer colours to reference"),
            )
            .clicked()
        {
            ui.close();
            self.cmd("link.match_colours", json!({}));
        }
    }

    /// "Copy to page, linked" (paste in place, linked).
    pub(crate) fn copy_linked_menu(&mut self, ui: &mut egui::Ui, pages: &[(String, String)]) {
        let cur = self.s.current_page().map(|p| p.id.0.clone());
        ui.menu_button("Copy to page, linked", |ui| {
            for (pid, title) in pages {
                if Some(pid) == cur.as_ref() {
                    continue;
                }
                if ui.button(title).clicked() {
                    ui.close();
                    self.cmd("item.copy_to_page", json!({ "page": pid, "linked": true }));
                }
            }
        });
    }

    /// Inspector section for a follower or a reference.
    pub(crate) fn link_section(&mut self, ui: &mut egui::Ui, it: &Item) {
        let Some(id) = it.id().cloned() else { return };
        let followers = link::followers(self.s.doc(), &id);
        let l = it.link();
        if l.is_none() && followers.is_empty() {
            return;
        }
        if !self.section(ui, "link", "Placement link") {
            return;
        }
        let t = self.tokens();
        if let Some(l) = l {
            let reference = self
                .s
                .find_item(&l.to)
                .map(|(p, r)| (p, crate::tree::display_name(r)));
            ui.horizontal(|ui| {
                Self::label_row(ui, "Reference");
                match &reference {
                    Some((p, name)) => {
                        let title = self.s.doc().pages[*p].title.clone().unwrap_or_default();
                        let full = format!("{name} (page {}: {title})", p + 1);
                        let shown = w::elide(ui, &full, text::body(), ui.available_width() - 4.0);
                        if ui
                            .link(shown)
                            .on_hover_text("Go to the reference")
                            .clicked()
                        {
                            self.cmd("link.select_reference", json!({ "id": id.0 }));
                        }
                    }
                    None => {
                        ui.colored_label(t.danger, "missing");
                    }
                }
            });
            let allowed = match self.s.find_item(&l.to).map(|(_, r)| r.clone()) {
                Some(r) => link::allowed_follow(it, &r),
                None => vec![Follow::Position],
            };
            ui.horizontal_wrapped(|ui| {
                Self::label_row(ui, "Follows");
                for f in allowed {
                    let on = l.follows(f);
                    if w::chip(ui, f.as_str(), on, true).clicked() {
                        let mut v: Vec<&str> = l
                            .follow
                            .iter()
                            .filter(|x| **x != f)
                            .map(|x| x.as_str())
                            .collect();
                        if !on {
                            v.push(f.as_str());
                        }
                        if v.is_empty() {
                            self.warn("a link follows at least one property (or unlink it)");
                        } else {
                            self.cmd("link.set_follow", json!({ "id": id.0, "follow": v }));
                        }
                    }
                }
            });
            ui.horizontal(|ui| {
                Self::label_row(ui, "Offset");
                let (a, b) = (id.0.clone(), id.0.clone());
                let o = l.offset;
                self.num(
                    ui,
                    "link-dx",
                    Lead::Letter("X"),
                    Some(o.x),
                    "mm",
                    (0.5, 0.1),
                    82.0,
                    "link.set_offset",
                    &move |v| json!({ "ids": [a], "offset": [v, o.y] }),
                );
                self.num(
                    ui,
                    "link-dy",
                    Lead::Letter("Y"),
                    Some(o.y),
                    "mm",
                    (0.5, 0.1),
                    82.0,
                    "link.set_offset",
                    &move |v| json!({ "ids": [b], "offset": [o.x, v] }),
                );
            });
            ui.horizontal(|ui| {
                if w::button(ui, Some(crate::icons::Icon::Unlink), "Unlink", false, true).clicked()
                {
                    self.cmd("link.clear", json!({ "ids": [id.0] }));
                }
            });
            ui.label(
                egui::RichText::new(
                    "Moving any linked item moves the whole group. Alt-drag or the offset moves this one alone.",
                )
                .font(text::small())
                .color(t.text_secondary),
            );
        }
        if !followers.is_empty() {
            ui.label(
                egui::RichText::new(format!(
                    "{} linked item(s) follow this one",
                    followers.len()
                ))
                .font(text::small()),
            );
            for f in followers {
                let Some((p, it)) = self.s.find_item(&f).map(|(p, i)| (p, i.clone())) else {
                    continue;
                };
                let title = self.s.doc().pages[p].title.clone().unwrap_or_default();
                let label = format!(
                    "{} (page {}: {title})",
                    crate::tree::display_name(&it),
                    p + 1
                );
                let shown = w::elide(ui, &label, text::body(), ui.available_width() - 4.0);
                if ui.link(shown).on_hover_text(label).clicked() {
                    let page = self.s.doc().pages[p].id.0.clone();
                    self.cmd("select.set", json!({ "page": page, "ids": [f.0] }));
                }
            }
        }
    }

    /// The "Layer pages" window.
    pub(crate) fn layer_pages_window(&mut self, ctx: &egui::Context) {
        use kdraft_engine::layer_pages as lp;
        let Some(mut d) = self.layer_pages.clone() else {
            return;
        };
        let Some(board) = self.s.board_arc() else {
            self.layer_pages = None;
            return;
        };
        let choices = lp::reference_choices(&self.s);
        let cur = self.s.current_page_index();
        let reusable = cur.is_some_and(|c| lp::page_reusable(&self.s, c));
        if !reusable {
            d.use_current = false;
        }
        // Board order.
        d.chosen
            .sort_by_key(|n| board.layers.iter().position(|l| &l.name == n));
        let args: lp::LayerPagesArgs = serde_json::from_value(d.args()).unwrap_or_default();
        let plan = lp::plan(&mut self.s, &args);
        let t = self.tokens();
        let mut open = true;
        let mut run = false;
        egui::Window::new("Layer pages")
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new(
                        "One page per layer, each with a view of that layer at the same position, scale and rotation.",
                    )
                    .font(text::small()),
                );
                ui.add_space(4.0);
                ui.label(egui::RichText::new("Placement reference").strong());
                egui::ScrollArea::vertical()
                    .id_salt("lp-ref")
                    .max_height(150.0)
                    .show(ui, |ui| {
                        if ui
                            .selectable_label(
                                d.view.is_none(),
                                "New placement (centred, scale fitted to the sheet)",
                            )
                            .clicked()
                        {
                            d.view = None;
                        }
                        for c in &choices {
                            ui.label(
                                egui::RichText::new(format!("Page {}: {}", c.page + 1, c.title))
                                    .font(text::small())
                                    .color(t.text_secondary),
                            );
                            for (id, name, scale) in &c.views {
                                let label = format!(
                                    "    {name}  \u{b7}  page {}  \u{b7}  {}",
                                    c.page + 1,
                                    kdraft_render::view::ratio(*scale)
                                );
                                if ui
                                    .selectable_label(d.view.as_ref() == Some(id), label)
                                    .clicked()
                                {
                                    d.view = Some(id.clone());
                                }
                            }
                        }
                    });
                ui.add_space(4.0);
                ui.label(egui::RichText::new("Layers").strong());
                ui.horizontal_wrapped(|ui| {
                    for (label, set) in [
                        ("All copper", "copper"),
                        ("Silkscreen", "silkscreen"),
                        ("Fab", "fab"),
                        ("Mask", "mask"),
                        ("Courtyard", "courtyard"),
                    ] {
                        let ls = kdraft_engine::layer_set(&board, set);
                        let on = !ls.is_empty() && ls.iter().all(|l| d.chosen.contains(l));
                        if w::chip(ui, label, on, !ls.is_empty()).clicked() {
                            if on {
                                d.chosen.retain(|l| !ls.contains(l));
                            } else {
                                for l in ls {
                                    if !d.chosen.contains(&l) {
                                        d.chosen.push(l);
                                    }
                                }
                            }
                        }
                    }
                });
                egui::ScrollArea::vertical()
                    .id_salt("lp-layers")
                    .max_height(200.0)
                    .show(ui, |ui| {
                        for l in &board.layers {
                            let mut on = d.chosen.contains(&l.name);
                            if ui.checkbox(&mut on, &l.name).changed() {
                                if on {
                                    d.chosen.push(l.name.clone());
                                } else {
                                    d.chosen.retain(|x| x != &l.name);
                                }
                            }
                        }
                    });
                ui.checkbox(&mut d.common, "Add Edge.Cuts to every page");
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.radio_value(&mut d.at_end, false, "After the current page");
                    ui.radio_value(&mut d.at_end, true, "At the end");
                });
                if let (true, Some(c)) = (reusable, cur) {
                    ui.checkbox(
                        &mut d.use_current,
                        format!("Use the empty page {} as the first layer page", c + 1),
                    );
                }
                ui.add_space(4.0);
                let ok = match &plan {
                    Ok(p) => {
                        ui.label(egui::RichText::new(p.summary()).font(text::small()));
                        !p.layers.is_empty()
                    }
                    Err(e) => {
                        ui.colored_label(t.danger, e.to_string());
                        false
                    }
                };
                let n = plan.as_ref().map_or(0, |p| p.layers.len());
                if w::button(ui, None, &format!("Create {n} page(s)"), true, ok).clicked() {
                    run = true;
                }
            });
        if run {
            self.cmd("view.pages_per_layer", d.args());
            self.layer_pages = None;
        } else if open {
            self.layer_pages = Some(d);
        } else {
            self.layer_pages = None;
        }
    }

    /// Opens the layer pages window: the reference is the selected view,
    /// else the only view on the current page, else a new placement.
    pub fn open_layer_pages(&mut self) {
        let view = kdraft_engine::layer_pages::default_reference(&self.s);
        let chosen = self
            .s
            .board()
            .map(|b| kdraft_engine::layer_set(b, "copper"))
            .unwrap_or_default();
        let use_current = self
            .s
            .current_page_index()
            .is_some_and(|c| kdraft_engine::layer_pages::page_reusable(&self.s, c));
        self.layer_pages = Some(LayerPagesDialog {
            view,
            chosen,
            common: true,
            at_end: false,
            use_current,
        });
    }
}
