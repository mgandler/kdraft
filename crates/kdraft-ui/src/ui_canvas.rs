//! The canvas: pasteboard, paper, sheet and item textures, rulers, grid,
//! item labels, selection and hover feedback, snap guides, Alt-hover
//! measurements, the floating tool bar, tool previews with anchor
//! snapping, and pointer input.

use eframe::egui::{self, Align2, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, Vec2};
use kdraft_board::geom::{BBox, Point};
use kdraft_doc::{Id, Item};
use kdraft_engine::snap::GuideKind;
use serde_json::json;

use crate::app::{App, DRAG_THRESHOLD_PX, SNAP_TOLERANCE_PX};
use crate::drag::{Action, Press};
use crate::hit;
use crate::icons::Icon;
use crate::jobs::{self, RasterWhat};
use crate::movectx::MoveCtx;
use crate::theme::{text, Tokens};
use crate::tools::CalloutTool;
use crate::tools::{LinearKind, Out, TableTool, ToolKind, ViewTool};
use crate::widgets as w;

/// Ruler thickness, points.
pub const RULER: f32 = 20.0;
/// Pointer distance for anchor snapping, points.
pub const ANCHOR_SNAP_PX: f64 = 10.0;

fn pos(p: Point, origin: Pos2) -> Pos2 {
    Pos2::new(origin.x + p.x as f32, origin.y + p.y as f32)
}

fn pt(p: Pos2, origin: Pos2) -> Point {
    Point::new(f64::from(p.x - origin.x), f64::from(p.y - origin.y))
}

/// Tool-bar group of a tool (for the dropdown carets).
pub fn tool_group(k: ToolKind) -> Option<&'static str> {
    match k {
        ToolKind::View(_) => Some("view"),
        ToolKind::Table(_) => Some("table"),
        ToolKind::DimLinear(_) | ToolKind::DimRadius { .. } | ToolKind::DimOrdinate => Some("dim"),
        ToolKind::Text | ToolKind::Notes | ToolKind::Callout(_) => Some("note"),
        ToolKind::Line | ToolKind::Rect | ToolKind::Polyline | ToolKind::Circle => Some("shape"),
        _ => None,
    }
}

/// Tool groups of the floating bar: id, default tool, members.
const GROUPS: &[(&str, ToolKind, &[ToolKind])] = &[
    (
        "view",
        ToolKind::View(ViewTool::Fabrication),
        &[
            ToolKind::View(ViewTool::Fabrication),
            ToolKind::View(ViewTool::Assembly),
            ToolKind::View(ViewTool::Drill),
        ],
    ),
    (
        "table",
        ToolKind::Table(TableTool::Drill),
        &[
            ToolKind::Table(TableTool::Drill),
            ToolKind::Table(TableTool::Stackup),
            ToolKind::Table(TableTool::Bom),
        ],
    ),
    (
        "dim",
        ToolKind::DimLinear(LinearKind::Horizontal),
        &[
            ToolKind::DimLinear(LinearKind::Horizontal),
            ToolKind::DimLinear(LinearKind::Vertical),
            ToolKind::DimLinear(LinearKind::Aligned),
            ToolKind::DimRadius { diameter: false },
            ToolKind::DimRadius { diameter: true },
            ToolKind::DimOrdinate,
        ],
    ),
    (
        "note",
        ToolKind::Callout(CalloutTool::Number),
        &[
            ToolKind::Callout(CalloutTool::Number),
            ToolKind::Callout(CalloutTool::Bom),
            ToolKind::Callout(CalloutTool::Note),
            ToolKind::Notes,
            ToolKind::Text,
        ],
    ),
    (
        "shape",
        ToolKind::Line,
        &[
            ToolKind::Line,
            ToolKind::Polyline,
            ToolKind::Rect,
            ToolKind::Circle,
        ],
    ),
];

fn tool_meta(k: ToolKind) -> (&'static str, Icon, &'static str) {
    crate::actions::TOOLS
        .iter()
        .find(|x| x.0 == k)
        .map(|x| (x.2, x.3, x.4))
        .unwrap_or(("Tool", Icon::Select, ""))
}

