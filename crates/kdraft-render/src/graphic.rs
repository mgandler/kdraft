//! Free graphics (lines, polylines, rectangles, circles, arcs, polygons,
//! text) with their effective style, and recolouring of rendered items.

use kdraft_board::geom::{Affine, BBox, Point};
use kdraft_doc::style::{Arrows, ItemStyle, LinePattern, Rgba, TextAlign};
use kdraft_doc::{Graphic, GraphicShape};
use kdraft_layout::text::{self as ltext, Align};
use kdraft_sheet::TextVars;

use crate::dl::{Color, Fill, HAlign, Item, Path, PathItem, Stroke, TextRun, VAlign};
use crate::font::Fonts;

/// Display colour of a style colour at `opacity` (0..1).
pub fn color(c: Rgba, opacity: f64) -> Color {
    Color::rgba(c.r, c.g, c.b, c.a).faded(opacity)
}

fn opacity(s: &ItemStyle) -> f64 {
    s.opacity.unwrap_or(1.0)
}

/// The stroke an effective style describes.
pub fn stroke(s: &ItemStyle) -> Stroke {
    let w = s.width.unwrap_or(0.25);
    let mut st = Stroke::solid(w, color(s.stroke.unwrap_or(Rgba::BLACK), opacity(s)));
    st.dash = s.pattern.unwrap_or(LinePattern::Solid).dashes(w);
    st
}

fn fill(s: &ItemStyle) -> Option<Fill> {
    let f = s.fill.filter(|f| !f.is_none())?;
    Some(Fill {
        color: color(f, opacity(s)),
        even_odd: false,
    })
}

fn closed(path: Path, s: &ItemStyle) -> Item {
    Item::Path(PathItem {
        path,
        stroke: Some(stroke(s)),
        fill: fill(s),
    })
}

/// Filled arrowhead with its tip at `tip`, pointing away from `from`.
pub fn arrowhead(tip: Point, from: Point, len: f64, c: Color) -> Option<Item> {
    let d = Point::new(tip.x - from.x, tip.y - from.y);
    let l = d.x.hypot(d.y);
    if l < 1e-9 || len <= 0.0 {
        return None;
    }
    let (ux, uy) = (d.x / l, d.y / l);
    let half = len / 3.0 / 2.0 * 1.2;
    let base = Point::new(tip.x - ux * len, tip.y - uy * len);
    let a = Point::new(base.x - uy * half, base.y + ux * half);
    let b = Point::new(base.x + uy * half, base.y - ux * half);
    Some(Item::filled(Path::polyline(&[tip, a, b], true), c))
}

/// Points of an open line or polyline with ends pulled back under the
/// arrowheads, so round caps do not poke past the tips.
fn shortened(points: &[Point], arrows: Arrows, len: f64) -> Vec<Point> {
    let mut p = points.to_vec();
    let pull = |a: Point, b: Point| {
        let d = Point::new(b.x - a.x, b.y - a.y);
        let l = d.x.hypot(d.y);
        let k = (len * 0.6).min(l * 0.5);
        if l < 1e-9 {
            a
        } else {
            Point::new(a.x + d.x / l * k, a.y + d.y / l * k)
        }
    };
    let n = p.len();
    if n >= 2 {
        if arrows.at_start() {
            p[0] = pull(points[0], points[1]);
        }
        if arrows.at_end() {
            p[n - 1] = pull(points[n - 1], points[n - 2]);
        }
    }
    p
}

fn open_line(points: &[Point], s: &ItemStyle) -> Vec<Item> {
    let arrows = s.arrows.unwrap_or(Arrows::None);
    let len = s.arrow_size.unwrap_or(2.5);
    let c = color(s.stroke.unwrap_or(Rgba::BLACK), opacity(s));
    let pts = if arrows == Arrows::None {
        points.to_vec()
    } else {
        shortened(points, arrows, len)
    };
    let mut out = vec![Item::stroked(Path::polyline(&pts, false), stroke(s))];
    let n = points.len();
    if n >= 2 {
        if arrows.at_start() {
            out.extend(arrowhead(points[0], points[1], len, c));
        }
        if arrows.at_end() {
            out.extend(arrowhead(points[n - 1], points[n - 2], len, c));
        }
    }
    out
}

/// Rounded rectangle path between two corners.
pub fn rect_path(start: Point, end: Point, radius: f64) -> Option<Path> {
    let b = BBox::from_points([start, end])?;
    let r = radius.max(0.0).min(b.width() / 2.0).min(b.height() / 2.0);
    if r <= 1e-6 {
        return Some(Path::rect(&b));
    }
    let c = b.center();
    Some(
        Path::round_rect_centered(b.width(), b.height(), r).transform(&Affine::translate(c.x, c.y)),
    )
}

