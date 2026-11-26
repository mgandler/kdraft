//! In-place editing of text graphics on the canvas: double-click a text
//! (or press Enter with one selected) to edit it where it is. Enter
//! starts a new line, Ctrl+Enter or a click outside commits (one
//! `text.set`), Escape cancels. Rotated texts are edited unrotated at
//! their anchor.

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
use kdraft_board::geom::Point;
use kdraft_doc::{GraphicShape, Id, Item};
use serde_json::json;

use crate::app::App;

/// An open in-place text editor.
#[derive(Debug, Clone, PartialEq)]
pub struct TextEdit {
    pub id: Id,
    pub buf: String,
    pub original: String,
    /// The text was just placed: committing it empty removes it.
    pub fresh: bool,
    /// Frames since opening (focus is requested on the first).
    pub frames: u32,
}

impl App {
    /// Opens the editor on text graphic `id`; false for other items.
    pub fn begin_text_edit(&mut self, id: &Id, fresh: bool) -> bool {
        let Some(Item::Graphic(g)) = self.s.find_item(id).map(|(_, it)| it.clone()) else {
            return false;
        };
        let GraphicShape::Text { text, .. } = &g.shape else {
            return false;
        };
        if self.s.find_item(id).is_some_and(|(_, it)| it.locked()) {
            return false;
        }
        self.text_edit = Some(TextEdit {
            id: id.clone(),
            buf: if fresh { String::new() } else { text.clone() },
            original: text.clone(),
            fresh,
            frames: 0,
        });
        true
    }

    /// Commits the open editor (no-op when unchanged).
    pub fn commit_text_edit(&mut self) {
        let Some(e) = self.text_edit.take() else {
            return;
        };
        if e.fresh && e.buf.trim().is_empty() {
            self.cmd("item.remove", json!({ "ids": [e.id.0] }));
            return;
        }
        if e.buf != e.original {
            self.cmd("text.set", json!({ "id": e.id.0, "text": e.buf }));
        }
    }

    pub fn cancel_text_edit(&mut self) {
        if let Some(e) = self.text_edit.take() {
            if e.fresh && e.original.trim().is_empty() {
                self.cmd("item.remove", json!({ "ids": [e.id.0] }));
            }
        }
    }

    /// Draws the editor over the canvas (`sp`: sheet to screen).
    pub(crate) fn text_editor(&mut self, ui: &mut egui::Ui, sp: &dyn Fn(Point) -> Pos2) {
        let Some(mut e) = self.text_edit.clone() else {
            return;
        };
        let Some((_, Item::Graphic(g))) = self.s.find_item(&e.id).map(|(p, i)| (p, i.clone()))
        else {
            self.text_edit = None;
            return;
        };
        let ctx = self
            .s
            .bounds_ctx(kdraft_engine::bounds::BoundsMode::Content);
        if kdraft_engine::bounds::text_layout(&g, &ctx).is_none() {
            self.text_edit = None;
            return;
        }
        let eff = Item::Graphic(g.clone()).effective_style(&self.s.doc().styles);
        let h = eff.text_height.unwrap_or(2.5);
        let zoom = self.camera.zoom;
        let em = (self.s.fonts().regular.em_for_cap(h) * zoom) as f32;
        let em = em.clamp(8.0, 200.0);
        // Unrotated top-left of the block (rotation is shown after commit).
        let GraphicShape::Text { at, wrap, .. } = &g.shape else {
            return;
        };
        // The block as typed so far: its width, placed by the alignment
        // as the layout will place it (unrotated).
        let bold = eff.bold.unwrap_or(false);
        let typed_w = e
            .buf
            .split('\n')
            .map(|l| self.s.fonts().width(l, h, bold))
            .fold(0.0_f64, f64::max);
        let width_mm = wrap.unwrap_or(typed_w).max(10.0 / zoom.max(1e-6));
        let factor = match eff.align {
            Some(kdraft_doc::style::TextAlign::Center) => 0.5,
            Some(kdraft_doc::style::TextAlign::Right) => 1.0,
            _ => 0.0,
        };
        let tl = Point::new(at.x - factor * width_mm, at.y - h);
        let w_px = ((width_mm * zoom) as f32 + em).clamp(80.0, 900.0);
        let origin = sp(tl) - Vec2::new(4.0, 3.0);
        let lines = e.buf.split('\n').count().max(1) as f32;
        let h_px = lines * em * 1.25 + 10.0;
        let rect = Rect::from_min_size(origin, Vec2::new(w_px, h_px));
        let t = self.tokens();
        let commit = ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter));
        let cancel = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if cancel {
            self.cancel_text_edit();
            return;
        }
        let painter = ui.painter();
        painter.rect_filled(rect.expand(2.0), 3.0, Color32::WHITE);
        painter.rect_stroke(
            rect.expand(2.0),
            3.0,
            Stroke::new(1.5, t.accent),
            egui::StrokeKind::Outside,
        );
        let color = eff
            .text_color
            .map_or(Color32::BLACK, |c| Color32::from_rgb(c.r, c.g, c.b));
        let family = if eff.bold.unwrap_or(false) {
            crate::theme::bold()
        } else {
            egui::FontFamily::Proportional
        };
        let align = match eff.align {
            Some(kdraft_doc::style::TextAlign::Center) => egui::Align::Center,
            Some(kdraft_doc::style::TextAlign::Right) => egui::Align::Max,
            _ => egui::Align::Min,
        };
        let edit_id = egui::Id::unique(("text-edit", &e.id.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        let resp = child.add(
            egui::TextEdit::multiline(&mut e.buf)
                .id(edit_id)
                .font(egui::FontId::new(em, family))
                .text_color(color)
                .frame(egui::Frame::NONE)
                .desired_width(w_px - 8.0)
                .desired_rows(1)
                .align(egui::Align2([align, egui::Align::Min]))
                .background_color(Color32::TRANSPARENT),
        );
        if e.frames == 0 {
            resp.request_focus();
        }
        e.frames += 1;
        let lost = e.frames > 2 && !resp.has_focus();
        self.text_edit = Some(e);
        if commit || lost {
            self.commit_text_edit();
        }
    }

    /// The single selected text graphic, if any.
    pub(crate) fn selected_text(&self) -> Option<Id> {
        let sel = self.selected_items();
        match sel.as_slice() {
            [Item::Graphic(g)] if matches!(g.shape, GraphicShape::Text { .. }) => {
                Some(g.id.clone())
            }
            _ => None,
        }
    }
}
