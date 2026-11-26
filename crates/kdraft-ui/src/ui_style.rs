//! Inspector "Style" section: colours (swatch with a picker of fixed
//! palette, recent colours, hex entry and a full picker), line width and
//! pattern, fill, arrows, text attributes and opacity, for one or many
//! items (mixed values shown as such); copy/paste and reset. Also the
//! document styles window. Everything runs through `item.set_style`,
//! `item.reset_style`, `style.copy`, `style.paste` and
//! `document.set_styles`.

use eframe::egui::{self, Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, Vec2};
use kdraft_doc::style::{ItemStyle, Rgba, StyleKind};
use kdraft_doc::Item;
use serde_json::{json, Value};

use crate::app::App;
use crate::theme::text;
use crate::widgets::{self as w, FieldEdit, Lead, NumberField, Seg};

/// The fixed palette: neutrals, then saturated and muted hues that print
/// well on white paper.
pub const PALETTE: [&str; 20] = [
    "#000000", "#404040", "#808080", "#B0B0B0", "#FFFFFF", "#C81E1E", "#E06C00", "#B8860B",
    "#2E8B57", "#008080", "#1E50C8", "#5A3CB4", "#B4288C", "#8B4513", "#556B2F", "#F4C7C3",
    "#FCE8B2", "#B7E1CD", "#C6DAFC", "#E1D5F0",
];

const SWATCH: f32 = 18.0;

/// Where a style edit goes: the selected items or a document category.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    Items,
    Category(StyleKind),
}

impl Target {
    fn cmd(self) -> &'static str {
        match self {
            Target::Items => "item.set_style",
            Target::Category(_) => "document.set_styles",
        }
    }

    fn args(self, key: &str, v: Value) -> Value {
        let mut m = serde_json::Map::new();
        if let Target::Category(k) = self {
            m.insert("category".into(), json!(k.as_str()));
        }
        m.insert(key.to_string(), v);
        Value::Object(m)
    }
}

/// Text form of `key` shared by all `styles`; `None` when they differ.
pub fn common(styles: &[ItemStyle], key: &str) -> Option<String> {
    let first = styles.first()?.get(key)?;
    styles
        .iter()
        .all(|s| s.get(key).as_deref() == Some(first.as_str()))
        .then_some(first)
}

/// Keys shown for a mix of categories: those all of them have, in the
/// first one's order.
pub fn common_keys(kinds: &[StyleKind]) -> Vec<&'static str> {
    let Some(first) = kinds.first() else {
        return Vec::new();
    };
    first
        .keys()
        .iter()
        .copied()
        .filter(|k| kinds.iter().all(|x| x.keys().contains(k)))
        .collect()
}

fn label(key: &str) -> &'static str {
    match key {
        "stroke" => "Line colour",
        "width" => "Width",
        "pattern" => "Pattern",
        "fill" => "Fill",
        "opacity" => "Opacity",
        "arrows" => "Arrows",
        "arrow_size" => "Arrow size",
        "text_color" => "Text colour",
        "text_height" => "Text height",
        "bold" => "Bold",
        "italic" => "Italic",
        "align" => "Align",
        "line_spacing" => "Spacing",
        "decimals" => "Decimals",
        "header_fill" => "Header fill",
        "shape" => "Shape",
        _ => "Style",
    }
}

fn to32(c: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

fn from32(c: Color32) -> Rgba {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Rgba { r, g, b, a }
}

/// Paints a swatch: the colour over a checkerboard (for transparency),
/// a red diagonal for "none", a question mark for mixed values.
pub fn paint_swatch(p: &egui::Painter, r: Rect, c: Option<Rgba>, border: Color32) {
    let cr = CornerRadius::same(3);
    match c {
        Some(c) if c.is_none() => {
            p.rect_filled(r, cr, Color32::WHITE);
            p.line_segment(
                [r.left_bottom(), r.right_top()],
                Stroke::new(1.5, Color32::from_rgb(200, 30, 30)),
            );
        }
        Some(c) => {
            if c.a < 255 {
                let h = r.width() / 2.0;
                for (i, j) in [(0.0, 0.0), (1.0, 1.0)] {
                    p.rect_filled(
                        Rect::from_min_size(r.min + Vec2::new(i * h, j * h), Vec2::splat(h)),
                        0,
                        Color32::from_gray(200),
                    );
                }
            }
            p.rect_filled(r, cr, to32(c));
        }
        None => {
            p.rect_filled(r, cr, Color32::from_gray(225));
            p.text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                "?",
                text::small(),
                Color32::from_gray(80),
            );
        }
    }
    p.rect_stroke(r, cr, Stroke::new(1.0, border), StrokeKind::Inside);
}

