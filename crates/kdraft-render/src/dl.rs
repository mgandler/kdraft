//! Display list: back-end independent drawing primitives.
//!
//! Units are millimetres on the page, origin top-left, Y down. Angles are
//! degrees, counter-clockwise as seen on the page.

use kdraft_board::geom::{circle_through, Affine, BBox, Point};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    /// Alpha, 255 = opaque.
    pub a: u8,
}

impl Color {
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const WHITE: Color = Color::rgb(255, 255, 255);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color { r, g, b, a }
    }

    pub fn gray(v: u8) -> Color {
        Color::rgb(v, v, v)
    }

    /// The same colour with its alpha multiplied by `opacity` (0..1).
    pub fn faded(self, opacity: f64) -> Color {
        let o = opacity.clamp(0.0, 1.0);
        Color {
            a: (f64::from(self.a) * o).round() as u8,
            ..self
        }
    }

    /// Alpha as 0..1.
    pub fn alpha(self) -> f32 {
        f32::from(self.a) / 255.0
    }

    pub fn is_opaque(self) -> bool {
        self.a == 255
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cmd {
    Move(Point),
    Line(Point),
    Cubic(Point, Point, Point),
    Close,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Path {
    pub cmds: Vec<Cmd>,
}

/// Control-point factor for a quarter circle.
const KAPPA: f64 = 0.552_284_749_830_793_6;

impl Path {
    pub fn new() -> Path {
        Path::default()
    }

    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty()
    }

    pub fn move_to(&mut self, p: Point) -> &mut Self {
        self.cmds.push(Cmd::Move(p));
        self
    }

    pub fn line_to(&mut self, p: Point) -> &mut Self {
        self.cmds.push(Cmd::Line(p));
        self
    }

    pub fn cubic_to(&mut self, c1: Point, c2: Point, p: Point) -> &mut Self {
        self.cmds.push(Cmd::Cubic(c1, c2, p));
        self
    }

    pub fn close(&mut self) -> &mut Self {
        self.cmds.push(Cmd::Close);
        self
    }

    pub fn line(a: Point, b: Point) -> Path {
        let mut p = Path::new();
        p.move_to(a).line_to(b);
        p
    }

    pub fn polyline(points: &[Point], closed: bool) -> Path {
        let mut p = Path::new();
        if let Some((first, rest)) = points.split_first() {
            p.move_to(*first);
            for q in rest {
                p.line_to(*q);
            }
            if closed {
                p.close();
            }
        }
        p
    }

    pub fn rect(b: &BBox) -> Path {
        Path::polyline(
            &[
                b.min,
                Point::new(b.max.x, b.min.y),
                b.max,
                Point::new(b.min.x, b.max.y),
            ],
            true,
        )
    }

    pub fn circle(c: Point, r: f64) -> Path {
        let mut p = Path::new();
        p.move_to(Point::new(c.x + r, c.y));
        p.arc_cubics(c, r, 0.0, std::f64::consts::TAU);
        p.close();
        p
    }

    /// Appends cubics for a circular arc around `c` from angle `a0` over
    /// `sweep` radians (positive: increasing angle in Y-down coordinates,
    /// i.e. clockwise on the page). The current point must be the arc start.
    pub fn arc_cubics(&mut self, c: Point, r: f64, a0: f64, sweep: f64) -> &mut Self {
        let n = (sweep.abs() / std::f64::consts::FRAC_PI_2 - 1e-9)
            .ceil()
            .max(1.0) as usize;
        let step = sweep / n as f64;
        let k = 4.0 / 3.0 * (step / 4.0).tan();
        let mut a = a0;
        for _ in 0..n {
            let b = a + step;
            let (sa, ca) = a.sin_cos();
            let (sb, cb) = b.sin_cos();
            let p0 = Point::new(c.x + r * ca, c.y + r * sa);
            let p3 = Point::new(c.x + r * cb, c.y + r * sb);
            let c1 = Point::new(p0.x - k * r * sa, p0.y + k * r * ca);
            let c2 = Point::new(p3.x + k * r * sb, p3.y - k * r * cb);
            self.cubic_to(c1, c2, p3);
            a = b;
        }
        self
    }

    /// Arc through three points; a straight line if they are collinear.
    pub fn arc3(start: Point, mid: Point, end: Point) -> Path {
        let mut p = Path::new();
        p.move_to(start);
        p.arc3_to(start, mid, end);
        p
    }

    /// Appends the arc from the current point `start` through `mid` to `end`.
    pub fn arc3_to(&mut self, start: Point, mid: Point, end: Point) -> &mut Self {
        let Some((c, r)) = circle_through(start, mid, end) else {
            return self.line_to(end);
        };
        let tau = std::f64::consts::TAU;
        let ang = |p: Point| (p.y - c.y).atan2(p.x - c.x);
        let (a0, am, a1) = (ang(start), ang(mid), ang(end));
        let pos = |f: f64, t: f64| (t - f).rem_euclid(tau);
        let sweep = if pos(a0, am) <= pos(a0, a1) {
            pos(a0, a1)
        } else {
            -pos(a1, a0)
        };
        self.arc_cubics(c, r, a0, sweep);
        // Land exactly on the end point.
        if let Some(Cmd::Cubic(_, _, last)) = self.cmds.last_mut() {
            *last = end;
        }
        self
    }

    /// Rectangle of size `w` x `h` centred at the origin with rounded
    /// corners of radius `r`.
    pub fn round_rect_centered(w: f64, h: f64, r: f64) -> Path {
        let (hw, hh) = (w / 2.0, h / 2.0);
        let r = r.clamp(0.0, hw.min(hh));
        if r <= 0.0 {
            return Path::rect(&BBox {
                min: Point::new(-hw, -hh),
                max: Point::new(hw, hh),
            });
        }
        let mut p = Path::new();
        let k = KAPPA * r;
        p.move_to(Point::new(-hw + r, -hh))
            .line_to(Point::new(hw - r, -hh))
            .cubic_to(
                Point::new(hw - r + k, -hh),
                Point::new(hw, -hh + r - k),
                Point::new(hw, -hh + r),
            )
            .line_to(Point::new(hw, hh - r))
            .cubic_to(
                Point::new(hw, hh - r + k),
                Point::new(hw - r + k, hh),
                Point::new(hw - r, hh),
            )
            .line_to(Point::new(-hw + r, hh))
            .cubic_to(
                Point::new(-hw + r - k, hh),
                Point::new(-hw, hh - r + k),
                Point::new(-hw, hh - r),
            )
            .line_to(Point::new(-hw, -hh + r))
            .cubic_to(
                Point::new(-hw, -hh + r - k),
                Point::new(-hw + r - k, -hh),
                Point::new(-hw + r, -hh),
            )
            .close();
        p
    }

    pub fn transform(&self, m: &Affine) -> Path {
        Path {
            cmds: self
                .cmds
                .iter()
                .map(|c| match *c {
                    Cmd::Move(p) => Cmd::Move(m.apply(p)),
                    Cmd::Line(p) => Cmd::Line(m.apply(p)),
                    Cmd::Cubic(a, b, p) => Cmd::Cubic(m.apply(a), m.apply(b), m.apply(p)),
                    Cmd::Close => Cmd::Close,
                })
                .collect(),
        }
    }

    /// Box of all points including control points (contains the path).
    pub fn control_bbox(&self) -> Option<BBox> {
        BBox::from_points(self.cmds.iter().flat_map(|c| match *c {
            Cmd::Move(p) | Cmd::Line(p) => vec![p],
            Cmd::Cubic(a, b, p) => vec![a, b, p],
            Cmd::Close => vec![],
        }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineCap {
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineJoin {
    Miter,
    Round,
    Bevel,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stroke {
    pub width: f64,
    pub color: Color,
    pub cap: LineCap,
    pub join: LineJoin,
    pub dash: Option<Vec<f64>>,
}

impl Stroke {
    pub fn solid(width: f64, color: Color) -> Stroke {
        Stroke {
            width,
            color,
            cap: LineCap::Round,
            join: LineJoin::Round,
            dash: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fill {
    pub color: Color,
    pub even_odd: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PathItem {
    pub path: Path,
    pub stroke: Option<Stroke>,
    pub fill: Option<Fill>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HAlign {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VAlign {
    /// The anchor is at the top of capital letters.
    Top,
    /// The anchor is half a cap height above the baseline.
    Center,
    /// The anchor is on the baseline.
    Baseline,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    pub text: String,
    pub at: Point,
    /// Cap height, mm.
    pub size: f64,
    pub h_align: HAlign,
    pub v_align: VAlign,
    pub rotation: f64,
    pub bold: bool,
    pub italic: bool,
    pub color: Color,
    /// Searchable but not painted (text over a plotted image of itself).
    pub invisible: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub transform: Affine,
    /// Clip rectangle in the group's parent coordinates.
    pub clip: Option<BBox>,
    pub items: Vec<Item>,
}

/// A raster picture (a 3D render) placed in a sheet rectangle.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageItem {
    /// Where it is drawn (stretched to fill), sheet mm.
    pub rect: BBox,
    /// The encoded PNG (PDF and SVG embed it as is).
    pub png: std::sync::Arc<Vec<u8>>,
    /// Pixel size of the PNG.
    pub width: u32,
    pub height: u32,
}

impl ImageItem {
    /// Reads the pixel size from a PNG header; `None` if not a PNG.
    pub fn from_png(png: std::sync::Arc<Vec<u8>>, rect: BBox) -> Option<ImageItem> {
        let b = png.get(0..24)?;
        if b[0..8] != [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] || &b[12..16] != b"IHDR" {
            return None;
        }
        let w = u32::from_be_bytes([b[16], b[17], b[18], b[19]]);
        let h = u32::from_be_bytes([b[20], b[21], b[22], b[23]]);
        (w > 0 && h > 0).then_some(ImageItem {
            rect,
            png,
            width: w,
            height: h,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Path(PathItem),
    Text(TextRun),
    Group(Group),
    Image(ImageItem),
}

impl Item {
    pub fn stroked(path: Path, stroke: Stroke) -> Item {
        Item::Path(PathItem {
            path,
            stroke: Some(stroke),
            fill: None,
        })
    }

    pub fn filled(path: Path, color: Color) -> Item {
        Item::Path(PathItem {
            path,
            stroke: None,
            fill: Some(Fill {
                color,
                even_odd: false,
            }),
        })
    }
}

/// One page.
#[derive(Debug, Clone, PartialEq)]
pub struct PageList {
    pub width: f64,
    pub height: f64,
    pub title: String,
    pub items: Vec<Item>,
}

/// A rendered document.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DocList {
    pub title: String,
    pub pages: Vec<PageList>,
}

/// Counts items recursively by kind: (paths, texts, groups).
pub fn count(items: &[Item]) -> (usize, usize, usize) {
    let mut c = (0, 0, 0);
    for it in items {
        match it {
            Item::Path(_) | Item::Image(_) => c.0 += 1,
            Item::Text(_) => c.1 += 1,
            Item::Group(g) => {
                c.2 += 1;
                let s = count(&g.items);
                c.0 += s.0;
                c.1 += s.1;
                c.2 += s.2;
            }
        }
    }
    c
}

/// Box around the painted paths (control points, plus half the stroke
/// width), recursively through group transforms; group clips are ignored
/// and text runs are not measured.
pub fn paths_bbox(items: &[Item]) -> Option<BBox> {
    fn walk(items: &[Item], m: &Affine, out: &mut Option<BBox>) {
        for it in items {
            match it {
                Item::Path(p) => {
                    let Some(b) = p.path.transform(m).control_bbox() else {
                        continue;
                    };
                    let hw = p
                        .stroke
                        .as_ref()
                        .map_or(0.0, |s| s.width / 2.0 * m.mean_scale());
                    let b = BBox {
                        min: Point::new(b.min.x - hw, b.min.y - hw),
                        max: Point::new(b.max.x + hw, b.max.y + hw),
                    };
                    *out = Some(out.map_or(b, |o| o.union(&b)));
                }
                Item::Group(g) => walk(&g.items, &g.transform.then(m), out),
                Item::Image(i) => {
                    let b = m.apply_bbox(&i.rect);
                    *out = Some(out.map_or(b, |o| o.union(&b)));
                }
                Item::Text(_) => {}
            }
        }
    }
    let mut out = None;
    walk(items, &Affine::IDENTITY, &mut out);
    out
}

/// Box around paths and text runs (text measured with the bundled font:
/// advance width, cap height plus a descender allowance), recursively
/// through group transforms; group clips are applied.
pub fn items_bbox(items: &[Item], fonts: &crate::font::Fonts) -> Option<BBox> {
    fn walk(items: &[Item], m: &Affine, fonts: &crate::font::Fonts, out: &mut Option<BBox>) {
        let add = |b: BBox, out: &mut Option<BBox>| *out = Some(out.map_or(b, |o| o.union(&b)));
        for it in items {
            match it {
                Item::Path(_) | Item::Image(_) => {
                    if let Some(b) = paths_bbox(std::slice::from_ref(it)).map(|b| m.apply_bbox(&b))
                    {
                        add(b, out);
                    }
                }
                Item::Text(t) => {
                    if t.text.is_empty() {
                        continue;
                    }
                    let p = crate::font::place(t, fonts);
                    let local = BBox {
                        min: Point::new(p.origin.x, p.origin.y - t.size * 1.2),
                        max: Point::new(p.origin.x + p.width, p.origin.y + t.size * 0.35),
                    };
                    add(m.apply_bbox(&p.transform.apply_bbox(&local)), out);
                }
                Item::Group(g) => {
                    let mut inner = None;
                    walk(&g.items, &g.transform.then(m), fonts, &mut inner);
                    if let Some(mut b) = inner {
                        if let Some(c) = g.clip.map(|c| m.apply_bbox(&c)) {
                            b = BBox {
                                min: Point::new(b.min.x.max(c.min.x), b.min.y.max(c.min.y)),
                                max: Point::new(b.max.x.min(c.max.x), b.max.y.min(c.max.y)),
                            };
                            if b.min.x > b.max.x || b.min.y > b.max.y {
                                continue;
                            }
                        }
                        add(b, out);
                    }
                }
            }
        }
    }
    let mut out = None;
    walk(items, &Affine::IDENTITY, fonts, &mut out);
    out
}

/// All text runs, recursively, in drawing order.
pub fn texts(items: &[Item]) -> Vec<&TextRun> {
    let mut v = Vec::new();
    for it in items {
        match it {
            Item::Text(t) => v.push(t),
            Item::Group(g) => v.extend(texts(&g.items)),
            Item::Path(_) | Item::Image(_) => {}
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arc3_runs_through_mid_and_ends_exactly() {
        let p = Path::arc3(
            Point::new(1.0, 0.0),
            Point::new(0.0, -1.0),
            Point::new(-1.0, 0.0),
        );
        let b = p.control_bbox().unwrap();
        // Upper half (Y down): reaches y = -1, never below 0.
        assert!(b.min.y < -0.99 && b.max.y < 1e-9, "{b:?}");
        assert!(matches!(p.cmds.last(), Some(Cmd::Cubic(_, _, e)) if *e == Point::new(-1.0, 0.0)));
        // Collinear degrades to a line.
        let l = Path::arc3(Point::ORIGIN, Point::new(1.0, 1.0), Point::new(2.0, 2.0));
        assert_eq!(l.cmds.len(), 2);
    }

    #[test]
    fn circle_and_round_rect() {
        let c = Path::circle(Point::new(1.0, 1.0), 2.0);
        assert_eq!(c.cmds.len(), 6);
        let b = c.control_bbox().unwrap();
        assert!((b.width() - 4.0).abs() < 1e-9);
        let r = Path::round_rect_centered(4.0, 2.0, 0.5);
        let b = r.control_bbox().unwrap();
        assert!((b.width() - 4.0).abs() < 1e-12 && (b.height() - 2.0).abs() < 1e-12);
        assert_eq!(Path::round_rect_centered(4.0, 2.0, 0.0).cmds.len(), 5);
    }

    #[test]
    fn transform_and_count() {
        let p = Path::line(Point::ORIGIN, Point::new(1.0, 0.0)).transform(&Affine::scale(2.0, 2.0));
        assert_eq!(p.cmds[1], Cmd::Line(Point::new(2.0, 0.0)));
        let items = vec![
            Item::stroked(p, Stroke::solid(0.1, Color::BLACK)),
            Item::Group(Group {
                transform: Affine::IDENTITY,
                clip: None,
                items: vec![Item::Text(TextRun {
                    text: "x".into(),
                    at: Point::ORIGIN,
                    size: 1.0,
                    h_align: HAlign::Left,
                    v_align: VAlign::Baseline,
                    rotation: 0.0,
                    bold: false,
                    italic: false,
                    color: Color::BLACK,
                    invisible: false,
                })],
            }),
        ];
        assert_eq!(count(&items), (1, 1, 1));
        assert_eq!(texts(&items).len(), 1);
    }
}
