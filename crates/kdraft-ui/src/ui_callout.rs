//! Callouts in the editor: the inspector's Callout section (content kind,
//! label with minus/plus, note picker, BOM item, designator), the
//! "Renumber callouts" window, in-place editing of a balloon's label and
//! the plus/minus keys.

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
use kdraft_board::geom::Point;
use kdraft_doc::{Callout, CalloutKind, Id, Item};
use serde_json::json;

use crate::app::App;
use crate::theme::text;
use crate::widgets::{self as w, Seg};

/// Inspector width of a full row.
const FULL: f32 = 232.0;

/// State of the "Renumber callouts" window.
#[derive(Debug, Clone, PartialEq)]
pub struct RenumberDialog {
    /// 0 rows, 1 columns, 2 selection order.
    pub order: usize,
    pub start: String,
    pub step: String,
    pub prefix: String,
}

impl Default for RenumberDialog {
    fn default() -> Self {
        RenumberDialog {
            order: 0,
            start: "1".into(),
            step: "1".into(),
            prefix: String::new(),
        }
    }
}

/// An open in-place editor of a balloon's label.
#[derive(Debug, Clone, PartialEq)]
pub struct LabelEdit {
    pub id: Id,
    pub buf: String,
    pub original: String,
    pub frames: u32,
}

const KIND_LABELS: [&str; 4] = ["Number", "Note", "BOM item", "Designator"];

fn kind_index(k: CalloutKind) -> usize {
    match k {
        CalloutKind::Label => 0,
        CalloutKind::Note => 1,
        CalloutKind::Bom => 2,
        CalloutKind::Refdes => 3,
    }
}

impl App {
    /// The text callout `c` shows now.
    pub fn callout_shown(&self, c: &Callout) -> String {
        kdraft_engine::commands_callout::shown_label(&self.s, c).unwrap_or_else(|| "?".into())
    }

    /// Selected callouts.
    pub fn selected_callouts(&self) -> Vec<Callout> {
        self.selected_items()
            .into_iter()
            .filter_map(|i| match i {
                Item::Callout(c) => Some(c.clone()),
                _ => None,
            })
            .collect()
    }