fn align(a: Option<TextAlign>) -> Align {
    match a.unwrap_or(TextAlign::Left) {
        TextAlign::Left => Align::Left,
        TextAlign::Center => Align::Center,
        TextAlign::Right => Align::Right,
    }
}

/// A laid-out text graphic: lines with their sheet positions.
#[derive(Debug, Clone, PartialEq)]
pub struct TextLayout {
    pub runs: Vec<TextRun>,
    /// Corners of the unrotated block box, rotated onto the sheet (first
    /// cap top-left, top-right, bottom-right, bottom-left).
    pub corners: [Point; 4],
    /// Block width (wrap width or widest line), mm.
    pub width: f64,
    pub height: f64,
}

/// Lays out text `text` at `at` with the effective style `s`, rotation
/// (degrees CCW) and optional wrap width. `vars` expands `${...}`.
pub fn layout_text(
    text: &str,
    at: Point,
    rotation: f64,
    wrap: Option<f64>,
    s: &ItemStyle,
    vars: Option<&TextVars>,
    fonts: &Fonts,
) -> TextLayout {
    let h = s.text_height.unwrap_or(2.5);
    let bold = s.bold.unwrap_or(false);
    let expanded = match vars {
        Some(v) => v.expand(text),
        None => text.to_string(),
    };
    let measure = |l: &str| fonts.width(l, h, bold);
    let lines = ltext::break_lines(&expanded, wrap, &measure);
    let block = ltext::layout_block(
        lines,
        wrap,
        h,
        s.line_spacing.unwrap_or(1.5),
        align(s.align),
        &measure,
    );
    let rot = Affine::rotate_deg(-rotation);
    let place = |x: f64, y: f64| {
        let p = rot.apply(Point::new(x, y));
        Point::new(at.x + p.x, at.y + p.y)
    };
    let c = color(s.text_color.unwrap_or(Rgba::BLACK), opacity(s));
    let runs = block
        .lines
        .iter()
        .filter(|(l, _, _)| !l.is_empty())
        .map(|(l, x, y)| TextRun {
            text: l.clone(),
            at: place(*x, *y),
            size: h,
            h_align: HAlign::Left,
            v_align: VAlign::Baseline,
            rotation,
            bold,
            italic: s.italic.unwrap_or(false),
            color: c,
            invisible: false,
        })
        .collect();
    let (x0, y0, x1, y1) = block.extent;
    // A little below the last baseline for descenders.
    let y1 = y1 + 0.3 * h;
    TextLayout {
        runs,
        corners: [place(x0, y0), place(x1, y0), place(x1, y1), place(x0, y1)],
        width: block.width,
        height: y1 - y0,
    }
}

/// Renders a graphic with its effective style. `None` for images (drawn
/// elsewhere) and degenerate shapes.
pub fn render(g: &Graphic, s: &ItemStyle, vars: Option<&TextVars>, fonts: &Fonts) -> Vec<Item> {
    match &g.shape {
        GraphicShape::Line { start, end } => open_line(&[*start, *end], s),
        GraphicShape::Polyline { points } => open_line(points, s),
        GraphicShape::Rect { start, end, radius } => rect_path(*start, *end, *radius)
            .map(|p| vec![closed(p, s)])
            .unwrap_or_default(),
        GraphicShape::Circle { center, radius } => vec![closed(Path::circle(*center, *radius), s)],
        GraphicShape::Arc { start, mid, end } => {
            vec![Item::stroked(Path::arc3(*start, *mid, *end), stroke(s))]
        }
        GraphicShape::Polygon { points } => vec![closed(Path::polyline(points, true), s)],
        GraphicShape::Text {
            at,
            text,
            height,
            rotation,
            wrap,
        } => {
            let mut st = s.clone();
            if let Some(h) = height {
                st.text_height = Some(*h);
            }
            layout_text(text, *at, *rotation, *wrap, &st, vars, fonts)
                .runs
                .into_iter()
                .map(Item::Text)
                .collect()
        }
        GraphicShape::Image { .. } => Vec::new(),
    }
}

/// Recolouring of an already rendered item (dimensions, callouts, notes,
/// tables): strokes get `stroke`, text gets `text`, every colour is faded
/// by `opacity`. Fills that are the stroke colour (arrowheads, anchor
/// dots) follow the stroke; other fills (white knock-outs) stay.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Recolor {
    pub stroke: Option<Color>,
    pub text: Option<Color>,
    pub opacity: f64,
}

