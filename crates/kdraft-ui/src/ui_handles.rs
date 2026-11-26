//! Edit handles on the canvas: drawing them for the single selected item,
//! pressing and dragging them (one gesture per drag, live preview through
//! engine commands), vertex selection, insertion and removal.

use eframe::egui::{self, Pos2, Stroke};
use kdraft_board::geom::Point;
use kdraft_doc::{GraphicShape, Id, Item};
use serde_json::json;

use crate::app::App;
use crate::handles::{self, EditHandle, HandleKind};
use crate::theme::Tokens;

/// Screen distance for picking a handle, px.
pub const HANDLE_PX: f64 = 6.0;
/// Board anchors snap within this distance, px.
const ANCHOR_PX: f64 = 10.0;

/// A handle drag in progress.
#[derive(Debug, Clone)]
pub struct EditDrag {
    pub handle: EditHandle,
    /// The item when the drag started.
    pub item: Item,
    /// Dimension line at the start (for text moves).
    pub dim_line: Option<(Point, Point)>,
    pub shift0: f64,
}

impl App {
    /// Handles of the single selected, unlocked item.
    pub fn edit_handles(&self) -> Vec<EditHandle> {
        let sel = self.selected_items();
        let [it] = sel.as_slice() else {
            return Vec::new();
        };
        if it.locked() {
            return Vec::new();
        }
        let px = 1.0 / self.camera.zoom.max(1e-9);
        match it {
            Item::Graphic(g) => match &g.shape {
                GraphicShape::Text { .. } => {
                    let ctx = self
                        .s
                        .bounds_ctx(kdraft_engine::bounds::BoundsMode::Content);
                    kdraft_engine::bounds::text_layout(g, &ctx)
                        .map(|l| handles::text_handles(&g.id, &l.corners, px))
                        .unwrap_or_default()
                }
                s => handles::shape_handles(&g.id, s),
            },
            Item::Dimension(d) => {
                let Some(geo) = self.dim_geometry(d) else {
                    return Vec::new();
                };
                let mut v: Vec<EditHandle> = geo
                    .anchors
                    .iter()
                    .take(if d.to.is_some() { 2 } else { 1 })
                    .enumerate()
                    .map(|(i, a)| EditHandle {
                        item: d.id.clone(),
                        kind: HandleKind::DimAnchor(i),
                        at: *a,
                    })
                    .collect();
                if is_linear(d) {
                    v.push(EditHandle {
                        item: d.id.clone(),
                        kind: HandleKind::DimText,
                        at: geo.text_at,
                    });
                }
                v
            }
            Item::Callout(c) => self
                .callout_anchor_point(c)
                .map(|at| {
                    vec![EditHandle {
                        item: c.id.clone(),
                        kind: HandleKind::CalloutAnchor,
                        at,
                    }]
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// Sheet position of a callout's anchored leader end.
    pub fn callout_anchor_point(&self, c: &kdraft_doc::Callout) -> Option<Point> {
        let board = self.s.board()?;
        let vid = c.view.as_ref()?;
        let v = self
            .s
            .doc()
            .pages
            .iter()
            .flat_map(|p| p.views())
            .find(|v| &v.id == vid)?;
        let p = kdraft_doc::anchor::resolve(c.anchor.as_ref()?, board, &board.holes()).ok()?;
        Some(kdraft_render::view::ViewMap::new(v, self.s.board_box()).point(p))
    }

    /// The handle under the pointer, if any.
    pub fn handle_at(&self, p: Point, mm_per_px: f64) -> Option<EditHandle> {
        let hs = self.edit_handles();
        handles::hit(&hs, p, HANDLE_PX * mm_per_px).cloned()
    }

    fn snap_point(&mut self, raw: Point, anchors_only: bool) -> Point {
        let mm_per_px = 1.0 / self.camera.zoom.max(1e-9);
        if let (Some(board), Some(page)) = (self.s.board_arc(), self.s.current_page().cloned()) {
            let bb = self.s.board_box();
            let views: Vec<(&kdraft_doc::View, kdraft_board::geom::BBox)> = page
                .views()
                .map(|v| (v, kdraft_doc::ops::view_bounds(v, bb)))
                .collect();
            self.anchors.ensure(self.s.revision(), &board, bb, &views);
        }
        self.snapped = if self.snap || anchors_only {
            self.anchors.nearest(raw, ANCHOR_PX * mm_per_px, false)
        } else {
            None
        };
        if let Some(a) = &self.snapped {
            return a.sheet;
        }
        let grid = self.s.doc().styles.snap_grid;
        if self.snap && grid > 0.0 && !anchors_only {
            Point::new((raw.x / grid).round() * grid, (raw.y / grid).round() * grid)
        } else {
            raw
        }
    }

    pub(crate) fn begin_edit(&mut self, h: EditHandle) {
        let Some(item) = self.s.find_item(&h.item).map(|(_, i)| i.clone()) else {
            return;
        };
        let dim_line = match &item {
            Item::Dimension(d) => self.dim_geometry(d).map(|g| g.dim_line),
            _ => None,
        };
        let shift0 = match &item {
            Item::Dimension(d) => d.text_shift(),
            _ => 0.0,
        };
        self.s.begin_gesture(&h.label());
        self.edit_drag = Some(EditDrag {
            handle: h,
            item,
            dim_line,
            shift0,
        });
    }

    pub(crate) fn preview_edit(&mut self, raw: Point) {
        let Some(e) = self.edit_drag.clone() else {
            return;
        };
        let id = e.handle.item.0.clone();
        let shift = self.shift_held;
        let free = self.free_scale;
        let (cmd, args) = match (&e.item, e.handle.kind) {
            (Item::Graphic(g), HandleKind::Rotate) => {
                let GraphicShape::Text { at, rotation, .. } = &g.shape else {
                    return;
                };
                let r = handles::text_rotation(*at, e.handle.at, raw, *rotation, free);
                ("text.set", json!({ "id": id, "rotation": r }))
            }
            (Item::Graphic(g), HandleKind::Wrap) => {
                let ctx = self
                    .s
                    .bounds_ctx(kdraft_engine::bounds::BoundsMode::Content);
                let Some(l) = kdraft_engine::bounds::text_layout(g, &ctx) else {
                    return;
                };
                let [tl, tr, _, _] = l.corners;
                let len = tl.dist(tr).max(1e-9);
                let dir = Point::new((tr.x - tl.x) / len, (tr.y - tl.y) / len);
                let w = handles::wrap_width(tl, dir, raw);
                let w = if self.snap { w.round() } else { w };
                ("text.set", json!({ "id": id, "wrap": w.max(2.0) }))
            }
            (Item::Graphic(g), k) => {
                let p = self.snap_point(raw, false);
                let Some(pts) = handles::edited_points(&g.shape, k, p, shift) else {
                    return;
                };
                let v: Vec<serde_json::Value> = pts.iter().map(|q| json!([q.x, q.y])).collect();
                ("graphic.set_points", json!({ "id": id, "points": v }))
            }
            (Item::Dimension(d), HandleKind::DimAnchor(i)) => {
                self.snap_point(raw, true);
                let end = if i == 0 { "from" } else { "to" };
                let orig = if i == 0 { &d.from } else { &d.to };
                let pick = self
                    .snapped
                    .clone()
                    .filter(|a| Some(&a.view) == d.view.as_ref());
                let anchor = match (&pick, orig) {
                    (Some(a), _) => kdraft_doc::anchor_text(&a.anchor).ok(),
                    (None, Some(o)) => kdraft_doc::anchor_text(o).ok(),
                    _ => None,
                };
                if pick.is_none() {
                    self.snapped = None;
                }
                let Some(anchor) = anchor else { return };
                (
                    "dimension.set_anchor",
                    json!({ "id": id, "end": end, "anchor": anchor }),
                )
            }
            (Item::Dimension(_), HandleKind::DimText) => {
                let Some((a, b)) = e.dim_line else { return };
                let len = a.dist(b).max(1e-9);
                let u = Point::new((b.x - a.x) / len, (b.y - a.y) / len);
                let d = (raw.x - e.handle.at.x) * u.x + (raw.y - e.handle.at.y) * u.y;
                let mut s = e.shift0 + d;
                if s.abs() < 1.0 {
                    s = 0.0;
                }
                (
                    "dimension.set_text_shift",
                    json!({ "ids": [id], "shift": s }),
                )
            }
            (Item::Callout(c), HandleKind::CalloutAnchor) => {
                self.snap_point(raw, true);
                match self.snapped.clone() {
                    Some(a) => (
                        "callout.set_anchor",
                        json!({
                            "id": id,
                            "anchor": kdraft_doc::anchor_text(&a.anchor).unwrap_or_default(),
                            "view": a.view.0,
                        }),
                    ),
                    None => {
                        let Some(o) = c
                            .anchor
                            .as_ref()
                            .and_then(|o| kdraft_doc::anchor_text(o).ok())
                        else {
                            return;
                        };
                        ("callout.set_anchor", json!({ "id": id, "anchor": o }))
                    }
                }
            }
            _ => return,
        };
        // An invalid intermediate state (for example a zero-size circle)
        // keeps the last good preview.
        let _ = self.reg.preview(&mut self.s, cmd, args);
    }

    pub(crate) fn end_edit(&mut self) {
        self.edit_drag = None;
        self.snapped = None;
    }

    /// Double-click on a selected polyline or polygon: insert a vertex on
    /// the nearest segment. Returns whether it did.
    pub(crate) fn insert_vertex_at(&mut self, p: Point, mm_per_px: f64) -> bool {
        let sel = self.selected_items();
        let [Item::Graphic(g)] = sel.as_slice() else {
            return false;
        };
        let closed = match &g.shape {
            GraphicShape::Polyline { .. } | GraphicShape::Line { .. } => false,
            GraphicShape::Polygon { .. } => true,
            _ => return false,
        };
        let points = g.shape.points();
        let Some((idx, d)) = handles::insert_index(&points, closed, p) else {
            return false;
        };
        if d > 4.0 * mm_per_px {
            return false;
        }
        let id = g.id.0.clone();
        self.cmd(
            "graphic.insert_point",
            json!({ "id": id, "index": idx, "at": [p.x, p.y] }),
        );
        self.sel_vertex = Some((Id(id), idx));
        true
    }

    /// Delete with a selected vertex removes it. Returns whether it did.
    pub(crate) fn delete_selected_vertex(&mut self) -> bool {
        let Some((id, i)) = self.sel_vertex.clone() else {
            return false;
        };
        if !self.s.selection.contains(&id) {
            self.sel_vertex = None;
            return false;
        }
        let ok = self.cmd("graphic.remove_point", json!({ "id": id.0, "index": i }));
        self.sel_vertex = None;
        ok
    }

    /// Draws the handles of the selection.
    pub(crate) fn paint_handles(
        &self,
        painter: &egui::Painter,
        sp: &dyn Fn(Point) -> Pos2,
        t: &Tokens,
    ) {
        let hs = self.edit_handles();
        for h in &hs {
            let c = sp(h.at);
            let selected = matches!(
                (&self.sel_vertex, h.kind),
                (Some((id, i)), HandleKind::Vertex(k)) if *id == h.item && *i == k
            );
            match h.kind {
                HandleKind::Rotate => {
                    painter.circle(c, 4.5, t.paper, Stroke::new(1.5, t.accent));
                    painter.circle_filled(c, 1.5, t.accent);
                }
                HandleKind::DimAnchor(_) | HandleKind::CalloutAnchor => {
                    painter.circle(c, 4.5, t.paper, Stroke::new(1.5, t.anchor));
                }
                HandleKind::Radius
                | HandleKind::Wrap
                | HandleKind::Edge(_)
                | HandleKind::DimText => {
                    let r = egui::Rect::from_center_size(c, egui::vec2(8.0, 8.0));
                    painter.rect(
                        r,
                        1.0,
                        t.paper,
                        Stroke::new(1.5, t.accent),
                        egui::StrokeKind::Middle,
                    );
                }
                _ => {
                    let r = egui::Rect::from_center_size(c, egui::vec2(9.0, 9.0));
                    let fill = if selected { t.accent } else { t.paper };
                    painter.rect(
                        r,
                        1.0,
                        fill,
                        Stroke::new(1.5, t.accent),
                        egui::StrokeKind::Middle,
                    );
                }
            }
        }
        // Anchor under a dragged anchor handle: marker and name.
        if let (Some(_), Some(a)) = (&self.edit_drag, &self.snapped) {
            let c = sp(a.sheet);
            painter.circle_stroke(c, 6.0, Stroke::new(2.0, t.anchor));
            crate::widgets::pill(
                painter,
                c + egui::vec2(0.0, -18.0),
                &a.label,
                t.anchor,
                egui::Color32::WHITE,
            );
        }
    }
}

fn is_linear(d: &kdraft_doc::Dimension) -> bool {
    use kdraft_doc::DimensionKind as K;
    !matches!(d.kind, K::Radius | K::Radial | K::Diameter | K::Ordinate)
}