    /// Inspector section for one or more callouts.
    pub(crate) fn callout_section(&mut self, ui: &mut egui::Ui, cs: &[Callout]) {
        let Some(first) = cs.first() else { return };
        let title = if cs.len() == 1 {
            "Callout".to_string()
        } else {
            format!("{} callouts", cs.len())
        };
        if !self.section(ui, "callout", &title) {
            return;
        }
        let t = self.tokens();
        let ids: Vec<String> = cs.iter().map(|c| c.id.0.clone()).collect();
        let kind = first.kind();
        let same_kind = cs.iter().all(|c| c.kind() == kind);
        let segs: Vec<Seg<'_>> = KIND_LABELS.iter().map(|l| Seg::text(l)).collect();
        let on_part = cs.iter().all(|c| c.anchor_reference().is_some());
        let mut hint: Option<String> = None;
        {
            let width = ui.available_width().min(FULL + 20.0);
            if let Some(k) = w::segmented(
                ui,
                "callout-kind",
                &segs,
                same_kind.then_some(kind_index(kind)),
                width,
            ) {
                let kind = CalloutKind::ALL[k];
                if matches!(kind, CalloutKind::Bom | CalloutKind::Refdes) && !on_part {
                    hint = Some("Anchor the callout on a pad or part origin first".into());
                } else {
                    self.cmd(
                        "callout.set_kind",
                        json!({ "ids": ids, "kind": kind.as_str() }),
                    );
                }
            }
        }
        match (same_kind, kind) {
            (false, _) | (true, CalloutKind::Label) => {
                let labels: Vec<String> = cs
                    .iter()
                    .map(|c| match c.kind() {
                        CalloutKind::Label => c.label.clone().unwrap_or_default(),
                        _ => self.callout_shown(c),
                    })
                    .collect();
                let same = labels.iter().all(|l| *l == labels[0]);
                let value = if same {
                    labels[0].clone()
                } else {
                    String::new()
                };
                w::row(ui, "Label", |ui| {
                    let wid = FULL - w::LABEL_W - 2.0 * 26.0 - 8.0;
                    if let Some(v) = w::text_field_named(
                        ui,
                        ("callout-label", &ids),
                        &value,
                        wid,
                        if same { "1" } else { "mixed" },
                        Some("Callout label"),
                    ) {
                        if !v.trim().is_empty() {
                            self.cmd("callout.set_label", json!({ "ids": ids, "label": v }));
                        }
                    }
                    if ui
                        .add_sized([24.0, 22.0], egui::Button::new("−"))
                        .on_hover_text("Number - 1 (minus key)")
                        .clicked()
                    {
                        self.cmd("callout.bump", json!({ "ids": ids, "delta": -1 }));
                    }
                    if ui
                        .add_sized([24.0, 22.0], egui::Button::new("+"))
                        .on_hover_text("Number + 1 (plus key)")
                        .clicked()
                    {
                        self.cmd("callout.bump", json!({ "ids": ids, "delta": 1 }));
                    }
                });
            }
            (true, CalloutKind::Note) => {
                let page = self.s.current_page().cloned();
                let notes: Vec<(String, String)> = page
                    .iter()
                    .flat_map(|p| p.items.iter())
                    .filter_map(|i| match i {
                        Item::NoteList(n) => Some(n),
                        _ => None,
                    })
                    .flat_map(|n| {
                        n.notes.iter().enumerate().map(|(k, x)| {
                            let words: String = x
                                .text
                                .split_whitespace()
                                .take(5)
                                .collect::<Vec<_>>()
                                .join(" ");
                            (x.id.clone(), format!("{}  {}", k + 1, words))
                        })
                    })
                    .collect();
                let cur = first
                    .target
                    .as_deref()
                    .and_then(|t| t.strip_prefix("note:"))
                    .and_then(|id| notes.iter().find(|n| n.0 == id))
                    .map_or_else(|| "?".to_string(), |n| n.1.clone());
                let mut opts: Vec<String> = notes.iter().map(|n| n.1.clone()).collect();
                opts.push("New note…".into());
                let refs: Vec<&str> = opts.iter().map(String::as_str).collect();
                w::row(ui, "Note", |ui| {
                    if let Some(k) =
                        w::dropdown(ui, "callout-note", &cur, &refs, FULL - w::LABEL_W - 8.0)
                    {
                        let note = notes.get(k).map_or("new".to_string(), |n| n.0.clone());
                        self.cmd(
                            "callout.set_kind",
                            json!({ "ids": ids, "kind": "note", "note": note }),
                        );
                    }
                });
            }
            (true, CalloutKind::Bom) => {
                let bom = self.s.effective_bom();
                let line = first
                    .target
                    .as_deref()
                    .and_then(|t| t.strip_prefix("bom:"))
                    .and_then(|k| bom.lines.iter().find(|l| l.key() == k));
                let text = match line {
                    Some(l) => format!("Item {}  ·  {}", self.callout_shown(first), l.value),
                    None => "no BOM line".to_string(),
                };
                w::row(ui, "BOM item", |ui| {
                    ui.label(egui::RichText::new(text).font(text::body()));
                });
            }
            (true, CalloutKind::Refdes) => {
                let r = first.anchor_reference().unwrap_or("?").to_string();
                w::row(ui, "Designator", |ui| {
                    ui.label(egui::RichText::new(r).font(text::body()));
                });
            }
        }
        if cs.len() == 1 {
            ui.horizontal(|ui| {
                Self::label_row(ui, "Anchor");
                ui.label(
                    first
                        .anchor
                        .as_ref()
                        .map_or("none".to_string(), crate::summary::anchor_label),
                );
            });
        }
        if let Some(h) = hint {
            self.info(h);
        }
        ui.horizontal(|ui| {
            if w::button(ui, None, "Renumber…", false, true).clicked() {
                self.renumber = Some(RenumberDialog::default());
            }
        });
        ui.label(
            egui::RichText::new(
                "Double-click or Enter edits the label on the canvas; + and - change the number.",
            )
            .font(text::small())
            .color(t.text_secondary),
        );
    }

