//! Right panel: the inspector. Header with the selection's name and the
//! zoom control, the arrange bar, then sections built from the widget
//! kit: position and scale, view, layers, assembly, table, dimension,
//! notes, callout, graphic, page and document.

use eframe::egui::{self, Align2, Pos2, Rect, Sense, Stroke, Vec2};
use kdraft_doc::{GraphicShape, Item, ViewKind, ViewSide, STANDARD_SCALES};
use serde_json::{json, Value};

use crate::app::App;
use crate::icons::Icon;
use crate::theme::text;
use crate::widgets::{self as w, FieldEdit, Lead, NumberField, Seg};

const PAPERS: [&str; 6] = ["A4", "A3", "A2", "A1", "A0", "USLetter"];
/// Two fields side by side.
const HALF: f32 = 118.0;
const FULL: f32 = 242.0;

impl App {
    pub(crate) fn inspector(&mut self, ui: &mut egui::Ui) {
        let t = self.tokens();
        let full = ui.max_rect();
        // See `left_panel`: claim the rect so the panel keeps its width.
        ui.allocate_rect(full, Sense::hover());
        ui.painter()
            .vline(full.min.x + 0.5, full.y_range(), Stroke::new(1.0, t.border));
        let mut inner =
            ui.new_child(egui::UiBuilder::new().max_rect(full.shrink2(Vec2::new(12.0, 6.0))));
        let ui = &mut inner;
        self.inspector_header(ui);
        self.arrange_bar(ui);
        w::separator(ui);
        egui::ScrollArea::vertical()
            .id_salt("inspector-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                let items: Vec<Item> = self.selected_items().into_iter().cloned().collect();
                match items.len() {
                    0 => self.page_sections(ui),
                    1 => self.item_sections(ui, &items[0]),
                    _ => self.multi_sections(ui, &items),
                }
                ui.add_space(70.0);
            });
    }

    fn inspector_header(&mut self, ui: &mut egui::Ui) {
        let t = self.tokens();
        let wd = ui.available_width();
        let (r, _) = ui.allocate_exact_size(Vec2::new(wd, 34.0), Sense::hover());
        let sel = self.selected_items();
        let title = match sel.len() {
            0 => self
                .s
                .current_page()
                .map(|p| p.title.clone().unwrap_or_else(|| "Page".into()))
                .unwrap_or_default(),
            1 => crate::tree::display_name(sel[0]),
            n => format!("{n} items"),
        };
        let s = w::elide(ui, &title, text::heading(), wd - 90.0);
        ui.painter().text(
            Pos2::new(r.min.x, r.center().y),
            Align2::LEFT_CENTER,
            s,
            text::heading(),
            t.text,
        );
        // Zoom control.
        let zr = Rect::from_min_size(
            Pos2::new(r.max.x - 76.0, r.center().y - 12.0),
            Vec2::new(76.0, 24.0),
        );
        let resp = ui
            .interact(zr, egui::Id::unique("zoom-control"), Sense::click())
            .on_hover_text("Zoom");
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Zoom"));
        if resp.hovered() {
            ui.painter().rect_filled(zr, 4.0, t.hover);
        }
        ui.painter().text(
            Pos2::new(zr.max.x - 18.0, zr.center().y),
            Align2::RIGHT_CENTER,
            format!("{:.0} %", self.zoom_percent()),
            text::body(),
            t.text,
        );
        crate::icons::paint(
            ui.painter(),
            Rect::from_center_size(Pos2::new(zr.max.x - 9.0, zr.center().y), Vec2::splat(10.0)),
            Icon::ChevronDown,
            t.icon_secondary,
        );
        egui::Popup::menu(&resp).show(|ui| {
            ui.set_min_width(230.0);
            for id in [
                "app.zoom_in",
                "app.zoom_out",
                "app.zoom_100",
                "app.fit_page",
                "app.fit_selection",
            ] {
                if let Some(e) = self.entry(id) {
                    if self.menu_row(ui, &e) {
                        self.run_entry(&e);
                    }
                }
            }
        });
    }

    fn arrange_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (k, id) in [
                "layout.align:left",
                "layout.align:hcenter",
                "layout.align:right",
                "layout.align:top",
                "layout.align:vcenter",
                "layout.align:bottom",
                "-",
                "layout.distribute:horizontal",
                "layout.distribute:vertical",
            ]
            .iter()
            .enumerate()
            {
                if *id == "-" {
                    ui.add_space(8.0);
                    continue;
                }
                let Some(e) = self.entry(id) else { continue };
                let enabled = self.enabled(&e);
                // Short labels the tests and tooltips use.
                let tip = match *id {
                    "layout.align:hcenter" => "Align centre",
                    "layout.align:vcenter" => "Align middle",
                    "layout.distribute:horizontal" => "Distribute H",
                    "layout.distribute:vertical" => "Distribute V",
                    _ => e.title.as_str(),
                };
                if w::icon_button(
                    ui,
                    e.icon.unwrap_or(Icon::More),
                    tip,
                    e.shortcut_text.as_deref(),
                    enabled,
                    false,
                )
                .clicked()
                {
                    self.run_entry(&e);
                }
                let _ = k;
            }
        });
    }

    /// A section with a header; returns whether it is open.
    pub(crate) fn section(&mut self, ui: &mut egui::Ui, key: &'static str, title: &str) -> bool {
        ui.add_space(4.0);
        let open = !self.closed_sections.contains(key);
        let (_, toggled) = w::section_header(ui, title, Some(open), &[]);
        if toggled {
            if open {
                self.closed_sections.insert(key);
            } else {
                self.closed_sections.remove(key);
            }
        }
        open && !toggled || (!open && toggled)
    }

    /// A number field running `cmd`: scrubbing is one undo step.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn num(
        &mut self,
        ui: &mut egui::Ui,
        key: &str,
        lead: Lead,
        value: Option<f64>,
        suffix: &'static str,
        step: (f64, f64),
        width: f32,
        cmd: &'static str,
        make: &dyn Fn(f64) -> Value,
    ) {
        let (_, edit) = NumberField::new(key, lead, value)
            .suffix(suffix)
            .step(step.0, step.1)
            .width(width)
            .decimals(3)
            .show(ui);
        match edit {
            Some(FieldEdit::Begin) => {
                let title = self.reg.spec(cmd).map_or(cmd, |sp| sp.title);
                self.s.begin_gesture(title);
                self.prop_drag = Some(cmd);
            }
            Some(FieldEdit::Preview(v)) => {
                if self.s.in_gesture() && self.prop_drag == Some(cmd) {
                    if let Err(e) = self.reg.preview(&mut self.s, cmd, make(v)) {
                        self.error(format!("{cmd}: {e}"));
                    }
                }
            }
            Some(FieldEdit::Commit) => {
                if self.prop_drag == Some(cmd) {
                    self.s.commit_gesture();
                    self.prop_drag = None;
                }
            }
            Some(FieldEdit::Set(v)) => {
                self.cmd(cmd, make(v));
            }
            None => {}
        }
    }

    pub(crate) fn label_row(ui: &mut egui::Ui, label: &str) {
        let t = crate::theme::Tokens::get(ui.ctx());
        ui.add_sized(
            [w::LABEL_W, 20.0],
            egui::Label::new(
                egui::RichText::new(label)
                    .font(text::small())
                    .color(t.text_secondary),
            )
            .selectable(false),
        );
    }

    // ------------------------------------------------------------ page

    fn page_sections(&mut self, ui: &mut egui::Ui) {
        let Some(pi) = self.s.current_page_index() else {
            return;
        };
        let page = self.s.doc().pages[pi].clone();
        if self.section(ui, "page", "Page") {
            ui.horizontal(|ui| {
                Self::label_row(ui, "Title");
                if let Some(s) = w::text_field(
                    ui,
                    ("page-title", pi),
                    page.title.as_deref().unwrap_or(""),
                    FULL - w::LABEL_W,
                    "Page title",
                ) {
                    self.cmd("page.rename", json!({ "title": s }));
                }
            });
            ui.horizontal(|ui| {
                Self::label_row(ui, "Paper");
                let names: Vec<&str> = PAPERS.to_vec();
                if let Some(k) = w::dropdown(ui, "paper", &page.size.name, &names, 148.0) {
                    let portrait = page.size.height > page.size.width;
                    self.cmd(
                        "page.set_paper",
                        json!({ "paper": names[k], "portrait": portrait }),
                    );
                }
            });
            ui.horizontal(|ui| {
                Self::label_row(ui, "Orientation");
                let portrait = page.size.height > page.size.width;
                if let Some(k) = w::segmented(
                    ui,
                    "orient",
                    &[Seg::text("Landscape"), Seg::text("Portrait")],
                    Some(usize::from(portrait)),
                    160.0,
                ) {
                    if (k == 1) != portrait && page.size.name != "User" {
                        self.cmd(
                            "page.set_paper",
                            json!({ "paper": page.size.name, "portrait": k == 1 }),
                        );
                    }
                }
            });
            ui.horizontal(|ui| {
                Self::label_row(ui, "Size");
                ui.label(
                    egui::RichText::new(format!("{} x {} mm", page.size.width, page.size.height))
                        .font(text::small()),
                );
            });
            let (_, origin, msg) = self.s.sheet_for_page(pi);
            ui.horizontal(|ui| {
                Self::label_row(ui, "Sheet");
                let name = page.sheet.as_deref().map_or_else(
                    || format!("{origin:?}"),
                    |s| s.rsplit(['/', '\\']).next().unwrap_or(s).to_string(),
                );
                ui.label(
                    egui::RichText::new(w::elide(ui, &name, text::body(), 90.0)).font(text::body()),
                );
                if w::button(ui, None, "Choose…", false, true).clicked() {
                    self.app_act(crate::actions::AppAct::ChooseSheet);
                }
                if page.sheet.is_some()
                    && w::icon_button(
                        ui,
                        Icon::Close,
                        "Use the document's sheet",
                        None,
                        true,
                        false,
                    )
                    .clicked()
                {
                    self.cmd("page.set_sheet", json!({ "sheet": Value::Null }));
                }
            });
            if let Some(m) = msg {
                ui.label(
                    egui::RichText::new(m)
                        .font(text::small())
                        .color(self.tokens().warning),
                );
            }
        }
        if self.section(ui, "document", "Document") {
            ui.horizontal(|ui| {
                Self::label_row(ui, "Snap grid");
                let g = self.s.doc().styles.snap_grid;
                self.num(
                    ui,
                    "snap-grid",
                    Lead::Icon(Icon::Grid),
                    Some(g),
                    "mm",
                    (0.05, 0.01),
                    HALF,
                    "document.set_snap_grid",
                    &|v| json!({ "grid": v.max(0.01) }),
                );
            });
            ui.horizontal(|ui| {
                Self::label_row(ui, "Board");
                let b = self
                    .s
                    .board_state()
                    .path
                    .as_deref()
                    .and_then(|p| p.file_name())
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "none".into());
                ui.label(egui::RichText::new(w::elide(ui, &b, text::body(), 130.0)));
                if w::icon_button(
                    ui,
                    Icon::Reload,
                    "Reload board",
                    Some("F5"),
                    self.s.board_state().path.is_some(),
                    false,
                )
                .clicked()
                {
                    self.app_act(crate::actions::AppAct::ReloadBoard);
                }
            });
            let has_bom = self.s.doc().pages.iter().any(|p| {
                p.items
                    .iter()
                    .any(|i| matches!(i, Item::Table(t) if t.kind == kdraft_doc::TableKind::Bom))
            });
            if has_bom {
                ui.horizontal(|ui| {
                    Self::label_row(ui, "BOM");
                    let src = if self.s.schematic_bom().is_some() {
                        "from the schematic"
                    } else {
                        "from the PCB"
                    };
                    ui.label(egui::RichText::new(src).font(text::small()));
                    if w::button(ui, None, "Renumber", false, true)
                        .on_hover_text("Number the BOM items 1, 2, ... in table order")
                        .clicked()
                    {
                        self.cmd("bom.renumber", json!({}));
                    }
                });
            }
            self.callout_numbering_rows(ui);
            self.document_palette_row(ui);
        }
    }

    // ------------------------------------------------------------ items

    fn multi_sections(&mut self, ui: &mut egui::Ui, items: &[Item]) {
        if self.section(ui, "match", "Match and centre") {
            ui.horizontal(|ui| {
                for id in [
                    "layout.match:width",
                    "layout.match:height",
                    "layout.match:scale",
                    "layout.center",
                ] {
                    if let Some(e) = self.entry(id) {
                        let en = self.enabled(&e);
                        if w::icon_button(
                            ui,
                            e.icon.unwrap_or(Icon::More),
                            &e.title,
                            e.shortcut_text.as_deref(),
                            en,
                            false,
                        )
                        .clicked()
                        {
                            self.run_entry(&e);
                        }
                    }
                }
            });
            ui.label(
                egui::RichText::new("The first selected item is the reference.")
                    .font(text::small())
                    .color(self.tokens().text_secondary),
            );
        }
        let views: Vec<&kdraft_doc::View> = items
            .iter()
            .filter_map(|i| match i {
                Item::View(v) => Some(v),
                _ => None,
            })
            .collect();
        if let Some(v) = views.first() {
            let scale = v.scale;
            let same = views.iter().all(|x| (x.scale - scale).abs() < 1e-12);
            if self.section(ui, "scale", &format!("Scale of {} views", views.len())) {
                self.scale_row(ui, same.then_some(scale));
            }
        }
        let callouts: Vec<kdraft_doc::Callout> = items
            .iter()
            .filter_map(|i| match i {
                Item::Callout(c) => Some(c.clone()),
                _ => None,
            })
            .collect();
        if callouts.len() == items.len() {
            self.callout_section(ui, &callouts);
        }
        self.style_section(ui, items);
        self.item_actions(ui);
    }

    fn scale_row(&mut self, ui: &mut egui::Ui, scale: Option<f64>) {
        let labels: Vec<String> = STANDARD_SCALES
            .iter()
            .map(|s| kdraft_render::view::scale_label(*s).replace("SCALE ", ""))
            .collect();
        let segs: Vec<Seg<'_>> = labels.iter().map(|l| Seg::text(l)).collect();
        let active = scale.and_then(|s| STANDARD_SCALES.iter().position(|x| (x - s).abs() < 1e-9));
        if let Some(k) = w::segmented(ui, "scales", &segs, active, FULL) {
            self.cmd("item.set_scale", json!({ "scale": STANDARD_SCALES[k] }));
        }
        ui.horizontal(|ui| {
            Self::label_row(ui, "Custom");
            self.num(
                ui,
                "scale",
                Lead::Letter("S"),
                scale,
                ":1",
                (0.1, 0.01),
                HALF,
                "item.set_scale",
                &|v| json!({ "scale": v.max(0.001) }),
            );
        });
    }

    fn position_section(&mut self, ui: &mut egui::Ui, it: &Item) {
        let Some(p) = it.position() else { return };
        let id = it.id().map(|i| i.0.clone()).unwrap_or_default();
        if !self.section(ui, "position", "Position") {
            return;
        }
        ui.horizontal(|ui| {
            let (a, b) = (id.clone(), id.clone());
            self.num(
                ui,
                ("x", &id).0,
                Lead::Letter("X"),
                Some(p.x),
                "mm",
                (0.5, 0.1),
                HALF,
                "item.set_position",
                &move |v| json!({ "id": a, "at": [v, p.y] }),
            );
            self.num(
                ui,
                ("y", &id).0,
                Lead::Letter("Y"),
                Some(p.y),
                "mm",
                (0.5, 0.1),
                HALF,
                "item.set_position",
                &move |v| json!({ "id": b, "at": [p.x, v] }),
            );
        });
    }

    fn item_sections(&mut self, ui: &mut egui::Ui, it: &Item) {
        self.position_section(ui, it);
        self.link_section(ui, it);
        let id = it.id().map(|i| i.0.clone()).unwrap_or_default();
        match it {
            Item::View(v) => self.view_sections(ui, v),
            Item::Table(t) => {
                if self.section(ui, "table", "Table") {
                    ui.horizontal(|ui| {
                        Self::label_row(ui, "Kind");
                        let kinds = ["drill", "stackup", "bom", "custom"];
                        if let Some(k) =
                            w::dropdown(ui, "table-kind", t.kind.as_str(), &kinds, 120.0)
                        {
                            self.cmd("item.set_kind", json!({ "kind": kinds[k] }));
                        }
                    });
                    if let Some((start, n)) = t.rows {
                        ui.label(
                            egui::RichText::new(format!(
                                "Rows {} to {} of a split table",
                                start + 1,
                                start + n
                            ))
                            .font(text::small()),
                        );
                    }
                    if t.kind == kdraft_doc::TableKind::Bom {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} stored item numbers",
                                    t.bom_items.len()
                                ))
                                .font(text::small()),
                            );
                            if w::button(ui, None, "Renumber BOM", false, true).clicked() {
                                self.cmd("bom.renumber", json!({}));
                            }
                        });
                    }
                    if t.kind == kdraft_doc::TableKind::Custom {
                        ui.label(
                            egui::RichText::new("Custom tables are not rendered yet.")
                                .font(text::small()),
                        );
                    }
                }
            }
            Item::Dimension(d) => self.dimension_section(ui, d),
            Item::NoteList(n) => self.notes_section(ui, n),
            Item::Callout(c) => self.callout_section(ui, std::slice::from_ref(c)),
            Item::Graphic(g) => self.graphic_section(ui, &id, &g.shape),
            Item::Unknown(_) => {
                ui.label("Unknown item kind (kept as written).");
            }
        }
        self.style_section(ui, std::slice::from_ref(it));
        self.item_actions(ui);
    }

    fn item_actions(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        let acts = [
            (Icon::Duplicate, "Duplicate"),
            (Icon::Raise, "Bring forward"),
            (Icon::Lower, "Send backward"),
            (Icon::Trash, "Delete"),
        ];
        let (hit, _) = w::section_header(ui, "Item", None, &acts);
        match hit {
            Some(0) => self.run_id("item.duplicate"),
            Some(1) => self.run_id("item.raise:forward"),
            Some(2) => self.run_id("item.lower:backward"),
            Some(3) => self.run_id("item.remove"),
            _ => {}
        }
        let sel: Vec<Item> = self.selected_items().into_iter().cloned().collect();
        if sel.len() == 1 {
            let it = sel[0].clone();
            ui.horizontal(|ui| {
                Self::label_row(ui, "Name");
                let id = it.id().map(|i| i.0.clone()).unwrap_or_default();
                if let Some(s) = w::text_field(
                    ui,
                    ("item-name", &id),
                    it.name().unwrap_or(""),
                    FULL - w::LABEL_W,
                    &crate::tree::default_name(&it),
                ) {
                    self.cmd("item.rename", json!({ "id": id, "name": s }));
                }
            });
        }
        let hidden = sel.iter().all(|i| i.hidden());
        let locked = sel.iter().all(|i| i.locked());
        ui.horizontal(|ui| {
            let mut h = hidden;
            if ui.checkbox(&mut h, "Hidden").changed() {
                self.cmd("item.set_hidden", json!({ "hidden": h }));
            }
            let mut l = locked;
            if ui.checkbox(&mut l, "Locked").changed() {
                self.cmd("item.set_locked", json!({ "locked": l }));
            }
        });
    }

    // ------------------------------------------------------------ view

    fn render3d_section(&mut self, ui: &mut egui::Ui, v: &kdraft_doc::View) {
        let bb = self.s.board_box();
        if v.render3d().to_scale() {
            if self.section(ui, "scale1", "Scale") {
                self.scale_row(ui, Some(v.scale));
            }
        } else if self.section(ui, "scale1", "Size") {
            // No single scale for this camera: the width on paper of the
            // board box the picture is fitted to.
            let bw = bb.width().max(1e-6);
            ui.horizontal(|ui| {
                Self::label_row(ui, "Width");
                self.num(
                    ui,
                    "r3d-width",
                    Lead::Letter("W"),
                    Some(bw * v.scale),
                    "mm",
                    (5.0, 0.2),
                    HALF,
                    "item.set_scale",
                    &move |w| json!({ "scale": (w / bw).max(0.01) }),
                );
            });
        }
        if !self.section(ui, "render3d", "3D view") {
            return;
        }
        let o = v.render3d();
        let id = v.id.0.clone();
        let presets = ["top", "bottom", "isometric", "isometric_bottom"];
        let labels = ["Top", "Bottom", "Isometric", "Iso bottom"];
        let segs: Vec<Seg<'_>> = labels.iter().map(|l| Seg::text(l)).collect();
        let cur = match (o.side.as_str(), o.rotate) {
            ("top", None) => Some(0),
            ("bottom", None) => Some(1),
            ("top", Some(r)) if r == kdraft_doc::render3d::ISOMETRIC => Some(2),
            ("bottom", Some(r)) if r == kdraft_doc::render3d::ISOMETRIC => Some(3),
            _ => None,
        };
        if let Some(k) = w::segmented(ui, "r3d-preset", &segs, cur, FULL) {
            self.cmd(
                "view.set_render3d",
                json!({ "ids": [id], "preset": presets[k] }),
            );
        }
        ui.horizontal(|ui| {
            Self::label_row(ui, "Side");
            if let Some(k) =
                w::dropdown(ui, "r3d-side", &o.side, &kdraft_doc::render3d::SIDES, 120.0)
            {
                self.cmd(
                    "view.set_render3d",
                    json!({ "ids": [id], "side": kdraft_doc::render3d::SIDES[k] }),
                );
            }
        });
        ui.horizontal(|ui| {
            let mut p = o.perspective;
            if ui.checkbox(&mut p, "Perspective").changed() {
                self.cmd(
                    "view.set_render3d",
                    json!({ "ids": [id], "perspective": p }),
                );
            }
            let mut white = o.background == "white";
            if ui.checkbox(&mut white, "White background").changed() {
                let b = if white { "white" } else { "transparent" };
                self.cmd("view.set_render3d", json!({ "ids": [id], "background": b }));
            }
        });
        ui.horizontal(|ui| {
            let mut st = o.stackup_colors;
            if ui.checkbox(&mut st, "Stackup colours").changed() {
                self.cmd(
                    "view.set_render3d",
                    json!({ "ids": [id], "stackup_colors": st }),
                );
            }
            let mut fl = o.floor;
            if ui.checkbox(&mut fl, "Floor and shadows").changed() {
                self.cmd("view.set_render3d", json!({ "ids": [id], "floor": fl }));
            }
        });
        ui.horizontal(|ui| {
            Self::label_row(ui, "Appearance");
            let opts = ["follow_plot_settings", "follow_pcb_editor"];
            let cur = o.appearance.clone().unwrap_or_else(|| opts[0].to_string());
            if let Some(k) = w::dropdown(ui, "r3d-appearance", &cur, &opts, 160.0) {
                self.cmd(
                    "view.set_render3d",
                    json!({ "ids": [id], "appearance": opts[k] }),
                );
            }
        });
        ui.horizontal(|ui| {
            Self::label_row(ui, "Quality");
            let q = ["basic", "high"];
            let segs: Vec<Seg<'_>> = q.iter().map(|l| Seg::text(l)).collect();
            if let Some(k) = w::segmented(
                ui,
                "r3d-quality",
                &segs,
                q.iter().position(|x| *x == o.quality),
                120.0,
            ) {
                self.cmd("view.set_render3d", json!({ "ids": [id], "quality": q[k] }));
            }
        });
        ui.horizontal(|ui| {
            Self::label_row(ui, "Resolution");
            let b = id.clone();
            self.num(
                ui,
                "r3d-dpi",
                Lead::None,
                Some(f64::from(o.dpi)),
                "dpi",
                (50.0, 1.0),
                HALF,
                "view.set_render3d",
                &move |d| json!({ "ids": [b], "dpi": d.round().clamp(10.0, 1200.0) as i64 }),
            );
        });
        let key = kdraft_engine::pipeline::render_key(v, bb);
        let status = match (self.kicad_found(), self.s.renders.get(&key)) {
            (false, _) => "kicad-cli not found: the view shows a placeholder.".to_string(),
            (true, Some(kdraft_render::render3d::State::Ready(p))) => format!(
                "{} from kicad-cli pcb render, trimmed to the board (cached).{}",
                p.note,
                if p.outline_px.is_some() {
                    " Board outline at the view's scale."
                } else {
                    " Not to scale."
                }
            ),
            (true, Some(kdraft_render::render3d::State::Pending(m))) => m.clone(),
            (true, Some(kdraft_render::render3d::State::Failed(e))) => format!("Failed: {e}"),
            (true, None) => "Waiting to render...".to_string(),
        };
        ui.label(
            egui::RichText::new(status)
                .font(text::small())
                .color(self.tokens().text_secondary),
        );
    }

    fn view_sections(&mut self, ui: &mut egui::Ui, v: &kdraft_doc::View) {
        if v.kind == ViewKind::Render3d {
            self.render3d_section(ui, v);
            return;
        }
        if self.section(ui, "scale1", "Scale and rotation") {
            self.scale_row(ui, Some(v.scale));
            ui.horizontal(|ui| {
                Self::label_row(ui, "Rotation");
                self.num(
                    ui,
                    "rotation",
                    Lead::Letter("R"),
                    Some(v.rotation),
                    "°",
                    (90.0, 1.0),
                    74.0,
                    "item.set_rotation",
                    &|r| json!({ "rotation": r }),
                );
                let segs = [
                    Seg::text("0"),
                    Seg::text("90"),
                    Seg::text("180"),
                    Seg::text("270"),
                ];
                let cur = [0.0, 90.0, 180.0, 270.0]
                    .iter()
                    .position(|a| (a - v.rotation).abs() < 1e-9);
                if let Some(k) = w::segmented(ui, "rot", &segs, cur, 92.0) {
                    self.cmd(
                        "item.set_rotation",
                        json!({ "rotation": ([0.0, 90.0, 180.0, 270.0])[k] }),
                    );
                }
            });
        }
        if self.section(ui, "view", "View") {
            ui.horizontal(|ui| {
                Self::label_row(ui, "Kind");
                let kinds = ["fabrication", "assembly", "drill", "layer"];
                if let Some(k) = w::dropdown(ui, "view-kind", v.kind.as_str(), &kinds, 150.0) {
                    self.cmd("item.set_kind", json!({ "kind": kinds[k] }));
                }
            });
            ui.horizontal(|ui| {
                Self::label_row(ui, "Side");
                let cur = usize::from(v.side == ViewSide::Back);
                if let Some(k) = w::segmented(
                    ui,
                    "side",
                    &[Seg::text("Top"), Seg::text("Bottom")],
                    Some(cur),
                    150.0,
                ) {
                    self.cmd(
                        "item.set_side",
                        json!({ "side": if k == 0 { "front" } else { "back" } }),
                    );
                }
            });
            if v.kind != ViewKind::Drill {
                ui.horizontal(|ui| {
                    Self::label_row(ui, "Render");
                    let cur = match v.render.as_ref().map(|r| r.as_str()) {
                        Some("plot") => 1,
                        Some("native") => 2,
                        _ => 0,
                    };
                    let segs = [Seg::text("Default"), Seg::text("Plot"), Seg::text("Native")];
                    if let Some(k) = w::segmented(ui, "render", &segs, Some(cur), 168.0) {
                        self.cmd(
                            "item.set_render",
                            json!({ "mode": (["default", "plot", "native"])[k] }),
                        );
                    }
                });
            }
            ui.horizontal(|ui| {
                Self::label_row(ui, "Drill marks");
                let marks = ["default", "none", "small", "actual"];
                let cur = v
                    .drill_marks
                    .as_ref()
                    .map_or("default".to_string(), |m| m.as_str().to_string());
                if let Some(k) = w::dropdown(ui, "marks", &cur, &marks, 150.0) {
                    self.cmd("item.set_drill_marks", json!({ "marks": marks[k] }));
                }
            });
            ui.horizontal(|ui| {
                Self::label_row(ui, "");
                let mut f = v.frame;
                if ui.checkbox(&mut f, "Frame").changed() {
                    self.cmd("item.set_frame", json!({ "frame": f }));
                }
            });
        }
        if v.kind != ViewKind::Drill {
            self.layers_section(ui, v);
        }
        let fab = v.kind == ViewKind::Assembly || v.layers.iter().any(|l| l.ends_with(".Fab"));
        if fab && self.section(ui, "assembly", "Assembly") {
            {
                let a = v.assembly.clone();
                let plot = v.wants_plot();
                let mut items = vec![
                    ("values", "Show values", a.values()),
                    ("references", "Show reference designators", a.references()),
                ];
                if plot {
                    items.push(("fab_other", "Show other fab texts", a.fab_other()));
                }
                if v.kind == ViewKind::Assembly && !plot {
                    items.push(("pads", "Pad outlines", a.pads()));
                    items.push(("holes", "Holes", a.holes()));
                    items.push(("pin1", "Pin-1 marks", a.pin1()));
                }
                for (key, label, val) in items {
                    let mut b = val;
                    if ui.checkbox(&mut b, label).changed() {
                        self.cmd("item.set_assembly", json!({ key: b }));
                    }
                }
                if v.kind == ViewKind::Assembly && !plot {
                    for (key, label, cur, values) in [
                        (
                            "dnp",
                            "DNP parts",
                            a.dnp().as_str().to_string(),
                            &["none", "crossed", "greyed", "hidden"][..],
                        ),
                        (
                            "refs",
                            "Designators",
                            a.refs().as_str().to_string(),
                            &["placed", "pos_files", "all"][..],
                        ),
                    ] {
                        ui.horizontal(|ui| {
                            Self::label_row(ui, label);
                            if let Some(k) = w::dropdown(ui, key, &cur, values, 120.0) {
                                self.cmd("item.set_assembly", json!({ key: values[k] }));
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        Self::label_row(ui, "Text height");
                        self.num(
                            ui,
                            "text-min",
                            Lead::Letter("↓"),
                            Some(a.text_min()),
                            "mm",
                            (0.1, 0.01),
                            82.0,
                            "item.set_assembly",
                            &|x| json!({ "text_min": x.clamp(0.3, 10.0) }),
                        );
                        self.num(
                            ui,
                            "text-max",
                            Lead::Letter("↑"),
                            Some(a.text_max()),
                            "mm",
                            (0.1, 0.01),
                            82.0,
                            "item.set_assembly",
                            &|x| json!({ "text_max": x.clamp(0.3, 10.0) }),
                        );
                    });
                }
            }
        }
    }

    fn layers_section(&mut self, ui: &mut egui::Ui, v: &kdraft_doc::View) {
        if !self.section(ui, "layers", "Layers") {
            return;
        }
        let board: Vec<String> = self.layer_names();
        let presets = crate::layers::presets(&board);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
            for p in &presets {
                let on = crate::layers::is_preset(&v.layers, p);
                if w::chip(ui, p.label, on, true).clicked() && !on {
                    self.cmd("item.set_layers", json!({ "layers": p.layers }));
                }
            }
        });
        self.layer_colour_presets(ui, v);
        let filter = self.layer_filter.clone();
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.layer_filter)
                    .hint_text("Filter layers")
                    .desired_width(FULL)
                    .font(text::body()),
            );
            r.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Filter layers")
            });
        });
        let rows = crate::layers::ordered(&board, &v.layers, &filter);
        let n_checked = v.layers.len();
        let limit = if self.layers_expanded || !filter.is_empty() {
            rows.len()
        } else {
            crate::layers::COLLAPSED_ROWS.max(n_checked)
        };
        let mut layers = v.layers.clone();
        let mut changed = false;
        let mut moved: Option<Vec<String>> = None;
        for (name, on) in rows.iter().take(limit) {
            let mut b = *on;
            let row = ui.horizontal(|ui| {
                let r = ui.checkbox(&mut b, "");
                let label = ui.add(
                    egui::Label::new(egui::RichText::new(name.as_str()).font(text::body()))
                        .sense(if *on { Sense::drag() } else { Sense::hover() })
                        .selectable(false),
                );
                if *on {
                    label.dnd_set_drag_payload(name.clone());
                    ui.add_space((128.0 - label.rect.width()).max(4.0));
                    self.layer_colour_cell(ui, v, name);
                }
                (r, label)
            });
            let (r, _) = row.inner;
            if r.changed() {
                changed = true;
                if b {
                    // Newly shown layers go on top.
                    layers.push(name.clone());
                } else {
                    layers.retain(|l| l != name);
                }
            }
            if *on {
                if let Some(src) = row.response.dnd_hover_payload::<String>() {
                    if *src != *name {
                        let y = row.response.rect.top();
                        w::insertion_line(
                            ui.painter(),
                            row.response.rect.left(),
                            row.response.rect.right(),
                            y,
                        );
                    }
                }
                if let Some(src) = row.response.dnd_release_payload::<String>() {
                    if *src != *name {
                        moved = Some(crate::layers::reordered(&v.layers, &src, name));
                    }
                }
            }
        }
        if let Some(l) = moved {
            self.cmd("item.set_layers", json!({ "layers": l }));
        }
        if rows.len() > limit || self.layers_expanded {
            let label = if self.layers_expanded {
                "Show fewer layers".to_string()
            } else {
                format!("Show all {} layers", rows.len())
            };
            if w::disclosure(ui, &label, self.layers_expanded) {
                self.layers_expanded = !self.layers_expanded;
            }
        }
        if changed {
            self.cmd("item.set_layers", json!({ "layers": layers }));
        }
    }

    /// The board's layer names, cached by board hash.
    fn layer_names(&mut self) -> Vec<String> {
        let h = self.s.board().map(|b| b.hash.0);
        if let Some((k, v)) = &self.layer_cache {
            if Some(*k) == h {
                return v.clone();
            }
        }
        let v: Vec<String> = self
            .s
            .board()
            .map(|b| b.layers.iter().map(|l| l.name.clone()).collect())
            .unwrap_or_default();
        if let Some(h) = h {
            self.layer_cache = Some((h, v.clone()));
        }
        v
    }

    // ------------------------------------------------------------ others

    fn dimension_section(&mut self, ui: &mut egui::Ui, d: &kdraft_doc::Dimension) {
        if !self.section(ui, "dimension", "Dimension") {
            return;
        }
        let id = d.id.0.clone();
        let g = self.dim_geometry(d);
        let t = self.tokens();
        ui.horizontal(|ui| {
            Self::label_row(ui, "Kind");
            ui.label(d.kind.as_str());
        });
        if let Some(g) = &g {
            ui.horizontal(|ui| {
                Self::label_row(ui, "Value");
                ui.label(egui::RichText::new(&g.text).font(text::body()));
                if g.broken {
                    ui.label(
                        egui::RichText::new("broken anchor")
                            .color(t.danger)
                            .font(text::small()),
                    );
                }
            });
        }
        for (label, a) in [("From", &d.from), ("To", &d.to)] {
            if let Some(a) = a {
                ui.horizontal(|ui| {
                    Self::label_row(ui, label);
                    ui.label(crate::summary::anchor_label(a));
                });
            }
        }
        ui.horizontal(|ui| {
            Self::label_row(ui, "Offset");
            let i1 = id.clone();
            self.num(
                ui,
                "dim-offset",
                Lead::Icon(Icon::DimVertical),
                Some(d.offset),
                "mm",
                (0.5, 0.05),
                HALF,
                "dimension.set_offset",
                &move |v| json!({ "ids": [i1], "offset": v }),
            );
        });
        ui.horizontal(|ui| {
            Self::label_row(ui, "Tolerance");
            if let Some(s) = w::text_field(
                ui,
                ("dim-tol", &id),
                d.tolerance.as_deref().unwrap_or(""),
                HALF,
                "e.g. ±0.1",
            ) {
                let v = if s.trim().is_empty() {
                    Value::Null
                } else {
                    json!(s.trim())
                };
                self.cmd(
                    "dimension.set_tolerance",
                    json!({ "ids": [id], "tolerance": v }),
                );
            }
        });
        let st = self.s.doc().styles.dimension;
        ui.label(
            egui::RichText::new("Style (all dimensions)")
                .font(text::small())
                .color(t.text_secondary),
        );
        ui.horizontal(|ui| {
            Self::label_row(ui, "Decimals");
            self.num(
                ui,
                "dim-dec",
                Lead::None,
                Some(st.decimals as f64),
                "",
                (1.0, 0.05),
                74.0,
                "dimension.set_style",
                &|v| json!({ "decimals": v.round().clamp(0.0, 6.0) as u64 }),
            );
        });
        ui.horizontal(|ui| {
            Self::label_row(ui, "Text height");
            self.num(
                ui,
                "dim-th",
                Lead::Letter("H"),
                Some(st.text_height),
                "mm",
                (0.1, 0.01),
                94.0,
                "dimension.set_style",
                &|v| json!({ "text_height": v.max(0.5) }),
            );
        });
    }

    fn notes_section(&mut self, ui: &mut egui::Ui, n: &kdraft_doc::NoteList) {
        if !self.section(ui, "notes", n.title.as_deref().unwrap_or("Notes")) {
            return;
        }
        let id = n.id.0.clone();
        let mut remove = None;
        for (k, note) in n.notes.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("{}.", k + 1)).font(text::small()));
                if let Some(t) = w::text_field(
                    ui,
                    ("note", &id, &note.id),
                    &note.text,
                    FULL - 50.0,
                    "Note text",
                ) {
                    self.cmd(
                        "notes.edit",
                        json!({ "id": id, "note": note.id, "text": t }),
                    );
                }
                if w::icon_button(ui, Icon::Close, "Remove note", None, true, false).clicked() {
                    remove = Some(note.id.clone());
                }
            });
        }
        if let Some(nid) = remove {
            self.cmd("notes.remove", json!({ "id": id, "note": nid }));
        }
        if w::button(ui, Some(Icon::Plus), "Add note", false, true).clicked() {
            self.cmd("notes.add", json!({ "id": id, "text": "NOTE" }));
        }
    }

    fn graphic_section(&mut self, ui: &mut egui::Ui, id: &str, shape: &GraphicShape) {
        if !self.section(ui, "graphic", "Geometry") {
            return;
        }
        let id = id.to_string();
        match shape.clone() {
            GraphicShape::Line { end, .. } | GraphicShape::Rect { end, .. } => {
                if let GraphicShape::Rect { start, radius, .. } = shape {
                    let (w0, h0) = ((end.x - start.x).abs(), (end.y - start.y).abs());
                    let r0 = *radius;
                    ui.horizontal(|ui| {
                        Self::label_row(ui, "Size");
                        let (a, b) = (id.clone(), id.clone());
                        self.num(
                            ui,
                            "rect-w",
                            Lead::Letter("W"),
                            Some(w0),
                            "mm",
                            (0.5, 0.1),
                            82.0,
                            "item.set_size",
                            &move |v| json!({ "ids": [a], "width": v.max(0.01) }),
                        );
                        self.num(
                            ui,
                            "rect-h",
                            Lead::Letter("H"),
                            Some(h0),
                            "mm",
                            (0.5, 0.1),
                            82.0,
                            "item.set_size",
                            &move |v| json!({ "ids": [b], "height": v.max(0.01) }),
                        );
                    });
                    ui.horizontal(|ui| {
                        Self::label_row(ui, "Corners");
                        let a = id.clone();
                        self.num(
                            ui,
                            "rect-r",
                            Lead::Letter("R"),
                            Some(r0),
                            "mm",
                            (0.5, 0.05),
                            HALF,
                            "item.set_geometry",
                            &move |v| json!({ "id": a, "radius": v.max(0.0) }),
                        );
                    });
                }
                ui.horizontal(|ui| {
                    Self::label_row(ui, "End");
                    let (a, b) = (id.clone(), id.clone());
                    self.num(
                        ui,
                        "end-x",
                        Lead::Letter("X"),
                        Some(end.x),
                        "mm",
                        (0.5, 0.1),
                        82.0,
                        "item.set_geometry",
                        &move |v| json!({ "id": a, "end": [v, end.y] }),
                    );
                    self.num(
                        ui,
                        "end-y",
                        Lead::Letter("Y"),
                        Some(end.y),
                        "mm",
                        (0.5, 0.1),
                        82.0,
                        "item.set_geometry",
                        &move |v| json!({ "id": b, "end": [end.x, v] }),
                    );
                });
            }
            GraphicShape::Polyline { points } | GraphicShape::Polygon { points } => {
                for (i, q) in points.iter().enumerate() {
                    ui.horizontal(|ui| {
                        Self::label_row(ui, &format!("Point {}", i + 1));
                        for (axis, lead) in [(0usize, "X"), (1, "Y")] {
                            let pts = points.clone();
                            let a = id.clone();
                            self.num(
                                ui,
                                &format!("vtx-{i}-{axis}"),
                                Lead::Letter(lead),
                                Some(if axis == 0 { q.x } else { q.y }),
                                "mm",
                                (0.5, 0.1),
                                82.0,
                                "graphic.set_points",
                                &move |v| {
                                    let w: Vec<Value> = pts
                                        .iter()
                                        .enumerate()
                                        .map(|(k, r)| {
                                            if k == i && axis == 0 {
                                                json!([v, r.y])
                                            } else if k == i {
                                                json!([r.x, v])
                                            } else {
                                                json!([r.x, r.y])
                                            }
                                        })
                                        .collect();
                                    json!({ "id": a, "points": w })
                                },
                            );
                        }
                    });
                }
                ui.label(
                    egui::RichText::new(
                        "Drag vertices on the canvas; double-click a segment to add one; click a vertex and press Delete to remove it.",
                    )
                    .font(text::small())
                    .color(self.tokens().text_secondary),
                );
            }
            GraphicShape::Circle { radius, .. } => {
                ui.horizontal(|ui| {
                    Self::label_row(ui, "Radius");
                    self.num(
                        ui,
                        "radius",
                        Lead::Letter("R"),
                        Some(radius),
                        "mm",
                        (0.5, 0.05),
                        HALF,
                        "item.set_geometry",
                        &move |v| json!({ "id": id, "radius": v.max(0.01) }),
                    );
                });
            }
            GraphicShape::Text {
                text: s,
                rotation,
                wrap,
                ..
            } => {
                ui.horizontal(|ui| {
                    Self::label_row(ui, "Text");
                    if w::button(ui, None, "Edit on canvas", false, true).clicked() {
                        self.begin_text_edit(&kdraft_doc::Id(id.clone()), false);
                    }
                });
                if let Some(t) = w::text_area(ui, ("gtext", &id), &s, FULL, "Text") {
                    self.cmd("text.set", json!({ "id": id, "text": t }));
                }
                ui.horizontal(|ui| {
                    Self::label_row(ui, "Wrap");
                    let a = id.clone();
                    self.num(
                        ui,
                        "twrap",
                        Lead::Letter("W"),
                        Some(wrap.unwrap_or(0.0)),
                        "mm",
                        (1.0, 0.1),
                        HALF,
                        "text.set",
                        &move |v| json!({ "id": a, "wrap": v.max(0.0) }),
                    );
                });
                ui.horizontal(|ui| {
                    Self::label_row(ui, "Rotation");
                    let a = id.clone();
                    self.num(
                        ui,
                        "trot",
                        Lead::Letter("R"),
                        Some(rotation),
                        "°",
                        (15.0, 0.5),
                        HALF,
                        "text.set",
                        &move |v| json!({ "id": a, "rotation": v }),
                    );
                });
            }
            _ => {
                ui.label("No editable geometry for this shape yet.");
            }
        }
    }
}
