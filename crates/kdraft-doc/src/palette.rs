//! Layer colour palettes: a named map from layers to colour and opacity
//! (the colour's alpha). Copper layers go by position (`copper.front`,
//! `copper.in1` ... `copper.back`), technical and user layers by their
//! canonical KiCad names (`F.SilkS`, `Edge.Cuts`, `User.1`), and the
//! native views' roles by `role.<name>` (outline, bodies, designators,
//! pads, holes, dnp; when absent they follow the layers).
//!
//! Built-ins: Black, kdraft layer colours, Greyscale, and KiCad's own
//! "KiCad Default" and "KiCad Classic" board colours, taken from KiCad
//! 10.0.8 `common/settings/builtin_color_themes.h` (GPL-3.0-or-later,
//! see NOTICE); KiCad's legacy colour names resolved through
//! `common/gal/color4d.cpp`.
//!
//! A document's default palette is stored resolved in the document
//! (`(styles (layer_palette (name "...") (color "F.SilkS" "#RRGGBB") ...))`)
//! so a drawing looks the same on a PC without that palette.

use std::collections::BTreeMap;

use kdraft_sexpr::Value;

use crate::layers::{palette_color, to_grey};
use crate::style::Rgba;

/// Head of the document node.
pub const LAYER_PALETTE: &str = "layer_palette";

/// Native-view roles a palette may colour.
pub const ROLES: [&str; 6] = ["outline", "bodies", "designators", "pads", "holes", "dnp"];

/// Technical and user layers by canonical name, in the editor's order.
pub const TECH_LAYERS: [&str; 20] = [
    "F.SilkS",
    "B.SilkS",
    "F.Mask",
    "B.Mask",
    "F.Paste",
    "B.Paste",
    "F.Fab",
    "B.Fab",
    "F.CrtYd",
    "B.CrtYd",
    "F.Adhes",
    "B.Adhes",
    "Edge.Cuts",
    "Margin",
    "Dwgs.User",
    "Cmts.User",
    "Eco1.User",
    "Eco2.User",
    "User.1",
    "User.2",
];

pub const BLACK: &str = "Black";
pub const LAYER: &str = "kdraft layer colours";
pub const GREY: &str = "Greyscale";
pub const KICAD_DEFAULT_NAME: &str = "KiCad Default";
pub const KICAD_CLASSIC_NAME: &str = "KiCad Classic";
/// KiCad's colours adapted for white paper (contrast 3:1, hue kept).
pub const KICAD_DEFAULT_PAPER: &str = "KiCad Default (paper)";
pub const KICAD_CLASSIC_PAPER: &str = "KiCad Classic (paper)";

/// Names of the built-in palettes, factory default first; the paper
/// variants of KiCad's themes before the screen colours.
pub const BUILTIN: [&str; 7] = [
    BLACK,
    LAYER,
    GREY,
    KICAD_DEFAULT_PAPER,
    KICAD_CLASSIC_PAPER,
    KICAD_DEFAULT_NAME,
    KICAD_CLASSIC_NAME,
];

