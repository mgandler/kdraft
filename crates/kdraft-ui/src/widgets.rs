//! The editor's widget kit: number fields with a lead letter or icon,
//! unit suffix, drag-to-scrub, arrow-key nudge and mixed values; segmented
//! controls; dropdowns; icon buttons with tooltip and shortcut; section
//! headers; disclosure; text fields with elision; list-drag insertion
//! line and ghost; inline rename; chips and canvas pills.
//!
//! The set of widgets and their interaction model follow FigCraft's
//! `crates/ui-egui/src/widgets.rs` (MIT OR Apache-2.0, see NOTICE); the
//! code is written for kdraft. Arithmetic (`parse_expr`, `scrubbed`,
//! `nudged`) is plain and unit-tested.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Id, Pos2, Rect, Response, Sense,
    Stroke, StrokeKind, Ui, Vec2,
};

use crate::icons::{self, Icon};
use crate::theme::{text, Tokens};

/// Height of fields, buttons and rows.
pub const ROW: f32 = 24.0;
/// Icon size.
pub const ICON: f32 = 16.0;

// ------------------------------------------------------------ arithmetic

/// Evaluates `+ - * /`, parentheses, unary minus and decimal numbers
/// (comma accepted as decimal point). `None` on anything else.
pub fn parse_expr(s: &str) -> Option<f64> {
    struct Pz<'a> {
        b: &'a [u8],
        i: usize,
    }
    impl Pz<'_> {
        fn ws(&mut self) {
            while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
        }
        fn peek(&mut self) -> Option<u8> {
            self.ws();
            self.b.get(self.i).copied()
        }
        fn expr(&mut self, depth: u32) -> Option<f64> {
            if depth > 32 {
                return None;
            }
            let mut v = self.term(depth)?;
            while let Some(c @ (b'+' | b'-')) = self.peek() {
                self.i += 1;
                let r = self.term(depth)?;
                v = if c == b'+' { v + r } else { v - r };
            }
            Some(v)
        }
        fn term(&mut self, depth: u32) -> Option<f64> {
            let mut v = self.factor(depth)?;
            while let Some(c @ (b'*' | b'/' | b'x')) = self.peek() {
                self.i += 1;
                let r = self.factor(depth)?;
                v = if c == b'/' { v / r } else { v * r };
            }
            Some(v)
        }
        fn factor(&mut self, depth: u32) -> Option<f64> {
            match self.peek()? {
                b'-' => {
                    self.i += 1;
                    Some(-self.factor(depth + 1)?)
                }
                b'+' => {
                    self.i += 1;
                    self.factor(depth + 1)
                }
                b'(' => {
                    self.i += 1;
                    let v = self.expr(depth + 1)?;
                    (self.peek()? == b')').then(|| self.i += 1)?;
                    Some(v)
                }
                _ => {
                    let start = self.i;
                    while self.i < self.b.len()
                        && (self.b[self.i].is_ascii_digit()
                            || self.b[self.i] == b'.'
                            || self.b[self.i] == b',')
                    {
                        self.i += 1;
                    }
                    let t = std::str::from_utf8(&self.b[start..self.i])
                        .ok()?
                        .replace(',', ".");
                    t.parse().ok()
                }
            }
        }
    }
    let t = s.trim().trim_end_matches("mm").trim_end_matches('°');
    let mut p = Pz {
        b: t.as_bytes(),
        i: 0,
    };
    let v = p.expr(0)?;
    p.ws();
    (p.i == p.b.len() && v.is_finite()).then_some(v)
}

/// Value after scrubbing `dx` pixels from `start`: `speed` units per
/// pixel, ten times with Shift, rounded to `quantum` (0: no rounding).
pub fn scrubbed(start: f64, dx: f64, speed: f64, shift: bool, quantum: f64) -> f64 {
    let v = start + dx * speed * if shift { 10.0 } else { 1.0 };
    if quantum > 0.0 {
        (v / quantum).round() * quantum
    } else {
        v
    }
}