impl Recolor {
    pub fn from_style(s: &ItemStyle) -> Recolor {
        Recolor {
            stroke: s.stroke.map(|c| color(c, 1.0)),
            text: s.text_color.map(|c| color(c, 1.0)),
            opacity: s.opacity.unwrap_or(1.0),
        }
    }

    pub fn is_identity(&self) -> bool {
        let black = |c: Option<Color>| c.is_none_or(|c| c == Color::BLACK);
        black(self.stroke) && black(self.text) && self.opacity >= 1.0
    }

    pub fn apply(&self, items: &mut [Item]) {
        if self.is_identity() {
            return;
        }
        for it in items {
            match it {
                Item::Path(p) => {
                    if let Some(st) = &mut p.stroke {
                        if st.color == Color::BLACK {
                            st.color = self.stroke.unwrap_or(st.color);
                        }
                        st.color = st.color.faded(self.opacity);
                    }
                    if let Some(f) = &mut p.fill {
                        if f.color == Color::BLACK {
                            f.color = self.stroke.unwrap_or(f.color);
                        }
                        f.color = f.color.faded(self.opacity);
                    }
                }
                Item::Text(t) => {
                    if t.color == Color::BLACK {
                        t.color = self.text.unwrap_or(t.color);
                    }
                    t.color = t.color.faded(self.opacity);
                }
                Item::Group(g) => self.apply(&mut g.items),
                Item::Image(_) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kdraft_doc::Id;

    fn g(shape: GraphicShape) -> Graphic {
        Graphic {
            id: Id("g".into()),
            shape,
            style: None,
            extra: vec![],
        }
    }

    #[test]
    fn styled_line_with_arrows_and_dashes() {
        let f = Fonts::load().unwrap();
        let mut s = ItemStyle::default();
        s.set("stroke", Some("#FF0000")).unwrap();
        s.set("width", Some("0.5")).unwrap();
        s.set("pattern", Some("dashed")).unwrap();
        s.set("arrows", Some("both")).unwrap();
        s.set("arrow_size", Some("3")).unwrap();
        s.set("opacity", Some("0.5")).unwrap();
        let items = render(
            &g(GraphicShape::Line {
                start: Point::new(0.0, 0.0),
                end: Point::new(20.0, 0.0),
            }),
            &s,
            None,
            &f,
        );
        assert_eq!(items.len(), 3);
        let Item::Path(p) = &items[0] else { panic!() };
        let st = p.stroke.as_ref().unwrap();
        assert_eq!((st.color.r, st.color.g, st.color.a), (255, 0, 128));
        assert!(st.dash.is_some());
        let Item::Path(a) = &items[2] else { panic!() };
        assert_eq!(a.fill.unwrap().color, Color::rgba(255, 0, 0, 128));
    }

    #[test]
    fn multiline_text_aligns_and_rotates() {
        let f = Fonts::load().unwrap();
        let mut s = ItemStyle::default();
        s.set("text_height", Some("2")).unwrap();
        s.set("align", Some("right")).unwrap();
        s.set("line_spacing", Some("2")).unwrap();
        let l = layout_text("A\nBBBB", Point::new(50.0, 50.0), 0.0, None, &s, None, &f);
        assert_eq!(l.runs.len(), 2);
        let w0 = f.width("A", 2.0, false);
        let w1 = f.width("BBBB", 2.0, false);
        assert!((l.runs[0].at.x + w0 - 50.0).abs() < 1e-9);
        assert!((l.runs[1].at.x + w1 - 50.0).abs() < 1e-9);
        assert!((l.runs[1].at.y - 54.0).abs() < 1e-9);
        s.set("align", Some("left")).unwrap();
        let r = layout_text("A\nB", Point::new(0.0, 0.0), 90.0, None, &s, None, &f);
        // Rotated CCW: the second line moves right (+x), not down.
        assert!(
            r.runs[1].at.x > 3.9 && r.runs[1].at.y.abs() < 1e-9,
            "{:?}",
            r.runs[1].at
        );
        assert_eq!(r.runs[1].rotation, 90.0);
    }

    #[test]
    fn recolor_keeps_white() {
        let mut items = vec![
            Item::filled(Path::circle(Point::ORIGIN, 1.0), Color::WHITE),
            Item::filled(Path::circle(Point::ORIGIN, 1.0), Color::BLACK),
        ];
        Recolor {
            stroke: Some(Color::rgb(0, 0, 255)),
            text: None,
            opacity: 1.0,
        }
        .apply(&mut items);
        let fills: Vec<Color> = items
            .iter()
            .map(|i| match i {
                Item::Path(p) => p.fill.unwrap().color,
                _ => Color::BLACK,
            })
            .collect();
        assert_eq!(fills, vec![Color::WHITE, Color::rgb(0, 0, 255)]);
    }
}
