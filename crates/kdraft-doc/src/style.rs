//! Item appearance: colours, line patterns, text attributes.
//!
//! An item stores only the attributes it overrides, in an `(appearance ...)`
//! node among its extra nodes; the document stores defaults per item
//! category in `(styles (appearance (graphic ...) (text ...) ...))`.
//! Effective values are built-in defaults, overridden by the document
//! category defaults, overridden by the item.
//!
//! Colours are sRGB hex, `#RRGGBB` or `#RRGGBBAA`.

use std::fmt;

use kdraft_layout::dimension::DimStyle;
use kdraft_sexpr::Value;

use crate::model::{Dimension, GraphicShape, Item, Styles};

/// Head of the per-item override node and of the document defaults node.
pub const APPEARANCE: &str = "appearance";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const BLACK: Rgba = Rgba::rgb(0, 0, 0);
    pub const WHITE: Rgba = Rgba::rgb(255, 255, 255);
    /// Fully transparent: "no fill".
    pub const NONE: Rgba = Rgba {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };

    pub const fn rgb(r: u8, g: u8, b: u8) -> Rgba {
        Rgba { r, g, b, a: 255 }
    }

    /// Parses `#RGB`, `#RRGGBB`, `#RRGGBBAA` (the `#` is optional) or
    /// `none`.
    pub fn parse(s: &str) -> Option<Rgba> {
        let t = s.trim();
        if t.eq_ignore_ascii_case("none") || t.eq_ignore_ascii_case("transparent") {
            return Some(Rgba::NONE);
        }
        let h = t.strip_prefix('#').unwrap_or(t);
        if !h.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
        match h.len() {
            3 => {
                let n = |i: usize| {
                    let d = u8::from_str_radix(h.get(i..i + 1)?, 16).ok()?;
                    Some(d * 17)
                };
                Some(Rgba::rgb(n(0)?, n(1)?, n(2)?))
            }
            6 => Some(Rgba::rgb(byte(0)?, byte(2)?, byte(4)?)),
            8 => Some(Rgba {
                r: byte(0)?,
                g: byte(2)?,
                b: byte(4)?,
                a: byte(6)?,
            }),
            _ => None,
        }
    }

    pub fn is_none(self) -> bool {
        self.a == 0
    }
}