/// Value after one arrow-key nudge (`dir` +1 / -1): `step`, Shift 10x.
pub fn nudged(v: f64, step: f64, shift: bool, dir: f64) -> f64 {
    v + dir * step * if shift { 10.0 } else { 1.0 }
}

/// A number with at most `decimals` decimals, trailing zeros dropped.
pub fn fmt_num(v: f64, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    };
    if s == "-0" {
        "0".into()
    } else {
        s
    }
}

// ------------------------------------------------------------ number field

/// What leads a field: a letter (X, Y, W) or an icon.
#[derive(Clone, Copy, Debug)]
pub enum Lead {
    None,
    Letter(&'static str),
    Icon(Icon),
}

/// A change from a number field. Scrubbing gives `Begin`, `Preview`s and
/// `Commit` (one undo step); typing, nudging give one `Set`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FieldEdit {
    Begin,
    Preview(f64),
    Commit,
    Set(f64),
}

pub struct NumberField {
    pub id: Id,
    pub lead: Lead,
    /// `None` shows "Mixed".
    pub value: Option<f64>,
    pub suffix: &'static str,
    pub width: f32,
    /// Arrow-key step.
    pub step: f64,
    /// Scrub: units per pixel.
    pub speed: f64,
    pub decimals: usize,
    pub min: f64,
    pub max: f64,
    pub tooltip: Option<String>,
    /// Accessible name (tests, screen readers).
    pub name: String,
}

impl NumberField {
    pub fn new(
        id: impl std::hash::Hash + std::fmt::Debug,
        lead: Lead,
        value: Option<f64>,
    ) -> NumberField {
        let name = format!("{id:?}").trim_matches('"').to_string();
        NumberField {
            name,
            id: Id::unique(("nf", id)),
            lead,
            value,
            suffix: "",
            width: 96.0,
            step: 0.5,
            speed: 0.1,
            decimals: 2,
            min: f64::NEG_INFINITY,
            max: f64::INFINITY,
            tooltip: None,
        }
    }
    pub fn suffix(mut self, s: &'static str) -> Self {
        self.suffix = s;
        self
    }
    pub fn width(mut self, w: f32) -> Self {
        self.width = w;
        self
    }
    pub fn step(mut self, step: f64, speed: f64) -> Self {
        self.step = step;
        self.speed = speed;
        self
    }
    pub fn decimals(mut self, d: usize) -> Self {
        self.decimals = d;
        self
    }
    pub fn range(mut self, min: f64, max: f64) -> Self {
        self.min = min;
        self.max = max;
        self
    }
    pub fn tooltip(mut self, t: impl Into<String>) -> Self {
        self.tooltip = Some(t.into());
        self
    }

    fn clamp(&self, v: f64) -> f64 {
        v.clamp(self.min, self.max)
    }

