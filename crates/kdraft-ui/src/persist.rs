//! Per-document view state (last page, camera per page) kept in the user
//! settings, and the window placement check.

use eframe::egui;
use kdraft_board::geom::{BBox, Point};
use kdraft_doc::Id;

use crate::app::App;
use crate::camera::Camera;
use crate::settings::{doc_key, DocView, PageCam};

impl App {
    /// The camera for a page as stored, when it still fits the page (same
    /// sheet size) and shows part of it.
    pub fn saved_camera(&self, page_id: &Id, page_rect: &BBox) -> Option<Camera> {
        let c = self.page_cams.get(&page_id.0)?;
        let (vw, vh) = self.viewport;
        let same_size =
            (c.w - page_rect.width()).abs() < 0.01 && (c.h - page_rect.height()).abs() < 0.01;
        if !same_size || !(crate::camera::MIN_ZOOM..=crate::camera::MAX_ZOOM).contains(&c.zoom) {
            return None;
        }
        let cam = Camera {
            zoom: c.zoom,
            pan: Point::new(vw / 2.0 - c.cx * c.zoom, vh / 2.0 - c.cy * c.zoom),
        };
        // Never a blank canvas: some of the page must be in view.
        let s = cam.box_to_screen(page_rect);
        let visible =
            s.max.x > 20.0 && s.min.x < vw - 20.0 && s.max.y > 20.0 && s.min.y < vh - 20.0;
        visible.then_some(cam)
    }

    /// Records the camera of the page shown; true when it changed.
    pub fn note_camera(&mut self, page_id: &Id, page_rect: &BBox) -> bool {
        let (vw, vh) = self.viewport;
        if vw < 10.0 || vh < 10.0 {
            return false;
        }
        let centre = self.camera.to_sheet(Point::new(vw / 2.0, vh / 2.0));
        let cam = PageCam {
            zoom: self.camera.zoom,
            cx: centre.x,
            cy: centre.y,
            w: page_rect.width(),
            h: page_rect.height(),
        };
        let old = self.page_cams.insert(page_id.0.clone(), cam);
        old.is_none_or(|o| {
            (o.zoom - cam.zoom).abs() > 1e-9
                || (o.cx - cam.cx).abs() > 1e-6
                || (o.cy - cam.cy).abs() > 1e-6
        })
    }

    /// The current document's view state into the settings.
    pub fn capture_doc_view(&mut self) {
        let Some(path) = self.s.doc_path().map(|p| p.to_path_buf()) else {
            return;
        };
        let pages: Vec<String> = self.s.doc().pages.iter().map(|p| p.id.0.clone()).collect();
        let dv = DocView {
            page: self.s.current_page().map(|p| p.id.0.clone()),
            cams: self
                .page_cams
                .iter()
                .filter(|(k, _)| pages.contains(k))
                .map(|(k, v)| (k.clone(), *v))
                .collect(),
        };
        self.settings.docs.insert(doc_key(&path), dv);
        self.settings.prune_docs();
    }

    /// After opening a document: its last page and the cameras.
    pub fn restore_doc_view(&mut self) {
        self.page_cams.clear();
        let Some(path) = self.s.doc_path().map(|p| p.to_path_buf()) else {
            return;
        };
        let Some(dv) = self.settings.docs.get(&doc_key(&path)).cloned() else {
            return;
        };
        self.page_cams = dv.cams.into_iter().collect();
        if let Some(pid) = dv.page {
            let id = Id(pid);
            if self.s.doc().pages.iter().any(|p| p.id == id) {
                self.s.selection = kdraft_engine::Selection {
                    page: Some(id),
                    items: Vec::new(),
                };
            }
        }
    }

    /// Records the window placement; on the first frame with monitor
    /// information, moves a window that is off every monitor back.
    pub fn track_window(&mut self, ctx: &egui::Context) {
        let (outer, inner, maximized, monitor) = ctx.input(|i| {
            let v = i.viewport();
            (v.outer_rect, v.inner_rect, v.maximized, v.monitor_size)
        });
        let (Some(outer), Some(inner)) = (outer, inner) else {
            return;
        };
        if !self.window_checked {
            self.window_checked = true;
            let m = monitor.map(|m| (m.x, m.y));
            let plan = crate::settings::window_plan(self.settings.window, m);
            if self.settings.window.is_some() && Some(plan) != self.settings.window {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(plan.w, plan.h)));
                if plan.x.is_finite() && plan.y.is_finite() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                        plan.x, plan.y,
                    )));
                }
                self.settings.window = Some(plan);
                return;
            }
        }
        let max = maximized.unwrap_or(false);
        let mut g = self.settings.window.unwrap_or(crate::settings::WindowGeom {
            x: outer.min.x,
            y: outer.min.y,
            w: inner.width(),
            h: inner.height(),
            maximized: max,
        });
        // A maximised window keeps the size to go back to.
        if !max {
            g.x = outer.min.x;
            g.y = outer.min.y;
            g.w = inner.width();
            g.h = inner.height();
        }
        g.maximized = max;
        if Some(g) != self.settings.window
            && g.w >= crate::settings::MIN_WINDOW.0
            && g.h >= crate::settings::MIN_WINDOW.1
        {
            self.settings.window = Some(g);
            self.save_settings_soon();
        }
    }
}