/// WCAG relative luminance of an sRGB colour (alpha ignored).
pub fn luminance(c: Rgba) -> f64 {
    let lin = |v: u8| {
        let s = f64::from(v) / 255.0;
        if s <= 0.040_45 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

/// WCAG contrast ratio of two colours (1 to 21).
pub fn contrast(a: Rgba, b: Rgba) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Contrast against white that "Adapt for white paper" keeps at least
/// (WCAG's 3:1 for graphical objects).
pub const PAPER_CONTRAST: f64 = 3.0;

/// HSL of a colour: hue in degrees, saturation and lightness 0..1.
pub fn to_hsl(c: Rgba) -> (f64, f64, f64) {
    let (r, g, b) = (
        f64::from(c.r) / 255.0,
        f64::from(c.g) / 255.0,
        f64::from(c.b) / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d < 1e-12 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if (max - r).abs() < 1e-12 {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if (max - g).abs() < 1e-12 {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, s.clamp(0.0, 1.0), l)
}

/// The colour for HSL (alpha `a`).
pub fn from_hsl(h: f64, s: f64, l: f64, a: u8) -> Rgba {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let q = |v: f64| ((v + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgba {
        r: q(r),
        g: q(g),
        b: q(b),
        a,
    }
}

/// `c` darkened (same hue and saturation, lower lightness) until its
/// contrast against white is at least `min`; unchanged when it already
/// is. Alpha is kept.
pub fn adapt_for_paper(c: Rgba, min: f64) -> Rgba {
    if contrast(c, Rgba::WHITE) >= min {
        return c;
    }
    let (h, s, l) = to_hsl(c);
    // Bisection on lightness: contrast falls monotonically with it.
    let (mut lo, mut hi) = (0.0_f64, l);
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if contrast(from_hsl(h, s, mid, c.a), Rgba::WHITE) >= min {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let out = from_hsl(h, s, lo, c.a);
    // Rounding to bytes can land just below the threshold.
    if contrast(out, Rgba::WHITE) >= min {
        out
    } else {
        from_hsl(h, s, (lo - 0.01).max(0.0), c.a)
    }
}

impl fmt::Display for Rgba {
    /// `#RRGGBB` when opaque, `#RRGGBBAA` otherwise.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02X}{:02X}{:02X}", self.r, self.g, self.b)?;
        if self.a != 255 {
            write!(f, "{:02X}", self.a)?;
        }
        Ok(())
    }
}

macro_rules! word_enum {
    ($(#[$m:meta])* $name:ident { $($var:ident = $s:literal),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name { $($var),* }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$var),*];

            pub fn as_str(self) -> &'static str {
                match self { $($name::$var => $s),* }
            }

            pub fn parse(s: &str) -> Option<$name> {
                match s.trim() {
                    $($s => Some($name::$var),)*
                    _ => None,
                }
            }
        }
    };
}

word_enum!(
    /// Stroke pattern.
    LinePattern {
        Solid = "solid",
        Dashed = "dashed",
        Dotted = "dotted",
        DashDot = "dash-dot",
    }
);

impl LinePattern {
    /// Dash array (on, off, ...) in mm for a stroke of `width`; `None` for
    /// solid lines. Dots assume round caps.
    pub fn dashes(self, width: f64) -> Option<Vec<f64>> {
        let w = width.max(0.05);
        match self {
            LinePattern::Solid => None,
            LinePattern::Dashed => Some(vec![(6.0 * w).max(1.2), (4.0 * w).max(0.8)]),
            LinePattern::Dotted => Some(vec![0.001, (3.0 * w).max(0.5)]),
            LinePattern::DashDot => Some(vec![
                (6.0 * w).max(1.2),
                (3.0 * w).max(0.6),
                0.001,
                (3.0 * w).max(0.6),
            ]),
        }
    }
}

word_enum!(
    /// Arrowheads on a line or polyline.
    Arrows {
        None = "none",
        Start = "start",
        End = "end",
        Both = "both",
    }
);

impl Arrows {
    pub fn at_start(self) -> bool {
        matches!(self, Arrows::Start | Arrows::Both)
    }

    pub fn at_end(self) -> bool {
        matches!(self, Arrows::End | Arrows::Both)
    }
}

word_enum!(
    /// Horizontal text alignment.
    TextAlign {
        Left = "left",
        Center = "center",
        Right = "right",
    }
);

word_enum!(
    /// Outline of a callout balloon.
    BalloonShape {
        Circle = "circle",
        Square = "square",
        Hexagon = "hexagon",
        None = "none",
    }
);

word_enum!(
    /// Item categories with their own document defaults.
    StyleKind {
        Graphic = "graphic",
        Text = "text",
        Dimension = "dimension",
        Callout = "callout",
        Notes = "notes",
        Table = "table",
    }
);

impl StyleKind {
    /// Category of a document item; `None` for views and unknown items.
    pub fn of(item: &Item) -> Option<StyleKind> {
        Some(match item {
            Item::Graphic(g) => match g.shape {
                GraphicShape::Text { .. } => StyleKind::Text,
                _ => StyleKind::Graphic,
            },
            Item::Dimension(_) => StyleKind::Dimension,
            Item::Callout(_) => StyleKind::Callout,
            Item::NoteList(_) => StyleKind::Notes,
            Item::Table(_) => StyleKind::Table,
            Item::View(_) | Item::Unknown(_) => return None,
        })
    }

    /// Attribute keys that apply to this category, in inspector order.
    pub fn keys(self) -> &'static [&'static str] {
        match self {
            StyleKind::Graphic => &[
                "stroke",
                "width",
                "pattern",
                "fill",
                "opacity",
                "arrows",
                "arrow_size",
            ],
            StyleKind::Text => &[
                "text_color",
                "text_height",
                "bold",
                "italic",
                "align",
                "line_spacing",
                "opacity",
            ],
            StyleKind::Dimension => &[
                "stroke",
                "width",
                "text_color",
                "text_height",
                "arrow_size",
                "decimals",
                "opacity",
            ],
            StyleKind::Callout => &[
                "shape",
                "stroke",
                "width",
                "text_color",
                "text_height",
                "opacity",
            ],
            StyleKind::Notes => &["text_color", "text_height", "line_spacing", "opacity"],
            StyleKind::Table => &[
                "stroke",
                "width",
                "text_color",
                "text_height",
                "header_fill",
                "opacity",
            ],
        }
    }
}

/// Attribute keys that apply to one item: its category's, without
/// arrows on closed shapes and without fill on open ones.
pub fn keys_for(item: &Item) -> Vec<&'static str> {
    let Some(k) = StyleKind::of(item) else {
        return Vec::new();
    };
    let (open, closed) = match item {
        Item::Graphic(g) => match g.shape {
            GraphicShape::Line { .. } | GraphicShape::Polyline { .. } => (true, false),
            GraphicShape::Arc { .. } => (false, false),
            GraphicShape::Rect { .. }
            | GraphicShape::Circle { .. }
            | GraphicShape::Polygon { .. } => (false, true),
            _ => (true, true),
        },
        _ => (true, true),
    };
    k.keys()
        .iter()
        .copied()
        .filter(|key| match *key {
            "arrows" | "arrow_size" if k == StyleKind::Graphic => open,
            "fill" => closed,
            _ => true,
        })
        .collect()
}

/// Every attribute key, in canonical write order.
pub const STYLE_KEYS: [&str; 16] = [
    "stroke",
    "width",
    "pattern",
    "fill",
    "opacity",
    "arrows",
    "arrow_size",
    "text_color",
    "text_height",
    "bold",
    "italic",
    "align",
    "line_spacing",
    "decimals",
    "header_fill",
    "shape",
];

/// A set of appearance attributes; `None` fields are not overridden.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ItemStyle {
    /// Line / border colour.
    pub stroke: Option<Rgba>,
    /// Line width, mm.
    pub width: Option<f64>,
    pub pattern: Option<LinePattern>,
    /// Fill of closed shapes; [`Rgba::NONE`] for no fill.
    pub fill: Option<Rgba>,
    /// 0..1, applied to every colour of the item.
    pub opacity: Option<f64>,
    pub arrows: Option<Arrows>,
    /// Arrowhead length, mm.
    pub arrow_size: Option<f64>,
    pub text_color: Option<Rgba>,
    /// Cap height, mm.
    pub text_height: Option<f64>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub align: Option<TextAlign>,
    /// Baseline distance in text heights.
    pub line_spacing: Option<f64>,
    pub decimals: Option<usize>,
    /// Table header row fill; [`Rgba::NONE`] for none.
    pub header_fill: Option<Rgba>,
    /// Callout balloon outline (default circle).
    pub shape: Option<BalloonShape>,
    /// Unknown attributes, preserved.
    pub extra: Vec<Value>,
}

/// Reasons a style attribute cannot be set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleError(pub String);

impl fmt::Display for StyleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StyleError {}

fn parse_bool(s: &str) -> Option<bool> {
    match s.trim() {
        "yes" | "true" | "on" | "1" => Some(true),
        "no" | "false" | "off" | "0" => Some(false),
        _ => None,
    }
}

fn parse_num(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

impl ItemStyle {
    pub fn is_empty(&self) -> bool {
        STYLE_KEYS.iter().all(|k| self.get(k).is_none()) && self.extra.is_empty()
    }

    /// `self` with every attribute set in `over` replaced by `over`'s.
    pub fn merged(&self, over: &ItemStyle) -> ItemStyle {
        let mut out = self.clone();
        for k in STYLE_KEYS {
            if let Some(v) = over.get(k) {
                // Values from `get` always parse.
                let _ = out.set(k, Some(&v));
            }
        }
        out
    }

    /// Text form of attribute `key` (as written to the file), or `None`
    /// when unset or unknown.
    pub fn get(&self, key: &str) -> Option<String> {
        let num = |v: f64| kdraft_sexpr::format_number(v).ok();
        let yn = |b: bool| if b { "yes" } else { "no" }.to_string();
        match key {
            "stroke" => self.stroke.map(|c| c.to_string()),
            "width" => self.width.and_then(num),
            "pattern" => self.pattern.map(|p| p.as_str().to_string()),
            "fill" => self.fill.map(fill_text),
            "opacity" => self.opacity.and_then(num),
            "arrows" => self.arrows.map(|a| a.as_str().to_string()),
            "arrow_size" => self.arrow_size.and_then(num),
            "text_color" => self.text_color.map(|c| c.to_string()),
            "text_height" => self.text_height.and_then(num),
            "bold" => self.bold.map(yn),
            "italic" => self.italic.map(yn),
            "align" => self.align.map(|a| a.as_str().to_string()),
            "line_spacing" => self.line_spacing.and_then(num),
            "decimals" => self.decimals.map(|d| d.to_string()),
            "header_fill" => self.header_fill.map(fill_text),
            "shape" => self.shape.map(|x| x.as_str().to_string()),
            _ => None,
        }
    }

    /// Sets (`Some`) or clears (`None`) attribute `key` from its text form.
    pub fn set(&mut self, key: &str, value: Option<&str>) -> Result<(), StyleError> {
        let bad = |what: &str| StyleError(format!("{key}: expected {what}"));
        let color = |s: &str| Rgba::parse(s).ok_or_else(|| bad("a colour like #1A2B3C"));
        let pos = |s: &str, max: f64| {
            parse_num(s)
                .filter(|v| *v > 0.0 && *v <= max)
                .ok_or_else(|| bad(&format!("a number above 0 and at most {max}")))
        };
        let Some(v) = value else {
            match key {
                "stroke" => self.stroke = None,
                "width" => self.width = None,
                "pattern" => self.pattern = None,
                "fill" => self.fill = None,
                "opacity" => self.opacity = None,
                "arrows" => self.arrows = None,
                "arrow_size" => self.arrow_size = None,
                "text_color" => self.text_color = None,
                "text_height" => self.text_height = None,
                "bold" => self.bold = None,
                "italic" => self.italic = None,
                "align" => self.align = None,
                "line_spacing" => self.line_spacing = None,
                "decimals" => self.decimals = None,
                "header_fill" => self.header_fill = None,
                "shape" => self.shape = None,
                _ => return Err(StyleError(format!("unknown style attribute {key}"))),
            }
            return Ok(());
        };
        match key {
            "stroke" => self.stroke = Some(color(v)?),
            "width" => self.width = Some(pos(v, 20.0)?),
            "pattern" => {
                self.pattern = Some(
                    LinePattern::parse(v)
                        .ok_or_else(|| bad("solid, dashed, dotted or dash-dot"))?,
                )
            }
            "fill" => self.fill = Some(color(v)?),
            "opacity" => {
                self.opacity = Some(
                    parse_num(v)
                        .filter(|o| (0.0..=1.0).contains(o))
                        .ok_or_else(|| bad("a number from 0 to 1"))?,
                )
            }
            "arrows" => {
                self.arrows = Some(Arrows::parse(v).ok_or_else(|| bad("none, start, end or both"))?)
            }
            "arrow_size" => self.arrow_size = Some(pos(v, 50.0)?),
            "text_color" => self.text_color = Some(color(v)?),
            "text_height" => self.text_height = Some(pos(v, 100.0)?),
            "bold" => self.bold = Some(parse_bool(v).ok_or_else(|| bad("yes or no"))?),
            "italic" => self.italic = Some(parse_bool(v).ok_or_else(|| bad("yes or no"))?),
            "align" => {
                self.align = Some(TextAlign::parse(v).ok_or_else(|| bad("left, center or right"))?)
            }
            "line_spacing" => {
                self.line_spacing = Some(
                    parse_num(v)
                        .filter(|s| (0.5..=5.0).contains(s))
                        .ok_or_else(|| bad("a number from 0.5 to 5"))?,
                )
            }
            "decimals" => {
                self.decimals = Some(
                    v.trim()
                        .parse::<usize>()
                        .ok()
                        .filter(|d| *d <= 6)
                        .ok_or_else(|| bad("a whole number from 0 to 6"))?,
                )
            }
            "header_fill" => self.header_fill = Some(color(v)?),
            "shape" => {
                self.shape = Some(
                    BalloonShape::parse(v).ok_or_else(|| bad("circle, square, hexagon or none"))?,
                )
            }
            _ => return Err(StyleError(format!("unknown style attribute {key}"))),
        }
        Ok(())
    }

    /// Only the attributes in `keys`.
    pub fn only(&self, keys: &[&str]) -> ItemStyle {
        let mut out = ItemStyle::default();
        for k in keys {
            if let Some(v) = self.get(k) {
                let _ = out.set(k, Some(&v));
            }
        }
        out
    }

    /// Reads attributes from the children of an `(appearance ...)` or
    /// category node. Unknown or malformed attributes are kept verbatim.
    pub fn from_value(v: &Value) -> ItemStyle {
        let mut s = ItemStyle::default();
        for c in v.args() {
            let key = c.head().unwrap_or("");
            let arg = c.args().first().and_then(Value::atom_text);
            let ok = STYLE_KEYS.contains(&key)
                && c.args().len() == 1
                && arg.is_some_and(|a| s.set(key, Some(a)).is_ok());
            if !ok {
                s.extra.push(c.clone());
            }
        }
        s
    }

    /// Canonical attribute nodes (fixed order, extras last).
    pub fn to_values(&self) -> Vec<Value> {
        let mut out = Vec::new();
        for k in STYLE_KEYS {
            let Some(t) = self.get(k) else { continue };
            let arg = if t.starts_with('#') {
                Value::str(t)
            } else {
                Value::sym(t)
            };
            out.push(Value::node(k, [arg]));
        }
        out.extend(self.extra.iter().cloned());
        out
    }

    /// `(head ...)`, or `None` when empty.
    pub fn to_node(&self, head: &str) -> Option<Value> {
        (!self.is_empty()).then(|| Value::node(head, self.to_values()))
    }
}

fn fill_text(c: Rgba) -> String {
    if c.is_none() {
        "none".to_string()
    } else {
        c.to_string()
    }
}

/// Document defaults per category.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StyleSet {
    pub graphic: ItemStyle,
    pub text: ItemStyle,
    pub dimension: ItemStyle,
    pub callout: ItemStyle,
    pub notes: ItemStyle,
    pub table: ItemStyle,
    /// Unknown category nodes, preserved.
    pub extra: Vec<Value>,
}

impl StyleSet {
    pub fn get(&self, k: StyleKind) -> &ItemStyle {
        match k {
            StyleKind::Graphic => &self.graphic,
            StyleKind::Text => &self.text,
            StyleKind::Dimension => &self.dimension,
            StyleKind::Callout => &self.callout,
            StyleKind::Notes => &self.notes,
            StyleKind::Table => &self.table,
        }
    }

    pub fn get_mut(&mut self, k: StyleKind) -> &mut ItemStyle {
        match k {
            StyleKind::Graphic => &mut self.graphic,
            StyleKind::Text => &mut self.text,
            StyleKind::Dimension => &mut self.dimension,
            StyleKind::Callout => &mut self.callout,
            StyleKind::Notes => &mut self.notes,
            StyleKind::Table => &mut self.table,
        }
    }

    pub fn is_empty(&self) -> bool {
        StyleKind::ALL.iter().all(|k| self.get(*k).is_empty()) && self.extra.is_empty()
    }

    pub fn from_value(v: &Value) -> StyleSet {
        let mut s = StyleSet::default();
        for c in v.args() {
            match c.head().and_then(StyleKind::parse) {
                Some(k) => *s.get_mut(k) = ItemStyle::from_value(c),
                None => s.extra.push(c.clone()),
            }
        }
        s
    }

    pub fn to_node(&self) -> Option<Value> {
        if self.is_empty() {
            return None;
        }
        let mut o: Vec<Value> = StyleKind::ALL
            .iter()
            .filter_map(|k| self.get(*k).to_node(k.as_str()))
            .collect();
        o.extend(self.extra.iter().cloned());
        Some(Value::node(APPEARANCE, o))
    }
}

/// Built-in defaults for a category, from the document's scalar styles.
pub fn builtin(k: StyleKind, st: &Styles) -> ItemStyle {
    let d = &st.dimension;
    let mut s = ItemStyle {
        stroke: Some(Rgba::BLACK),
        opacity: Some(1.0),
        text_color: Some(Rgba::BLACK),
        bold: Some(false),
        italic: Some(false),
        align: Some(TextAlign::Left),
        line_spacing: Some(1.5),
        pattern: Some(LinePattern::Solid),
        fill: Some(Rgba::NONE),
        arrows: Some(Arrows::None),
        arrow_size: Some(d.arrow_length),
        width: Some(st.line_width),
        text_height: Some(st.text_height),
        ..ItemStyle::default()
    };
    match k {
        StyleKind::Dimension => {
            s.width = Some(d.line_width);
            s.text_height = Some(d.text_height);
            s.decimals = Some(d.decimals);
        }
        StyleKind::Table => s.header_fill = Some(Rgba::NONE),
        StyleKind::Notes => s.line_spacing = Some(1.8),
        StyleKind::Callout => s.width = Some(st.line_width * 0.7),
        _ => {}
    }
    s
}

/// Category default in effect: built-in overridden by the document's.
pub fn category_style(k: StyleKind, st: &Styles) -> ItemStyle {
    builtin(k, st).merged(st.appearance.get(k))
}

impl Item {
    /// The item's own overrides (empty when none).
    pub fn style_override(&self) -> ItemStyle {
        self.extra_values()
            .and_then(|e| e.iter().find(|v| v.is(APPEARANCE)))
            .map(ItemStyle::from_value)
            .unwrap_or_default()
    }

    /// Replaces the item's overrides (empty removes the node). False for
    /// unknown items.
    pub fn set_style_override(&mut self, s: &ItemStyle) -> bool {
        let Some(e) = self.extra_values_mut() else {
            return false;
        };
        let at = e.iter().position(|v| v.is(APPEARANCE));
        e.retain(|v| !v.is(APPEARANCE));
        if let Some(n) = s.to_node(APPEARANCE) {
            let i = at.unwrap_or(e.len()).min(e.len());
            e.insert(i, n);
        }
        true
    }

    /// Effective style: built-in, document category, item override.
    /// Views get the graphic category.
    pub fn effective_style(&self, st: &Styles) -> ItemStyle {
        let k = StyleKind::of(self).unwrap_or(StyleKind::Graphic);
        let mut e = category_style(k, st).merged(&self.style_override());
        // A text's own height (older documents, `item.add_text --height`)
        // wins over the style.
        if let Item::Graphic(g) = self {
            if let GraphicShape::Text {
                height: Some(h), ..
            } = g.shape
            {
                e.text_height = Some(h);
            }
        }
        e
    }
}

/// Dimension style for one dimension: the document's, with the item's (and
/// the document category's) text height, arrow size, line width and
/// decimals applied.
pub fn dim_style_for(st: &Styles, d: &Dimension) -> DimStyle {
    let it = Item::Dimension(d.clone());
    dim_style_from(st, &it.style_override())
}

/// [`dim_style_for`] from an override set.
pub fn dim_style_from(st: &Styles, over: &ItemStyle) -> DimStyle {
    let e = category_style(StyleKind::Dimension, st).merged(over);
    let mut ds = st.dimension;
    if let Some(h) = e.text_height {
        ds.text_height = h;
    }
    if let Some(a) = e.arrow_size {
        // Keep the document's arrow proportions.
        let ratio = if ds.arrow_length > 0.0 {
            ds.arrow_width / ds.arrow_length
        } else {
            1.0 / 3.0
        };
        ds.arrow_length = a;
        ds.arrow_width = a * ratio;
    }
    if let Some(w) = e.width {
        ds.line_width = w;
    }
    if let Some(d) = e.decimals {
        ds.decimals = d;
    }
    ds
}