    /// Document options for callout numbering (page inspector).
    pub(crate) fn callout_numbering_rows(&mut self, ui: &mut egui::Ui) {
        let st = self.s.doc().styles.clone();
        ui.horizontal(|ui| {
            Self::label_row(ui, "Callouts");
            let segs = [Seg::text("Per page"), Seg::text("Document")];
            if let Some(k) = w::segmented(
                ui,
                "callout-scope",
                &segs,
                Some(usize::from(st.callouts_per_document)),
                150.0,
            ) {
                let scope = if k == 1 { "document" } else { "page" };
                self.cmd("document.set_callout_numbering", json!({ "scope": scope }));
            }
        });
        let mut strict = st.strict_callout_numbering;
        ui.horizontal(|ui| {
            ui.add_space(w::LABEL_W);
            if ui
                .checkbox(&mut strict, "Strict callout numbering")
                .on_hover_text("Report gaps in callout numbers as issues (info)")
                .changed()
            {
                self.cmd(
                    "document.set_callout_numbering",
                    json!({ "strict": strict }),
                );
            }
        });
    }

    /// The "Renumber callouts" window.
    pub(crate) fn renumber_window(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.renumber.clone() else {
            return;
        };
        let sel = self.selected_callouts().len();
        let mut open = true;
        let mut run = false;
        egui::Window::new("Renumber callouts")
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(300.0)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new(if sel > 0 {
                        format!("The {sel} selected numbered callout(s).")
                    } else {
                        "All numbered callouts of this page.".to_string()
                    })
                    .font(text::small()),
                );
                ui.label("Order");
                ui.radio_value(&mut d.order, 0, "Rows: top to bottom, then left to right");
                ui.radio_value(
                    &mut d.order,
                    1,
                    "Columns: left to right, then top to bottom",
                );
                ui.add_enabled_ui(sel > 0, |ui| {
                    ui.radio_value(&mut d.order, 2, "Selection order");
                });
                w::row(ui, "Start", |ui| {
                    ui.add(egui::TextEdit::singleline(&mut d.start).desired_width(60.0));
                });
                w::row(ui, "Step", |ui| {
                    ui.add(egui::TextEdit::singleline(&mut d.step).desired_width(60.0));
                });
                w::row(ui, "Prefix", |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut d.prefix)
                            .hint_text("none")
                            .desired_width(60.0),
                    );
                });
                let ok = d.start.trim().parse::<i64>().is_ok_and(|v| v >= 0)
                    && d.step.trim().parse::<i64>().is_ok_and(|v| v >= 1);
                if !ok {
                    ui.colored_label(self.tokens().danger, "Start 0 or more, step 1 or more");
                }
                if w::button(ui, None, "Renumber", true, ok).clicked() {
                    run = true;
                }
            });
        if run {
            let ids: Vec<String> = self
                .selected_callouts()
                .iter()
                .map(|c| c.id.0.clone())
                .collect();
            let order = ["rows", "columns", "selection"][d.order.min(2)];
            self.cmd(
                "callout.renumber",
                json!({
                    "ids": ids,
                    "order": order,
                    "start": d.start.trim().parse::<i64>().unwrap_or(1),
                    "step": d.step.trim().parse::<i64>().unwrap_or(1),
                    "prefix": d.prefix.trim(),
                }),
            );
            self.renumber = None;
        } else if open {
            self.renumber = Some(d);
        } else {
            self.renumber = None;
        }
    }

    /// Opens the in-place label editor on callout `id`; false for other
    /// items.
    pub fn begin_label_edit(&mut self, id: &Id) -> bool {
        let Some((_, Item::Callout(c))) = self.s.find_item(id).map(|(p, i)| (p, i.clone())) else {
            return false;
        };
        if self.s.find_item(id).is_some_and(|(_, it)| it.locked()) {
            return false;
        }
        let shown = self.callout_shown(&c);
        self.label_edit = Some(LabelEdit {
            id: id.clone(),
            buf: shown.clone(),
            original: shown,
            frames: 0,
        });
        true
    }

    /// Commits the label editor (no-op when unchanged or empty).
    pub fn commit_label_edit(&mut self) {
        let Some(e) = self.label_edit.take() else {
            return;
        };
        let v = e.buf.trim();
        if !v.is_empty() && v != e.original {
            self.cmd("callout.set_label", json!({ "ids": [e.id.0], "label": v }));
        }
    }

    /// Draws the label editor over the balloon (`sp`: sheet to screen).
    pub(crate) fn label_editor(&mut self, ui: &mut egui::Ui, sp: &dyn Fn(Point) -> Pos2) {
        let Some(mut e) = self.label_edit.clone() else {
            return;
        };
        let Some((_, Item::Callout(c))) = self.s.find_item(&e.id).map(|(p, i)| (p, i.clone()))
        else {
            self.label_edit = None;
            return;
        };
        let eff = Item::Callout(c.clone()).effective_style(&self.s.doc().styles);
        let th = eff.text_height.unwrap_or(self.s.doc().styles.text_height);
        let zoom = self.camera.zoom;
        let em = ((self.s.fonts().regular.em_for_cap(th) * zoom) as f32).clamp(10.0, 160.0);
        let tw = self.s.fonts().width(&e.buf, th, false) * zoom;
        let w_px = ((tw as f32) + em * 1.5).clamp(48.0, 400.0);
        let h_px = em * 1.4 + 6.0;
        let rect = Rect::from_center_size(sp(c.at), Vec2::new(w_px, h_px));
        // The Enter that opened the editor is still in this frame's input.
        let commit = e.frames > 0
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        let cancel = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if cancel {
            self.label_edit = None;
            return;
        }
        let t = self.tokens();
        let painter = ui.painter();
        painter.rect_filled(rect.expand(2.0), 3.0, Color32::WHITE);
        painter.rect_stroke(
            rect.expand(2.0),
            3.0,
            Stroke::new(1.5, t.accent),
            egui::StrokeKind::Outside,
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        let resp = child.add(
            egui::TextEdit::singleline(&mut e.buf)
                .id(egui::Id::unique(("label-edit", &e.id.0)))
                .font(egui::FontId::proportional(em))
                .text_color(Color32::BLACK)
                .frame(egui::Frame::NONE)
                .align(egui::Align2::CENTER_CENTER)
                .desired_width(w_px - 6.0)
                .background_color(Color32::TRANSPARENT),
        );
        resp.widget_info(|| {
            let mut i = egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Balloon label");
            i.current_text_value = Some(e.buf.clone());
            i
        });
        if e.frames == 0 {
            resp.request_focus();
            // Select everything so typing replaces the label.
            if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
                let all = egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(e.buf.chars().count()),
                );
                st.cursor.set_char_range(Some(all));
                st.store(ui.ctx(), resp.id);
            }
        }
        e.frames += 1;
        let lost = e.frames > 2 && !resp.has_focus();
        self.label_edit = Some(e);
        if commit || lost {
            self.commit_label_edit();
        }
    }

    /// Plus and minus keys with callouts selected; true when used.
    pub(crate) fn callout_keys(&mut self, presses: &[(egui::Key, egui::Modifiers)]) -> bool {
        let ids: Vec<String> = self
            .selected_callouts()
            .iter()
            .map(|c| c.id.0.clone())
            .collect();
        if ids.is_empty() {
            return false;
        }
        let mut delta = 0;
        for (k, m) in presses {
            if m.command || m.alt {
                continue;
            }
            match k {
                egui::Key::Plus | egui::Key::Equals => delta += 1,
                egui::Key::Minus => delta -= 1,
                _ => {}
            }
        }
        if delta == 0 {
            return false;
        }
        self.cmd("callout.bump", json!({ "ids": ids, "delta": delta }));
        true
    }
}