    pub fn show(self, ui: &mut Ui) -> (Response, Option<FieldEdit>) {
        let t = Tokens::get(ui.ctx());
        let (rect, _) = ui.allocate_exact_size(Vec2::new(self.width, ROW), Sense::hover());
        let lead_w = match self.lead {
            Lead::None => 6.0,
            _ => 20.0,
        };
        let lead_rect = Rect::from_min_size(rect.min, Vec2::new(lead_w, ROW));
        let body = Rect::from_min_max(Pos2::new(rect.min.x + lead_w, rect.min.y), rect.max);
        let edit_id = self.id.with("text");
        let scrub_id = self.id.with("scrub");
        let editing: Option<String> = ui.data(|d| d.get_temp(edit_id));
        let mut out = None;

        // Lead: drag to scrub.
        let lead = ui.interact(lead_rect, scrub_id, Sense::drag());
        let body_r = ui.interact(body, self.id, Sense::click());
        let hovered = lead.hovered() || body_r.hovered();
        let focused = editing.is_some();
        let painter = ui.painter_at(rect.expand(1.0));
        painter.rect(
            rect,
            CornerRadius::same(4),
            t.field,
            Stroke::new(
                1.0,
                if focused {
                    t.accent
                } else if hovered {
                    t.border_strong
                } else {
                    t.field_border
                },
            ),
            StrokeKind::Inside,
        );
        match self.lead {
            Lead::Letter(l) => {
                painter.text(
                    lead_rect.center() + Vec2::new(1.0, 0.0),
                    Align2::CENTER_CENTER,
                    l,
                    text::small(),
                    t.text_tertiary,
                );
            }
            Lead::Icon(i) => icons::paint(
                &painter,
                Rect::from_center_size(lead_rect.center() + Vec2::new(1.0, 0.0), Vec2::splat(14.0)),
                i,
                t.icon_secondary,
            ),
            Lead::None => {}
        }
        if lead.hovered() || lead.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
        if let Some(v) = self.value {
            if lead.drag_started() {
                ui.data_mut(|d| d.insert_temp(scrub_id, v));
                out = Some(FieldEdit::Begin);
            }
            if lead.dragged() {
                let start: f64 = ui.data(|d| d.get_temp(scrub_id)).unwrap_or(v);
                let total = lead.total_drag_delta().map_or(0.0, |d| f64::from(d.x));
                let shift = ui.input(|i| i.modifiers.shift);
                let q = 10f64.powi(-(self.decimals as i32));
                let nv = self.clamp(scrubbed(start, total, self.speed, shift, q));
                if out.is_none() {
                    out = Some(FieldEdit::Preview(nv));
                }
            }
            if lead.drag_stopped() {
                out = Some(FieldEdit::Commit);
            }
        }

        // Body: click to type.
        let shown = match self.value {
            Some(v) => fmt_num(v, self.decimals),
            None => crate::strings::MIXED.to_string(),
        };
        if body_r.clicked() && editing.is_none() {
            let s = if self.value.is_some() {
                shown.clone()
            } else {
                String::new()
            };
            ui.data_mut(|d| d.insert_temp(edit_id, s));
            ui.memory_mut(|m| m.request_focus(edit_id.with("te")));
        }
        let suffix_w = if self.suffix.is_empty() { 4.0 } else { 26.0 };
        let text_rect = Rect::from_min_max(body.min, Pos2::new(body.max.x - suffix_w, body.max.y));
        if !self.suffix.is_empty() {
            painter.text(
                Pos2::new(rect.max.x - 6.0, rect.center().y),
                Align2::RIGHT_CENTER,
                self.suffix,
                text::small(),
                t.text_tertiary,
            );
        }
        if let Some(mut s) = editing {
            let te = egui::TextEdit::singleline(&mut s)
                .id(edit_id.with("te"))
                .frame(egui::Frame::NONE)
                .font(text::body())
                .margin(Vec2::new(0.0, 4.0))
                .desired_width(text_rect.width());
            let r = ui.put(text_rect, te);
            let (enter, esc, up, down, shift) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::Enter),
                    i.key_pressed(egui::Key::Escape),
                    i.key_pressed(egui::Key::ArrowUp),
                    i.key_pressed(egui::Key::ArrowDown),
                    i.modifiers.shift,
                )
            });
            if r.has_focus() && (up || down) {
                let base = parse_expr(&s).or(self.value).unwrap_or(0.0);
                let nv = self.clamp(nudged(base, self.step, shift, if up { 1.0 } else { -1.0 }));
                s = fmt_num(nv, self.decimals.max(3));
                out = Some(FieldEdit::Set(nv));
            }
            if esc {
                ui.data_mut(|d| d.remove::<String>(edit_id));
            } else if r.lost_focus() || enter {
                ui.data_mut(|d| d.remove::<String>(edit_id));
                if let Some(v) = parse_expr(&s) {
                    let v = self.clamp(v);
                    if self.value.is_none_or(|old| (old - v).abs() > 1e-12) {
                        out = Some(FieldEdit::Set(v));
                    }
                }
            } else {
                ui.data_mut(|d| d.insert_temp(edit_id, s));
            }
        } else {
            painter.text(
                Pos2::new(text_rect.min.x, rect.center().y),
                Align2::LEFT_CENTER,
                shown,
                text::body(),
                if self.value.is_some() {
                    t.text
                } else {
                    t.text_tertiary
                },
            );
        }
        let name = self.name.clone();
        body_r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::SpinButton, true, &name));
        let mut resp = body_r.union(lead);
        if let Some(tip) = &self.tooltip {
            resp = resp.on_hover_text(tip);
        }
        (resp, out)
    }
}

