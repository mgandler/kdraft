//! Layer colours of views.
//!
//! Every colour a view draws (its layers and, in native views, the roles
//! pads, holes, outline, bodies, designators, DNP) resolves in one order:
//! the view's per-layer or per-role override, then the view's own palette
//! (stored by value: `(layer_colors (layer_palette ...) (layer "F.Cu"
//! "#C83434" 0.8))`), then the document palette (`(styles (layer_palette
//! ...))`; not for fabrication and drill views, which are black line
//! drawings unless they have their own palette), then black. A role
//! without its own colour takes the colour of its layer.
//!
//! Older files: a view's `(preset black|layer|grey)` and the document's
//! `(layer_preset ...)` are read as the matching built-in palettes and
//! written in the new form.

use kdraft_sexpr::Value;

use crate::model::{Styles, View, ViewKind};
use crate::palette::Palette;
use crate::style::Rgba;

/// Head of the view node.
pub const LAYER_COLORS: &str = "layer_colors";
/// Head of the older document default (read and migrated only).
pub const LAYER_PRESET: &str = "layer_preset";

/// The older colour presets, read from files and the `preset` arguments
/// and turned into built-in palettes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayerPreset {
    Black,
    Layer,
    Grey,
}

impl LayerPreset {
    pub fn as_str(self) -> &'static str {
        match self {
            LayerPreset::Black => "black",
            LayerPreset::Layer => "layer",
            LayerPreset::Grey => "grey",
        }
    }

    pub fn parse(s: &str) -> Option<LayerPreset> {
        match s.trim() {
            "black" => Some(LayerPreset::Black),
            "layer" | "layers" | "colour" | "color" => Some(LayerPreset::Layer),
            "grey" | "gray" | "greyscale" | "grayscale" => Some(LayerPreset::Grey),
            _ => None,
        }
    }
}

/// Distinct hues for inner copper layers (In1.Cu, In2.Cu, ...).
const INNER: [Rgba; 8] = [
    Rgba::rgb(0xC8, 0x8C, 0x00),
    Rgba::rgb(0x2E, 0x8B, 0x57),
    Rgba::rgb(0xB4, 0x28, 0x8C),
    Rgba::rgb(0x00, 0x86, 0xA8),
    Rgba::rgb(0x8B, 0x45, 0x13),
    Rgba::rgb(0x5A, 0x3C, 0xB4),
    Rgba::rgb(0x6B, 0x8E, 0x23),
    Rgba::rgb(0xD2, 0x69, 0x1E),
];

/// kdraft's layer palette (chosen for print on white paper).
pub fn palette_color(name: &str) -> Rgba {
    match name {
        "F.Cu" => Rgba::rgb(0xC8, 0x34, 0x34),
        "B.Cu" => Rgba::rgb(0x34, 0x50, 0xC8),
        "F.SilkS" => Rgba::rgb(0x00, 0x6E, 0x6E),
        "B.SilkS" => Rgba::rgb(0x6E, 0x32, 0x8C),
        "F.Fab" => Rgba::rgb(0x78, 0x78, 0x78),
        "B.Fab" => Rgba::rgb(0x5A, 0x5A, 0x6E),
        "F.CrtYd" | "B.CrtYd" => Rgba::rgb(0xC0, 0x30, 0xC0),
        "Edge.Cuts" => Rgba::rgb(0xA0, 0x82, 0x00),
        "Margin" => Rgba::rgb(0xB4, 0xA0, 0x64),
        "F.Mask" => Rgba::rgb(0x8C, 0x50, 0x78),
        "B.Mask" => Rgba::rgb(0x50, 0x64, 0x8C),
        "F.Paste" => Rgba::rgb(0x8C, 0x64, 0x64),
        "B.Paste" => Rgba::rgb(0x64, 0x78, 0x8C),
        "F.Adhes" | "B.Adhes" => Rgba::rgb(0x82, 0x6E, 0x96),
        "Dwgs.User" => Rgba::rgb(0x5A, 0x6E, 0x82),
        "Cmts.User" => Rgba::rgb(0x6E, 0x78, 0x8C),
        "Eco1.User" | "Eco2.User" => Rgba::rgb(0x6E, 0x82, 0x6E),
        n if n.starts_with("In") && n.ends_with(".Cu") => {
            let k: usize = n[2..n.len() - 3].parse().unwrap_or(1);
            INNER[(k.max(1) - 1) % INNER.len()]
        }
        // User.1 ... and anything else: muted.
        _ => Rgba::rgb(0x82, 0x78, 0x6E),
    }
}

/// Luminance grey of a colour.
pub fn to_grey(c: Rgba) -> Rgba {
    let y = (0.2126 * f64::from(c.r) + 0.7152 * f64::from(c.g) + 0.0722 * f64::from(c.b))
        .round()
        .clamp(0.0, 200.0) as u8;
    Rgba {
        r: y,
        g: y,
        b: y,
        a: c.a,
    }
}