/// A clickable swatch; returns its response.
pub(crate) fn swatch(
    ui: &mut egui::Ui,
    c: Option<Rgba>,
    tip: &str,
    selected: bool,
) -> egui::Response {
    let t = crate::theme::Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(SWATCH), Sense::click());
    let border = if selected || resp.hovered() {
        t.accent
    } else {
        t.border_strong
    };
    paint_swatch(ui.painter(), r, c, border);
    if selected {
        ui.painter().rect_stroke(
            r.expand(2.0),
            CornerRadius::same(4),
            Stroke::new(1.5, t.accent),
            StrokeKind::Outside,
        );
    }
    let label = tip.to_string();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
    resp.on_hover_text(tip)
}

/// What the colour popup did.
pub(crate) enum ColorEdit {
    /// A final choice (palette, recent, hex, none).
    Set(Rgba),
    /// A full-picker change (previewed; committed when the popup closes).
    Live(Rgba),
}

impl App {
    /// The "Style" section for the selected items.
    pub(crate) fn style_section(&mut self, ui: &mut egui::Ui, items: &[Item]) {
        let kinds: Vec<StyleKind> = items.iter().filter_map(StyleKind::of).collect();
        if kinds.is_empty() || kinds.len() != items.len() {
            return;
        }
        let per_item: Vec<Vec<&'static str>> =
            items.iter().map(kdraft_doc::style::keys_for).collect();
        let keys: Vec<&'static str> = common_keys(&kinds)
            .into_iter()
            .filter(|k| per_item.iter().all(|v| v.contains(k)))
            .collect();
        if keys.is_empty() {
            return;
        }
        let st = self.s.doc().styles.clone();
        let styles: Vec<ItemStyle> = items.iter().map(|i| i.effective_style(&st)).collect();
        let overridden = items.iter().any(|i| !i.style_override().is_empty());
        let title = if overridden { "Style (own)" } else { "Style" };
        if !self.section(ui, "style", title) {
            return;
        }
        self.style_rows(ui, Target::Items, &keys, &styles);
        ui.horizontal_wrapped(|ui| {
            if w::button(ui, None, "Reset to document style", false, overridden).clicked() {
                self.cmd("item.reset_style", json!({}));
            }
            if w::button(ui, None, "Copy", false, items.len() == 1).clicked() {
                self.cmd("style.copy", json!({}));
            }
            if w::button(ui, None, "Paste", false, self.s.style_clip.is_some()).clicked() {
                self.cmd("style.paste", json!({}));
            }
        });
        if ui
            .link(egui::RichText::new("Document styles...").font(text::small()))
            .clicked()
        {
            self.show_doc_styles = true;
        }
    }