// ------------------------------------------------------------ buttons

/// Tooltip text "Title (Shortcut)".
pub fn tip_text(title: &str, shortcut: Option<&str>) -> String {
    match shortcut {
        Some(s) if !s.is_empty() => format!("{title}  ({s})"),
        _ => title.to_string(),
    }
}

/// A square icon button; `active` draws it in the accent colour (active
/// tool, toggled option).
pub fn icon_button(
    ui: &mut Ui,
    icon: Icon,
    tip: &str,
    shortcut: Option<&str>,
    enabled: bool,
    active: bool,
) -> Response {
    icon_button_sized(ui, icon, tip, shortcut, enabled, active, ROW)
}

pub fn icon_button_sized(
    ui: &mut Ui,
    icon: Icon,
    tip: &str,
    shortcut: Option<&str>,
    enabled: bool,
    active: bool,
    size: f32,
) -> Response {
    let t = Tokens::get(ui.ctx());
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, r) = ui.allocate_exact_size(Vec2::splat(size), sense);
    let r = r.on_hover_text(tip_text(tip, shortcut));
    // AccessKit label for tests and screen readers.
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, tip));
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let bg = if active {
            Some(t.accent)
        } else if enabled && r.is_pointer_button_down_on() {
            Some(t.pressed)
        } else if enabled && r.hovered() {
            Some(t.hover)
        } else {
            None
        };
        if let Some(bg) = bg {
            p.rect_filled(rect, CornerRadius::same(4), bg);
        }
        let c = if active {
            t.on_accent
        } else if enabled {
            t.icon
        } else {
            t.text_tertiary.gamma_multiply(0.7)
        };
        icons::paint(
            p,
            Rect::from_center_size(rect.center(), Vec2::splat(ICON)),
            icon,
            c,
        );
    }
    r
}

/// A text button with an optional leading icon; `primary` fills it with
/// the accent colour.
pub fn button(
    ui: &mut Ui,
    icon: Option<Icon>,
    label: &str,
    primary: bool,
    enabled: bool,
) -> Response {
    let t = Tokens::get(ui.ctx());
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), text::body(), t.text);
    let w = galley.size().x + 20.0 + if icon.is_some() { 20.0 } else { 0.0 };
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, r) = ui.allocate_exact_size(Vec2::new(w, ROW + 4.0), sense);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    let p = ui.painter();
    let (bg, fg, stroke) = if primary {
        (
            if r.hovered() && enabled {
                t.accent_hover
            } else {
                t.accent
            },
            t.on_accent,
            Stroke::NONE,
        )
    } else {
        (
            if r.hovered() && enabled {
                t.hover
            } else {
                t.field
            },
            if enabled { t.text } else { t.text_tertiary },
            Stroke::new(1.0, t.field_border),
        )
    };
    p.rect(rect, CornerRadius::same(5), bg, stroke, StrokeKind::Inside);
    let mut x = rect.min.x + 10.0;
    if let Some(i) = icon {
        icons::paint(
            p,
            Rect::from_min_size(Pos2::new(x, rect.center().y - 8.0), Vec2::splat(16.0)),
            i,
            fg,
        );
        x += 20.0;
    }
    p.galley_with_override_text_color(
        Pos2::new(x, rect.center().y - galley.size().y / 2.0),
        galley,
        fg,
    );
    r
}

/// One cell of a segmented control.
#[derive(Clone, Copy, Debug)]
pub struct Seg<'a> {
    pub icon: Option<Icon>,
    pub label: &'a str,
    pub tip: &'a str,
}

