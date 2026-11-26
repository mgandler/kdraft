//! KiCad colour themes (`.json` files in KiCad's `colors` folder) as
//! layer palettes.
//!
//! Format, from KiCad 10.0.8 `common/settings/color_settings.cpp` and
//! `json_settings.cpp`: nested JSON objects whose paths are the dotted
//! keys `meta.name`, `board.copper.f`, `board.copper.in1` ...
//! `board.copper.in30`, `board.copper.b`, `board.f_silks`, `board.b_silks`,
//! `board.f_mask` ... `board.edge_cuts`, `board.margin`,
//! `board.dwgs_user`, `board.cmts_user`, `board.eco1_user`,
//! `board.eco2_user`, `board.user_1` ... `board.user_45`. Colours are
//! written as `rgb(r, g, b)` or `rgba(r, g, b, a)` (`COLOR4D::
//! ToCSSString`) and read through `#RRGGBB[AA]` or wxWidgets' colour
//! parser (`COLOR4D::SetFromHexString`, `SetFromWxString`); kdraft reads
//! the rgb/rgba and hex forms. A file may hold only some keys: the rest
//! come from KiCad Default, as in KiCad.

use std::path::{Path, PathBuf};

use kdraft_doc::palette::{Palette, KICAD_DEFAULT_NAME};
use kdraft_doc::style::{adapt_for_paper, Rgba, PAPER_CONTRAST};
use serde_json::Value;

use crate::kicad_cli::{Environment, Os};

/// kdraft palette key of a KiCad `board.*` key.
pub fn palette_key(kicad: &str) -> Option<String> {
    let k = kicad.strip_prefix("board.")?;
    if let Some(c) = k.strip_prefix("copper.") {
        return match c {
            "f" => Some("copper.front".into()),
            "b" => Some("copper.back".into()),
            n => {
                let i: u32 = n.strip_prefix("in")?.parse().ok()?;
                (1..=30).contains(&i).then(|| format!("copper.in{i}"))
            }
        };
    }
    if let Some(n) = k.strip_prefix("user_") {
        let i: u32 = n.parse().ok()?;
        return (1..=45).contains(&i).then(|| format!("User.{i}"));
    }
    Some(
        match k {
            "f_silks" => "F.SilkS",
            "b_silks" => "B.SilkS",
            "f_mask" => "F.Mask",
            "b_mask" => "B.Mask",
            "f_paste" => "F.Paste",
            "b_paste" => "B.Paste",
            "f_fab" => "F.Fab",
            "b_fab" => "B.Fab",
            "f_crtyd" => "F.CrtYd",
            "b_crtyd" => "B.CrtYd",
            "f_adhes" => "F.Adhes",
            "b_adhes" => "B.Adhes",
            "edge_cuts" => "Edge.Cuts",
            "margin" => "Margin",
            "dwgs_user" => "Dwgs.User",
            "cmts_user" => "Cmts.User",
            "eco1_user" => "Eco1.User",
            "eco2_user" => "Eco2.User",
            _ => return None,
        }
        .to_string(),
    )
}

/// Parses `rgb(r, g, b)`, `rgba(r, g, b, a)` (a 0..1) and `#RRGGBB[AA]`.
pub fn parse_color(s: &str) -> Option<Rgba> {
    let t = s.trim();
    if t.starts_with('#') {
        return Rgba::parse(t);
    }
    let (inner, alpha) = match t.strip_prefix("rgba(") {
        Some(x) => (x.strip_suffix(')')?, true),
        None => (t.strip_prefix("rgb(")?.strip_suffix(')')?, false),
    };
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if parts.len() != if alpha { 4 } else { 3 } {
        return None;
    }
    let ch = |p: &str| {
        p.parse::<f64>()
            .ok()
            .map(|v| v.clamp(0.0, 255.0).round() as u8)
    };
    let a = if alpha {
        let v = parts[3].parse::<f64>().ok()?;
        (v.clamp(0.0, 1.0) * 255.0).round() as u8
    } else {
        255
    };
    Some(Rgba {
        r: ch(parts[0])?,
        g: ch(parts[1])?,
        b: ch(parts[2])?,
        a,
    })
}

fn walk(prefix: &str, v: &Value, out: &mut Vec<(String, String)>) {
    match v {
        Value::Object(m) => {
            for (k, c) in m {
                let p = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                walk(&p, c, out);
            }
        }
        Value::String(s) => out.push((prefix.to_string(), s.clone())),
        _ => {}
    }
}

/// An imported theme: the palette as read (gaps from KiCad Default) and,
/// when asked, adapted for white paper.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    pub name: String,
    /// As in the file (plus KiCad Default for missing keys).
    pub original: Palette,
    /// What kdraft keeps (adapted or not).
    pub palette: Palette,
    /// Keys found in the file.
    pub from_file: usize,
}