    /// One row per key for `styles` (one per target item), editing
    /// `target`.
    pub(crate) fn style_rows(
        &mut self,
        ui: &mut egui::Ui,
        target: Target,
        keys: &[&'static str],
        styles: &[ItemStyle],
    ) {
        let salt = format!("{target:?}");
        for &key in keys {
            let v = common(styles, key);
            match key {
                "stroke" | "fill" | "text_color" | "header_fill" => {
                    let c = v.as_deref().and_then(Rgba::parse);
                    w::row(ui, label(key), |ui| {
                        self.color_row(ui, (&salt, key), target, key, c, styles.len() > 1)
                    });
                }
                "width" | "text_height" | "arrow_size" | "line_spacing" | "opacity"
                | "decimals" => {
                    let n = v.as_deref().and_then(|s| s.parse::<f64>().ok());
                    let (suffix, step, lo, hi, dec) = match key {
                        "opacity" => ("%", (5.0, 0.5), 0.0, 100.0, 0),
                        "line_spacing" => ("x", (0.1, 0.01), 0.5, 5.0, 2),
                        "decimals" => ("", (1.0, 0.05), 0.0, 6.0, 0),
                        "text_height" => ("mm", (0.1, 0.01), 0.1, 100.0, 2),
                        _ => ("mm", (0.05, 0.005), 0.01, 20.0, 3),
                    };
                    let shown = if key == "opacity" {
                        n.map(|o| o * 100.0)
                    } else {
                        n
                    };
                    w::row(ui, label(key), |ui| {
                        let (_, edit) = NumberField::new((&salt, key), Lead::None, shown)
                            .suffix(suffix)
                            .step(step.0, step.1)
                            .range(lo, hi)
                            .decimals(dec)
                            .width(118.0)
                            .show(ui);
                        let conv = move |x: f64| -> Value {
                            match key {
                                "opacity" => json!((x / 100.0).clamp(0.0, 1.0)),
                                "decimals" => json!(x.round().clamp(0.0, 6.0) as i64),
                                _ => json!(x),
                            }
                        };
                        self.style_edit(target, key, edit, &conv);
                    });
                }
                "pattern" => {
                    let opts = ["solid", "dashed", "dotted", "dash-dot"];
                    let cur = v.unwrap_or_else(|| "mixed".into());
                    w::row(ui, label(key), |ui| {
                        if let Some(k) = w::dropdown(ui, (&salt, key), &cur, &opts, 104.0) {
                            self.cmd(target.cmd(), target.args(key, json!(opts[k])));
                        }
                    });
                }
                "arrows" | "align" | "shape" => {
                    let opts: &[&str] = match key {
                        "arrows" => &["none", "start", "end", "both"],
                        "shape" => &["circle", "square", "hexagon", "none"],
                        _ => &["left", "center", "right"],
                    };
                    let segs: Vec<Seg<'_>> = opts.iter().map(|o| Seg::text(o)).collect();
                    let active = v.as_deref().and_then(|c| opts.iter().position(|o| *o == c));
                    w::row(ui, label(key), |ui| {
                        if let Some(k) = w::segmented(ui, (&salt, key), &segs, active, 166.0) {
                            self.cmd(target.cmd(), target.args(key, json!(opts[k])));
                        }
                    });
                }
                "bold" | "italic" => {
                    let on = v.as_deref() == Some("yes");
                    let mut b = on;
                    w::row(ui, label(key), |ui| {
                        if ui.checkbox(&mut b, "").changed() {
                            self.cmd(target.cmd(), target.args(key, json!(b)));
                        }
                        if v.is_none() {
                            ui.label(egui::RichText::new("mixed").font(text::small()));
                        }
                    });
                }
                _ => {}
            }
        }
    }

    /// Number edits: scrubbing previews in one gesture, typing sets.
    fn style_edit(
        &mut self,
        target: Target,
        key: &'static str,
        edit: Option<FieldEdit>,
        conv: &dyn Fn(f64) -> Value,
    ) {
        let cmd = target.cmd();
        match edit {
            Some(FieldEdit::Begin) => {
                self.s.begin_gesture(label(key));
                self.prop_drag = Some(cmd);
            }
            Some(FieldEdit::Preview(x)) => {
                if self.s.in_gesture() && self.prop_drag == Some(cmd) {
                    if let Err(e) = self
                        .reg
                        .preview(&mut self.s, cmd, target.args(key, conv(x)))
                    {
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
            Some(FieldEdit::Set(x)) => {
                self.cmd(cmd, target.args(key, conv(x)));
            }
            None => {}
        }
    }

    /// Swatch, hex text and the picker popup for a colour attribute.
    fn color_row(
        &mut self,
        ui: &mut egui::Ui,
        id: (&str, &str),
        target: Target,
        key: &'static str,
        c: Option<Rgba>,
        multi: bool,
    ) {
        let t = self.tokens();
        let resp = swatch(ui, c, &format!("{} colour", label(key)), false);
        let hex = match c {
            Some(c) if c.is_none() => "none".to_string(),
            Some(c) => c.to_string(),
            None if multi => "mixed".to_string(),
            None => "-".to_string(),
        };
        ui.label(
            egui::RichText::new(hex)
                .font(text::small())
                .color(t.text_secondary),
        );
        let allow_none = matches!(key, "fill" | "header_fill");
        let popup = egui::Popup::from_toggle_button_response(&resp)
            .id(egui::Id::unique(("color-popup", id.0, id.1)))
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
        let popup_id = popup.get_id();
        let mut edit = None;
        popup.show(|ui| {
            ui.set_min_width(212.0);
            edit = self.color_popup(ui, (id.0, id.1), c, allow_none);
        });
        let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
        let live_key = (target, key);
        match edit {
            Some(ColorEdit::Set(v)) => {
                if self.color_live == Some(live_key) {
                    self.s.cancel_gesture();
                    self.color_live = None;
                }
                self.cmd(target.cmd(), target.args(key, json!(fill_text(v))));
                self.remember_color(v);
                egui::Popup::close_id(ui.ctx(), popup_id);
            }
            Some(ColorEdit::Live(v)) => {
                if self.color_live != Some(live_key) {
                    if self.s.in_gesture() {
                        self.s.commit_gesture();
                    }
                    self.s.begin_gesture(label(key));
                    self.color_live = Some(live_key);
                }
                let args = target.args(key, json!(fill_text(v)));
                if let Err(e) = self.reg.preview(&mut self.s, target.cmd(), args) {
                    self.error(format!("{}: {e}", target.cmd()));
                }
                self.color_pending = Some(v);
            }
            None => {}
        }
        if !open && self.color_live == Some(live_key) {
            self.s.commit_gesture();
            self.color_live = None;
            if let Some(v) = self.color_pending.take() {
                self.remember_color(v);
            }
        }
    }

    fn remember_color(&mut self, c: Rgba) {
        if c.is_none() {
            return;
        }
        self.settings.add_recent_color(&c.to_string());
        self.save_settings();
    }

    pub(crate) fn color_popup(
        &mut self,
        ui: &mut egui::Ui,
        id: (&str, &str),
        c: Option<Rgba>,
        allow_none: bool,
    ) -> Option<ColorEdit> {
        let t = self.tokens();
        let mut out = None;
        let small = |s: &str| {
            egui::RichText::new(s.to_string())
                .font(text::small())
                .color(t.text_secondary)
        };
        ui.label(small("Palette"));
        for row in PALETTE.chunks(10) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                for hex in row {
                    let p = Rgba::parse(hex);
                    if swatch(ui, p, hex, p == c).clicked() {
                        out = p.map(ColorEdit::Set);
                    }
                }
            });
        }
        if allow_none {
            ui.horizontal(|ui| {
                if swatch(ui, Some(Rgba::NONE), "No fill", c == Some(Rgba::NONE)).clicked() {
                    out = Some(ColorEdit::Set(Rgba::NONE));
                }
                ui.label(small("None"));
            });
        }
        let recent: Vec<Rgba> = self
            .settings
            .recent_colors
            .iter()
            .filter_map(|h| Rgba::parse(h))
            .collect();
        if !recent.is_empty() {
            ui.label(small("Recent"));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                for r in recent {
                    if swatch(ui, Some(r), &r.to_string(), Some(r) == c).clicked() {
                        out = Some(ColorEdit::Set(r));
                    }
                }
            });
        }
        ui.horizontal(|ui| {
            ui.label(small("Hex"));
            let cur = c.map(|c| c.to_string()).unwrap_or_default();
            if let Some(s) = w::text_field(ui, ("hex", id.0, id.1), &cur, 110.0, "#RRGGBB") {
                match Rgba::parse(&s) {
                    Some(v) => out = Some(ColorEdit::Set(v)),
                    None => self.warn(format!("{s:?} is not a colour (use #RRGGBB or #RRGGBBAA)")),
                }
            }
        });
        // Full picker: hue, saturation, value and alpha sliders (egui's
        // own picker draws a clipboard emoji the bundled font lacks).
        let base = c.filter(|c| !c.is_none()).unwrap_or(Rgba::BLACK);
        let mut hsva = egui::ecolor::Hsva::from(to32(base));
        let mut changed = false;
        for (name, v) in [
            ("Hue", &mut hsva.h),
            ("Saturation", &mut hsva.s),
            ("Value", &mut hsva.v),
            ("Alpha", &mut hsva.a),
        ] {
            ui.horizontal(|ui| {
                ui.add_sized([62.0, 18.0], egui::Label::new(small(name)));
                changed |= ui
                    .add(egui::Slider::new(v, 0.0..=1.0).show_value(false))
                    .changed();
            });
        }
        if changed {
            out = Some(ColorEdit::Live(from32(Color32::from(hsva))));
        }
        out
    }

    /// The document styles window: category defaults.
    pub(crate) fn doc_styles_window(&mut self, ctx: &egui::Context) {
        if !self.show_doc_styles {
            return;
        }
        let mut open = true;
        let st = self.s.doc().styles.clone();
        egui::Window::new("Document styles")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(300.0)
            .show(ctx, |ui| {
                let names: Vec<&str> = StyleKind::ALL.iter().map(|k| k.as_str()).collect();
                let k = self.doc_style_kind;
                let segs: Vec<Seg<'_>> = names.iter().map(|n| Seg::text(n)).collect();
                let active = StyleKind::ALL.iter().position(|x| *x == k);
                if let Some(i) = w::segmented(ui, "doc-style-kind", &segs, active, 290.0) {
                    self.doc_style_kind = StyleKind::ALL[i];
                }
                ui.add_space(4.0);
                let k = self.doc_style_kind;
                let eff = kdraft_doc::style::category_style(k, &st);
                self.style_rows(ui, Target::Category(k), k.keys(), &[eff]);
                ui.label(
                    egui::RichText::new(
                        "Items follow these defaults unless they have their own style.",
                    )
                    .font(text::small())
                    .color(self.tokens().text_secondary),
                );
            });
        if !open {
            self.show_doc_styles = false;
        }
    }
}

