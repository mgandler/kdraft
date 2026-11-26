//! Drawing sheet resolved to absolute page geometry (mm, origin top-left,
//! Y down) for one paper size and page.

use kdraft_board::geom::{BBox, Point};
use serde::Serialize;

use crate::vars::TextVars;
use crate::wks::{Coord, Corner, HJustify, ItemKind, PageOption, TextItem, VJustify, WksFile};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SLine {
    pub a: Point,
    pub b: Point,
    pub width: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SRect {
    pub bbox: BBox,
    pub width: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SText {
    pub text: String,
    pub at: Point,
    /// Glyph width and height, mm.
    pub size: (f64, f64),
    pub bold: bool,
    pub italic: bool,
    pub h_justify: HJustify,
    pub v_justify: VJustify,
    /// Degrees, counter-clockwise on the page.
    pub rotation: f64,
    pub max_len: Option<f64>,
    pub max_height: Option<f64>,
    pub line_width: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SPolygon {
    pub points: Vec<Point>,
    pub width: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ResolvedSheet {
    pub width: f64,
    pub height: f64,
    /// Margin corners: top-left and bottom-right.
    pub lt: Point,
    pub rb: Point,
    pub lines: Vec<SLine>,
    pub rects: Vec<SRect>,
    pub texts: Vec<SText>,
    pub polygons: Vec<SPolygon>,
    /// Extents of bitmaps (content is not drawn).
    pub bitmaps: Vec<BBox>,
}

/// Page to resolve for.
#[derive(Debug, Clone, PartialEq)]
pub struct PageSpec {
    pub width: f64,
    pub height: f64,
    /// 1-based.
    pub page_number: usize,
}

fn place(c: &Coord, lt: Point, rb: Point, i: u32, incr: Point) -> Point {
    let x = c.x + incr.x * f64::from(i);
    let y = c.y + incr.y * f64::from(i);
    match c.corner {
        Corner::RightBottom => Point::new(rb.x - x, rb.y - y),
        Corner::RightTop => Point::new(rb.x - x, lt.y + y),
        Corner::LeftBottom => Point::new(lt.x + x, rb.y - y),
        Corner::LeftTop => Point::new(lt.x + x, lt.y + y),
    }
}

fn inside(p: Point, lt: Point, rb: Point) -> bool {
    let e = 1e-6;
    p.x >= lt.x - e && p.x <= rb.x + e && p.y >= lt.y - e && p.y <= rb.y + e
}

/// Increments a label: numbers by `n`, single letters through the
/// alphabet; other text unchanged.
pub fn increment_label(text: &str, n: i64) -> String {
    if n == 0 {
        return text.to_string();
    }
    if !text.is_empty() && text.chars().all(|c| c.is_ascii_digit()) {
        if let Ok(v) = text.parse::<i64>() {
            return (v + n).to_string();
        }
    }
    let mut cs = text.chars();
    if let (Some(c), None) = (cs.next(), cs.next()) {
        if c.is_ascii_alphabetic() {
            let base = if c.is_ascii_uppercase() { b'A' } else { b'a' };
            let off = (i64::from(c as u8 - base) + n).rem_euclid(26) as u8;
            return ((base + off) as char).to_string();
        }
    }
    text.to_string()
}

impl WksFile {
    /// Absolute geometry for `page`, texts expanded with `vars`.
    ///
    /// Repeated copies after the first are dropped when their start (or end)
    /// leaves the margin area, as KiCad does.
    pub fn resolve(&self, page: &PageSpec, vars: &TextVars) -> ResolvedSheet {
        let s = &self.setup;
        let lt = Point::new(s.left_margin, s.top_margin);
        let rb = Point::new(page.width - s.right_margin, page.height - s.bottom_margin);
        let mut out = ResolvedSheet {
            width: page.width,
            height: page.height,
            lt,
            rb,
            ..Default::default()
        };
        for it in &self.items {
            let c = &it.common;
            let shown = match c.option {
                PageOption::All => true,
                PageOption::Page1Only => page.page_number == 1,
                PageOption::NotOnPage1 => page.page_number != 1,
            };
            if !shown {
                continue;
            }
            let incr = Point::new(c.incrx, c.incry);
            for i in 0..c.repeat {
                match &it.kind {
                    ItemKind::Line { start, end } | ItemKind::Rect { start, end } => {
                        let a = place(start, lt, rb, i, incr);
                        let b = place(end, lt, rb, i, incr);
                        if i > 0 && !(inside(a, lt, rb) && inside(b, lt, rb)) {
                            continue;
                        }
                        let width = c.line_width.unwrap_or(s.line_width);
                        if matches!(it.kind, ItemKind::Line { .. }) {
                            out.lines.push(SLine { a, b, width });
                        } else if let Some(bbox) = BBox::from_points([a, b]) {
                            out.rects.push(SRect { bbox, width });
                        }
                    }
                    ItemKind::Text(t) => {
                        let at = place(&t.pos, lt, rb, i, incr);
                        if i > 0 && !inside(at, lt, rb) {
                            continue;
                        }
                        out.texts
                            .push(text(t, at, i, s.text_size, c, s.text_line_width, vars));
                    }
                    ItemKind::Polygon {
                        pos,
                        rotate,
                        polygons,
                    } => {
                        let at = place(pos, lt, rb, i, incr);
                        if i > 0 && !inside(at, lt, rb) {
                            continue;
                        }
                        for poly in polygons {
                            let points = poly
                                .iter()
                                // KiCad rotates counter-clockwise on screen.
                                .map(|p| at + p.rotate_kicad(*rotate))
                                .collect();
                            out.polygons.push(SPolygon {
                                points,
                                width: c.line_width.unwrap_or(0.0),
                            });
                        }
                    }
                    ItemKind::Bitmap {
                        pos,
                        scale,
                        pixels,
                        ppi,
                    } => {
                        let at = place(pos, lt, rb, i, incr);
                        if i > 0 && !inside(at, lt, rb) {
                            continue;
                        }
                        if let Some((w, h)) = pixels {
                            let wmm = f64::from(*w) / ppi * 25.4 * scale;
                            let hmm = f64::from(*h) / ppi * 25.4 * scale;
                            // UNVERIFIED: bitmap position taken as its centre.
                            out.bitmaps.push(BBox {
                                min: Point::new(at.x - wmm / 2.0, at.y - hmm / 2.0),
                                max: Point::new(at.x + wmm / 2.0, at.y + hmm / 2.0),
                            });
                        }
                    }
                }
            }
        }
        out
    }
}

fn text(
    t: &TextItem,
    at: Point,
    i: u32,
    default_size: (f64, f64),
    c: &crate::wks::Common,
    default_width: f64,
    vars: &TextVars,
) -> SText {
    let label = increment_label(&t.text, t.incr_label * i64::from(i));
    SText {
        text: vars.expand(&label),
        at,
        size: t.size.unwrap_or(default_size),
        bold: t.bold,
        italic: t.italic,
        h_justify: t.h_justify,
        v_justify: t.v_justify,
        rotation: t.rotate,
        max_len: t.max_len,
        max_height: t.max_height,
        line_width: c.line_width.unwrap_or(default_width),
    }
}

impl ResolvedSheet {
    /// Box of everything drawn (texts as anchor points only).
    pub fn extent(&self) -> Option<BBox> {
        let pts = self
            .lines
            .iter()
            .flat_map(|l| [l.a, l.b])
            .chain(self.rects.iter().flat_map(|r| [r.bbox.min, r.bbox.max]))
            .chain(self.texts.iter().map(|t| t.at))
            .chain(self.polygons.iter().flat_map(|p| p.points.iter().copied()))
            .chain(self.bitmaps.iter().flat_map(|b| [b.min, b.max]));
        BBox::from_points(pts)
    }

    /// The innermost frame: the smallest rectangle spanning more than half
    /// the page in both directions; the margin box if there is none.
    pub fn frame(&self) -> BBox {
        let big = |b: &BBox| b.width() > self.width / 2.0 && b.height() > self.height / 2.0;
        self.rects
            .iter()
            .map(|r| r.bbox)
            .filter(big)
            .min_by(|a, b| (a.width() * a.height()).total_cmp(&(b.width() * b.height())))
            .unwrap_or(BBox {
                min: self.lt,
                max: self.rb,
            })
    }

    /// Heuristic title block box: lines, rectangles and texts that are not
    /// frame-sized and lie inside the frame's bottom-right quarter.
    pub fn title_block(&self) -> Option<BBox> {
        let f = self.frame();
        let cx = f.min.x + f.width() / 2.0;
        let cy = f.min.y + f.height() / 2.0;
        let in_quarter = |p: Point| p.x >= cx && p.y >= cy;
        let small = |b: &BBox| b.width() < f.width() / 2.0 && b.height() < f.height() / 2.0;
        let mut pts: Vec<Point> = Vec::new();
        for r in &self.rects {
            if small(&r.bbox) && in_quarter(r.bbox.min) {
                pts.extend([r.bbox.min, r.bbox.max]);
            }
        }
        for l in &self.lines {
            if let Some(b) = BBox::from_points([l.a, l.b]) {
                // Short lines near the frame edge are grid ticks, not part of
                // the title block.
                if small(&b)
                    && in_quarter(l.a)
                    && in_quarter(l.b)
                    && b.width().max(b.height()) > 5.0
                {
                    pts.extend([l.a, l.b]);
                }
            }
        }
        BBox::from_points(pts)
    }

    /// Largest rectangle inside the frame that avoids the title block,
    /// shrunk by `clearance` on every side.
    pub fn drawing_area(&self, clearance: f64) -> BBox {
        let f = self.frame();
        let shrink = |b: BBox| BBox {
            min: Point::new(b.min.x + clearance, b.min.y + clearance),
            max: Point::new(
                (b.max.x - clearance).max(b.min.x + clearance),
                (b.max.y - clearance).max(b.min.y + clearance),
            ),
        };
        let Some(tb) = self.title_block() else {
            return shrink(f);
        };
        // Either everything above the title block, or everything left of it.
        let above = BBox {
            min: f.min,
            max: Point::new(f.max.x, tb.min.y.max(f.min.y)),
        };
        let left = BBox {
            min: f.min,
            max: Point::new(tb.min.x.max(f.min.x), f.max.y),
        };
        let area = |b: &BBox| b.width().max(0.0) * b.height().max(0.0);
        shrink(if area(&above) >= area(&left) {
            above
        } else {
            left
        })
    }
}