impl<'a> Seg<'a> {
    pub fn text(label: &'a str) -> Seg<'a> {
        Seg {
            icon: None,
            label,
            tip: label,
        }
    }
    pub fn icon(icon: Icon, tip: &'a str) -> Seg<'a> {
        Seg {
            icon: Some(icon),
            label: "",
            tip,
        }
    }
}

/// A segmented control; returns the clicked index.
pub fn segmented(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    items: &[Seg<'_>],
    active: Option<usize>,
    width: f32,
) -> Option<usize> {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, ROW), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(5), t.hover);
    let n = items.len().max(1) as f32;
    let cw = rect.width() / n;
    let mut clicked = None;
    for (k, it) in items.iter().enumerate() {
        let r = Rect::from_min_size(
            Pos2::new(rect.min.x + cw * k as f32, rect.min.y),
            Vec2::new(cw, ROW),
        );
        let resp = ui.interact(r, Id::unique(("seg", &id, k)), Sense::click());
        let resp = if it.tip.is_empty() {
            resp
        } else {
            resp.on_hover_text(it.tip)
        };
        let label = if it.label.is_empty() {
            it.tip
        } else {
            it.label
        };
        resp.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::Button, true, active == Some(k), label)
        });
        let on = active == Some(k);
        if on {
            ui.painter().rect(
                r.shrink(2.0),
                CornerRadius::same(4),
                t.field,
                Stroke::new(1.0, t.border_strong),
                StrokeKind::Inside,
            );
        } else if resp.hovered() {
            ui.painter()
                .rect_filled(r.shrink(2.0), CornerRadius::same(4), t.pressed);
        }
        let fg = if on { t.text } else { t.text_secondary };
        if let Some(i) = it.icon {
            icons::paint(
                ui.painter(),
                Rect::from_center_size(r.center(), Vec2::splat(ICON)),
                i,
                fg,
            );
        } else {
            ui.painter().text(
                r.center(),
                Align2::CENTER_CENTER,
                it.label,
                text::small(),
                fg,
            );
        }
        if resp.clicked() {
            clicked = Some(k);
        }
    }
    clicked
}

/// A dropdown over `options` (labels); returns the chosen index.
pub fn dropdown(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    current: &str,
    options: &[&str],
    width: f32,
) -> Option<usize> {
    let mut chosen = None;
    egui::ComboBox::from_id_salt(id)
        .selected_text(current)
        .width(width)
        .height(320.0)
        .show_ui(ui, |ui| {
            for (k, o) in options.iter().enumerate() {
                if ui.selectable_label(*o == current, *o).clicked() {
                    chosen = Some(k);
                }
            }
        });
    chosen
}

/// A labelled row: a fixed-width label column, then `add`.
pub fn row<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::new(LABEL_W, ROW), Sense::hover());
        let s = elide(ui, label, text::small(), LABEL_W - 4.0);
        ui.painter().text(
            Pos2::new(r.min.x, r.center().y),
            Align2::LEFT_CENTER,
            s,
            text::small(),
            t.text_secondary,
        );
        add(ui)
    })
    .inner
}

/// Width of the label column in the inspector.
pub const LABEL_W: f32 = 74.0;