impl App {
    /// The palette a view draws with and the chooser: follow the
    /// document, or a palette of its own (by value).
    pub(crate) fn layer_colour_presets(&mut self, ui: &mut egui::Ui, v: &kdraft_doc::View) {
        use kdraft_doc::layers::PaletteSource;
        let st = self.s.doc().styles.clone();
        let doc_name = st
            .layer_palette
            .as_ref()
            .map_or("Black".to_string(), |p| p.name.clone());
        let follow = match (&v.kind, st.layer_palette.is_some()) {
            (kdraft_doc::ViewKind::Fabrication | kdraft_doc::ViewKind::Drill, true) => {
                format!("Document ({doc_name}; black for this view)")
            }
            _ => format!("Document ({doc_name})"),
        };
        let cur = match v.palette_source(&st) {
            PaletteSource::Own(n) => kdraft_doc::palette::display_name(&n),
            PaletteSource::Document(_) | PaletteSource::Black => follow.clone(),
        };
        let mut names = vec![follow];
        names.extend(self.user.palette_names());
        let shown: Vec<String> = names
            .iter()
            .enumerate()
            .map(|(i, n)| {
                if i == 0 {
                    n.clone()
                } else {
                    kdraft_doc::palette::display_name(n)
                }
            })
            .collect();
        let refs: Vec<&str> = shown.iter().map(String::as_str).collect();
        let n_over = v.layer_colors().layers.len();
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Palette")
                    .font(text::small())
                    .color(self.tokens().text_secondary),
            );
            if let Some(k) = w::dropdown(ui, ("view-palette", &v.id.0), &cur, &refs, 200.0) {
                let args = if k == 0 {
                    json!({ "ids": [v.id.0], "name": "document" })
                } else {
                    let n = &names[k];
                    match self.user.palette(n) {
                        Some(p) if !kdraft_doc::palette::BUILTIN.contains(&n.as_str()) => json!({
                            "ids": [v.id.0],
                            "palette": kdraft_source::settings::palette_to_json(&p)
                        }),
                        _ => json!({ "ids": [v.id.0], "name": n }),
                    }
                };
                self.cmd("view.apply_palette", args);
            }
        });
        self.link_colour_note(ui, v);
        if n_over > 0 {
            // Wrapped: at the default panel width the button does not fit
            // beside the label and would be cut off at the panel edge.
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(format!("{n_over} layer(s) with their own colour"))
                        .font(text::small())
                        .color(self.tokens().text_secondary),
                );
                if w::button(ui, None, "Reset all to palette", false, true).clicked() {
                    self.cmd(
                        "view.set_layer_style",
                        json!({ "ids": [v.id.0], "reset": true }),
                    );
                }
            });
        }
    }

    /// Swatch (with a small picker) and opacity of one layer of a view.
    pub(crate) fn layer_colour_cell(
        &mut self,
        ui: &mut egui::Ui,
        v: &kdraft_doc::View,
        layer: &str,
    ) {
        let st = self.s.doc().styles.clone();
        let (c, o) = v.layer_color(&st, layer);
        let resp = swatch(ui, Some(c), &format!("{layer} colour"), false);
        let popup = egui::Popup::from_toggle_button_response(&resp)
            .id(egui::Id::unique(("layer-colour", v.id.0.as_str(), layer)))
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
        let pid = popup.get_id();
        let mut set: Option<Option<Rgba>> = None;
        popup.show(|ui| {
            ui.set_min_width(212.0);
            for row in PALETTE.chunks(10) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    for hex in row {
                        let p = Rgba::parse(hex);
                        if swatch(ui, p, hex, p == Some(c)).clicked() {
                            set = Some(p);
                        }
                    }
                });
            }
            let (def, _) = v.layer_default(&st, layer);
            ui.horizontal(|ui| {
                if swatch(ui, Some(def), "Palette colour", false).clicked() {
                    set = Some(None);
                }
                ui.label(egui::RichText::new("Palette colour").font(text::small()));
            });
            ui.horizontal(|ui| {
                let cur = c.to_string();
                if let Some(s) = w::text_field(ui, ("layer-hex", layer), &cur, 110.0, "#RRGGBB") {
                    match Rgba::parse(&s) {
                        Some(x) => set = Some(Some(x)),
                        None => self.warn(format!("{s:?} is not a colour")),
                    }
                }
            });
        });
        if let Some(x) = set {
            let color = match x {
                Some(x) => json!(x.to_string()),
                None => serde_json::Value::Null,
            };
            self.cmd(
                "view.set_layer_style",
                json!({ "ids": [v.id.0], "layer": layer, "color": color }),
            );
            if let Some(x) = x {
                self.remember_color(x);
            }
            egui::Popup::close_id(ui.ctx(), pid);
        }
        let (_, edit) = NumberField::new(
            ("layer-op", v.id.0.as_str(), layer),
            Lead::None,
            Some(o * 100.0),
        )
        .suffix("%")
        .range(0.0, 100.0)
        .step(10.0, 0.5)
        .decimals(0)
        .width(60.0)
        .tooltip(format!("{layer} opacity"))
        .show(ui);
        let id = v.id.0.clone();
        let layer_s = layer.to_string();
        let args = move |x: f64| json!({ "ids": [id], "layer": layer_s, "opacity": (x / 100.0).clamp(0.0, 1.0) });
        match edit {
            Some(FieldEdit::Set(x)) => {
                self.cmd("view.set_layer_style", args(x));
            }
            Some(FieldEdit::Begin) => {
                self.s.begin_gesture("Layer opacity");
                self.prop_drag = Some("view.set_layer_style");
            }
            Some(FieldEdit::Preview(x)) => {
                let _ = self
                    .reg
                    .preview(&mut self.s, "view.set_layer_style", args(x));
            }
            Some(FieldEdit::Commit) => {
                self.s.commit_gesture();
                self.prop_drag = None;
            }
            None => {}
        }
        // Overridden: a mark and "Reset to palette".
        if v.layer_colors().get(layer).is_some()
            && w::icon_button(
                ui,
                crate::icons::Icon::Reload,
                &format!("Reset {layer} to palette"),
                None,
                true,
                false,
            )
            .clicked()
        {
            self.cmd(
                "view.set_layer_style",
                json!({ "ids": [v.id.0], "layer": layer, "color": null, "opacity": null }),
            );
        }
    }
}