impl LayerPreset {
    /// The built-in palette of an older preset.
    pub fn palette(self) -> Palette {
        let name = match self {
            LayerPreset::Black => crate::palette::BLACK,
            LayerPreset::Layer => crate::palette::LAYER,
            LayerPreset::Grey => crate::palette::GREY,
        };
        Palette::builtin(name).unwrap_or_default()
    }
}

/// One layer's (or role's, `role.<name>`) own settings in a view.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerOverride {
    pub layer: String,
    pub color: Option<Rgba>,
    /// 0..1.
    pub opacity: Option<f64>,
}

/// A view's layer colour settings: its own palette (by value, covering
/// every layer and role) and per-layer or per-role overrides.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LayerColors {
    pub palette: Option<Palette>,
    pub layers: Vec<LayerOverride>,
    pub extra: Vec<Value>,
}

impl LayerColors {
    pub fn is_empty(&self) -> bool {
        self.palette.is_none() && self.layers.is_empty() && self.extra.is_empty()
    }

    pub fn get(&self, layer: &str) -> Option<&LayerOverride> {
        self.layers.iter().find(|l| l.layer == layer)
    }

    /// Sets a layer's colour and/or opacity (`Some(None)` clears one);
    /// entries without either are dropped.
    pub fn set(&mut self, layer: &str, color: Option<Option<Rgba>>, opacity: Option<Option<f64>>) {
        let mut e = self.get(layer).cloned().unwrap_or(LayerOverride {
            layer: layer.to_string(),
            color: None,
            opacity: None,
        });
        if let Some(c) = color {
            e.color = c;
        }
        if let Some(o) = opacity {
            e.opacity = o.map(|o| o.clamp(0.0, 1.0));
        }
        self.layers.retain(|l| l.layer != layer);
        if e.color.is_some() || e.opacity.is_some() {
            self.layers.push(e);
            self.layers.sort_by(|a, b| a.layer.cmp(&b.layer));
        }
    }

    /// Reads `(layer_colors ...)`; an older `(preset black|layer|grey)`
    /// becomes the matching built-in palette.
    pub fn from_value(v: &Value) -> LayerColors {
        let mut out = LayerColors::default();
        for c in v.args() {
            match c.head() {
                Some("preset") => {
                    match c
                        .args()
                        .first()
                        .and_then(Value::atom_text)
                        .and_then(LayerPreset::parse)
                    {
                        Some(p) => out.palette = Some(p.palette()),
                        None => out.extra.push(c.clone()),
                    }
                }
                Some(crate::palette::LAYER_PALETTE) => {
                    out.palette = Some(Palette::from_value(c));
                }
                Some("layer") => {
                    let a = c.args();
                    let name = a.first().and_then(Value::atom_text);
                    let color = a.get(1).and_then(Value::atom_text).and_then(|t| {
                        if t == "-" {
                            None
                        } else {
                            Rgba::parse(t)
                        }
                    });
                    let opacity = a
                        .get(2)
                        .and_then(Value::atom_text)
                        .and_then(|t| t.parse::<f64>().ok())
                        .filter(|o| o.is_finite())
                        .map(|o| o.clamp(0.0, 1.0));
                    match name {
                        Some(n) if color.is_some() || opacity.is_some() => {
                            out.set(n, Some(color), Some(opacity));
                        }
                        _ => out.extra.push(c.clone()),
                    }
                }
                _ => out.extra.push(c.clone()),
            }
        }
        out
    }

    pub fn to_value(&self) -> Option<Value> {
        if self.is_empty() {
            return None;
        }
        let mut o = Vec::new();
        if let Some(p) = &self.palette {
            o.push(p.to_value());
        }
        for l in &self.layers {
            let mut a = vec![
                Value::str(&l.layer),
                Value::str(l.color.map_or("-".to_string(), |c| c.to_string())),
            ];
            if let Some(op) = l.opacity {
                a.push(Value::num(op).unwrap_or_else(|_| Value::int(1)));
            }
            o.push(Value::node("layer", a));
        }
        o.extend(self.extra.iter().cloned());
        Some(Value::node(LAYER_COLORS, o))
    }
}

/// Where a view's colours come from.
#[derive(Debug, Clone, PartialEq)]
pub enum PaletteSource {
    /// The view's own palette.
    Own(String),
    /// The document's palette.
    Document(String),
    /// None: black (fabrication and drill views, or no document palette).
    Black,
}

impl View {
    pub fn layer_colors(&self) -> LayerColors {
        self.extra
            .iter()
            .find(|v| v.is(LAYER_COLORS))
            .map(LayerColors::from_value)
            .unwrap_or_default()
    }