/// A section header with a disclosure chevron and action icons; returns
/// (clicked action index, header toggled).
pub fn section_header(
    ui: &mut Ui,
    title: &str,
    open: Option<bool>,
    actions: &[(Icon, &str)],
) -> (Option<usize>, bool) {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    let (rect, r) = ui.allocate_exact_size(Vec2::new(w, 28.0), Sense::click());
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, title));
    let mut x = rect.min.x;
    if let Some(o) = open {
        icons::paint(
            ui.painter(),
            Rect::from_center_size(Pos2::new(x + 7.0, rect.center().y), Vec2::splat(14.0)),
            if o {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            },
            t.icon_secondary,
        );
        x += 16.0;
    }
    ui.painter().text(
        Pos2::new(x, rect.center().y),
        Align2::LEFT_CENTER,
        title,
        text::heading(),
        t.text,
    );
    let mut hit = None;
    let mut ax = rect.max.x;
    for (k, (icon, tip)) in actions.iter().enumerate().rev() {
        let br = Rect::from_min_size(
            Pos2::new(ax - 22.0, rect.center().y - 11.0),
            Vec2::splat(22.0),
        );
        ax -= 24.0;
        let b = ui
            .interact(br, Id::unique(("sect-act", title, k)), Sense::click())
            .on_hover_text(*tip);
        b.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, *tip));
        if b.hovered() {
            ui.painter().rect_filled(br, CornerRadius::same(4), t.hover);
        }
        icons::paint(
            ui.painter(),
            Rect::from_center_size(br.center(), Vec2::splat(ICON)),
            *icon,
            t.icon_secondary,
        );
        if b.clicked() {
            hit = Some(k);
        }
    }
    let toggled = open.is_some() && hit.is_none() && r.clicked();
    (hit, toggled)
}

/// A thin separator line across the available width.
pub fn separator(ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(Vec2::new(w, 1.0), Sense::hover());
    ui.painter()
        .hline(r.x_range(), r.center().y, Stroke::new(1.0, t.border));
}

/// A disclosure row ("Show all 42 layers"); returns true when clicked.
pub fn disclosure(ui: &mut Ui, label: &str, open: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), text::small(), t.text_secondary);
    let (rect, r) = ui.allocate_exact_size(Vec2::new(galley.size().x + 20.0, 20.0), Sense::click());
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            Pos2::new(rect.min.x + 7.0, rect.center().y),
            Vec2::splat(12.0),
        ),
        if open {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        },
        t.icon_secondary,
    );
    let c = if r.hovered() {
        t.text
    } else {
        t.text_secondary
    };
    ui.painter().galley_with_override_text_color(
        Pos2::new(rect.min.x + 16.0, rect.center().y - galley.size().y / 2.0),
        galley,
        c,
    );
    r.clicked()
}

/// `s` shortened with an ellipsis to fit `max_w`.
pub fn elide(ui: &Ui, s: &str, font: FontId, max_w: f32) -> String {
    let fits = |x: &str| {
        ui.fonts_mut(|f| {
            f.layout_no_wrap(x.to_string(), font.clone(), Color32::WHITE)
                .size()
                .x
        }) <= max_w
    };
    if fits(s) {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let cand: String = chars[..mid].iter().collect::<String>() + "…";
        if fits(&cand) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars[..lo].iter().collect::<String>() + "…"
}

/// [`elide`] with a painter's fonts.
pub fn elide_painter(p: &egui::Painter, s: &str, font: FontId, max_w: f32) -> String {
    let fits = |x: &str| {
        p.layout_no_wrap(x.to_string(), font.clone(), Color32::WHITE)
            .size()
            .x
            <= max_w
    };
    if fits(s) {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let cand: String = chars[..mid].iter().collect::<String>() + "…";
        if fits(&cand) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars[..lo].iter().collect::<String>() + "…"
}

/// A single-line text field that commits on Enter or focus loss; shows
/// the value elided when not focused. Returns the committed text.
pub fn text_field(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: &str,
    width: f32,
    hint: &str,
) -> Option<String> {
    text_field_named(ui, id, value, width, hint, None)
}

/// [`text_field`] with an accessible name (for screen readers and tests).
pub fn text_field_named(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: &str,
    width: f32,
    hint: &str,
    name: Option<&str>,
) -> Option<String> {
    let id = Id::unique(("tf", id));
    let mut s: String = ui
        .data(|d| d.get_temp(id))
        .unwrap_or_else(|| value.to_string());
    let r = ui.add(
        egui::TextEdit::singleline(&mut s)
            .id(id.with("te"))
            .hint_text(hint)
            .desired_width(width)
            .font(text::body())
            .margin(Vec2::new(6.0, 4.0)),
    );
    if let Some(n) = name {
        let v = s.clone();
        r.widget_info(|| {
            let mut i = egui::WidgetInfo::labeled(egui::Role::TextInput, true, n);
            i.current_text_value = Some(v.clone());
            i
        });
    }
    let mut out = None;
    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, s.clone()));
    }
    let (enter, esc) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    if r.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(id));
        if !esc && s != value {
            out = Some(s);
        }
    } else if enter && r.has_focus() && s != value {
        out = Some(s);
    }
    out
}

