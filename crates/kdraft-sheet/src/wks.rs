//! `.kicad_wks` reader (also the legacy `page_layout` root).

use kdraft_board::geom::Point;
use kdraft_sexpr::{Node, NodeKind, Tree};
use serde::Serialize;

use crate::error::SheetError;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Setup {
    pub text_size: (f64, f64),
    pub line_width: f64,
    pub text_line_width: f64,
    pub left_margin: f64,
    pub right_margin: f64,
    pub top_margin: f64,
    pub bottom_margin: f64,
}

impl Default for Setup {
    fn default() -> Self {
        Setup {
            text_size: (1.5, 1.5),
            line_width: 0.15,
            text_line_width: 0.15,
            left_margin: 10.0,
            right_margin: 10.0,
            top_margin: 10.0,
            bottom_margin: 10.0,
        }
    }
}

/// Origin corner of a coordinate. Coordinates grow from the corner towards
/// the opposite corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Corner {
    #[default]
    RightBottom,
    RightTop,
    LeftBottom,
    LeftTop,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Coord {
    pub x: f64,
    pub y: f64,
    pub corner: Corner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PageOption {
    #[default]
    All,
    Page1Only,
    NotOnPage1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum HJustify {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum VJustify {
    Top,
    #[default]
    Center,
    Bottom,
}

/// Fields common to all items.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Common {
    pub name: Option<String>,
    pub comment: Option<String>,
    pub repeat: u32,
    pub incrx: f64,
    pub incry: f64,
    pub option: PageOption,
    /// `None` uses the setup default.
    pub line_width: Option<f64>,
}

impl Default for Common {
    fn default() -> Self {
        Common {
            name: None,
            comment: None,
            repeat: 1,
            incrx: 0.0,
            incry: 0.0,
            option: PageOption::All,
            line_width: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TextItem {
    pub text: String,
    pub pos: Coord,
    /// `None` uses the setup text size.
    pub size: Option<(f64, f64)>,
    pub bold: bool,
    pub italic: bool,
    pub h_justify: HJustify,
    pub v_justify: VJustify,
    pub rotate: f64,
    pub max_len: Option<f64>,
    pub max_height: Option<f64>,
    pub incr_label: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ItemKind {
    Line {
        start: Coord,
        end: Coord,
    },
    Rect {
        start: Coord,
        end: Coord,
    },
    Text(TextItem),
    Polygon {
        pos: Coord,
        rotate: f64,
        /// Each polygon's corners relative to `pos`.
        polygons: Vec<Vec<Point>>,
    },
    /// Image content is not kept; only its extent.
    Bitmap {
        pos: Coord,
        scale: f64,
        /// Pixel size from the PNG header, if readable.
        pixels: Option<(u32, u32)>,
        ppi: f64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Item {
    pub common: Common,
    pub kind: ItemKind,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct WksFile {
    /// File format version; 0 if absent (legacy `page_layout`).
    pub version: i64,
    pub legacy_root: bool,
    pub setup: Setup,
    pub items: Vec<Item>,
    /// Items or values that were skipped.
    pub warnings: Vec<String>,
}

/// First version with `${VAR}` text; older files use `%` codes.
/// UNVERIFIED: the exact version where KiCad switched (taken as the version
/// of KiCad's own default sheet, 20210606).
pub const VAR_SYNTAX_VERSION: i64 = 20210606;

impl WksFile {
    pub fn parse(text: &str) -> Result<WksFile, SheetError> {
        let tree = Tree::parse(text)?;
        let root = tree.require_root()?;
        let legacy_root = match root.head() {
            Some("kicad_wks") => false,
            Some("page_layout") => true,
            other => {
                return Err(SheetError::NotASheet {
                    found: other.map(str::to_string),
                })
            }
        };
        let mut warnings = Vec::new();
        let version = match root.find("version") {
            Some(v) => v.i64_at(1).unwrap_or_else(|e| {
                warnings.push(format!("bad version: {e}"));
                0
            }),
            None => 0,
        };
        let mut setup = Setup::default();
        let mut items = Vec::new();
        for n in root.lists() {
            match n.head() {
                Some("setup") => read_setup(n, &mut setup, &mut warnings),
                Some("line") | Some("rect") | Some("tbtext") | Some("polygon") | Some("bitmap") => {
                    match read_item(n) {
                        Ok(Some(it)) => items.push(it),
                        Ok(None) => {}
                        Err(e) => warnings.push(format!("skipped {}: {e}", n.head().unwrap_or(""))),
                    }
                }
                _ => {}
            }
        }
        if version < VAR_SYNTAX_VERSION {
            for it in &mut items {
                if let ItemKind::Text(t) = &mut it.kind {
                    t.text = crate::vars::legacy_to_vars(&t.text);
                }
            }
        }
        Ok(WksFile {
            version,
            legacy_root,
            setup,
            items,
            warnings,
        })
    }
}

type R<T> = Result<T, kdraft_sexpr::Error>;

fn num(n: Node<'_>, head: &str, i: usize) -> R<Option<f64>> {
    match n.find(head) {
        Some(c) => c.f64_at(i).map(Some),
        None => Ok(None),
    }
}

fn read_setup(n: Node<'_>, s: &mut Setup, warnings: &mut Vec<String>) {
    let mut get = |head: &str, i: usize| match num(n, head, i) {
        Ok(v) => v,
        Err(e) => {
            warnings.push(format!("setup {head}: {e}"));
            None
        }
    };
    if let (Some(w), Some(h)) = (get("textsize", 1), get("textsize", 2)) {
        s.text_size = (w, h);
    }
    if let Some(v) = get("linewidth", 1) {
        s.line_width = v;
    }
    if let Some(v) = get("textlinewidth", 1) {
        s.text_line_width = v;
    }
    if let Some(v) = get("left_margin", 1) {
        s.left_margin = v;
    }
    if let Some(v) = get("right_margin", 1) {
        s.right_margin = v;
    }
    if let Some(v) = get("top_margin", 1) {
        s.top_margin = v;
    }
    if let Some(v) = get("bottom_margin", 1) {
        s.bottom_margin = v;
    }
}

fn coord(n: Node<'_>) -> R<Coord> {
    let x = n.f64_at(1)?;
    let y = n.f64_at(2)?;
    let corner = if n.has_symbol("ltcorner") {
        Corner::LeftTop
    } else if n.has_symbol("lbcorner") {
        Corner::LeftBottom
    } else if n.has_symbol("rtcorner") {
        Corner::RightTop
    } else {
        Corner::RightBottom
    };
    Ok(Coord { x, y, corner })
}

fn opt_str(n: Node<'_>, head: &str) -> Option<String> {
    n.find(head)
        .and_then(|c| c.child(1))
        .and_then(|a| a.atom())
        .map(|s| s.into_owned())
}

fn read_common(n: Node<'_>) -> R<Common> {
    let mut c = Common {
        name: opt_str(n, "name"),
        comment: opt_str(n, "comment"),
        ..Default::default()
    };
    if let Some(r) = n.find("repeat") {
        c.repeat = r.i64_at(1)?.clamp(1, 10_000) as u32;
    }
    c.incrx = num(n, "incrx", 1)?.unwrap_or(0.0);
    c.incry = num(n, "incry", 1)?.unwrap_or(0.0);
    c.line_width = num(n, "linewidth", 1)?.filter(|w| *w > 0.0);
    if let Some(o) = n.find("option") {
        if o.has_symbol("page1only") {
            c.option = PageOption::Page1Only;
        } else if o.has_symbol("notonpage1") {
            c.option = PageOption::NotOnPage1;
        }
    }
    Ok(c)
}

fn read_item(n: Node<'_>) -> R<Option<Item>> {
    let common = read_common(n)?;
    let kind = match n.head() {
        Some("line") | Some("rect") => {
            let start = coord(n.require("start")?)?;
            let end = coord(n.require("end")?)?;
            if n.is("line") {
                ItemKind::Line { start, end }
            } else {
                ItemKind::Rect { start, end }
            }
        }
        Some("tbtext") => {
            let text = n
                .child(1)
                .filter(|c| c.kind() != NodeKind::List)
                .and_then(|c| c.atom())
                .map(|s| s.into_owned())
                .unwrap_or_default();
            let pos = coord(n.require("pos")?)?;
            let mut t = TextItem {
                text,
                pos,
                size: None,
                bold: false,
                italic: false,
                h_justify: HJustify::Left,
                v_justify: VJustify::Center,
                rotate: num(n, "rotate", 1)?.unwrap_or(0.0),
                max_len: num(n, "maxlen", 1)?.filter(|v| *v > 0.0),
                max_height: num(n, "maxheight", 1)?.filter(|v| *v > 0.0),
                incr_label: n
                    .find("incrlabel")
                    .map(|c| c.i64_at(1))
                    .transpose()?
                    .unwrap_or(1),
            };
            if let Some(f) = n.find("font") {
                if let Some(s) = f.find("size") {
                    t.size = Some((s.f64_at(1)?, s.f64_at(2)?));
                }
                t.bold = f.has_symbol("bold");
                t.italic = f.has_symbol("italic");
            }
            if let Some(j) = n.find("justify") {
                for c in j.children().skip(1) {
                    match c.atom().as_deref() {
                        Some("left") => t.h_justify = HJustify::Left,
                        Some("right") => t.h_justify = HJustify::Right,
                        Some("center") => t.h_justify = HJustify::Center,
                        Some("top") => t.v_justify = VJustify::Top,
                        Some("bottom") => t.v_justify = VJustify::Bottom,
                        _ => {}
                    }
                }
            }
            ItemKind::Text(t)
        }
        Some("polygon") => {
            let pos = coord(n.require("pos")?)?;
            let mut polygons = Vec::new();
            for pts in n.find_all("pts") {
                let mut v = Vec::new();
                for xy in pts.find_all("xy") {
                    v.push(Point::new(xy.f64_at(1)?, xy.f64_at(2)?));
                }
                if v.len() >= 2 {
                    polygons.push(v);
                }
            }
            ItemKind::Polygon {
                pos,
                rotate: num(n, "rotate", 1)?.unwrap_or(0.0),
                polygons,
            }
        }
        Some("bitmap") => {
            let pos = coord(n.require("pos")?)?;
            let pixels = n
                .find("pngdata")
                .or_else(|| n.find("data"))
                .and_then(png_size);
            ItemKind::Bitmap {
                pos,
                scale: num(n, "scale", 1)?.unwrap_or(1.0),
                pixels,
                ppi: num(n, "ppi", 1)?.filter(|p| *p > 0.0).unwrap_or(300.0),
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(Item { common, kind }))
}

/// Width and height from the IHDR of hex-encoded PNG data in `(data ...)`
/// children.
fn png_size(n: Node<'_>) -> Option<(u32, u32)> {
    let mut hex = String::new();
    for d in n.find_all("data") {
        for c in d.children().skip(1) {
            if let Some(a) = c.atom() {
                hex.push_str(&a);
                hex.push(' ');
            }
            if hex.len() > 200 {
                break;
            }
        }
        if hex.len() > 200 {
            break;
        }
    }
    let bytes: Vec<u8> = hex
        .split_whitespace()
        .filter_map(|b| u8::from_str_radix(b, 16).ok())
        .take(24)
        .collect();
    if bytes.len() < 24 || &bytes[1..4] != b"PNG" {
        return None;
    }
    let be = |i: usize| u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    Some((be(16), be(20)))
}