impl App {
    pub(crate) fn canvas(&mut self, ui: &mut egui::Ui) {
        let t = self.tokens();
        let full = ui.max_rect();
        let rulers = self.settings.show_rulers;
        let rect = if rulers {
            Rect::from_min_max(full.min + Vec2::splat(RULER), full.max)
        } else {
            full
        };
        let resp = ui.interact(rect, egui::Id::unique("canvas"), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let origin = rect.min;
        self.canvas_origin = Point::new(f64::from(origin.x), f64::from(origin.y));
        self.viewport = (f64::from(rect.width()), f64::from(rect.height()));
        painter.rect_filled(rect, 0.0, t.canvas);
        let Some(pi) = self.s.current_page_index() else {
            return;
        };
        let page_id = self.s.doc().pages[pi].id.clone();
        let page_rect = self.page_rect().unwrap_or(BBox::from_point(Point::ORIGIN));
        if (self.fit_pending || self.camera_page.as_ref() != Some(&page_id)) && rect.width() > 10.0
        {
            // The page's own camera when it was shown before (and still
            // fits), else the whole page.
            let restored = if self.fit_pending {
                None
            } else {
                self.saved_camera(&page_id, &page_rect)
            };
            self.camera = restored.unwrap_or_else(|| {
                crate::camera::Camera::fit(&page_rect, self.viewport.0, self.viewport.1, 32.0)
            });
            self.camera_page = Some(page_id.clone());
            self.fit_pending = false;
            self.note_zoom_change();
        }
        if rect.width() > 10.0 && self.note_camera(&page_id, &page_rect) {
            self.save_settings_soon();
        }
        self.canvas_input(ui, &resp, origin);
        // Right click: pick what is under the pointer first.
        if resp.secondary_clicked() {
            if let Some(p) = self.cursor_mm {
                match self.pick_item(p, 3.0 / self.camera.zoom) {
                    Some(id) if !self.s.selection.contains(&id) => {
                        self.cmd("select.set", json!({ "ids": [id.0] }));
                    }
                    Some(_) => {}
                    None => {
                        self.cmd("select.clear", json!({}));
                    }
                }
            }
        }
        resp.context_menu(|ui| {
            if self.s.selection.items.is_empty() {
                self.canvas_context_menu(ui);
            } else {
                self.item_context_menu(ui);
            }
        });

        let cam = self.camera;
        let sp = move |p: Point| pos(cam.to_screen(p), origin);
        let to_rect = move |b: &BBox| Rect::from_min_max(sp(b.min), sp(b.max));
        // Paper with a soft shadow.
        let pr = to_rect(&page_rect);
        for (k, a) in [(6.0, 0.25), (3.0, 0.5), (1.0, 1.0)] {
            painter.rect_filled(
                pr.translate(Vec2::new(0.0, k * 0.5)).expand(k),
                CornerRadius::same(2),
                t.paper_shadow.gamma_multiply(a * 0.35),
            );
        }
        painter.rect_filled(pr, 0.0, t.paper);
        let bucket = self.bucket();
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        if self.inputs.is_some() {
            let slot = format!("sheet:{page_id}");
            let key = jobs::sheet_key(self.s.doc(), pi, &self.inputs_gen);
            self.want_texture(
                &slot,
                key,
                RasterWhat::Sheet {
                    page: self.s.doc().pages[pi].id.clone(),
                },
                bucket,
            );
            if let Some(x) = self.textures.get(&slot) {
                painter.image(x.handle.id(), to_rect(&x.region), uv, Color32::WHITE);
            }
        }
        if self.settings.show_grid {
            self.paint_grid(&painter, &page_rect, &sp, &t);
        }
        // Items, bottom first: views, tables and note lists from textures,
        // annotations live.
        let items: Vec<Item> = self.s.doc().pages[pi].items.clone();
        for it in items.iter().filter(|i| !i.hidden()) {
            let Some(id) = it.id() else { continue };
            if crate::app::is_live(it) {
                let dl = self.live_items(it);
                crate::vector::paint(
                    &painter,
                    &dl,
                    &kdraft_board::geom::Affine::IDENTITY,
                    &sp,
                    cam.zoom,
                    self.s.fonts(),
                    None,
                );
                continue;
            }
            let slot = format!("item:{id}");
            if self.inputs.is_some() {
                let key = jobs::item_key(it, self.s.doc(), &self.inputs_gen);
                self.want_texture(
                    &slot,
                    key,
                    RasterWhat::Item {
                        item: Box::new(it.clone()),
                    },
                    bucket,
                );
            }
            match self.textures.get(&slot) {
                Some(x) => {
                    let d = match (x.at, it.position()) {
                        (Some(a), Some(b)) => b - a,
                        _ => Point::ORIGIN,
                    };
                    let r = BBox {
                        min: x.region.min + d,
                        max: x.region.max + d,
                    };
                    painter.image(x.handle.id(), to_rect(&r), uv, Color32::WHITE);
                }
                None => {
                    if let Some(b) = self.s.item_bounds(it) {
                        painter.rect_stroke(
                            to_rect(&b),
                            0.0,
                            Stroke::new(1.0, Color32::from_gray(200)),
                            egui::StrokeKind::Inside,
                        );
                    }
                }
            }
        }
        self.paint_overlays(&painter, pi, &sp, &to_rect, &t);
        self.paint_handles(&painter, &sp, &t);
        self.text_editor(ui, &sp);
        self.label_editor(ui, &sp);
        self.paint_tool_preview(&painter, &sp, &t);
        if rulers {
            self.paint_rulers(ui, full, rect, &t);
        }
        if let Some(b) = self.banner.clone() {
            let br = Rect::from_min_size(
                rect.min + Vec2::new(12.0, 10.0),
                Vec2::new((rect.width() - 24.0).min(620.0), 30.0),
            );
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br));
            if crate::ui::banner(&mut child, &t, &b) {
                self.banner = None;
            }
        }
        self.tool_bar(ui, rect, &t);
    }

    fn paint_grid(
        &self,
        painter: &egui::Painter,
        page: &BBox,
        sp: &dyn Fn(Point) -> Pos2,
        t: &Tokens,
    ) {
        let grid = self.s.doc().styles.snap_grid;
        let Some(step) = crate::rulers::grid_step(grid, self.camera.zoom, 8.0) else {
            return;
        };
        let st = Stroke::new(1.0, t.grid);
        let clip = painter.clip_rect();
        let lo = self.camera.to_sheet(pt(
            clip.min,
            Pos2::new(self.canvas_origin.x as f32, self.canvas_origin.y as f32),
        ));
        let hi = self.camera.to_sheet(pt(
            clip.max,
            Pos2::new(self.canvas_origin.x as f32, self.canvas_origin.y as f32),
        ));
        let (x0, x1) = (lo.x.max(page.min.x), hi.x.min(page.max.x));
        let (y0, y1) = (lo.y.max(page.min.y), hi.y.min(page.max.y));
        for x in crate::rulers::tick_values(x0, x1, step) {
            painter.line_segment([sp(Point::new(x, y0)), sp(Point::new(x, y1))], st);
        }
        for y in crate::rulers::tick_values(y0, y1, step) {
            painter.line_segment([sp(Point::new(x0, y)), sp(Point::new(x1, y))], st);
        }
    }

    /// Selection, hover, labels, warnings, guides, marquee, measurements.
    fn paint_overlays(
        &mut self,
        painter: &egui::Painter,
        pi: usize,
        sp: &dyn Fn(Point) -> Pos2,
        to_rect: &dyn Fn(&BBox) -> Rect,
        t: &Tokens,
    ) {
        let boxes = self.item_boxes();
        let page_items = self.s.doc().pages[pi].items.clone();
        let item_of = |id: &Id| page_items.iter().find(|i| i.id() == Some(id));
        // Items reaching outside the drawing area (the check's rule).
        let area = self.s.drawing_area(pi);
        let strip = {
            let size = self.s.doc().pages[pi].size.clone();
            let (sheet, _, _) = self.s.sheet_for_page(pi);
            kdraft_doc::notes_strip(&sheet, &size, 0.0)
        };
        if let Some(a) = area {
            for (_, b) in &boxes {
                if kdraft_engine::check::extends_outside(b, &a, strip.as_ref()) {
                    painter.rect_stroke(
                        to_rect(b),
                        0.0,
                        Stroke::new(1.5, t.danger),
                        egui::StrokeKind::Middle,
                    );
                }
            }
        }
        // Hover (canvas or item tree), not on selected items.
        let hover = self.tree_hover.clone().or_else(|| self.hover_item.clone());
        if let Some(h) = hover.filter(|h| !self.s.selection.contains(h)) {
            match item_of(&h) {
                Some(Item::Dimension(d)) => {
                    if let Some(g) = self.dim_geometry(d) {
                        for (a, b) in &g.lines {
                            painter.line_segment(
                                [sp(*a), sp(*b)],
                                Stroke::new(1.5, t.accent.gamma_multiply(0.7)),
                            );
                        }
                    }
                }
                Some(_) => {
                    if let Some((_, b)) = boxes.iter().find(|(i, _)| *i == h) {
                        painter.rect_stroke(
                            to_rect(b),
                            0.0,
                            Stroke::new(1.0, t.accent.gamma_multiply(0.7)),
                            egui::StrokeKind::Outside,
                        );
                    }
                }
                None => {}
            }
        }
        // Name labels above views and tables (when there is room).
        for (id, b) in &boxes {
            let Some(it) = item_of(id) else { continue };
            if !matches!(it, Item::View(_) | Item::Table(_)) {
                continue;
            }
            let r = to_rect(b);
            if r.width() < 70.0 {
                continue;
            }
            let sel = self.s.selection.contains(id);
            // Only for the selection and the hovered item: the paper stays
            // clean.
            let hovered =
                self.hover_item.as_ref() == Some(id) || self.tree_hover.as_ref() == Some(id);
            if !sel && !hovered {
                continue;
            }
            // Above the view's own dimensions too, so the label never sits
            // on a dimension line.
            let mut top = r.min.y;
            if let Item::View(v) = it {
                for d in page_items.iter().filter_map(|x| match x {
                    Item::Dimension(d) if d.view.as_ref() == Some(&v.id) => Some(d),
                    _ => None,
                }) {
                    if let Some(g) = self.dim_geometry(d) {
                        let gb = to_rect(&g.bbox);
                        // Only dimensions over the top edge push it up.
                        if gb.max.y <= r.min.y + 2.0 && gb.max.x >= r.min.x && gb.min.x <= r.max.x {
                            top = top.min(gb.min.y);
                        }
                    }
                }
            }
            let name = crate::tree::display_name(it);
            let s = w::elide_painter(painter, &name, text::small(), r.width());
            painter.text(
                Pos2::new(r.min.x, top - 4.0),
                Align2::LEFT_BOTTOM,
                s,
                text::small(),
                if sel { t.accent_text } else { t.ruler_text },
            );
        }
        // Selection.
        let n_sel = self.s.selection.items.len();
        let reference = (n_sel > 1).then(|| self.s.selection.items[0].clone());
        let single_view = self.single_selected_view();
        let mut sel_union: Option<BBox> = None;
        for (id, b) in &boxes {
            if !self.s.selection.contains(id) {
                continue;
            }
            sel_union = Some(sel_union.map_or(*b, |u| u.union(b)));
            let c = if reference.as_ref() == Some(id) {
                t.reference
            } else {
                t.accent
            };
            if let Some(Item::Dimension(d)) = item_of(id) {
                let Some(g) = self.dim_geometry(d) else {
                    continue;
                };
                for (a, bb) in &g.lines {
                    painter.line_segment([sp(*a), sp(*bb)], Stroke::new(2.0, c));
                }
                for tri in &g.arrows {
                    painter.add(egui::Shape::convex_polygon(
                        tri.iter().map(|p| sp(*p)).collect(),
                        c,
                        Stroke::NONE,
                    ));
                }
                painter.rect_stroke(
                    to_rect(&g.text_box),
                    0.0,
                    Stroke::new(1.0, c),
                    egui::StrokeKind::Outside,
                );
                let (a, bb) = g.dim_line;
                for h in [
                    Point::new(a.x + (bb.x - a.x) * 0.25, a.y + (bb.y - a.y) * 0.25),
                    Point::new(g.text_box.max.x, g.text_box.min.y),
                ] {
                    handle(painter, sp(h), c, t);
                }
                continue;
            }
            let r = to_rect(b);
            painter.rect_stroke(
                r,
                0.0,
                Stroke::new(
                    if reference.as_ref() == Some(id) {
                        2.0
                    } else {
                        1.0
                    },
                    c,
                ),
                egui::StrokeKind::Outside,
            );
            if n_sel == 1 {
                for (hk, p) in hit::handles(b) {
                    // Only the corner handles of a view do something.
                    if single_view.is_some() && hk.is_corner() {
                        handle(painter, sp(p), c, t);
                    }
                }
            }
        }
        // Value pill under the selection, below everything the items draw
        // (a view's scale label, a table's last row, a 3D view's caption).
        if let (Some(u), false) = (sel_union, self.drag.is_idle() && n_sel == 0) {
            let label = self.selection_pill(n_sel, &u);
            if !label.is_empty() {
                let r = to_rect(&u);
                let drawn = self
                    .s
                    .selection
                    .items
                    .iter()
                    .filter_map(|id| {
                        let x = self.textures.get(&format!("item:{id}"))?;
                        // The texture is where the item was when rendered.
                        let it = self.s.find_item(id)?.1;
                        let dy = match (x.at, it.position()) {
                            (Some(a), Some(b)) => b.y - a.y,
                            _ => 0.0,
                        };
                        Some(x.region.max.y + dy)
                    })
                    .fold(u.max.y, f64::max);
                let bottom = sp(Point::new(u.min.x, drawn)).y.max(r.max.y);
                w::pill(
                    painter,
                    Pos2::new(r.center().x, bottom + 14.0),
                    &label,
                    t.accent,
                    t.on_accent,
                );
            }
        }
        // Snap guides.
        for g in &self.guides {
            let c = match g.kind {
                GuideKind::Spacing => t.guide_spacing,
                GuideKind::Page | GuideKind::Area => t.guide_page,
                _ => t.guide_object,
            };
            painter.line_segment([sp(g.a), sp(g.b)], Stroke::new(1.0, c));
            for q in [g.a, g.b] {
                painter.circle_filled(sp(q), 2.0, c);
            }
        }
        if let Some(m) = &self.marquee {
            let r = to_rect(m);
            painter.rect_filled(r, 0.0, t.accent.gamma_multiply(0.12));
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
        }
        // Alt-hover measurements.
        if self.measure_on && self.drag.is_idle() && self.tool.kind.is_select() {
            if let Some(s) = sel_union {
                let other = self
                    .hover_item
                    .as_ref()
                    .filter(|h| !self.s.selection.contains(h))
                    .and_then(|h| boxes.iter().find(|(i, _)| i == h))
                    .map(|(_, b)| *b);
                let gaps = match (other, area) {
                    (Some(o), _) => crate::measure::between(&s, &o),
                    (None, Some(a)) => crate::measure::to_area(&s, &a),
                    _ => Vec::new(),
                };
                for g in gaps {
                    let (a, b) = (sp(g.a), sp(g.b));
                    painter.line_segment([a, b], Stroke::new(1.0, t.measure));
                    for e in [a, b] {
                        let d = if (a.x - b.x).abs() > (a.y - b.y).abs() {
                            Vec2::new(0.0, 4.0)
                        } else {
                            Vec2::new(4.0, 0.0)
                        };
                        painter.line_segment([e - d, e + d], Stroke::new(1.0, t.measure));
                    }
                    w::pill(
                        painter,
                        a + (b - a) * 0.5,
                        &crate::widgets::fmt_num(g.value, 2),
                        t.measure,
                        Color32::WHITE,
                    );
                }
                if let Some(o) = other {
                    painter.rect_stroke(
                        to_rect(&o),
                        0.0,
                        Stroke::new(1.0, t.measure),
                        egui::StrokeKind::Outside,
                    );
                }
            }
        }
    }

    /// "2:1 · X 163.5 Y 113.5" for one view, "W x H" otherwise.
    fn selection_pill(&self, n: usize, u: &BBox) -> String {
        let base = self.selection_pill_base(n, u);
        let linked = self.selected_items().iter().any(|i| {
            i.link().is_some()
                || i.id()
                    .is_some_and(|id| !kdraft_doc::link::followers(self.s.doc(), id).is_empty())
        });
        if linked {
            format!("linked · {base}")
        } else {
            base
        }
    }

    fn selection_pill_base(&self, n: usize, u: &BBox) -> String {
        let f = |v: f64| crate::widgets::fmt_num(v, 1);
        if n == 1 {
            if let Some(it) = self.selected_items().first() {
                match it {
                    Item::View(v) => {
                        return format!(
                            "{} · {} / {}",
                            kdraft_render::view::scale_label(v.scale).replace("SCALE ", ""),
                            f(v.at.x),
                            f(v.at.y)
                        );
                    }
                    Item::Dimension(d) => return format!("offset {}", f(d.offset)),
                    _ => {}
                }
            }
        }
        format!("{} x {}", f(u.width()), f(u.height()))
    }

    fn paint_tool_preview(&self, painter: &egui::Painter, sp: &dyn Fn(Point) -> Pos2, t: &Tokens) {
        if self.tool.kind.is_select() || matches!(self.tool.kind, ToolKind::Pan) {
            return;
        }
        let ptr = self.tool.pointer;
        // Picked anchors so far.
        let mut picked: Vec<&crate::tools::Pick> = Vec::new();
        match &self.tool.stage {
            crate::tools::Stage::One(Some(a), _) => picked.push(a),
            crate::tools::Stage::Two(a, b) => {
                picked.push(a);
                picked.push(b);
            }
            _ => {}
        }
        for a in &picked {
            painter.circle_stroke(sp(a.sheet), 5.0, Stroke::new(1.5, t.anchor));
            painter.circle_filled(sp(a.sheet), 2.0, t.anchor);
        }
        // The anchor under the pointer.
        if let Some(a) = &self.snapped {
            let c = sp(a.sheet);
            if let Some(r) = a.radius {
                painter.circle_stroke(c, (r * self.camera.zoom) as f32, Stroke::new(1.5, t.anchor));
            }
            painter.rect_stroke(
                Rect::from_center_size(c, Vec2::splat(10.0)),
                0.0,
                Stroke::new(1.5, t.anchor),
                egui::StrokeKind::Middle,
            );
            w::pill(
                painter,
                c + Vec2::new(0.0, -16.0),
                &a.label,
                t.anchor,
                Color32::WHITE,
            );
        }
        let Some(q) = ptr else { return };
        let stroke = Stroke::new(1.0, t.measure);
        match (&self.tool.kind, &self.tool.stage) {
            (ToolKind::Line, crate::tools::Stage::One(_, s)) => {
                painter.line_segment([sp(*s), sp(q)], stroke);
            }
            (ToolKind::Polyline, _) if !self.tool.poly.is_empty() => {
                let mut v: Vec<Pos2> = self.tool.poly.iter().map(|p| sp(*p)).collect();
                v.push(sp(q));
                painter.add(egui::Shape::line(v, stroke));
                for p in &self.tool.poly {
                    painter.circle_filled(sp(*p), 2.5, t.measure);
                }
            }
            (ToolKind::Circle, crate::tools::Stage::One(_, c)) => {
                let r = (c.dist(q) * self.camera.zoom) as f32;
                painter.circle_stroke(sp(*c), r, stroke);
                painter.line_segment([sp(*c), sp(q)], stroke);
            }
            (ToolKind::Rect, crate::tools::Stage::One(_, s)) => {
                painter.rect_stroke(
                    Rect::from_two_pos(sp(*s), sp(q)),
                    0.0,
                    stroke,
                    egui::StrokeKind::Middle,
                );
            }
            (ToolKind::DimLinear(_), crate::tools::Stage::One(Some(a), _)) => {
                painter.line_segment(
                    [sp(a.sheet), sp(q)],
                    Stroke::new(1.0, t.measure.gamma_multiply(0.6)),
                );
            }
            (ToolKind::DimLinear(k), crate::tools::Stage::Two(a, b)) => {
                // Rubber band: extension lines to the dimension line at the
                // offset under the pointer.
                let off = crate::tools::linear_offset(a.sheet, b.sheet, *k, q);
                let (s1, s2) = (a.sheet, b.sheet);
                let n0 = match k {
                    LinearKind::Horizontal => Point::new(0.0, -1.0),
                    LinearKind::Vertical => Point::new(1.0, 0.0),
                    LinearKind::Aligned => {
                        let d = s2 - s1;
                        let l = (d.x * d.x + d.y * d.y).sqrt().max(1e-9);
                        Point::new(d.y / l, -d.x / l)
                    }
                };
                let n = if off >= 0.0 {
                    n0
                } else {
                    Point::new(-n0.x, -n0.y)
                };
                let dot = |p: Point| p.x * n.x + p.y * n.y;
                let level = dot(s1).max(dot(s2)) + off.abs();
                let foot = |p: Point| {
                    Point::new(p.x + n.x * (level - dot(p)), p.y + n.y * (level - dot(p)))
                };
                let (f1, f2) = (foot(s1), foot(s2));
                painter.line_segment([sp(s1), sp(f1)], stroke);
                painter.line_segment([sp(s2), sp(f2)], stroke);
                painter.line_segment([sp(f1), sp(f2)], Stroke::new(1.5, t.measure));
                let len = match k {
                    LinearKind::Horizontal => (s2.x - s1.x).abs(),
                    LinearKind::Vertical => (s2.y - s1.y).abs(),
                    LinearKind::Aligned => s1.dist(s2),
                };
                let scale = self.view_scale(&a.view).unwrap_or(1.0);
                w::pill(
                    painter,
                    sp(Point::new((f1.x + f2.x) / 2.0, (f1.y + f2.y) / 2.0)),
                    &crate::widgets::fmt_num(len / scale, 2),
                    t.measure,
                    Color32::WHITE,
                );
            }
            (
                ToolKind::DimRadius { .. } | ToolKind::Callout(_),
                crate::tools::Stage::One(Some(a), _),
            ) => {
                painter.line_segment([sp(a.sheet), sp(q)], stroke);
            }
            (ToolKind::DimOrdinate, crate::tools::Stage::Two(_, b)) => {
                painter.line_segment([sp(b.sheet), sp(q)], stroke);
            }
            _ => {
                // Placement: a crosshair at the (grid-snapped) point.
                let c = sp(q);
                painter.line_segment([c - Vec2::new(8.0, 0.0), c + Vec2::new(8.0, 0.0)], stroke);
                painter.line_segment([c - Vec2::new(0.0, 8.0), c + Vec2::new(0.0, 8.0)], stroke);
            }
        }
        // Numbered callouts: the next label, as a balloon where it will be
        // placed (after the anchor) or beside the pointer (before).
        if self.tool.kind == ToolKind::Callout(CalloutTool::Number) {
            if let Some(pi) = self.s.current_page_index() {
                let label = kdraft_engine::commands_callout::next_label(&self.s, pi);
                let th = self.s.doc().styles.text_height;
                let b = kdraft_render::notes::Balloon::new(
                    &label,
                    th,
                    kdraft_doc::style::BalloonShape::Circle,
                    self.s.fonts(),
                );
                let zoom = self.camera.zoom;
                let placing = matches!(self.tool.stage, crate::tools::Stage::One(Some(_), _));
                let c = if placing {
                    sp(q)
                } else {
                    sp(q)
                        + Vec2::new(
                            14.0 + (b.half_w * zoom) as f32,
                            -14.0 - (b.half_h * zoom) as f32,
                        )
                };
                let r = ((b.half_w * zoom) as f32).max(7.0);
                painter.circle_filled(c, r, Color32::from_white_alpha(200));
                painter.circle_stroke(c, r, Stroke::new(1.5, t.measure));
                let em = (self.s.fonts().regular.em_for_cap(th) * zoom) as f32;
                painter.text(
                    c,
                    Align2::CENTER_CENTER,
                    &label,
                    egui::FontId::proportional(em.clamp(9.0, 120.0)),
                    t.measure,
                );
            }
        }
    }

    fn view_scale(&self, id: &Id) -> Option<f64> {
        self.s
            .current_page()?
            .views()
            .find(|v| &v.id == id)
            .map(|v| v.scale)
    }

    fn paint_rulers(&self, ui: &egui::Ui, full: Rect, canvas: Rect, t: &Tokens) {
        let p = ui.painter_at(full);
        let top = Rect::from_min_max(
            Pos2::new(canvas.min.x, full.min.y),
            Pos2::new(full.max.x, canvas.min.y),
        );
        let left = Rect::from_min_max(
            Pos2::new(full.min.x, canvas.min.y),
            Pos2::new(canvas.min.x, full.max.y),
        );
        let corner = Rect::from_min_max(full.min, canvas.min);
        for r in [top, left, corner] {
            p.rect_filled(r, 0.0, t.ruler);
        }
        p.hline(
            full.x_range(),
            canvas.min.y - 0.5,
            Stroke::new(1.0, t.border),
        );
        p.vline(
            canvas.min.x - 0.5,
            full.y_range(),
            Stroke::new(1.0, t.border),
        );
        let cam = self.camera;
        let (major, minor) = crate::rulers::ticks(cam.zoom, 56.0);
        let o = canvas.min;
        let x_lo = cam.to_sheet(Point::new(0.0, 0.0)).x;
        let x_hi = cam.to_sheet(Point::new(f64::from(canvas.width()), 0.0)).x;
        let y_lo = cam.to_sheet(Point::new(0.0, 0.0)).y;
        let y_hi = cam.to_sheet(Point::new(0.0, f64::from(canvas.height()))).y;
        let st = Stroke::new(1.0, t.ruler_text.gamma_multiply(0.6));
        let pt_clip = |r: Rect| ui.painter_at(r);
        let pt_top = pt_clip(top);
        for v in crate::rulers::tick_values(x_lo, x_hi, minor) {
            let x = o.x + (cam.to_screen(Point::new(v, 0.0)).x) as f32;
            let is_major = ((v / major).round() * major - v).abs() < 1e-6;
            let h = if is_major { 8.0 } else { 4.0 };
            pt_top.vline(x.round() + 0.5, (top.max.y - h)..=top.max.y, st);
            if is_major {
                pt_top.text(
                    Pos2::new(x + 3.0, top.min.y + 2.0),
                    Align2::LEFT_TOP,
                    crate::rulers::label(v),
                    text::mono(),
                    t.ruler_text,
                );
            }
        }
        let pt_left = pt_clip(left);
        for v in crate::rulers::tick_values(y_lo, y_hi, minor) {
            let y = o.y + (cam.to_screen(Point::new(0.0, v)).y) as f32;
            let is_major = ((v / major).round() * major - v).abs() < 1e-6;
            let h = if is_major { 8.0 } else { 4.0 };
            pt_left.hline((left.max.x - h)..=left.max.x, y.round() + 0.5, st);
            if is_major {
                pt_left.text(
                    Pos2::new(left.min.x + 2.0, y + 2.0),
                    Align2::LEFT_TOP,
                    crate::rulers::label(v),
                    egui::FontId::new(9.0, egui::FontFamily::Monospace),
                    t.ruler_text,
                );
            }
        }
        // Cursor position marks.
        if let Some(c) = self.cursor_mm {
            let s = cam.to_screen(c);
            let (x, y) = (o.x + s.x as f32, o.y + s.y as f32);
            pt_top.vline(x, top.y_range(), Stroke::new(1.0, t.accent));
            pt_left.hline(left.x_range(), y, Stroke::new(1.0, t.accent));
        }
    }

    fn tool_bar(&mut self, ui: &mut egui::Ui, canvas: Rect, t: &Tokens) {
        const B: f32 = 30.0;
        const CARET: f32 = 12.0;
        let n_groups = GROUPS.len() as f32;
        let width = 2.0 * B + n_groups * (B + CARET) + 2.0 * B + 2.0 * 13.0 + 16.0;
        let r = Rect::from_center_size(
            Pos2::new(canvas.center().x, canvas.max.y - 30.0),
            Vec2::new(width, B + 10.0),
        );
        let p = ui.painter_at(canvas);
        p.rect_filled(
            r.translate(Vec2::new(0.0, 2.0)).expand(1.0),
            CornerRadius::same(10),
            t.shadow,
        );
        p.rect(
            r,
            CornerRadius::same(9),
            t.float,
            Stroke::new(1.0, t.border_strong),
            egui::StrokeKind::Inside,
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(r.shrink2(Vec2::new(8.0, 5.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let ui = &mut child;
        ui.spacing_mut().item_spacing.x = 0.0;
        let cur = self.tool.kind;
        let mut set: Option<ToolKind> = None;
        for k in [ToolKind::Select, ToolKind::Pan] {
            let (title, icon, sc) = tool_meta(k);
            if w::icon_button_sized(
                ui,
                icon,
                title,
                (!sc.is_empty()).then_some(sc),
                true,
                cur == k,
                B,
            )
            .clicked()
            {
                set = Some(k);
            }
        }
        sep(ui, t);
        let has_board = self.s.board().is_some();
        for (g, default, members) in GROUPS {
            let k = self.group_tool.get(g).copied().unwrap_or(*default);
            let active = members.contains(&cur);
            let shown = if active { cur } else { k };
            let (title, icon, sc) = tool_meta(shown);
            if w::icon_button_sized(
                ui,
                icon,
                title,
                (!sc.is_empty()).then_some(sc),
                has_board,
                active,
                B,
            )
            .clicked()
            {
                set = Some(shown);
            }
            let (cr, cresp) = ui.allocate_exact_size(Vec2::new(CARET, B), Sense::click());
            let cresp = cresp.on_hover_text("More tools");
            cresp.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, has_board, format!("{g} tools"))
            });
            if cresp.hovered() {
                ui.painter().rect_filled(cr, CornerRadius::same(3), t.hover);
            }
            crate::icons::paint(
                ui.painter(),
                Rect::from_center_size(cr.center(), Vec2::splat(10.0)),
                Icon::ChevronDown,
                t.icon_secondary,
            );
            egui::Popup::menu(&cresp).show(|ui| {
                ui.set_min_width(230.0);
                for m in *members {
                    if let Some(e) = self
                        .catalogue
                        .entries
                        .iter()
                        .find(|e| {
                            e.run == crate::actions::Run::App(crate::actions::AppAct::Tool(*m))
                        })
                        .cloned()
                    {
                        if self.menu_row(ui, &e) {
                            set = Some(*m);
                        }
                    }
                }
            });
        }
        sep(ui, t);
        if w::icon_button_sized(ui, Icon::Snap, "Snap", Some("Shift+S"), true, self.snap, B)
            .clicked()
        {
            self.app_act(crate::actions::AppAct::ToggleSnap);
        }
        if w::icon_button_sized(
            ui,
            Icon::Grid,
            "Show grid",
            Some("Shift+G"),
            true,
            self.settings.show_grid,
            B,
        )
        .clicked()
        {
            self.app_act(crate::actions::AppAct::ToggleGrid);
        }
        if let Some(k) = set {
            self.set_tool(k);
        }
    }

    fn canvas_input(&mut self, ui: &egui::Ui, resp: &egui::Response, origin: Pos2) {
        let hover = if resp.contains_pointer() {
            ui.input(|i| i.pointer.latest_pos())
        } else {
            None
        };
        let alt = ui.input(|i| i.modifiers.alt);
        self.shift_held = ui.input(|i| i.modifiers.shift);
        self.free_scale = alt;
        self.measure_on = alt;
        let cursor = hover.map(|p| self.camera.to_sheet(pt(p, origin)));
        if cursor != self.cursor_mm {
            ui.ctx().request_repaint();
        }
        self.cursor_mm = cursor;
        self.canvas_cursor = None;
        let mm_per_px = 1.0 / self.camera.zoom;
        self.hover_item = match cursor {
            Some(p) if self.drag.is_idle() => self.pick_item(p, 3.0 * mm_per_px),
            _ => None,
        };
        let (scroll, zoom_delta, middle, space, primary_pressed, primary_released, shift, latest) =
            ui.input(|i| {
                (
                    i.smooth_scroll_delta,
                    i.zoom_delta(),
                    i.pointer.middle_down(),
                    i.key_down(egui::Key::Space),
                    i.pointer.primary_pressed(),
                    i.pointer.primary_released(),
                    i.modifiers.shift,
                    i.pointer.latest_pos(),
                )
            });
        let panning_tool = matches!(self.tool.kind, ToolKind::Pan);
        if let Some(p) = cursor {
            let handle = self
                .single_selected_view()
                .and_then(|(_, b, _)| hit::hit_handle(&b, p, 5.0 * mm_per_px))
                .filter(|h| h.is_corner() && self.tool.kind.is_select());
            let over = self.hover_item.is_some() && self.tool.kind.is_select();
            let moving = matches!(self.drag, crate::drag::Drag::Moving { .. });
            let kind = hit::cursor_for(handle, over, moving, middle || space || panning_tool);
            self.canvas_cursor = Some(kind);
            let icon = if !self.tool.kind.is_select() && !panning_tool {
                egui::CursorIcon::Crosshair
            } else {
                match kind {
                    hit::CursorKind::Default => egui::CursorIcon::Default,
                    hit::CursorKind::Grab => egui::CursorIcon::Grab,
                    hit::CursorKind::Grabbing => egui::CursorIcon::Grabbing,
                    hit::CursorKind::ResizeNwSe => egui::CursorIcon::ResizeNwSe,
                    hit::CursorKind::ResizeNeSw => egui::CursorIcon::ResizeNeSw,
                    hit::CursorKind::Pan => egui::CursorIcon::AllScroll,
                }
            };
            ui.ctx().set_cursor_icon(icon);
        }
        if let Some(h) = hover {
            let c = pt(h, origin);
            if zoom_delta != 1.0 {
                self.camera.zoom_at(c, f64::from(zoom_delta));
                self.note_zoom_change();
            } else if scroll.y != 0.0 {
                self.camera.zoom_at(c, 1.0015f64.powf(f64::from(scroll.y)));
                self.note_zoom_change();
            }
        }
        if middle || space || panning_tool {
            if middle || resp.dragged() {
                let d = if middle {
                    ui.input(|i| i.pointer.delta())
                } else {
                    resp.drag_delta()
                };
                self.camera.pan_by(f64::from(d.x), f64::from(d.y));
            }
            return;
        }
        // Placement tools.
        if !self.tool.kind.is_select() {
            self.tool_input(resp, cursor, mm_per_px);
            return;
        }
        // Picking a link reference.
        if self.link_pick.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Alias);
            if resp.clicked() {
                if let Some(id) = self
                    .cursor_mm
                    .and_then(|p| self.pick_item(p, 3.0 * mm_per_px))
                {
                    self.finish_link_pick(&id);
                }
            }
            return;
        }
        if resp.double_clicked() {
            if let Some(p) = self.cursor_mm {
                if self.insert_vertex_at(p, mm_per_px) {
                    return;
                }
                if let Some(id) = self.pick_item(p, 3.0 * mm_per_px) {
                    self.cmd("select.set", json!({ "ids": [id.0.clone()] }));
                    if self.begin_text_edit(&id, false) || self.begin_label_edit(&id) {
                        return;
                    }
                    self.cmd("select.set", json!({ "ids": [id.0] }));
                    self.fit_selection();
                }
            }
            return;
        }
        if primary_pressed && resp.hovered() {
            if let Some(p) = self.cursor_mm {
                let press = self.press_target(p, mm_per_px);
                for a in self.drag.press(p, press, shift) {
                    self.drag_action(a, Some(p));
                }
            }
        }
        if !self.drag.is_idle() {
            if let Some(l) = latest {
                let p = self.camera.to_sheet(pt(l, origin));
                for a in self.drag.moved(p, DRAG_THRESHOLD_PX * mm_per_px) {
                    self.drag_action(a, Some(p));
                }
                if primary_released {
                    for a in self.drag.release(p) {
                        self.drag_action(a, Some(p));
                    }
                }
            }
        }
    }

    /// Pointer input while a placement tool is active.
    fn tool_input(&mut self, resp: &egui::Response, cursor: Option<Point>, mm_per_px: f64) {
        let Some(raw) = cursor else {
            self.snapped = None;
            return;
        };
        if self.tool.kind.picks_anchors() {
            if let (Some(board), Some(page)) = (self.s.board_arc(), self.s.current_page().cloned())
            {
                let bb = self.s.board_box();
                let views: Vec<(&kdraft_doc::View, BBox)> = page
                    .views()
                    .map(|v| (v, kdraft_doc::ops::view_bounds(v, bb)))
                    .collect();
                self.anchors.ensure(self.s.revision(), &board, bb, &views);
            }
            self.snapped = self.anchors.nearest(
                raw,
                ANCHOR_SNAP_PX * mm_per_px,
                self.tool.kind.circular_only(),
            );
        } else {
            self.snapped = None;
        }
        // Grid snap for free points.
        let grid = self.s.doc().styles.snap_grid;
        let p = match &self.snapped {
            Some(a) => a.sheet,
            None if self.snap && grid > 0.0 => {
                Point::new((raw.x / grid).round() * grid, (raw.y / grid).round() * grid)
            }
            None => raw,
        };
        self.tool.moved(p);
        if resp.double_clicked() && self.tool.kind == ToolKind::Polyline {
            match self.tool.finish() {
                Out::Place(pl) => self.place(pl),
                Out::Hint(h) => self.info(h),
                _ => {}
            }
            return;
        }
        if resp.clicked() {
            match self.tool.click(p, self.snapped.clone()) {
                Out::Place(pl) => self.place(pl),
                Out::Hint(h) => self.info(h),
                Out::Exit => self.set_tool(ToolKind::Select),
                Out::Nothing => {}
            }
        }
    }
    fn single_selected_dimension(&self) -> Option<(Id, f64, kdraft_engine::bounds::DimGeom)> {
        let sel = self.selected_items();
        match sel.as_slice() {
            [Item::Dimension(d)] => Some((d.id.clone(), d.offset, self.dim_geometry(d)?)),
            _ => None,
        }
    }

    /// The single selected view: its box and scale.
    fn single_selected_view(&self) -> Option<(Id, BBox, f64)> {
        let sel = self.selected_items();
        if sel.len() != 1 {
            return None;
        }
        match sel[0] {
            Item::View(v) => Some((v.id.clone(), self.s.item_bounds(sel[0])?, v.scale)),
            _ => None,
        }
    }

    /// The cursor readout of the status bar.
    fn press_target(&self, p: Point, mm_per_px: f64) -> Press {
        if let Some(h) = self.handle_at(p, mm_per_px) {
            return Press::Handle(h);
        }
        if let Some((_, b, scale)) = self.single_selected_view() {
            if let Some(h) = hit::hit_handle(&b, p, 5.0 * mm_per_px) {
                if h.is_corner() {
                    return Press::ScaleHandle {
                        handle: h,
                        bbox: b,
                        scale,
                    };
                }
            }
        }
        match self.pick_item(p, 3.0 * mm_per_px) {
            Some(id) => Press::Item {
                selected: self.s.selection.contains(&id),
                id,
            },
            None => Press::Empty,
        }
    }

    /// Executes one drag action.
    pub fn drag_action(&mut self, a: Action, _at: Option<Point>) {
        match a {
            Action::BeginEdit(h) => self.begin_edit(h),
            Action::PreviewEdit(p) => self.preview_edit(p),
            Action::HandleClicked(h) => {
                self.sel_vertex = match h.kind {
                    crate::handles::HandleKind::Vertex(i) => Some((h.item, i)),
                    _ => None,
                };
            }
            Action::SelectOnly(id) => {
                self.sel_vertex = None;
                self.cmd("select.set", json!({ "ids": [id.0] }));
            }
            Action::Toggle(id) => {
                self.cmd("select.toggle", json!({ "id": id.0 }));
            }
            Action::ClearSelection => {
                self.sel_vertex = None;
                self.cmd("select.clear", json!({}));
            }
            Action::BeginMove => {
                self.dim_drag = self.single_selected_dimension();
                if self.dim_drag.is_some() {
                    self.s.begin_gesture("Dimension offset");
                } else {
                    self.move_ctx = self.capture_move_ctx();
                    self.s.begin_gesture("Move");
                }
            }
            Action::PreviewMove(raw) if self.dim_drag.is_some() => {
                let Some((id, start, g)) = self.dim_drag.clone() else {
                    return;
                };
                let d = self.s.current_page().and_then(|p| match p.item(&id) {
                    Some(Item::Dimension(d)) => Some(d.clone()),
                    _ => None,
                });
                let Some(d) = d else { return };
                if let Some(mut o) = kdraft_engine::bounds::dragged_offset(&d, &g, start, raw) {
                    let grid = self.s.doc().styles.snap_grid;
                    if self.snap && grid > 0.0 {
                        o = (o / grid).round() * grid;
                    }
                    if let Err(e) = self.reg.preview(
                        &mut self.s,
                        "dimension.set_offset",
                        json!({ "ids": [id.0], "offset": o }),
                    ) {
                        self.error(format!("dimension offset: {e}"));
                    }
                }
            }
            Action::PreviewMove(raw) => {
                let tol = SNAP_TOLERANCE_PX / self.camera.zoom;
                let (d, guides) = match &self.move_ctx {
                    Some(c) => c.delta(raw, self.snap, tol),
                    None => (raw, Vec::new()),
                };
                self.guides = guides;
                // Alt-drag of linked items moves them alone: it edits
                // their link offsets.
                let offsets = self.free_scale
                    && !self.s.selection.items.is_empty()
                    && self.selected_items().iter().all(|i| i.link().is_some());
                let (cmd, args) = if offsets {
                    ("link.set_offset", json!({ "delta": [d.x, d.y] }))
                } else {
                    ("item.move", json!({ "delta": [d.x, d.y] }))
                };
                if let Err(e) = self.reg.preview(&mut self.s, cmd, args) {
                    self.error(format!("move: {e}"));
                }
            }
            Action::BeginScale => self.s.begin_gesture("Scale"),
            Action::PreviewScale(raw) => {
                let free = self.free_scale;
                let f = crate::drag::snap_scale(raw, free);
                if let Err(e) =
                    self.reg
                        .preview(&mut self.s, "item.set_scale", json!({ "scale": f }))
                {
                    self.error(format!("scale: {e}"));
                }
            }
            Action::Commit => {
                self.s.commit_gesture();
                self.guides.clear();
                self.move_ctx = None;
                self.dim_drag = None;
                self.end_edit();
            }
            Action::Cancel => {
                self.s.cancel_gesture();
                self.guides.clear();
                self.move_ctx = None;
                self.dim_drag = None;
                self.end_edit();
            }
            Action::Marquee(r) => self.marquee = Some(r),
            Action::MarqueeSelect { rect, additive } => {
                self.marquee = None;
                let mut ids: Vec<String> = if additive {
                    self.s.selection.items.iter().map(|i| i.0.clone()).collect()
                } else {
                    Vec::new()
                };
                let page = self.s.current_page();
                let unlocked: Vec<(Id, BBox)> = self
                    .item_boxes()
                    .into_iter()
                    .filter(|(id, _)| page.and_then(|p| p.item(id)).is_some_and(|i| !i.locked()))
                    .collect();
                for id in hit::marquee(&unlocked, &rect) {
                    if !ids.contains(&id.0) {
                        ids.push(id.0);
                    }
                }
                self.cmd("select.set", json!({ "ids": ids }));
            }
        }
    }

    fn capture_move_ctx(&mut self) -> Option<MoveCtx> {
        let pi = self.s.current_page_index()?;
        let boxes = self.item_boxes();
        let sel = &self.s.selection.items;
        let start = boxes
            .iter()
            .filter(|(id, _)| sel.contains(id))
            .map(|(_, b)| *b)
            .reduce(|a, b| a.union(&b))?;
        let page = self.s.current_page()?;
        let others = boxes
            .iter()
            .filter(|(id, _)| !sel.contains(id))
            .filter(|(id, _)| !matches!(page.item(id), Some(Item::Dimension(_))))
            .map(|(_, b)| *b)
            .collect();
        Some(MoveCtx {
            start,
            others,
            page: self.page_rect()?,
            area: self.s.drawing_area(pi),
            grid: self.s.doc().styles.snap_grid,
        })
    }
}

fn handle(painter: &egui::Painter, c: Pos2, color: Color32, t: &Tokens) {
    let r = Rect::from_center_size(c, Vec2::splat(7.0));
    painter.rect_filled(r, 1.0, t.paper);
    painter.rect_stroke(r, 1.0, Stroke::new(1.0, color), egui::StrokeKind::Inside);
}

fn sep(ui: &mut egui::Ui, t: &Tokens) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(13.0, 22.0), Sense::hover());
    ui.painter()
        .vline(r.center().x, r.y_range(), Stroke::new(1.0, t.border_strong));
}