/// The name shown in lists: KiCad's raw themes are marked as screen
/// colours (several are nearly invisible on white paper).
pub fn display_name(name: &str) -> String {
    if name == KICAD_DEFAULT_NAME || name == KICAD_CLASSIC_NAME {
        format!("{name} (screen colours)")
    } else {
        name.to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Palette {
    pub name: String,
    /// Key (see the module docs) to colour; alpha is the opacity.
    pub colors: BTreeMap<String, Rgba>,
}

/// The palette key of a board layer name (`F.Cu` -> `copper.front`).
pub fn key_of(layer: &str) -> String {
    match layer {
        "F.Cu" => "copper.front".into(),
        "B.Cu" => "copper.back".into(),
        n if n.starts_with("In") && n.ends_with(".Cu") => {
            format!("copper.in{}", &n[2..n.len() - 3])
        }
        n => n.to_string(),
    }
}

/// The keys an editor lists: copper front, inner 1..`inner`, back, the
/// technical layers, then the roles.
pub fn editor_keys(inner: usize) -> Vec<String> {
    let mut k = vec!["copper.front".to_string()];
    k.extend((1..=inner).map(|i| format!("copper.in{i}")));
    k.push("copper.back".into());
    k.extend(TECH_LAYERS.iter().map(|s| s.to_string()));
    k.extend(ROLES.iter().map(|r| format!("role.{r}")));
    k
}

/// Human name of a key (`copper.in2` -> `In2.Cu`).
pub fn key_label(key: &str) -> String {
    match key {
        "copper.front" => "F.Cu".into(),
        "copper.back" => "B.Cu".into(),
        k if k.starts_with("copper.in") => format!("In{}.Cu", &k[9..]),
        k if k.starts_with("role.") => format!("{} (native views)", &k[5..]),
        k => k.to_string(),
    }
}

impl Palette {
    pub fn new(name: &str) -> Palette {
        Palette {
            name: name.to_string(),
            colors: BTreeMap::new(),
        }
    }

    /// Colour and opacity of a board layer; inner copper layers beyond the
    /// palette's entries repeat its inner colours.
    pub fn layer(&self, layer: &str) -> Option<(Rgba, f64)> {
        let key = key_of(layer);
        let c = self.colors.get(&key).copied().or_else(|| {
            let n: usize = key.strip_prefix("copper.in")?.parse().ok()?;
            let inner: Vec<Rgba> = (1..=64)
                .map_while(|i| self.colors.get(&format!("copper.in{i}")).copied())
                .collect();
            (!inner.is_empty()).then(|| inner[(n.max(1) - 1) % inner.len()])
        })?;
        Some(split(c))
    }

    /// A role's own colour, if the palette sets one.
    pub fn role(&self, role: &str) -> Option<(Rgba, f64)> {
        self.colors.get(&format!("role.{role}")).copied().map(split)
    }

    /// Whether every colour is opaque black (draws as plotted).
    pub fn is_black(&self) -> bool {
        self.colors.values().all(|c| *c == Rgba::BLACK)
    }

    /// The built-in palette `name`.
    pub fn builtin(name: &str) -> Option<Palette> {
        let mut p = Palette::new(name);
        let keys = editor_keys(30);
        let layer_of = |k: &str| key_label(k);
        match name {
            BLACK => {
                for k in keys.iter().filter(|k| !k.starts_with("role.")) {
                    p.colors.insert(k.clone(), Rgba::BLACK);
                }
            }
            LAYER | GREY => {
                for k in keys.iter().filter(|k| !k.starts_with("role.")) {
                    let c = palette_color(&layer_of(k));
                    p.colors
                        .insert(k.clone(), if name == GREY { to_grey(c) } else { c });
                }
            }
            KICAD_DEFAULT_NAME => {
                for (k, c) in KICAD_DEFAULT {
                    p.colors.insert(
                        (*k).to_string(),
                        Rgba {
                            r: c[0],
                            g: c[1],
                            b: c[2],
                            a: c[3],
                        },
                    );
                }
            }
            KICAD_CLASSIC_NAME => {
                for (k, c) in KICAD_CLASSIC {
                    p.colors.insert(
                        (*k).to_string(),
                        Rgba {
                            r: c[0],
                            g: c[1],
                            b: c[2],
                            a: c[3],
                        },
                    );
                }
            }
            KICAD_DEFAULT_PAPER | KICAD_CLASSIC_PAPER => {
                let raw = if name == KICAD_DEFAULT_PAPER {
                    KICAD_DEFAULT_NAME
                } else {
                    KICAD_CLASSIC_NAME
                };
                let mut q = Palette::builtin(raw)?;
                for c in q.colors.values_mut() {
                    *c = crate::style::adapt_for_paper(*c, crate::style::PAPER_CONTRAST);
                }
                q.name = name.to_string();
                return Some(q);
            }
            _ => return None,
        }
        Some(p)
    }

    /// `(layer_palette (name "...") (color "key" "#RRGGBB[AA]") ...)`.
    pub fn to_value(&self) -> Value {
        let mut o = vec![Value::node("name", [Value::str(&self.name)])];
        for (k, c) in &self.colors {
            o.push(Value::node(
                "color",
                [Value::str(k), Value::str(c.to_string())],
            ));
        }
        Value::node(LAYER_PALETTE, o)
    }

    pub fn from_value(v: &Value) -> Palette {
        let mut p = Palette::default();
        for c in v.args() {
            let a = c.args();
            match c.head() {
                Some("name") => {
                    if let Some(n) = a.first().and_then(Value::atom_text) {
                        p.name = n.to_string();
                    }
                }
                Some("color") => {
                    if let (Some(k), Some(col)) = (
                        a.first().and_then(Value::atom_text),
                        a.get(1).and_then(Value::atom_text).and_then(Rgba::parse),
                    ) {
                        p.colors.insert(k.to_string(), col);
                    }
                }
                _ => {}
            }
        }
        p
    }
}

fn split(c: Rgba) -> (Rgba, f64) {
    (Rgba { a: 255, ..c }, f64::from(c.a) / 255.0)
}

/// Opacity into a colour's alpha.
pub fn join(c: Rgba, opacity: f64) -> Rgba {
    Rgba {
        a: (opacity.clamp(0.0, 1.0) * 255.0).round() as u8,
        ..c
    }
}

// KiCad 10.0.8 built-in board colours (common/settings/builtin_color_themes.h,
// GPL-3.0-or-later, Copyright The KiCad Developers); generated, do not edit.
pub const KICAD_DEFAULT: &[(&str, [u8; 4])] = &[
    ("copper.front", [200, 52, 52, 255]),
    ("copper.in1", [127, 200, 127, 255]),
    ("copper.in2", [206, 125, 44, 255]),
    ("copper.in3", [79, 203, 203, 255]),
    ("copper.in4", [219, 98, 139, 255]),
    ("copper.in5", [167, 165, 198, 255]),
    ("copper.in6", [40, 204, 217, 255]),
    ("copper.in7", [232, 178, 167, 255]),
    ("copper.in8", [242, 237, 161, 255]),
    ("copper.in9", [141, 203, 129, 255]),
    ("copper.in10", [237, 124, 51, 255]),
    ("copper.in11", [91, 195, 235, 255]),
    ("copper.in12", [247, 111, 142, 255]),
    ("copper.in13", [167, 165, 198, 255]),
    ("copper.in14", [40, 204, 217, 255]),
    ("copper.in15", [232, 178, 167, 255]),
    ("copper.in16", [242, 237, 161, 255]),
    ("copper.in17", [237, 124, 51, 255]),
    ("copper.in18", [91, 195, 235, 255]),
    ("copper.in19", [247, 111, 142, 255]),
    ("copper.in20", [167, 165, 198, 255]),
    ("copper.in21", [40, 204, 217, 255]),
    ("copper.in22", [232, 178, 167, 255]),
    ("copper.in23", [242, 237, 161, 255]),
    ("copper.in24", [237, 124, 51, 255]),
    ("copper.in25", [91, 195, 235, 255]),
    ("copper.in26", [247, 111, 142, 255]),
    ("copper.in27", [167, 165, 198, 255]),
    ("copper.in28", [40, 204, 217, 255]),
    ("copper.in29", [232, 178, 167, 255]),
    ("copper.in30", [242, 237, 161, 255]),
    ("copper.back", [77, 127, 196, 255]),
    ("B.Adhes", [0, 0, 132, 255]),
    ("F.Adhes", [132, 0, 132, 255]),
    ("B.Paste", [0, 194, 194, 230]),
    ("F.Paste", [180, 160, 154, 230]),
    ("B.SilkS", [232, 178, 167, 255]),
    ("F.SilkS", [242, 237, 161, 255]),
    ("B.Mask", [2, 255, 238, 102]),
    ("F.Mask", [216, 100, 255, 102]),
    ("Dwgs.User", [194, 194, 194, 255]),
    ("Cmts.User", [89, 148, 220, 255]),
    ("Eco1.User", [180, 219, 210, 255]),
    ("Eco2.User", [216, 200, 82, 255]),
    ("Edge.Cuts", [208, 210, 205, 255]),
    ("Margin", [255, 38, 226, 255]),
    ("B.CrtYd", [38, 233, 255, 255]),
    ("F.CrtYd", [255, 38, 226, 255]),
    ("B.Fab", [88, 93, 132, 255]),
    ("F.Fab", [175, 175, 175, 255]),
    ("User.1", [194, 194, 194, 255]),
    ("User.2", [89, 148, 220, 255]),
    ("User.3", [180, 219, 210, 255]),
    ("User.4", [216, 200, 82, 255]),
    ("User.5", [194, 194, 194, 255]),
    ("User.6", [89, 148, 220, 255]),
    ("User.7", [180, 219, 210, 255]),
    ("User.8", [216, 200, 82, 255]),
    ("User.9", [232, 178, 167, 255]),
    ("User.10", [89, 148, 220, 255]),
    ("User.11", [180, 219, 210, 255]),
    ("User.12", [216, 200, 82, 255]),
    ("User.13", [194, 194, 194, 255]),
    ("User.14", [89, 148, 220, 255]),
    ("User.15", [180, 219, 210, 255]),
    ("User.16", [216, 200, 82, 255]),
    ("User.17", [194, 194, 194, 255]),
    ("User.18", [89, 148, 220, 255]),
    ("User.19", [180, 219, 210, 255]),
    ("User.20", [216, 200, 82, 255]),
    ("User.21", [194, 194, 194, 255]),
    ("User.22", [89, 148, 220, 255]),
    ("User.23", [180, 219, 210, 255]),
    ("User.24", [216, 200, 82, 255]),
    ("User.25", [194, 194, 194, 255]),
    ("User.26", [89, 148, 220, 255]),
    ("User.27", [180, 219, 210, 255]),
    ("User.28", [216, 200, 82, 255]),
    ("User.29", [194, 194, 194, 255]),
    ("User.30", [89, 148, 220, 255]),
    ("User.31", [180, 219, 210, 255]),
    ("User.32", [216, 200, 82, 255]),
    ("User.33", [194, 194, 194, 255]),
    ("User.34", [89, 148, 220, 255]),
    ("User.35", [180, 219, 210, 255]),
    ("User.36", [216, 200, 82, 255]),
    ("User.37", [194, 194, 194, 255]),
    ("User.38", [89, 148, 220, 255]),
    ("User.39", [180, 219, 210, 255]),
    ("User.40", [216, 200, 82, 255]),
    ("User.41", [194, 194, 194, 255]),
    ("User.42", [89, 148, 220, 255]),
    ("User.43", [180, 219, 210, 255]),
    ("User.44", [216, 200, 82, 255]),
    ("User.45", [194, 194, 194, 255]),
];

pub const KICAD_CLASSIC: &[(&str, [u8; 4])] = &[
    ("copper.front", [132, 0, 0, 255]),
    ("copper.in1", [194, 194, 0, 255]),
    ("copper.in2", [194, 0, 194, 255]),
    ("copper.in3", [194, 0, 0, 255]),
    ("copper.in4", [0, 132, 132, 255]),
    ("copper.in5", [0, 132, 0, 255]),
    ("copper.in6", [0, 0, 132, 255]),
    ("copper.in7", [132, 132, 132, 255]),
    ("copper.in8", [132, 0, 132, 255]),
    ("copper.in9", [194, 194, 194, 255]),
    ("copper.in10", [132, 0, 132, 255]),
    ("copper.in11", [132, 0, 0, 255]),
    ("copper.in12", [132, 132, 0, 255]),
    ("copper.in13", [194, 194, 194, 255]),
    ("copper.in14", [0, 0, 132, 255]),
    ("copper.in15", [0, 132, 0, 255]),
    ("copper.in16", [132, 0, 0, 255]),
    ("copper.in17", [194, 194, 0, 255]),
    ("copper.in18", [194, 0, 194, 255]),
    ("copper.in19", [194, 0, 0, 255]),
    ("copper.in20", [0, 132, 132, 255]),
    ("copper.in21", [0, 132, 0, 255]),
    ("copper.in22", [0, 0, 132, 255]),
    ("copper.in23", [132, 132, 132, 255]),
    ("copper.in24", [132, 0, 132, 255]),
    ("copper.in25", [194, 194, 194, 255]),
    ("copper.in26", [132, 0, 132, 255]),
    ("copper.in27", [132, 0, 0, 255]),
    ("copper.in28", [132, 132, 0, 255]),
    ("copper.in29", [194, 194, 194, 255]),
    ("copper.in30", [0, 0, 132, 255]),
    ("copper.back", [0, 132, 0, 255]),
    ("B.Adhes", [0, 0, 132, 255]),
    ("F.Adhes", [132, 0, 132, 255]),
    ("B.Paste", [0, 194, 194, 255]),
    ("F.Paste", [132, 0, 0, 255]),
    ("B.SilkS", [132, 0, 132, 255]),
    ("F.SilkS", [0, 132, 132, 255]),
    ("B.Mask", [132, 132, 0, 255]),
    ("F.Mask", [132, 0, 132, 255]),
    ("Dwgs.User", [194, 194, 194, 255]),
    ("Cmts.User", [0, 0, 132, 255]),
    ("Eco1.User", [0, 132, 0, 255]),
    ("Eco2.User", [194, 194, 0, 255]),
    ("Edge.Cuts", [194, 194, 0, 255]),
    ("Margin", [194, 0, 194, 255]),
    ("B.CrtYd", [132, 132, 132, 255]),
    ("F.CrtYd", [194, 194, 194, 255]),
    ("B.Fab", [0, 0, 132, 255]),
    ("F.Fab", [132, 132, 132, 255]),
    ("User.1", [0, 0, 132, 255]),
    ("User.2", [0, 0, 132, 255]),
    ("User.3", [0, 0, 132, 255]),
    ("User.4", [0, 0, 132, 255]),
    ("User.5", [0, 0, 132, 255]),
    ("User.6", [0, 0, 132, 255]),
    ("User.7", [0, 0, 132, 255]),
    ("User.8", [0, 0, 132, 255]),
    ("User.9", [0, 0, 132, 255]),
    ("User.10", [0, 0, 132, 255]),
    ("User.11", [0, 0, 132, 255]),
    ("User.12", [0, 0, 132, 255]),
    ("User.13", [0, 0, 132, 255]),
    ("User.14", [0, 0, 132, 255]),
    ("User.15", [0, 0, 132, 255]),
    ("User.16", [0, 0, 132, 255]),
    ("User.17", [0, 0, 132, 255]),
    ("User.18", [0, 0, 132, 255]),
    ("User.19", [0, 0, 132, 255]),
    ("User.20", [0, 0, 132, 255]),
    ("User.21", [0, 0, 132, 255]),
    ("User.22", [0, 0, 132, 255]),
    ("User.23", [0, 0, 132, 255]),
    ("User.24", [0, 0, 132, 255]),
    ("User.25", [0, 0, 132, 255]),
    ("User.26", [0, 0, 132, 255]),
    ("User.27", [0, 0, 132, 255]),
    ("User.28", [0, 0, 132, 255]),
    ("User.29", [0, 0, 132, 255]),
    ("User.30", [0, 0, 132, 255]),
    ("User.31", [0, 0, 132, 255]),
    ("User.32", [0, 0, 132, 255]),
    ("User.33", [0, 0, 132, 255]),
    ("User.34", [0, 0, 132, 255]),
    ("User.35", [0, 0, 132, 255]),
    ("User.36", [0, 0, 132, 255]),
    ("User.37", [0, 0, 132, 255]),
    ("User.38", [0, 0, 132, 255]),
    ("User.39", [0, 0, 132, 255]),
    ("User.40", [0, 0, 132, 255]),
    ("User.41", [0, 0, 132, 255]),
    ("User.42", [0, 0, 132, 255]),
    ("User.43", [0, 0, 132, 255]),
    ("User.44", [0, 0, 132, 255]),
    ("User.45", [0, 0, 132, 255]),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{adapt_for_paper, contrast, to_hsl, PAPER_CONTRAST};
    use proptest::prelude::*;

    proptest! {
        /// Adapted colours reach the contrast against white and keep their
        /// hue (for colours with enough saturation to have one).
        #[test]
        fn adapt_reaches_contrast_and_keeps_hue(r in 0u8..=255, g in 0u8..=255, b in 0u8..=255) {
            let c = Rgba::rgb(r, g, b);
            let a = adapt_for_paper(c, PAPER_CONTRAST);
            prop_assert!(contrast(a, Rgba::WHITE) >= PAPER_CONTRAST - 1e-9, "{c} -> {a}");
            if contrast(c, Rgba::WHITE) >= PAPER_CONTRAST {
                prop_assert_eq!(a, c);
            } else {
                let (h0, s0, _) = to_hsl(c);
                let (h1, s1, _) = to_hsl(a);
                if s0 > 0.25 && s1 > 0.25 {
                    let d = (h0 - h1).abs();
                    prop_assert!(d.min(360.0 - d) < 8.0, "hue {h0} -> {h1} ({c} -> {a})");
                }
            }
        }
    }

    #[test]
    fn paper_variants_are_visible_on_paper() {
        for n in [KICAD_DEFAULT_PAPER, KICAD_CLASSIC_PAPER] {
            let p = Palette::builtin(n).unwrap();
            assert_eq!(p.name, n);
            for (k, c) in &p.colors {
                let opaque = Rgba { a: 255, ..*c };
                assert!(
                    contrast(opaque, Rgba::WHITE) >= PAPER_CONTRAST,
                    "{n} {k} {c}"
                );
            }
        }
        assert_eq!(
            display_name(KICAD_DEFAULT_NAME),
            "KiCad Default (screen colours)"
        );
        assert_eq!(display_name(KICAD_DEFAULT_PAPER), KICAD_DEFAULT_PAPER);
    }

    #[test]
    fn builtins_and_lookup() {
        for n in BUILTIN {
            let p = Palette::builtin(n).unwrap();
            assert!(p.layer("F.Cu").is_some(), "{n}");
            assert!(p.layer("Edge.Cuts").is_some(), "{n}");
            assert!(p.layer("In7.Cu").is_some(), "{n}");
        }
        let k = Palette::builtin(KICAD_DEFAULT_NAME).unwrap();
        assert_eq!(k.layer("F.Cu"), Some((Rgba::rgb(200, 52, 52), 1.0)));
        let (c, o) = k.layer("F.Mask").unwrap();
        assert_eq!(c, Rgba::rgb(216, 100, 255));
        assert!((o - 0.4).abs() < 0.01);
        // Classic: legacy names (RED = 132, 0, 0).
        let c = Palette::builtin(KICAD_CLASSIC_NAME).unwrap();
        assert_eq!(c.layer("F.Cu").unwrap().0, Rgba::rgb(132, 0, 0));
        assert!(Palette::builtin(BLACK).unwrap().is_black());
        assert_eq!(key_of("In12.Cu"), "copper.in12");
        assert_eq!(key_label("copper.in12"), "In12.Cu");
        // Inner copper beyond the entries repeats.
        let mut p = Palette::new("x");
        p.colors.insert("copper.in1".into(), Rgba::rgb(1, 1, 1));
        p.colors.insert("copper.in2".into(), Rgba::rgb(2, 2, 2));
        assert_eq!(p.layer("In3.Cu").unwrap().0, Rgba::rgb(1, 1, 1));
        let v = k.to_value();
        assert_eq!(Palette::from_value(&v), k);
    }
}