/// Reads a KiCad colour theme from its JSON text. `name` overrides the
/// theme's `meta.name`.
pub fn import_text(text: &str, name: Option<&str>, adapt: bool) -> Result<Imported, String> {
    let v: Value =
        serde_json::from_str(text).map_err(|e| format!("not a KiCad colour theme (JSON: {e})"))?;
    let board = v
        .get("board")
        .filter(|b| b.is_object())
        .ok_or("the file has no \"board\" colours (is it a KiCad colour theme?)")?;
    let mut flat = Vec::new();
    walk("board", board, &mut flat);
    let theme_name = name
        .map(str::to_string)
        .or_else(|| {
            v.get("meta")
                .and_then(|m| m.get("name"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "Imported theme".to_string());
    let mut original = Palette::builtin(KICAD_DEFAULT_NAME).unwrap_or_default();
    original.name = theme_name.clone();
    let mut from_file = 0;
    for (k, c) in &flat {
        if let (Some(key), Some(col)) = (palette_key(k), parse_color(c)) {
            original.colors.insert(key, col);
            from_file += 1;
        }
    }
    if from_file == 0 {
        return Err("the theme's \"board\" section has no layer colours kdraft can use".into());
    }
    let mut palette = original.clone();
    if adapt {
        for c in palette.colors.values_mut() {
            *c = adapt_for_paper(*c, PAPER_CONTRAST);
        }
    }
    Ok(Imported {
        name: theme_name,
        original,
        palette,
        from_file,
    })
}

pub fn import_file(path: &Path, name: Option<&str>, adapt: bool) -> Result<Imported, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    import_text(&text, name, adapt).map_err(|e| format!("{}: {e}", path.display()))
}

/// KiCad's configuration folder (holding version folders such as `10.0`).
pub fn kicad_config_dir(env: &dyn Environment) -> Option<PathBuf> {
    let var = |n: &str| env.var(n).filter(|v| !v.is_empty()).map(PathBuf::from);
    Some(match env.os() {
        Os::Windows => var("APPDATA")?.join("kicad"),
        Os::MacOs => var("HOME")?.join("Library/Preferences/kicad"),
        Os::Linux => var("XDG_CONFIG_HOME")
            .filter(|p| p.is_absolute())
            .or_else(|| var("HOME").map(|h| h.join(".config")))?
            .join("kicad"),
    })
}

/// A theme file found in KiCad's configuration folder.
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeFile {
    /// KiCad version folder (`10.0`).
    pub version: String,
    /// `meta.name`, else the file stem.
    pub name: String,
    pub path: PathBuf,
}

/// Every theme in `<config>/<version>/colors/*.json`, newest version first.
pub fn list_themes(env: &dyn Environment) -> Vec<ThemeFile> {
    let Some(base) = kicad_config_dir(env) else {
        return Vec::new();
    };
    let mut versions: Vec<String> = env
        .dir_names(&base)
        .into_iter()
        .filter(|n| crate::Version::parse(n).is_some())
        .collect();
    versions.sort_by_key(|n| std::cmp::Reverse(crate::Version::parse(n)));
    let mut out = Vec::new();
    for v in versions {
        let dir = base.join(&v).join("colors");
        let mut files: Vec<String> = env
            .dir_names(&dir)
            .into_iter()
            .filter(|n| n.to_ascii_lowercase().ends_with(".json"))
            .collect();
        files.sort();
        for f in files {
            let path = dir.join(&f);
            let name = std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .and_then(|j| j.get("meta")?.get("name")?.as_str().map(str::to_string))
                .unwrap_or_else(|| f.trim_end_matches(".json").to_string());
            out.push(ThemeFile {
                version: v.clone(),
                name,
                path,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/kicad-themes")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn colours() {
        assert_eq!(
            parse_color("rgb(200, 52, 52)"),
            Some(Rgba::rgb(200, 52, 52))
        );
        let c = parse_color("rgba(216, 100, 255, 0.400)").unwrap();
        assert_eq!((c.r, c.a), (216, 102));
        assert_eq!(parse_color("#C83434"), Some(Rgba::rgb(200, 52, 52)));
        assert_eq!(parse_color("#C8343480").unwrap().a, 0x80);
        assert_eq!(parse_color("red"), None);
        assert_eq!(parse_color("rgb(1, 2)"), None);
        assert_eq!(
            palette_key("board.copper.in7").as_deref(),
            Some("copper.in7")
        );
        assert_eq!(palette_key("board.user_12").as_deref(), Some("User.12"));
        assert_eq!(palette_key("board.f_silks").as_deref(), Some("F.SilkS"));
        assert_eq!(palette_key("board.grid"), None);
        assert_eq!(palette_key("schematic.wire"), None);
    }

    #[test]
    fn full_partial_and_bad_themes() {
        // A full theme: every board layer key, no gaps.
        let full = import_text(&fixture("full.json"), None, false).unwrap();
        assert_eq!(full.name, "Full Test Theme");
        assert_eq!(full.from_file, 95);
        assert_eq!(full.palette.layer("F.Cu").unwrap().0, Rgba::rgb(10, 20, 30));
        // A partial theme: missing keys from KiCad Default.
        let part = import_text(&fixture("partial.json"), None, false).unwrap();
        assert_eq!(part.name, "Partial");
        assert_eq!(part.from_file, 3);
        assert_eq!(
            part.palette.layer("F.SilkS").unwrap().0,
            Rgba::rgb(255, 255, 255)
        );
        let (mask, o) = part.palette.layer("F.Mask").unwrap();
        assert_eq!(mask, Rgba::rgb(10, 200, 10));
        assert!((o - 0.5).abs() < 0.01);
        assert_eq!(
            part.palette.layer("B.Cu").unwrap().0,
            Rgba::rgb(77, 127, 196)
        );
        // Adapted: white silkscreen becomes dark enough on paper.
        let ad = import_text(&fixture("partial.json"), Some("Mine"), true).unwrap();
        assert_eq!(ad.palette.name, "Mine");
        let silk = ad.palette.layer("F.SilkS").unwrap().0;
        assert!(kdraft_doc::style::contrast(silk, Rgba::WHITE) >= PAPER_CONTRAST);
        assert_eq!(
            ad.original.layer("F.SilkS").unwrap().0,
            Rgba::rgb(255, 255, 255)
        );
        // Malformed and board-less files.
        assert!(import_text("{ nope", None, true)
            .unwrap_err()
            .contains("JSON"));
        assert!(import_text(&fixture("no-board.json"), None, true)
            .unwrap_err()
            .contains("\"board\""));
    }
}