    pub fn set_layer_colors(&mut self, lc: &LayerColors) {
        let at = self.extra.iter().position(|v| v.is(LAYER_COLORS));
        self.extra.retain(|v| !v.is(LAYER_COLORS));
        if let Some(v) = lc.to_value() {
            let i = at.unwrap_or(self.extra.len()).min(self.extra.len());
            self.extra.insert(i, v);
        }
    }

    /// The palette this view draws with: its own, else the document's
    /// (not for fabrication and drill views: black line drawings), else
    /// none (black).
    pub fn effective_palette(&self, st: &Styles) -> Option<Palette> {
        if let Some(p) = self.layer_colors().palette {
            return Some(p);
        }
        if matches!(self.kind, ViewKind::Fabrication | ViewKind::Drill) {
            return None;
        }
        st.layer_palette.clone()
    }

    pub fn palette_source(&self, st: &Styles) -> PaletteSource {
        if let Some(p) = self.layer_colors().palette {
            return PaletteSource::Own(p.name);
        }
        match self.effective_palette(st) {
            Some(p) => PaletteSource::Document(p.name),
            None => PaletteSource::Black,
        }
    }

    /// Colour and opacity of `layer` from the palette in effect (no view
    /// override); black where the palette has none.
    pub fn layer_default(&self, st: &Styles, layer: &str) -> (Rgba, f64) {
        self.effective_palette(st)
            .and_then(|p| p.layer(layer))
            .unwrap_or((Rgba::BLACK, 1.0))
    }

    /// Colour and opacity a layer is drawn with in this view: the view's
    /// override, else the palette in effect, else black.
    pub fn layer_color(&self, st: &Styles, layer: &str) -> (Rgba, f64) {
        let lc = self.layer_colors();
        let (dc, dop) = self.layer_default(st, layer);
        let o = lc.get(layer);
        (
            o.and_then(|o| o.color).unwrap_or(dc),
            o.and_then(|o| o.opacity).unwrap_or(dop),
        )
    }

    /// Colour of a native-view role (`pads`, `designators` ...): the
    /// view's `role.<name>` override, else the palette's role entry, else
    /// the colour of `layer` (the layer the role belongs to).
    pub fn role_color(&self, st: &Styles, role: &str, layer: &str) -> (Rgba, f64) {
        self.role_own(st, role)
            .unwrap_or_else(|| self.layer_color(st, layer))
    }

    /// A role's colour when the view or the palette sets one.
    pub fn role_own(&self, st: &Styles, role: &str) -> Option<(Rgba, f64)> {
        let lc = self.layer_colors();
        let base = self.effective_palette(st).and_then(|p| p.role(role));
        let o = lc.get(&format!("role.{role}"));
        match (o, base) {
            (None, b) => b,
            (Some(o), b) => {
                let (bc, bo) = b.unwrap_or((Rgba::BLACK, 1.0));
                Some((o.color.unwrap_or(bc), o.opacity.unwrap_or(bo)))
            }
        }
    }

    /// Whether everything is plain black and opaque (drawn as plotted).
    pub fn plain_black(&self, st: &Styles) -> bool {
        self.layer_colors().layers.is_empty()
            && self.effective_palette(st).is_none_or(|p| p.is_black())
    }
}

/// Moves an older document default `(layer_preset X)` into the palette
/// model (`Styles::layer_palette`), unless a palette is set already.
pub fn migrate_doc_preset(st: &mut Styles) {
    let old = st
        .extra
        .iter()
        .find(|v| v.is(LAYER_PRESET))
        .and_then(|v| v.args().first())
        .and_then(Value::atom_text)
        .and_then(LayerPreset::parse);
    st.extra.retain(|v| !v.is(LAYER_PRESET));
    if st.layer_palette.is_none() {
        st.layer_palette = old
            .filter(|p| *p != LayerPreset::Black)
            .map(LayerPreset::palette);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palettes_overrides_and_old_presets() {
        assert_ne!(palette_color("F.Cu"), palette_color("B.Cu"));
        assert_ne!(palette_color("In1.Cu"), palette_color("In2.Cu"));
        let mut lc = LayerColors {
            palette: Palette::builtin(crate::palette::LAYER),
            ..Default::default()
        };
        lc.set("F.Cu", Some(Some(Rgba::rgb(1, 2, 3))), Some(Some(0.5)));
        lc.set("B.Cu", None, Some(Some(0.25)));
        let v = lc.to_value().unwrap();
        assert_eq!(LayerColors::from_value(&v), lc);
        lc.set("B.Cu", None, Some(None));
        assert!(lc.get("B.Cu").is_none());
        // An older view preset reads as the built-in palette.
        let old = Value::node(LAYER_COLORS, [Value::node("preset", [Value::sym("grey")])]);
        let lc = LayerColors::from_value(&old);
        assert_eq!(lc.palette.as_ref().unwrap().name, crate::palette::GREY);
        let back = lc.to_value().unwrap();
        assert!(!format!("{back:?}").contains("preset"));
    }
}