impl App {
    /// In a link group: which layers this view draws differently from its
    /// reference (or, on a reference, from its linked views), with a
    /// button to match; and a hint when its own colours are KiCad's screen
    /// colours (often from documents made before palettes were fixed).
    pub(crate) fn link_colour_note(&mut self, ui: &mut egui::Ui, v: &kdraft_doc::View) {
        use kdraft_doc::link;
        let d = self.s.doc().clone();
        let st = &d.styles;
        let ix = link::index(&d);
        let it = Item::View(v.clone());
        let t = self.tokens();
        let small = |s: String| {
            egui::RichText::new(s)
                .font(text::small())
                .color(t.text_secondary)
        };
        match it.link() {
            Some(l) => {
                if let Some(Item::View(r)) = link::get(&d, &ix, &l.to) {
                    let diff = link::colour_differences(st, r, v);
                    if !diff.is_empty() {
                        ui.label(small(format!(
                            "Colours differ from the reference view: {}",
                            diff.join(", ")
                        )));
                        if w::button(ui, None, "Match reference", false, true).clicked() {
                            self.cmd("link.match_colours", json!({ "ids": [v.id.0] }));
                        }
                    }
                }
            }
            None => {
                let mut diff: Vec<String> = Vec::new();
                for f in link::followers(&d, &v.id) {
                    if let Some(Item::View(fv)) = link::get(&d, &ix, &f) {
                        for l in link::colour_differences(st, v, fv) {
                            if !diff.contains(&l) {
                                diff.push(l);
                            }
                        }
                    }
                }
                if !diff.is_empty() {
                    ui.label(small(format!(
                        "Linked views draw {} in other colours",
                        diff.join(", ")
                    )));
                    if w::button(ui, None, "Match linked views to this one", false, true).clicked()
                    {
                        self.cmd("link.match_colours", json!({ "ids": [v.id.0] }));
                    }
                }
            }
        }
        // Own colours that are exactly KiCad's screen colours.
        let lc = v.layer_colors();
        if !lc.layers.is_empty() {
            let raw = [
                kdraft_doc::palette::KICAD_DEFAULT_NAME,
                kdraft_doc::palette::KICAD_CLASSIC_NAME,
            ];
            let hit = raw.iter().find(|n| {
                kdraft_doc::palette::Palette::builtin(n).is_some_and(|p| {
                    lc.layers
                        .iter()
                        .all(|o| o.color.is_some() && p.layer(&o.layer).map(|x| x.0) == o.color)
                })
            });
            if let Some(n) = hit {
                ui.label(small(format!(
                    "Its own colours are {n} screen colours (hard to see on paper)"
                )));
                if w::button(ui, None, "Reset to palette", false, true).clicked() {
                    self.cmd(
                        "view.set_layer_style",
                        json!({ "ids": [v.id.0], "reset": true }),
                    );
                }
            }
        }
    }
}

fn fill_text(c: Rgba) -> String {
    if c.is_none() {
        "none".into()
    } else {
        c.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_values_and_common_keys() {
        let mut a = ItemStyle::default();
        a.set("stroke", Some("#FF0000")).unwrap();
        a.set("width", Some("0.3")).unwrap();
        let mut b = a.clone();
        b.set("width", Some("0.5")).unwrap();
        assert_eq!(
            common(&[a.clone(), b.clone()], "stroke").as_deref(),
            Some("#FF0000")
        );
        assert_eq!(common(&[a, b], "width"), None);
        let k = common_keys(&[StyleKind::Graphic, StyleKind::Text]);
        assert_eq!(k, vec!["opacity"]);
        assert!(PALETTE.iter().all(|h| Rgba::parse(h).is_some()));
    }
}