/// A multi-line text field: Enter starts a new line, the value is
/// returned when the field loses focus (Escape reverts).
pub fn text_area(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: &str,
    width: f32,
    hint: &str,
) -> Option<String> {
    let id = Id::unique(("ta", id));
    let mut s: String = ui
        .data(|d| d.get_temp(id))
        .unwrap_or_else(|| value.to_string());
    let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
    let r = ui.add(
        egui::TextEdit::multiline(&mut s)
            .id(id.with("te"))
            .hint_text(hint)
            .desired_width(width)
            .desired_rows(2)
            .font(text::body())
            .margin(Vec2::new(6.0, 4.0)),
    );
    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, s.clone()));
    }
    if r.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(id));
        if !esc && s != value {
            return Some(s);
        }
    }
    None
}

/// What an inline rename did this frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Rename {
    Editing,
    Commit(String),
    Cancel,
}

/// An inline rename field over `rect`; `start` focuses it and selects
/// all on the first frame.
pub fn rename_field(ui: &mut Ui, id: Id, rect: Rect, initial: &str, start: bool) -> Rename {
    let t = Tokens::get(ui.ctx());
    let buf_id = id.with("buf");
    let mut s: String = ui
        .data(|d| d.get_temp(buf_id))
        .unwrap_or_else(|| initial.to_string());
    ui.painter().rect(
        rect,
        CornerRadius::same(3),
        t.field,
        Stroke::new(1.0, t.accent),
        StrokeKind::Inside,
    );
    let te_id = id.with("te");
    let mut out = egui::TextEdit::singleline(&mut s)
        .id(te_id)
        .frame(egui::Frame::NONE)
        .font(text::body())
        .margin(Vec2::new(4.0, 3.0))
        .desired_width(rect.width() - 8.0)
        .show(&mut ui.new_child(egui::UiBuilder::new().max_rect(rect)));
    if start {
        out.response.request_focus();
        let all = egui::text::CCursorRange::two(
            egui::text::CCursor::new(0),
            egui::text::CCursor::new(s.chars().count()),
        );
        out.state.cursor.set_char_range(Some(all));
        out.state.store(ui.ctx(), te_id);
    }
    let (enter, esc) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    if esc {
        ui.data_mut(|d| d.remove::<String>(buf_id));
        return Rename::Cancel;
    }
    if enter || (out.response.lost_focus() && !start) {
        ui.data_mut(|d| d.remove::<String>(buf_id));
        return Rename::Commit(s);
    }
    ui.data_mut(|d| d.insert_temp(buf_id, s));
    Rename::Editing
}

/// The insertion marker of a list drag.
pub fn insertion_line(painter: &egui::Painter, x0: f32, x1: f32, y: f32) {
    let t = Tokens::get(painter.ctx());
    painter.hline(x0..=x1, y, Stroke::new(2.0, t.accent));
    painter.circle_stroke(Pos2::new(x0 + 3.0, y), 3.0, Stroke::new(2.0, t.accent));
}

/// A floating label following the pointer during a list drag.
pub fn drag_ghost(ctx: &egui::Context, at: Pos2, label: &str) {
    let t = Tokens::get(ctx);
    let layer = egui::LayerId::new(egui::Order::Tooltip, Id::unique("drag-ghost"));
    let p = ctx.layer_painter(layer);
    let g = p.layout_no_wrap(label.to_string(), text::body(), t.text);
    let r = Rect::from_min_size(at + Vec2::new(14.0, 6.0), g.size() + Vec2::new(16.0, 8.0));
    p.rect(
        r,
        CornerRadius::same(4),
        t.float.gamma_multiply(0.95),
        Stroke::new(1.0, t.accent),
        StrokeKind::Inside,
    );
    p.galley(r.min + Vec2::new(8.0, 4.0), g, t.text);
}

