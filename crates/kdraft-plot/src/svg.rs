//! Generic SVG geometry reader.
//!
//! Reads any SVG 1.1 document, not a particular producer's dialect: root
//! `width`/`height` with units and `viewBox`, nested `transform`s, inherited
//! presentation attributes and `style`, and the basic shapes plus `path`
//! (all commands, arcs converted to cubics) and `use`. All geometry is
//! returned in root user units (the `viewBox` coordinate system).
//! `text` elements are counted but contribute no geometry.

use std::collections::{BTreeMap, HashMap};

use kdraft_board::geom::{Affine, BBox, Point};
use serde::Serialize;

use crate::error::PlotError;
use crate::path::{arc_to_cubics, bbox_of, ellipse, Seg, SubPath};

/// Millimetres per unit for absolute CSS units.
fn unit_mm(unit: &str) -> Option<f64> {
    Some(match unit {
        "mm" => 1.0,
        "cm" => 10.0,
        "in" => 25.4,
        "pt" => 25.4 / 72.0,
        "pc" => 25.4 / 6.0,
        "px" | "" => 25.4 / 96.0,
        _ => return None,
    })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Length {
    pub value: f64,
    /// As written: `mm`, `in`, `px`, ..., empty for a bare number.
    pub unit: String,
}

impl Length {
    pub fn parse(s: &str) -> Option<Length> {
        let s = s.trim();
        let end = s
            .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')))
            .unwrap_or(s.len());
        // "1e" followed by a unit letter would be ambiguous; units never
        // start with 'e' except "em"/"ex", which are not absolute anyway.
        let value: f64 = s[..end].parse().ok()?;
        Some(Length {
            value,
            unit: s[end..].trim().to_string(),
        })
    }

    pub fn to_mm(&self) -> Option<f64> {
        unit_mm(&self.unit).map(|f| self.value * f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ViewBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const BLACK: Rgb = Rgb { r: 0, g: 0, b: 0 };
    pub const WHITE: Rgb = Rgb {
        r: 255,
        g: 255,
        b: 255,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Paint {
    pub color: Rgb,
    pub opacity: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LineCap {
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LineJoin {
    Miter,
    Round,
    Bevel,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StrokeStyle {
    pub paint: Paint,
    /// In root user units (scaled by the element's transform).
    pub width: f64,
    pub cap: LineCap,
    pub join: LineJoin,
    pub dash: Option<Vec<f64>>,
}

/// One drawn element, geometry in root user units.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Shape {
    pub element: String,
    pub subpaths: Vec<SubPath>,
    pub fill: Option<Paint>,
    pub fill_evenodd: bool,
    pub stroke: Option<StrokeStyle>,
}

impl Shape {
    pub fn bbox(&self) -> Option<BBox> {
        bbox_of(&self.subpaths)
    }

    /// Box including half the stroke width.
    pub fn ink_bbox(&self) -> Option<BBox> {
        let b = self.bbox()?;
        let h = self.stroke.as_ref().map_or(0.0, |s| s.width / 2.0);
        Some(BBox {
            min: Point::new(b.min.x - h, b.min.y - h),
            max: Point::new(b.max.x + h, b.max.y + h),
        })
    }

    /// A white (or fully transparent) fill without a visible stroke: a
    /// background or knock-out, not plotted content.
    pub fn is_background(&self) -> bool {
        let fill_blank = self
            .fill
            .is_none_or(|f| f.color == Rgb::WHITE || f.opacity == 0.0);
        let stroke_blank = self
            .stroke
            .as_ref()
            .is_none_or(|s| s.paint.color == Rgb::WHITE || s.paint.opacity == 0.0);
        fill_blank && stroke_blank
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SvgDoc {
    pub width: Option<Length>,
    pub height: Option<Length>,
    pub view_box: Option<ViewBox>,
    /// Root user units to physical millimetres on the page.
    pub user_to_mm: Affine,
    /// False if the root size had no absolute unit and CSS px were assumed.
    pub physical_size: bool,
    pub shapes: Vec<Shape>,
    pub text_elements: usize,
    /// Element names that were not drawn, with counts.
    pub skipped: BTreeMap<String, usize>,
    pub warnings: Vec<String>,
}

impl SvgDoc {
    /// Visible, non-background shapes.
    pub fn content(&self) -> impl Iterator<Item = &Shape> {
        self.shapes.iter().filter(|s| !s.is_background())
    }

    /// Content plus white knock-outs (drill marks inside pads, slots), in
    /// document order: everything but blank shapes the size of the page.
    pub fn ink(&self) -> impl Iterator<Item = &Shape> {
        let page = self.view_box.map(|v| v.w * v.h);
        self.shapes.iter().filter(move |s| {
            if !s.is_background() {
                return true;
            }
            let painted = s.fill.is_some_and(|f| f.opacity > 0.0)
                || s.stroke.as_ref().is_some_and(|st| st.paint.opacity > 0.0);
            let big = match (page, s.bbox()) {
                (Some(a), Some(b)) => b.width() * b.height() >= 0.5 * a,
                _ => true,
            };
            painted && !big
        })
    }

    /// Box of the content geometry (centre lines), root user units.
    pub fn content_bbox(&self) -> Option<BBox> {
        self.content()
            .filter_map(Shape::bbox)
            .reduce(|a, b| a.union(&b))
    }

    /// Box of the content including half stroke widths.
    pub fn content_ink_bbox(&self) -> Option<BBox> {
        self.content()
            .filter_map(Shape::ink_bbox)
            .reduce(|a, b| a.union(&b))
    }

    /// Mean size of one user unit in mm.
    pub fn mm_per_user_unit(&self) -> f64 {
        self.user_to_mm.mean_scale()
    }
}

pub fn parse(text: &str) -> Result<SvgDoc, PlotError> {
    let opts = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let xml = roxmltree::Document::parse_with_options(text, opts)
        .map_err(|e| PlotError::Xml(e.to_string()))?;
    let root = xml.root_element();
    if root.tag_name().name() != "svg" {
        return Err(PlotError::NotSvg(root.tag_name().name().to_string()));
    }
    let width = root.attribute("width").and_then(Length::parse);
    let height = root.attribute("height").and_then(Length::parse);
    let view_box = root.attribute("viewBox").and_then(|v| {
        let n = numbers(v);
        (n.len() == 4 && n[2] > 0.0 && n[3] > 0.0).then(|| ViewBox {
            x: n[0],
            y: n[1],
            w: n[2],
            h: n[3],
        })
    });
    let mut warnings = Vec::new();
    let (user_to_mm, physical_size) = root_mapping(
        &width,
        &height,
        view_box,
        root.attribute("preserveAspectRatio"),
        &mut warnings,
    );

    let mut ids = HashMap::new();
    for n in xml.descendants().filter(|n| n.is_element()) {
        if let Some(id) = n.attribute("id") {
            ids.insert(id.to_string(), n);
        }
    }
    let mut r = Reader {
        ids,
        shapes: Vec::new(),
        text_elements: 0,
        skipped: BTreeMap::new(),
        warnings,
    };
    let state = Style::initial();
    // The root's own transform attribute (not valid SVG 1.1 but common).
    let m = parse_transform(root.attribute("transform").unwrap_or(""));
    r.children(root, &m, &state, 0);
    Ok(SvgDoc {
        width,
        height,
        view_box,
        user_to_mm,
        physical_size,
        shapes: r.shapes,
        text_elements: r.text_elements,
        skipped: r.skipped,
        warnings: r.warnings,
    })
}

fn root_mapping(
    width: &Option<Length>,
    height: &Option<Length>,
    vb: Option<ViewBox>,
    par: Option<&str>,
    warnings: &mut Vec<String>,
) -> (Affine, bool) {
    let px = 25.4 / 96.0;
    let w_mm = width.as_ref().and_then(Length::to_mm);
    let h_mm = height.as_ref().and_then(Length::to_mm);
    let physical = [width, height].iter().all(|l| {
        l.as_ref()
            .is_some_and(|l| !l.unit.is_empty() && l.unit != "px" && l.to_mm().is_some())
    });
    for l in [width, height].into_iter().flatten() {
        if l.to_mm().is_none() {
            warnings.push(format!("root size unit {:?} is not absolute", l.unit));
        }
    }
    let Some(vb) = vb else {
        // No viewBox: user units are CSS px.
        return (Affine::scale(px, px), false);
    };
    let (sx, sy) = match (w_mm, h_mm) {
        (Some(w), Some(h)) => (w / vb.w, h / vb.h),
        (Some(w), None) => (w / vb.w, w / vb.w),
        (None, Some(h)) => (h / vb.h, h / vb.h),
        (None, None) => (px, px),
    };
    let par = par.unwrap_or("xMidYMid meet").trim();
    if par.starts_with("none") {
        return (
            Affine::new(sx, 0.0, 0.0, sy, -vb.x * sx, -vb.y * sy),
            physical,
        );
    }
    let slice = par.ends_with("slice");
    let s = if slice { sx.max(sy) } else { sx.min(sy) };
    let (vw, vh) = (vb.w * sx, vb.h * sy);
    let align = par.split_whitespace().next().unwrap_or("xMidYMid");
    let fx = if align.starts_with("xMin") {
        0.0
    } else if align.starts_with("xMax") {
        1.0
    } else {
        0.5
    };
    let fy = if align.contains("YMin") {
        0.0
    } else if align.contains("YMax") {
        1.0
    } else {
        0.5
    };
    let tx = fx * (vw - vb.w * s);
    let ty = fy * (vh - vb.h * s);
    if (sx - sy).abs() > 1e-9 * sx.abs().max(sy.abs()) {
        warnings.push(format!(
            "viewBox aspect differs from width/height (x {sx} mm/unit, y {sy} mm/unit); preserveAspectRatio {par:?} applied"
        ));
    }
    (
        Affine::new(s, 0.0, 0.0, s, tx - vb.x * s, ty - vb.y * s),
        physical,
    )
}

/// Inherited presentation state.
#[derive(Debug, Clone)]
struct Style {
    fill: Option<Rgb>,
    fill_opacity: f64,
    evenodd: bool,
    stroke: Option<Rgb>,
    stroke_opacity: f64,
    stroke_width: f64,
    cap: LineCap,
    join: LineJoin,
    dash: Option<Vec<f64>>,
    /// Product of ancestor `opacity` values (not inherited in CSS, but it
    /// composes multiplicatively).
    opacity: f64,
    hidden: bool,
}

impl Style {
    fn initial() -> Style {
        Style {
            fill: Some(Rgb::BLACK),
            fill_opacity: 1.0,
            evenodd: false,
            stroke: None,
            stroke_opacity: 1.0,
            stroke_width: 1.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            dash: None,
            opacity: 1.0,
            hidden: false,
        }
    }

    fn apply(&mut self, node: roxmltree::Node<'_, '_>, warnings: &mut Vec<String>) {
        let mut props: Vec<(String, String)> = Vec::new();
        for a in node.attributes() {
            props.push((a.name().to_string(), a.value().to_string()));
        }
        if let Some(style) = node.attribute("style") {
            for decl in style.split(';') {
                if let Some((k, v)) = decl.split_once(':') {
                    props.push((k.trim().to_string(), v.trim().to_string()));
                }
            }
        }
        for (k, v) in props {
            let v = v.trim();
            match k.as_str() {
                "fill" => self.fill = paint(v, warnings),
                "fill-opacity" => self.fill_opacity = opacity(v),
                "fill-rule" => self.evenodd = v == "evenodd",
                "stroke" => self.stroke = paint(v, warnings),
                "stroke-opacity" => self.stroke_opacity = opacity(v),
                "stroke-width" => {
                    if let Some(l) = Length::parse(v) {
                        self.stroke_width = l.value;
                    }
                }
                "stroke-linecap" => {
                    self.cap = match v {
                        "round" => LineCap::Round,
                        "square" => LineCap::Square,
                        _ => LineCap::Butt,
                    }
                }
                "stroke-linejoin" => {
                    self.join = match v {
                        "round" => LineJoin::Round,
                        "bevel" => LineJoin::Bevel,
                        _ => LineJoin::Miter,
                    }
                }
                "stroke-dasharray" => {
                    let n = numbers(v);
                    self.dash =
                        (v != "none" && !n.is_empty() && n.iter().any(|x| *x > 0.0)).then_some(n);
                }
                "opacity" => self.opacity *= opacity(v),
                "display" if v == "none" => self.hidden = true,
                "visibility" => self.hidden = v == "hidden" || v == "collapse",
                _ => {}
            }
        }
    }
}

fn opacity(v: &str) -> f64 {
    let v = v.trim();
    let n = if let Some(p) = v.strip_suffix('%') {
        p.trim().parse::<f64>().map(|x| x / 100.0)
    } else {
        v.parse::<f64>()
    };
    n.unwrap_or(1.0).clamp(0.0, 1.0)
}

/// `None` for `none`; colours as RGB; unknown values become black.
fn paint(v: &str, warnings: &mut Vec<String>) -> Option<Rgb> {
    let v = v.trim();
    if v == "none" || v == "transparent" {
        return None;
    }
    if let Some(c) = color(v) {
        return Some(c);
    }
    if warnings.len() < 50 {
        warnings.push(format!("unsupported paint {v:?}, using black"));
    }
    Some(Rgb::BLACK)
}

pub fn color(v: &str) -> Option<Rgb> {
    let v = v.trim();
    if let Some(hex) = v.strip_prefix('#') {
        let d = |s: &str| u8::from_str_radix(s, 16).ok();
        return match hex.len() {
            3 => {
                let c: Vec<u8> = hex
                    .chars()
                    .map(|c| d(&c.to_string()).map(|x| x * 17))
                    .collect::<Option<_>>()?;
                Some(Rgb {
                    r: c[0],
                    g: c[1],
                    b: c[2],
                })
            }
            6 => Some(Rgb {
                r: d(&hex[0..2])?,
                g: d(&hex[2..4])?,
                b: d(&hex[4..6])?,
            }),
            _ => None,
        };
    }
    if let Some(inner) = v.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        if parts.len() != 3 {
            return None;
        }
        let comp = |s: &str| -> Option<u8> {
            if let Some(p) = s.strip_suffix('%') {
                let x: f64 = p.parse().ok()?;
                Some((x.clamp(0.0, 100.0) * 2.55).round() as u8)
            } else {
                let x: f64 = s.parse().ok()?;
                Some(x.clamp(0.0, 255.0).round() as u8)
            }
        };
        return Some(Rgb {
            r: comp(parts[0])?,
            g: comp(parts[1])?,
            b: comp(parts[2])?,
        });
    }
    let named = |r, g, b| Some(Rgb { r, g, b });
    match v.to_ascii_lowercase().as_str() {
        "black" | "currentcolor" => named(0, 0, 0),
        "white" => named(255, 255, 255),
        "red" => named(255, 0, 0),
        "green" => named(0, 128, 0),
        "lime" => named(0, 255, 0),
        "blue" => named(0, 0, 255),
        "yellow" => named(255, 255, 0),
        "gray" | "grey" => named(128, 128, 128),
        _ => None,
    }
}

/// Numbers in an attribute value (separators: whitespace and commas).
pub fn numbers(s: &str) -> Vec<f64> {
    let mut t = Tokens::new(s);
    let mut v = Vec::new();
    while let Some(n) = t.number() {
        v.push(n);
    }
    v
}

/// Parses a `transform` attribute; malformed parts are ignored.
pub fn parse_transform(s: &str) -> Affine {
    let mut m = Affine::IDENTITY;
    let mut rest = s.trim();
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim().trim_start_matches(',').trim();
        let Some(close) = rest[open..].find(')') else {
            break;
        };
        let args = numbers(&rest[open + 1..open + close]);
        rest = &rest[open + close + 1..];
        let t = match (name, args.as_slice()) {
            ("matrix", [a, b, c, d, e, f]) => Affine::new(*a, *b, *c, *d, *e, *f),
            ("translate", [x]) => Affine::translate(*x, 0.0),
            ("translate", [x, y]) => Affine::translate(*x, *y),
            ("scale", [s]) => Affine::scale(*s, *s),
            ("scale", [x, y]) => Affine::scale(*x, *y),
            ("rotate", [a]) => Affine::rotate_deg(*a),
            ("rotate", [a, cx, cy]) => Affine::translate(-cx, -cy)
                .then(&Affine::rotate_deg(*a))
                .then(&Affine::translate(*cx, *cy)),
            ("skewX", [a]) => Affine::new(1.0, 0.0, a.to_radians().tan(), 1.0, 0.0, 0.0),
            ("skewY", [a]) => Affine::new(1.0, a.to_radians().tan(), 0.0, 1.0, 0.0, 0.0),
            _ => continue,
        };
        // SVG lists apply right to left: the rightmost acts first.
        m = t.then(&m);
    }
    m
}

struct Reader<'a, 'i> {
    ids: HashMap<String, roxmltree::Node<'a, 'i>>,
    shapes: Vec<Shape>,
    text_elements: usize,
    skipped: BTreeMap<String, usize>,
    warnings: Vec<String>,
}

const MAX_USE_DEPTH: usize = 16;

impl<'a, 'i> Reader<'a, 'i> {
    fn children(&mut self, node: roxmltree::Node<'a, 'i>, m: &Affine, st: &Style, depth: usize) {
        for c in node.children().filter(|c| c.is_element()) {
            self.element(c, m, st, depth);
        }
    }

    fn skip(&mut self, name: &str) {
        *self.skipped.entry(name.to_string()).or_insert(0) += 1;
    }

    fn element(&mut self, n: roxmltree::Node<'a, 'i>, parent: &Affine, st: &Style, depth: usize) {
        let name = n.tag_name().name();
        let mut st = st.clone();
        st.apply(n, &mut self.warnings);
        let local = parse_transform(n.attribute("transform").unwrap_or(""));
        let m = local.then(parent);
        if st.hidden || st.opacity == 0.0 {
            self.skip(name);
            return;
        }
        let f = |k: &str| {
            n.attribute(k)
                .and_then(Length::parse)
                .map_or(0.0, |l| l.value)
        };
        let subpaths: Vec<SubPath> = match name {
            "g" | "a" | "switch" => {
                self.children(n, &m, &st, depth);
                return;
            }
            "svg" => {
                // Nested viewport: honour x/y, ignore its viewBox clipping.
                let m = Affine::translate(f("x"), f("y")).then(&m);
                self.children(n, &m, &st, depth);
                return;
            }
            "use" => {
                let href = n
                    .attribute(("http://www.w3.org/1999/xlink", "href"))
                    .or_else(|| n.attribute("href"))
                    .unwrap_or("");
                let target = href
                    .strip_prefix('#')
                    .and_then(|id| self.ids.get(id))
                    .copied();
                match target {
                    Some(t) if depth < MAX_USE_DEPTH => {
                        let m = Affine::translate(f("x"), f("y")).then(&m);
                        if matches!(t.tag_name().name(), "symbol") {
                            self.children(t, &m, &st, depth + 1);
                        } else {
                            self.element(t, &m, &st, depth + 1);
                        }
                    }
                    _ => {
                        self.warnings.push(format!("unresolved use href {href:?}"));
                    }
                }
                return;
            }
            "text" => {
                self.text_elements += 1;
                return;
            }
            "path" => path_data(n.attribute("d").unwrap_or("")),
            "rect" => {
                let (x, y, w, h) = (f("x"), f("y"), f("width"), f("height"));
                if w <= 0.0 || h <= 0.0 {
                    return;
                }
                let rx_attr = n.attribute("rx").and_then(Length::parse).map(|l| l.value);
                let ry_attr = n.attribute("ry").and_then(Length::parse).map(|l| l.value);
                let rx = rx_attr.or(ry_attr).unwrap_or(0.0).min(w / 2.0);
                let ry = ry_attr.or(rx_attr).unwrap_or(0.0).min(h / 2.0);
                vec![rect_path(x, y, w, h, rx, ry)]
            }
            "circle" => {
                let r = f("r");
                if r <= 0.0 {
                    return;
                }
                vec![ellipse(Point::new(f("cx"), f("cy")), r, r)]
            }
            "ellipse" => {
                let (rx, ry) = (f("rx"), f("ry"));
                if rx <= 0.0 || ry <= 0.0 {
                    return;
                }
                vec![ellipse(Point::new(f("cx"), f("cy")), rx, ry)]
            }
            "line" => {
                let mut sp = SubPath::new(Point::new(f("x1"), f("y1")));
                sp.segs.push(Seg::Line {
                    to: Point::new(f("x2"), f("y2")),
                });
                vec![sp]
            }
            "polyline" | "polygon" => {
                let v = numbers(n.attribute("points").unwrap_or(""));
                let pts: Vec<Point> = v.chunks_exact(2).map(|c| Point::new(c[0], c[1])).collect();
                let Some((first, rest)) = pts.split_first() else {
                    return;
                };
                let mut sp = SubPath::new(*first);
                sp.segs = rest.iter().map(|p| Seg::Line { to: *p }).collect();
                sp.closed = name == "polygon";
                vec![sp]
            }
            _ => {
                self.skip(name);
                return;
            }
        };
        if subpaths.is_empty() {
            return;
        }
        let subpaths: Vec<SubPath> = subpaths.iter().map(|s| s.transform(&m)).collect();
        let is_line_like = matches!(name, "line" | "polyline");
        let fill = if is_line_like {
            None
        } else {
            st.fill.map(|color| Paint {
                color,
                opacity: st.fill_opacity * st.opacity,
            })
        };
        let stroke = st.stroke.and_then(|color| {
            (st.stroke_width > 0.0).then(|| StrokeStyle {
                paint: Paint {
                    color,
                    opacity: st.stroke_opacity * st.opacity,
                },
                width: st.stroke_width * m.mean_scale(),
                cap: st.cap,
                join: st.join,
                dash: st.dash.clone(),
            })
        });
        if fill.is_none() && stroke.is_none() {
            self.skip(name);
            return;
        }
        self.shapes.push(Shape {
            element: name.to_string(),
            subpaths,
            fill,
            fill_evenodd: st.evenodd,
            stroke,
        });
    }
}

fn rect_path(x: f64, y: f64, w: f64, h: f64, rx: f64, ry: f64) -> SubPath {
    let p = Point::new;
    if rx <= 0.0 || ry <= 0.0 {
        let mut sp = SubPath::new(p(x, y));
        sp.segs = vec![
            Seg::Line { to: p(x + w, y) },
            Seg::Line {
                to: p(x + w, y + h),
            },
            Seg::Line { to: p(x, y + h) },
        ];
        sp.closed = true;
        return sp;
    }
    let k = 0.552_284_749_830_793_6;
    let (kx, ky) = (k * rx, k * ry);
    let mut sp = SubPath::new(p(x + rx, y));
    let segs = vec![
        Seg::Line {
            to: p(x + w - rx, y),
        },
        Seg::Cubic {
            c1: p(x + w - rx + kx, y),
            c2: p(x + w, y + ry - ky),
            to: p(x + w, y + ry),
        },
        Seg::Line {
            to: p(x + w, y + h - ry),
        },
        Seg::Cubic {
            c1: p(x + w, y + h - ry + ky),
            c2: p(x + w - rx + kx, y + h),
            to: p(x + w - rx, y + h),
        },
        Seg::Line {
            to: p(x + rx, y + h),
        },
        Seg::Cubic {
            c1: p(x + rx - kx, y + h),
            c2: p(x, y + h - ry + ky),
            to: p(x, y + h - ry),
        },
        Seg::Line { to: p(x, y + ry) },
        Seg::Cubic {
            c1: p(x, y + ry - ky),
            c2: p(x + rx - kx, y),
            to: p(x + rx, y),
        },
    ];
    sp.segs = segs;
    sp.closed = true;
    sp
}

/// Tokeniser for path data and number lists.
struct Tokens<'s> {
    s: &'s [u8],
    i: usize,
}

impl<'s> Tokens<'s> {
    fn new(s: &'s str) -> Self {
        Tokens {
            s: s.as_bytes(),
            i: 0,
        }
    }

    fn skip_sep(&mut self) {
        while self.i < self.s.len()
            && (self.s[self.i].is_ascii_whitespace() || self.s[self.i] == b',')
        {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_sep();
        self.s.get(self.i).copied()
    }

    fn command(&mut self) -> Option<u8> {
        match self.peek() {
            Some(c) if c.is_ascii_alphabetic() && c != b'e' && c != b'E' => {
                self.i += 1;
                Some(c)
            }
            _ => None,
        }
    }

    fn number(&mut self) -> Option<f64> {
        self.skip_sep();
        let start = self.i;
        let s = self.s;
        let mut j = self.i;
        if j < s.len() && (s[j] == b'+' || s[j] == b'-') {
            j += 1;
        }
        let mut digits = 0;
        while j < s.len() && s[j].is_ascii_digit() {
            j += 1;
            digits += 1;
        }
        if j < s.len() && s[j] == b'.' {
            j += 1;
            while j < s.len() && s[j].is_ascii_digit() {
                j += 1;
                digits += 1;
            }
        }
        if digits == 0 {
            return None;
        }
        if j < s.len() && (s[j] == b'e' || s[j] == b'E') {
            let mut k = j + 1;
            if k < s.len() && (s[k] == b'+' || s[k] == b'-') {
                k += 1;
            }
            let exp_start = k;
            while k < s.len() && s[k].is_ascii_digit() {
                k += 1;
            }
            if k > exp_start {
                j = k;
            }
        }
        let text = std::str::from_utf8(&s[start..j]).ok()?;
        let v = text.parse().ok()?;
        self.i = j;
        Some(v)
    }

    /// Arc flags may be written without separators ("a1 1 0 01 5 5").
    fn flag(&mut self) -> Option<bool> {
        match self.peek()? {
            b'0' => {
                self.i += 1;
                Some(false)
            }
            b'1' => {
                self.i += 1;
                Some(true)
            }
            _ => None,
        }
    }
}

/// Parses path data into subpaths (local coordinates). Parsing stops at the
/// first error, keeping what was read (as the SVG spec requires).
pub fn path_data(d: &str) -> Vec<SubPath> {
    let mut t = Tokens::new(d);
    let mut out: Vec<SubPath> = Vec::new();
    let mut cur = Point::ORIGIN;
    let mut start = Point::ORIGIN;
    let mut last_ctrl: Option<(u8, Point)> = None;
    let mut cmd: Option<u8> = None;
    fn current(out: &mut Vec<SubPath>, at: Point) -> &mut SubPath {
        if out.last().is_none_or(|s| s.closed) {
            out.push(SubPath::new(at));
        }
        let n = out.len() - 1;
        &mut out[n]
    }
    loop {
        let c = match t.command() {
            Some(c) => c,
            None => match (cmd, t.peek()) {
                (Some(c), Some(_)) => match c {
                    b'M' => b'L',
                    b'm' => b'l',
                    c => c,
                },
                _ => break,
            },
        };
        cmd = Some(c);
        let rel = c.is_ascii_lowercase();
        let base = if rel { cur } else { Point::ORIGIN };
        let pt = |t: &mut Tokens| -> Option<Point> {
            let x = t.number()?;
            let y = t.number()?;
            Some(Point::new(base.x + x, base.y + y))
        };
        let upper = c.to_ascii_uppercase();
        let ok = match upper {
            b'M' => match pt(&mut t) {
                Some(p) => {
                    out.push(SubPath::new(p));
                    cur = p;
                    start = p;
                    true
                }
                None => false,
            },
            b'L' => match pt(&mut t) {
                Some(p) => {
                    current(&mut out, cur).segs.push(Seg::Line { to: p });
                    cur = p;
                    true
                }
                None => false,
            },
            b'H' => match t.number() {
                Some(x) => {
                    let p = Point::new(if rel { cur.x + x } else { x }, cur.y);
                    current(&mut out, cur).segs.push(Seg::Line { to: p });
                    cur = p;
                    true
                }
                None => false,
            },
            b'V' => match t.number() {
                Some(y) => {
                    let p = Point::new(cur.x, if rel { cur.y + y } else { y });
                    current(&mut out, cur).segs.push(Seg::Line { to: p });
                    cur = p;
                    true
                }
                None => false,
            },
            b'C' | b'S' => {
                let c1 = if upper == b'C' {
                    pt(&mut t)
                } else {
                    Some(match last_ctrl {
                        Some((b'C', q)) => Point::new(2.0 * cur.x - q.x, 2.0 * cur.y - q.y),
                        _ => cur,
                    })
                };
                match (c1, pt(&mut t), pt(&mut t)) {
                    (Some(c1), Some(c2), Some(p)) => {
                        current(&mut out, cur)
                            .segs
                            .push(Seg::Cubic { c1, c2, to: p });
                        last_ctrl = Some((b'C', c2));
                        cur = p;
                        true
                    }
                    _ => false,
                }
            }
            b'Q' | b'T' => {
                let q = if upper == b'Q' {
                    pt(&mut t)
                } else {
                    Some(match last_ctrl {
                        Some((b'Q', q)) => Point::new(2.0 * cur.x - q.x, 2.0 * cur.y - q.y),
                        _ => cur,
                    })
                };
                match (q, pt(&mut t)) {
                    (Some(q), Some(p)) => {
                        let c1 = Point::new(
                            cur.x + 2.0 / 3.0 * (q.x - cur.x),
                            cur.y + 2.0 / 3.0 * (q.y - cur.y),
                        );
                        let c2 = Point::new(
                            p.x + 2.0 / 3.0 * (q.x - p.x),
                            p.y + 2.0 / 3.0 * (q.y - p.y),
                        );
                        current(&mut out, cur)
                            .segs
                            .push(Seg::Cubic { c1, c2, to: p });
                        last_ctrl = Some((b'Q', q));
                        cur = p;
                        true
                    }
                    _ => false,
                }
            }
            b'A' => {
                let r = (|| {
                    let rx = t.number()?;
                    let ry = t.number()?;
                    let rot = t.number()?;
                    let large = t.flag()?;
                    let sweep = t.flag()?;
                    let x = t.number()?;
                    let y = t.number()?;
                    Some((
                        rx,
                        ry,
                        rot,
                        large,
                        sweep,
                        Point::new(base.x + x, base.y + y),
                    ))
                })();
                match r {
                    Some((rx, ry, rot, large, sweep, p)) => {
                        let segs = arc_to_cubics(cur, rx, ry, rot, large, sweep, p);
                        current(&mut out, cur).segs.extend(segs);
                        cur = p;
                        true
                    }
                    None => false,
                }
            }
            b'Z' => {
                if let Some(s) = out.last_mut() {
                    if !s.closed {
                        s.closed = true;
                    }
                }
                cur = start;
                true
            }
            _ => false,
        };
        if !matches!(upper, b'C' | b'S' | b'Q' | b'T') {
            last_ctrl = None;
        }
        if !ok {
            break;
        }
    }
    // A lone moveto draws nothing.
    out.retain(|s| !s.segs.is_empty());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn lengths_and_units() {
        assert_eq!(Length::parse("297.0016mm").unwrap().to_mm(), Some(297.0016));
        assert!(close(Length::parse("1in").unwrap().to_mm().unwrap(), 25.4));
        assert!(close(Length::parse("96").unwrap().to_mm().unwrap(), 25.4));
        assert!(close(Length::parse("72pt").unwrap().to_mm().unwrap(), 25.4));
        assert!(Length::parse("50%").unwrap().to_mm().is_none());
        assert!(close(Length::parse("1e1mm").unwrap().value, 10.0));
    }

    #[test]
    fn transforms_compose_right_to_left() {
        let m = parse_transform("translate(10 20) scale(2)");
        let p = m.apply(Point::new(1.0, 1.0));
        assert!(close(p.x, 12.0) && close(p.y, 22.0));
        let m = parse_transform("rotate(90, 1, 1)");
        let p = m.apply(Point::new(2.0, 1.0));
        assert!(close(p.x, 1.0) && close(p.y, 2.0));
        let m = parse_transform("matrix(1 0 0 -1 0 5)");
        assert!(close(m.apply(Point::new(0.0, 1.0)).y, 4.0));
    }

    #[test]
    fn path_commands() {
        let sp = path_data("M10,10 h5 v5 H10 z m 1 1 l2-2.5.5.5");
        assert_eq!(sp.len(), 2);
        assert!(sp[0].closed);
        assert_eq!(sp[0].segs.len(), 3);
        let b = sp[0].bbox();
        assert!(close(b.min.x, 10.0) && close(b.max.x, 15.0) && close(b.max.y, 15.0));
        // Relative moveto after close starts at the subpath start.
        assert!(sp[1].start.approx_eq(Point::new(11.0, 11.0), 1e-12));
        assert_eq!(sp[1].segs.len(), 2);
        assert!(sp[1].segs[1].end().approx_eq(Point::new(13.5, 9.0), 1e-12));
        // Implicit lineto after moveto, exponent numbers, compact arc flags.
        let sp = path_data("M0 0 1e1 0 a5 5 0 015 5");
        assert_eq!(sp[0].segs.len(), 2);
        assert!(sp[0]
            .segs
            .last()
            .unwrap()
            .end()
            .approx_eq(Point::new(15.0, 5.0), 1e-9));
        // Smooth curves and quadratics.
        let sp = path_data("M0 0 C 0 1 1 1 1 0 S 2 -1 2 0 Q 3 1 4 0 T 6 0");
        assert_eq!(sp[0].segs.len(), 4);
        // Errors keep what was read.
        let sp = path_data("M0 0 L 1 1 L x");
        assert_eq!(sp[0].segs.len(), 1);
    }

    #[test]
    fn root_mapping_mm_with_viewbox() {
        let doc = parse(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="100mm" viewBox="0 0 2000 1000">
                 <rect x="100" y="100" width="200" height="100" fill="black"/>
               </svg>"#,
        )
        .unwrap();
        assert!(doc.physical_size);
        assert!(close(doc.mm_per_user_unit(), 0.1));
        let b = doc.content_bbox().unwrap();
        assert!(close(b.min.x, 100.0) && close(b.max.x, 300.0));
        let mm = doc.user_to_mm.apply(b.max);
        assert!(close(mm.x, 30.0) && close(mm.y, 20.0));
    }

    #[test]
    fn styles_inherit_and_background_is_ignored() {
        let doc = parse(
            r##"<?xml version="1.0" standalone="no"?>
<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10in" height="5in" viewBox="0 0 1000 500">
  <rect x="0" y="0" width="1000" height="500" fill="#FFFFFF" stroke="none"/>
  <g style="fill:none; stroke:#000000; stroke-width:4; stroke-linecap:round" transform="translate(10 0)">
    <path d="M 100 100 L 200 100"/>
    <g transform="scale(2)"><circle cx="100" cy="100" r="10" style="fill:#000"/></g>
    <text x="0" y="0">ignored</text>
    <use xlink:href="#c1" x="5" y="0"/>
  </g>
  <defs><circle id="c1" cx="0" cy="0" r="1" fill="black"/></defs>
  <g display="none"><rect x="-500" y="-500" width="1" height="1"/></g>
</svg>"##,
        )
        .unwrap();
        assert_eq!(doc.text_elements, 1);
        let content: Vec<&Shape> = doc.content().collect();
        assert_eq!(content.len(), 3, "{:?}", doc.shapes);
        let line = content[0];
        assert!(line.fill.is_none());
        let st = line.stroke.as_ref().unwrap();
        assert!(close(st.width, 4.0));
        assert_eq!(st.cap, LineCap::Round);
        let circ = content[1];
        assert!(close(circ.stroke.as_ref().unwrap().width, 8.0));
        let cb = circ.bbox().unwrap();
        assert!(close(cb.center().x, 210.0) && close(cb.center().y, 200.0));
        assert!(close(cb.width(), 40.0));
        let used = content[2].bbox().unwrap();
        assert!(close(used.center().x, 15.0));
        let b = doc.content_bbox().unwrap();
        assert!(close(b.min.x, 14.0) && close(b.max.x, 230.0));
        assert!(close(doc.mm_per_user_unit(), 0.254));
    }

    #[test]
    fn xml_errors() {
        assert!(matches!(parse("<svg"), Err(PlotError::Xml(_))));
        assert!(matches!(parse("<html/>"), Err(PlotError::NotSvg(_))));
    }
}
