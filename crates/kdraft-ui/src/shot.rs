//! Headless screenshots of the editor: egui runs without a window, its
//! tessellated meshes are rasterised in software (barycentric triangles,
//! bilinear texture sampling, clip rectangles, premultiplied blending in
//! gamma space as egui's own renderers do) and written as PNG.
//!
//! Used by the hidden `kdraft-gui --screenshot` dev command, by golden
//! tests and by the frame-time measurement. Nothing here needs a GPU or a
//! display.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, epaint, Event, Modifiers, Pos2, Rect, TextureId, Vec2};

use crate::app::App;

/// A texture as premultiplied sRGBA bytes.
struct Img {
    w: usize,
    h: usize,
    rgba: Vec<[u8; 4]>,
    nearest: bool,
}

/// Textures egui asked for, kept in step with each frame's delta.
#[derive(Default)]
pub struct TexStore {
    map: HashMap<TextureId, Img>,
}

impl TexStore {
    pub fn apply(&mut self, delta: &egui::TexturesDelta) {
        for (id, deltas) in &delta.set {
            for d in deltas {
                let epaint::ImageData::Color(img) = &d.image;
                let [w, h] = img.size;
                let px: Vec<[u8; 4]> = img.pixels.iter().map(|c| c.to_array()).collect();
                let nearest = d.options.magnification == egui::TextureFilter::Nearest;
                match d.pos {
                    None => {
                        self.map.insert(
                            *id,
                            Img {
                                w,
                                h,
                                rgba: px,
                                nearest,
                            },
                        );
                    }
                    Some([x0, y0]) => {
                        if let Some(t) = self.map.get_mut(id) {
                            for y in 0..h {
                                for x in 0..w {
                                    let (tx, ty) = (x0 + x, y0 + y);
                                    if tx < t.w && ty < t.h {
                                        t.rgba[ty * t.w + tx] = px[y * w + x];
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        for id in &delta.free {
            self.map.remove(id);
        }
    }
}

impl Img {
    fn texel(&self, x: i64, y: i64) -> [f32; 4] {
        let x = x.clamp(0, self.w as i64 - 1) as usize;
        let y = y.clamp(0, self.h as i64 - 1) as usize;
        let p = self.rgba[y * self.w + x];
        [
            f32::from(p[0]) / 255.0,
            f32::from(p[1]) / 255.0,
            f32::from(p[2]) / 255.0,
            f32::from(p[3]) / 255.0,
        ]
    }

    fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        if self.w == 0 || self.h == 0 {
            return [1.0; 4];
        }
        let fx = u * self.w as f32 - 0.5;
        let fy = v * self.h as f32 - 0.5;
        if self.nearest {
            return self.texel((fx + 0.5).floor() as i64, (fy + 0.5).floor() as i64);
        }
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let a = self.texel(x0, y0);
        let b = self.texel(x0 + 1, y0);
        let c = self.texel(x0, y0 + 1);
        let d = self.texel(x0 + 1, y0 + 1);
        let mut o = [0.0; 4];
        for i in 0..4 {
            let top = a[i] + (b[i] - a[i]) * tx;
            let bot = c[i] + (d[i] - c[i]) * tx;
            o[i] = top + (bot - top) * ty;
        }
        o
    }
}

/// An RGBA frame buffer (premultiplied, gamma space, 0..1).
pub struct Frame {
    pub w: usize,
    pub h: usize,
    px: Vec<[f32; 4]>,
}

impl Frame {
    pub fn new(w: usize, h: usize, bg: [f32; 4]) -> Frame {
        Frame {
            w,
            h,
            px: vec![bg; w * h],
        }
    }

    /// 8-bit RGBA bytes (opaque when the background was).
    pub fn rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.w * self.h * 4);
        for p in &self.px {
            for c in p {
                out.push((c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
            }
        }
        out
    }

    pub fn png(&self) -> Result<Vec<u8>, String> {
        kdraft_render::raster::encode_png_rgba(self.rgba(), self.w as u32, self.h as u32)
            .map_err(|e| e.to_string())
    }

    /// Draws egui's clipped primitives at `ppp` physical pixels per point.
    pub fn draw(&mut self, prims: &[epaint::ClippedPrimitive], ppp: f32, tex: &TexStore) {
        for cp in prims {
            let epaint::Primitive::Mesh(mesh) = &cp.primitive else {
                continue;
            };
            let clip = Rect::from_min_max(
                Pos2::new(
                    (cp.clip_rect.min.x * ppp).round().max(0.0),
                    (cp.clip_rect.min.y * ppp).round().max(0.0),
                ),
                Pos2::new(
                    (cp.clip_rect.max.x * ppp).round().min(self.w as f32),
                    (cp.clip_rect.max.y * ppp).round().min(self.h as f32),
                ),
            );
            if clip.width() <= 0.0 || clip.height() <= 0.0 {
                continue;
            }
            let img = tex.map.get(&mesh.texture_id);
            for tri in mesh.indices.chunks_exact(3) {
                let v = [
                    mesh.vertices.get(tri[0] as usize),
                    mesh.vertices.get(tri[1] as usize),
                    mesh.vertices.get(tri[2] as usize),
                ];
                if let [Some(a), Some(b), Some(c)] = v {
                    self.triangle([a, b, c], ppp, clip, img);
                }
            }
        }
    }

    fn triangle(&mut self, v: [&epaint::Vertex; 3], ppp: f32, clip: Rect, img: Option<&Img>) {
        let p: Vec<(f32, f32)> = v.iter().map(|x| (x.pos.x * ppp, x.pos.y * ppp)).collect();
        let (mut i0, mut i1, i2) = (0usize, 1usize, 2usize);
        let area = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| {
            (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
        };
        let mut ar = area(p[i0], p[i1], p[i2]);
        if ar.abs() < 1e-12 {
            return;
        }
        if ar < 0.0 {
            std::mem::swap(&mut i0, &mut i1);
            ar = -ar;
        }
        let (a, b, c) = (p[i0], p[i1], p[i2]);
        let (va, vb, vc) = (v[i0], v[i1], v[i2]);
        let minx = a.0.min(b.0).min(c.0).floor().max(clip.min.x) as i64;
        let maxx = a.0.max(b.0).max(c.0).ceil().min(clip.max.x) as i64;
        let miny = a.1.min(b.1).min(c.1).floor().max(clip.min.y) as i64;
        let maxy = a.1.max(b.1).max(c.1).ceil().min(clip.max.y) as i64;
        if minx >= maxx || miny >= maxy {
            return;
        }
        // Top-left rule so shared edges are covered once (with y down and
        // positive area, an edge is "top" when horizontal and going left,
        // "left" when going up).
        let top_left = |e0: (f32, f32), e1: (f32, f32)| {
            let (dx, dy) = (e1.0 - e0.0, e1.1 - e0.1);
            (dy == 0.0 && dx < 0.0) || dy < 0.0
        };
        let edges = [(b, c), (c, a), (a, b)];
        let tl = [
            top_left(edges[0].0, edges[0].1),
            top_left(edges[1].0, edges[1].1),
            top_left(edges[2].0, edges[2].1),
        ];
        let col = |x: &epaint::Vertex| {
            let [r, g, b, a] = x.color.to_array();
            [
                f32::from(r) / 255.0,
                f32::from(g) / 255.0,
                f32::from(b) / 255.0,
                f32::from(a) / 255.0,
            ]
        };
        let (ca, cb, cc) = (col(va), col(vb), col(vc));
        for y in miny..maxy {
            let py = y as f32 + 0.5;
            for x in minx..maxx {
                let px = x as f32 + 0.5;
                let q = (px, py);
                let w = [
                    area(edges[0].0, edges[0].1, q),
                    area(edges[1].0, edges[1].1, q),
                    area(edges[2].0, edges[2].1, q),
                ];
                if (0..3).any(|k| w[k] < 0.0 || (w[k] == 0.0 && !tl[k])) {
                    continue;
                }
                let (l0, l1, l2) = (w[0] / ar, w[1] / ar, w[2] / ar);
                let mut s = [0.0f32; 4];
                for k in 0..4 {
                    s[k] = ca[k] * l0 + cb[k] * l1 + cc[k] * l2;
                }
                if let Some(t) = img {
                    let u = va.uv.x * l0 + vb.uv.x * l1 + vc.uv.x * l2;
                    let v = va.uv.y * l0 + vb.uv.y * l1 + vc.uv.y * l2;
                    let tx = t.sample(u, v);
                    for k in 0..4 {
                        s[k] *= tx[k];
                    }
                }
                if s[3] <= 0.0 && s[0] <= 0.0 && s[1] <= 0.0 && s[2] <= 0.0 {
                    continue;
                }
                let d = &mut self.px[y as usize * self.w + x as usize];
                let inv = 1.0 - s[3];
                for k in 0..4 {
                    d[k] = s[k] + d[k] * inv;
                }
            }
        }
    }
}

/// Mean absolute difference per channel (0..255) between two RGBA buffers
/// of the same size; `None` if the sizes differ.
pub fn mean_abs_diff(a: &[u8], b: &[u8]) -> Option<f64> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let s: u64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| u64::from(x.abs_diff(*y)))
        .sum();
    Some(s as f64 / a.len() as f64)
}

/// Raw input for a headless frame of `size` points at `ppp`.
fn raw_input(size: Vec2, ppp: f32) -> egui::RawInput {
    let mut r = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
        ..Default::default()
    };
    if let Some(v) = r.viewports.get_mut(&egui::ViewportId::ROOT) {
        v.native_pixels_per_point = Some(ppp);
    } else {
        r.viewports.insert(
            egui::ViewportId::ROOT,
            egui::ViewportInfo {
                native_pixels_per_point: Some(ppp),
                ..Default::default()
            },
        );
    }
    r
}

/// Runs the editor headless and takes screenshots.
pub struct Shooter {
    pub ctx: egui::Context,
    tex: TexStore,
    pub size: Vec2,
    pub ppp: f32,
    time: f64,
    pointer: Pos2,
    /// Modifiers held in the next frames.
    pub modifiers: Modifiers,
    /// Duration of the last frame's `run_ui` plus tessellation.
    pub last_frame: Duration,
    last_prims: Vec<epaint::ClippedPrimitive>,
}

impl Shooter {
    pub fn new(ctx: egui::Context, size: Vec2, ppp: f32) -> Shooter {
        Shooter {
            ctx,
            tex: TexStore::default(),
            size,
            ppp,
            time: 0.0,
            pointer: Pos2::ZERO,
            modifiers: Modifiers::NONE,
            last_frame: Duration::ZERO,
            last_prims: Vec::new(),
        }
    }

    /// One frame with `events`.
    pub fn frame(&mut self, app: &mut App, events: Vec<Event>) {
        self.time += 0.1;
        let input = egui::RawInput {
            time: Some(self.time),
            predicted_dt: 0.1,
            focused: true,
            events,
            ..raw_input(self.size, self.ppp)
        };
        let t = Instant::now();
        let mut out = self.ctx.run_ui(input, |ui| app.show(ui));
        self.tex.apply(&out.textures_delta);
        out.textures_delta.clear();
        self.last_prims = self.ctx.tessellate(out.shapes, out.pixels_per_point);
        self.last_frame = t.elapsed();
    }

    /// Frames until the background workers are idle (textures arrived)
    /// and nothing animates, at most `timeout`.
    pub fn settle(&mut self, app: &mut App, timeout: Duration) {
        let t = Instant::now();
        let mut quiet = 0;
        while t.elapsed() < timeout {
            self.frame(app, Vec::new());
            if app.idle() {
                quiet += 1;
                if quiet >= 4 {
                    break;
                }
            } else {
                quiet = 0;
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        self.frame(app, Vec::new());
        self.frame(app, Vec::new());
    }

    /// The last frame rasterised.
    pub fn capture(&self, bg: egui::Color32) -> Frame {
        let w = (self.size.x * self.ppp).round() as usize;
        let h = (self.size.y * self.ppp).round() as usize;
        let [r, g, b, a] = bg.to_array();
        let mut f = Frame::new(
            w,
            h,
            [
                f32::from(r) / 255.0,
                f32::from(g) / 255.0,
                f32::from(b) / 255.0,
                f32::from(a) / 255.0,
            ],
        );
        f.draw(&self.last_prims, self.ppp, &self.tex);
        f
    }

    pub fn move_to(&mut self, app: &mut App, p: Pos2) {
        self.pointer = p;
        self.frame(app, vec![Event::PointerMoved(p)]);
    }

    fn button(&mut self, app: &mut App, p: Pos2, pressed: bool, button: egui::PointerButton) {
        self.frame(
            app,
            vec![Event::PointerButton {
                pos: p,
                button,
                pressed,
                modifiers: self.modifiers,
            }],
        );
    }

    pub fn click(&mut self, app: &mut App, p: Pos2, button: egui::PointerButton) {
        self.move_to(app, p);
        self.button(app, p, true, button);
        self.button(app, p, false, button);
        self.frame(app, Vec::new());
    }

    pub fn double_click(&mut self, app: &mut App, p: Pos2) {
        self.move_to(app, p);
        for _ in 0..2 {
            self.button(app, p, true, egui::PointerButton::Primary);
            self.button(app, p, false, egui::PointerButton::Primary);
            self.time -= 0.09;
        }
        self.frame(app, Vec::new());
    }

    pub fn drag(&mut self, app: &mut App, a: Pos2, b: Pos2, release: bool) {
        self.move_to(app, a);
        self.button(app, a, true, egui::PointerButton::Primary);
        for k in 1..=10 {
            let t = k as f32 / 10.0;
            self.move_to(app, a + (b - a) * t);
        }
        if release {
            self.button(app, b, false, egui::PointerButton::Primary);
        }
        self.frame(app, Vec::new());
    }

    pub fn key(&mut self, app: &mut App, key: egui::Key, modifiers: Modifiers) {
        let ev = |pressed| Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        };
        let held = self.modifiers;
        self.frame(app, vec![Event::ModifiersChanged(modifiers), ev(true)]);
        self.frame(app, vec![ev(false), Event::ModifiersChanged(held)]);
    }

    pub fn text(&mut self, app: &mut App, t: &str) {
        self.frame(app, vec![Event::Text(t.to_string())]);
    }
}

/// One step of a screenshot script (JSON array of objects).
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Move(Pos2),
    Click(Pos2),
    RightClick(Pos2),
    DoubleClick(Pos2),
    /// Drag from, to; `hold` keeps the button down (for a shot mid-drag).
    Drag(Pos2, Pos2, bool),
    /// Primary button down at a point; with `Move` steps and `Release`
    /// this is a drag in as many frames as the script wants to record.
    Press(Pos2),
    /// Primary button up at a point.
    Release(Pos2),
    Key(egui::Key, Modifiers),
    Text(String),
    /// An engine command.
    Cmd(String, serde_json::Value),
    /// A UI action by name (see [`App::run_named`]).
    Ui(String),
    Wait(u64),
    Settle,
    Shot(PathBuf),
}

fn p2(v: &serde_json::Value) -> Option<Pos2> {
    let a = v.as_array()?;
    Some(Pos2::new(
        a.first()?.as_f64()? as f32,
        a.get(1)?.as_f64()? as f32,
    ))
}

fn mods(s: &str) -> Modifiers {
    let mut m = Modifiers::NONE;
    for part in s.split('+') {
        match part.trim().to_ascii_lowercase().as_str() {
            "ctrl" | "cmd" => {
                m.ctrl = true;
                m.command = true;
            }
            "shift" => m.shift = true,
            "alt" => m.alt = true,
            _ => {}
        }
    }
    m
}

/// Parses a script; unknown steps are errors.
pub fn parse_script(text: &str, base: &Path) -> Result<Vec<Step>, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let arr = v.as_array().ok_or("script must be a JSON array")?;
    let mut out = Vec::new();
    for (i, s) in arr.iter().enumerate() {
        let bad = || format!("step {}: cannot read {s}", i + 1);
        let o = s.as_object().ok_or_else(bad)?;
        let step = if let Some(p) = o.get("move") {
            Step::Move(p2(p).ok_or_else(bad)?)
        } else if let Some(p) = o.get("click") {
            Step::Click(p2(p).ok_or_else(bad)?)
        } else if let Some(p) = o.get("right_click") {
            Step::RightClick(p2(p).ok_or_else(bad)?)
        } else if let Some(p) = o.get("double_click") {
            Step::DoubleClick(p2(p).ok_or_else(bad)?)
        } else if let Some(d) = o.get("drag") {
            let a = d.as_array().ok_or_else(bad)?;
            Step::Drag(
                p2(a.first().ok_or_else(bad)?).ok_or_else(bad)?,
                p2(a.get(1).ok_or_else(bad)?).ok_or_else(bad)?,
                o.get("hold").and_then(|h| h.as_bool()).unwrap_or(false),
            )
        } else if let Some(p) = o.get("press") {
            Step::Press(p2(p).ok_or_else(bad)?)
        } else if let Some(p) = o.get("release") {
            Step::Release(p2(p).ok_or_else(bad)?)
        } else if let Some(k) = o.get("key") {
            let name = k.as_str().ok_or_else(bad)?;
            let key = egui::Key::from_name(name).ok_or_else(bad)?;
            Step::Key(
                key,
                mods(o.get("mods").and_then(|m| m.as_str()).unwrap_or("")),
            )
        } else if let Some(t) = o.get("text") {
            Step::Text(t.as_str().ok_or_else(bad)?.to_string())
        } else if let Some(c) = o.get("cmd") {
            Step::Cmd(
                c.as_str().ok_or_else(bad)?.to_string(),
                o.get("args").cloned().unwrap_or(serde_json::json!({})),
            )
        } else if let Some(u) = o.get("ui") {
            Step::Ui(u.as_str().ok_or_else(bad)?.to_string())
        } else if let Some(w) = o.get("wait") {
            Step::Wait(w.as_u64().ok_or_else(bad)?)
        } else if o.contains_key("settle") {
            Step::Settle
        } else if let Some(p) = o.get("shot") {
            Step::Shot(base.join(p.as_str().ok_or_else(bad)?))
        } else {
            return Err(bad());
        };
        out.push(step);
    }
    Ok(out)
}

/// Runs a script; `Shot` steps write PNGs. Returns the files written.
pub fn run_script(app: &mut App, sh: &mut Shooter, steps: &[Step]) -> Result<Vec<PathBuf>, String> {
    let mut written = Vec::new();
    for s in steps {
        match s {
            Step::Move(p) => sh.move_to(app, *p),
            Step::Click(p) => sh.click(app, *p, egui::PointerButton::Primary),
            Step::RightClick(p) => sh.click(app, *p, egui::PointerButton::Secondary),
            Step::DoubleClick(p) => sh.double_click(app, *p),
            Step::Drag(a, b, hold) => sh.drag(app, *a, *b, !hold),
            Step::Press(p) => {
                sh.move_to(app, *p);
                sh.button(app, *p, true, egui::PointerButton::Primary);
            }
            Step::Release(p) => {
                sh.move_to(app, *p);
                sh.button(app, *p, false, egui::PointerButton::Primary);
                sh.frame(app, Vec::new());
            }
            Step::Key(k, m) => sh.key(app, *k, *m),
            Step::Text(t) => sh.text(app, t),
            Step::Cmd(c, a) => {
                app.cmd(c, a.clone());
                sh.frame(app, Vec::new());
            }
            Step::Ui(name) => {
                if !app.run_named(name) {
                    return Err(format!("unknown UI action {name:?}"));
                }
                sh.frame(app, Vec::new());
            }
            Step::Wait(ms) => {
                let t = Instant::now();
                while t.elapsed() < Duration::from_millis(*ms) {
                    sh.frame(app, Vec::new());
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            Step::Settle => sh.settle(app, Duration::from_secs(30)),
            Step::Shot(path) => {
                sh.frame(app, Vec::new());
                let f = sh.capture(egui::Color32::BLACK);
                std::fs::write(path, f.png()?).map_err(|e| format!("{}: {e}", path.display()))?;
                written.push(path.clone());
            }
        }
    }
    Ok(written)
}

/// The icon gallery: every icon at 16 and 32 px with its name, light and
/// dark theme side by side.
pub fn gallery(ppp: f32) -> Result<Frame, String> {
    use crate::icons::{self, Icon};
    use crate::theme::{self, Tokens};
    let ctx = egui::Context::default();
    crate::app::install_fonts(&ctx);
    let cols = 6usize;
    let cell = Vec2::new(150.0, 44.0);
    let rows = Icon::ALL.len().div_ceil(cols);
    let half = Vec2::new(cell.x * cols as f32 + 20.0, cell.y * rows as f32 + 40.0);
    let size = Vec2::new(half.x * 2.0, half.y);
    let mut tex = TexStore::default();
    let mut prims = Vec::new();
    for pass in 0..2 {
        let mut out = ctx.run_ui(raw_input(size, ppp), |ui| {
            for (k, dark) in [false, true].into_iter().enumerate() {
                let t = Tokens::for_dark(dark);
                theme::apply(ui.ctx(), t);
                let o = Pos2::new(half.x * k as f32, 0.0);
                let p = ui.painter();
                p.rect_filled(Rect::from_min_size(o, half), 0.0, t.panel);
                p.text(
                    o + Vec2::new(10.0, 10.0),
                    egui::Align2::LEFT_TOP,
                    if dark { "dark" } else { "light" },
                    theme::text::heading(),
                    t.text,
                );
                for (i, icon) in Icon::ALL.iter().enumerate() {
                    let c = o + Vec2::new(
                        10.0 + cell.x * (i % cols) as f32,
                        34.0 + cell.y * (i / cols) as f32,
                    );
                    icons::paint(p, Rect::from_min_size(c, Vec2::splat(16.0)), *icon, t.icon);
                    icons::paint(
                        p,
                        Rect::from_min_size(c + Vec2::new(22.0, 0.0), Vec2::splat(32.0)),
                        *icon,
                        t.icon,
                    );
                    p.text(
                        c + Vec2::new(60.0, 8.0),
                        egui::Align2::LEFT_CENTER,
                        icon.name(),
                        theme::text::small(),
                        t.text_secondary,
                    );
                }
            }
        });
        tex.apply(&out.textures_delta);
        out.textures_delta.clear();
        if pass == 1 {
            prims = ctx.tessellate(out.shapes, out.pixels_per_point);
        }
    }
    let w = (size.x * ppp).round() as usize;
    let h = (size.y * ppp).round() as usize;
    let mut f = Frame::new(w, h, [1.0, 1.0, 1.0, 1.0]);
    f.draw(&prims, ppp, &tex);
    Ok(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rasterises_a_filled_rect_with_shared_edges_once() {
        let ctx = egui::Context::default();
        let mut tex = TexStore::default();
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(40.0, 30.0))),
                ..Default::default()
            },
            |ui| {
                ui.painter().rect_filled(
                    Rect::from_min_max(Pos2::new(10.0, 10.0), Pos2::new(30.0, 20.0)),
                    0.0,
                    egui::Color32::from_rgba_premultiplied(0, 0, 128, 128),
                );
            },
        );
        tex.apply(&out.textures_delta);
        out.textures_delta.clear();
        let prims = ctx.tessellate(out.shapes, 1.0);
        let mut f = Frame::new(40, 30, [1.0, 1.0, 1.0, 1.0]);
        f.draw(&prims, 1.0, &tex);
        let px = f.rgba();
        let at = |x: usize, y: usize| &px[(y * 40 + x) * 4..(y * 40 + x) * 4 + 4];
        // Inside: half-transparent blue over white, no double blending
        // along the quad's diagonal.
        let inside = at(15, 15).to_vec();
        assert_eq!(at(25, 12), inside.as_slice());
        assert_eq!(at(11, 18), inside.as_slice());
        assert!((i32::from(inside[0]) - 127).abs() <= 2, "{inside:?}");
        assert!(inside[2] >= 254);
        assert_eq!(at(5, 5), &[255, 255, 255, 255]);
    }

    #[test]
    fn script_parses() {
        let s = parse_script(
            r#"[{"click":[10,20]},{"drag":[[1,2],[3,4]],"hold":true},{"key":"K","mods":"ctrl"},{"shot":"a.png"},{"settle":true}]"#,
            Path::new("/tmp"),
        )
        .unwrap();
        assert_eq!(s.len(), 5);
        assert!(matches!(s[1], Step::Drag(_, _, true)));
        assert!(matches!(&s[2], Step::Key(egui::Key::K, m) if m.ctrl));
        assert!(parse_script(r#"[{"nope":1}]"#, Path::new(".")).is_err());
    }

    #[test]
    fn press_and_release_parse() {
        let s = parse_script(
            r#"[{"press":[10,20]},{"move":[30,40]},{"release":[30,40]}]"#,
            Path::new("."),
        )
        .unwrap();
        assert_eq!(s[0], Step::Press(Pos2::new(10.0, 20.0)));
        assert_eq!(s[2], Step::Release(Pos2::new(30.0, 40.0)));
        assert!(parse_script(r#"[{"press":[10]}]"#, Path::new(".")).is_err());
    }
}