/// A rounded chip (layer presets, filters).
pub fn chip(ui: &mut Ui, label: &str, on: bool, enabled: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let g = ui
        .painter()
        .layout_no_wrap(label.to_string(), text::small(), t.text);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, r) = ui.allocate_exact_size(Vec2::new(g.size().x + 16.0, 22.0), sense);
    r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, on, label));
    let (bg, fg, st) = if on {
        (t.selected_row, t.accent_text, Stroke::new(1.0, t.accent))
    } else if r.hovered() && enabled {
        (t.hover, t.text, Stroke::new(1.0, t.border_strong))
    } else {
        (
            t.panel,
            if enabled {
                t.text_secondary
            } else {
                t.text_tertiary
            },
            Stroke::new(1.0, t.field_border),
        )
    };
    ui.painter()
        .rect(rect, CornerRadius::same(11), bg, st, StrokeKind::Inside);
    ui.painter().galley_with_override_text_color(
        rect.min + Vec2::new(8.0, (22.0 - g.size().y) / 2.0),
        g,
        fg,
    );
    r
}

/// A value pill on the canvas centred at `at`.
pub fn pill(painter: &egui::Painter, at: Pos2, text: &str, bg: Color32, fg: Color32) -> Rect {
    let g = painter.layout_no_wrap(text.to_string(), crate::theme::text::mono(), fg);
    let r = Rect::from_center_size(at, g.size() + Vec2::new(10.0, 4.0));
    painter.rect_filled(r, CornerRadius::same(3), bg);
    painter.galley(r.min + Vec2::new(5.0, 2.0), g, fg);
    r
}

/// A small badge with a count (status bar, issues).
pub fn badge(ui: &mut Ui, text: &str, bg: Color32, fg: Color32) {
    let g = ui
        .painter()
        .layout_no_wrap(text.to_string(), text::small(), fg);
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(g.size().x.max(8.0) + 10.0, 16.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(8), bg);
    ui.painter().galley(rect.center() - g.size() / 2.0, g, fg);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expressions() {
        assert_eq!(parse_expr("12.5"), Some(12.5));
        assert_eq!(parse_expr("12,5"), Some(12.5));
        assert_eq!(parse_expr(" 10 + 2.5*2 "), Some(15.0));
        assert_eq!(parse_expr("(1+2)*3"), Some(9.0));
        assert_eq!(parse_expr("-4 / 2"), Some(-2.0));
        assert_eq!(parse_expr("20mm"), Some(20.0));
        assert_eq!(parse_expr("1/0"), None);
        assert_eq!(parse_expr("abc"), None);
        assert_eq!(parse_expr(""), None);
        assert_eq!(parse_expr("(((("), None);
        let deep = "(".repeat(100) + "1" + &")".repeat(100);
        assert_eq!(parse_expr(&deep), None);
    }

    #[test]
    fn scrub_and_nudge() {
        assert_eq!(scrubbed(10.0, 20.0, 0.1, false, 0.01), 12.0);
        assert_eq!(scrubbed(10.0, 20.0, 0.1, true, 0.01), 30.0);
        assert_eq!(scrubbed(10.0, -3.0, 0.1, false, 0.5), 9.5);
        assert_eq!(nudged(1.0, 0.5, false, 1.0), 1.5);
        assert_eq!(nudged(1.0, 0.5, true, -1.0), -4.0);
    }

    #[test]
    fn number_format() {
        assert_eq!(fmt_num(12.5, 2), "12.5");
        assert_eq!(fmt_num(12.0, 2), "12");
        assert_eq!(fmt_num(-0.0001, 2), "0");
        assert_eq!(fmt_num(1.23456, 3), "1.235");
    }
}
