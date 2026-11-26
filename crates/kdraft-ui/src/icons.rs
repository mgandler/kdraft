//! kdraft's icon set, drawn in code from primitives on a 16 x 16 grid
//! (no icon font, no third-party artwork). One-unit strokes sit on half
//! coordinates; straight strokes are snapped to physical pixels so they
//! stay sharp at 16 px.
//!
//! The pixel-hinting painter helper follows FigCraft's
//! `crates/ui-egui/src/icons/draw.rs` (MIT OR Apache-2.0, see NOTICE);
//! the icon drawings are kdraft's own.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use eframe::egui::{pos2, Color32, Painter, Pos2, Rect, Shape, Stroke};

macro_rules! icons {
    ($($name:ident = $s:literal,)*) => {
        /// Every icon of the UI.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Icon {
            $($name,)*
        }

        impl Icon {
            pub const ALL: &'static [Icon] = &[$(Icon::$name,)*];

            /// Stable id, used by command metadata in the engine.
            pub fn name(self) -> &'static str {
                match self {
                    $(Icon::$name => $s,)*
                }
            }

            pub fn from_name(s: &str) -> Option<Icon> {
                match s {
                    $($s => Some(Icon::$name),)*
                    _ => None,
                }
            }
        }
    };
}

icons! {
    Select = "select",
    Pan = "pan",
    ViewFab = "view-fab",
    ViewAssembly = "view-assembly",
    ViewDrill = "view-drill",
    Table = "table",
    TableDrill = "table-drill",
    TableStackup = "table-stackup",
    TableBom = "table-bom",
    DimLinear = "dim-linear",
    DimVertical = "dim-vertical",
    DimAligned = "dim-aligned",
    DimRadius = "dim-radius",
    DimDiameter = "dim-diameter",
    DimOrdinate = "dim-ordinate",
    Callout = "callout",
    Notes = "notes",
    Text = "text",
    Line = "line",
    Rect = "rect",
    AlignLeft = "align-left",
    AlignHCenter = "align-hcenter",
    AlignRight = "align-right",
    AlignTop = "align-top",
    AlignVCenter = "align-vcenter",
    AlignBottom = "align-bottom",
    DistributeH = "distribute-h",
    DistributeV = "distribute-v",
    MatchWidth = "match-width",
    MatchHeight = "match-height",
    MatchScale = "match-scale",
    Center = "center",
    Undo = "undo",
    Redo = "redo",
    ZoomIn = "zoom-in",
    ZoomOut = "zoom-out",
    FitPage = "fit-page",
    FitSelection = "fit-selection",
    Export = "export",
    Reload = "reload",
    Snap = "snap",
    Grid = "grid",
    Layers = "layers",
    Page = "page",
    PageAdd = "page-add",
    PageDuplicate = "page-duplicate",
    PageDelete = "page-delete",
    Open = "open",
    Save = "save",
    New = "new",
    Eye = "eye",
    EyeOff = "eye-off",
    Lock = "lock",
    Unlock = "unlock",
    Plus = "plus",
    Minus = "minus",
    Close = "close",
    ChevronDown = "chevron-down",
    ChevronRight = "chevron-right",
    Search = "search",
    Sun = "sun",
    Moon = "moon",
    Warning = "warning",
    Error = "error",
    Info = "info",
    Check = "check",
    More = "more",
    Command = "command",
    Trash = "trash",
    Duplicate = "duplicate",
    Raise = "raise",
    Lower = "lower",
    Keyboard = "keyboard",
    Ruler = "ruler",
    Board = "board",
    Issues = "issues",
    Log = "log",
    Graphic = "graphic",
    Link = "link",
    Polyline = "polyline",
    View3d = "view-3d",
    Circle = "circle",
    Unlink = "unlink",
    Settings = "settings",
}

struct P<'a> {
    painter: &'a Painter,
    o: Pos2,
    s: f32,
    px: f32,
    stroke: Stroke,
    color: Color32,
}

impl P<'_> {
    fn raw(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.o.x + x * self.s, self.o.y + y * self.s)
    }
    fn snap_c(&self, v: f32) -> f32 {
        let odd = (self.stroke.width / self.px).round() as i32 % 2 == 1;
        let off = if odd { 0.5 } else { 0.0 };
        ((v / self.px - off).round() + off) * self.px
    }
    fn snap_e(&self, v: f32) -> f32 {
        (v / self.px).round() * self.px
    }
    fn sp(&self, x: f32, y: f32) -> Pos2 {
        let q = self.raw(x, y);
        pos2(self.snap_c(q.x), self.snap_c(q.y))
    }
    /// Straight stroke; axis-aligned ones are hinted.
    fn line(&self, x0: f32, y0: f32, x1: f32, y1: f32) {
        let (a, b) = (self.raw(x0, y0), self.raw(x1, y1));
        let (a, b) = if y0 == y1 {
            let y = self.snap_c(a.y);
            (pos2(self.snap_e(a.x), y), pos2(self.snap_e(b.x), y))
        } else if x0 == x1 {
            let x = self.snap_c(a.x);
            (pos2(x, self.snap_e(a.y)), pos2(x, self.snap_e(b.y)))
        } else {
            (a, b)
        };
        self.painter.line_segment([a, b], self.stroke);
    }
    fn poly(&self, pts: &[(f32, f32)], closed: bool) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.sp(x, y)).collect();
        if closed {
            self.painter.add(Shape::closed_line(v, self.stroke));
        } else {
            self.painter.add(Shape::line(v, self.stroke));
        }
    }
    fn path(&self, pts: &[(f32, f32)], closed: bool) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.raw(x, y)).collect();
        if closed {
            self.painter.add(Shape::closed_line(v, self.stroke));
        } else {
            self.painter.add(Shape::line(v, self.stroke));
        }
    }
    /// Stroked rectangle through stroke centres.
    fn rect(&self, x0: f32, y0: f32, x1: f32, y1: f32) {
        self.poly(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)], true);
    }
    /// Filled rectangle by edges.
    fn fill_rect(&self, x0: f32, y0: f32, x1: f32, y1: f32) {
        let a = self.raw(x0, y0);
        let b = self.raw(x1, y1);
        let r = Rect::from_min_max(
            pos2(self.snap_e(a.x), self.snap_e(a.y)),
            pos2(self.snap_e(b.x), self.snap_e(b.y)),
        );
        self.painter.rect_filled(r, 0.0, self.color);
    }
    fn fill(&self, pts: &[(f32, f32)]) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.raw(x, y)).collect();
        self.painter
            .add(Shape::convex_polygon(v, self.color, Stroke::NONE));
    }
    fn arc_pts(cx: f32, cy: f32, r: f32, a0: f32, a1: f32) -> Vec<(f32, f32)> {
        let n = (((a1 - a0).abs() / TAU) * 32.0).ceil().max(4.0) as usize;
        (0..=n)
            .map(|i| {
                let a = a0 + (a1 - a0) * i as f32 / n as f32;
                (cx + r * a.cos(), cy + r * a.sin())
            })
            .collect()
    }
    fn arc(&self, cx: f32, cy: f32, r: f32, a0: f32, a1: f32) {
        self.path(&Self::arc_pts(cx, cy, r, a0, a1), false);
    }
    fn circle(&self, cx: f32, cy: f32, r: f32) {
        self.painter
            .circle_stroke(self.raw(cx, cy), r * self.s, self.stroke);
    }
    fn disc(&self, cx: f32, cy: f32, r: f32) {
        self.painter
            .circle_filled(self.raw(cx, cy), r * self.s, self.color);
    }
    /// Filled arrow head with its tip at (x, y) pointing at `angle`
    /// (radians, y down).
    fn head(&self, x: f32, y: f32, angle: f32, len: f32) {
        let w = len * 0.55;
        let (c, s) = (angle.cos(), angle.sin());
        let bx = x - c * len;
        let by = y - s * len;
        self.fill(&[(x, y), (bx - s * w, by + c * w), (bx + s * w, by - c * w)]);
    }
}

/// Paints `icon` into `rect` (square; the 16-unit grid is scaled to it).
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let side = rect.width().min(rect.height());
    let s = side / 16.0;
    let px = 1.0 / painter.pixels_per_point();
    let o = pos2(
        (rect.center().x - side / 2.0) / px,
        (rect.center().y - side / 2.0) / px,
    );
    let o = pos2(o.x.round() * px, o.y.round() * px);
    let width = (s / px).round().max(1.0) * px;
    let p = P {
        painter,
        o,
        s,
        px,
        stroke: Stroke::new(width, color),
        color,
    };
    draw(&p, icon);
}

fn draw(p: &P<'_>, icon: Icon) {
    use Icon as I;
    match icon {
        I::Select => {
            p.fill(&[(4.0, 2.0), (4.0, 13.5), (7.0, 10.5), (12.0, 10.5)]);
            p.line(7.5, 10.5, 9.5, 14.5);
        }
        I::Pan => {
            p.line(8.0, 1.5, 8.0, 14.5);
            p.line(1.5, 8.0, 14.5, 8.0);
            p.head(8.0, 1.0, -FRAC_PI_2, 3.0);
            p.head(8.0, 15.0, FRAC_PI_2, 3.0);
            p.head(1.0, 8.0, PI, 3.0);
            p.head(15.0, 8.0, 0.0, 3.0);
        }
        I::ViewFab => {
            // Board outline with a notch and a hole.
            p.poly(
                &[
                    (2.5, 3.5),
                    (8.5, 3.5),
                    (8.5, 5.5),
                    (10.5, 5.5),
                    (10.5, 3.5),
                    (13.5, 3.5),
                    (13.5, 12.5),
                    (2.5, 12.5),
                ],
                true,
            );
            p.circle(5.5, 9.0, 1.4);
            p.line(9.0, 10.5, 11.5, 10.5);
        }
        I::ViewAssembly => {
            p.rect(1.5, 2.5, 14.5, 13.5);
            p.rect(4.5, 5.5, 8.5, 10.5);
            p.fill_rect(10.0, 5.0, 13.0, 7.0);
            p.fill_rect(10.0, 9.0, 13.0, 11.0);
            p.disc(5.5, 6.5, 0.8);
        }
        I::ViewDrill => {
            p.rect(1.5, 2.5, 14.5, 13.5);
            p.circle(5.5, 6.5, 1.6);
            p.line(9.0, 5.0, 12.0, 8.0);
            p.line(12.0, 5.0, 9.0, 8.0);
            p.line(5.5, 9.5, 5.5, 12.0);
            p.line(4.2, 10.75, 6.8, 10.75);
            p.circle(10.5, 10.5, 1.2);
        }
        I::Table | I::TableDrill | I::TableStackup | I::TableBom => {
            p.rect(1.5, 2.5, 14.5, 13.5);
            p.fill_rect(1.5, 2.5, 14.5, 5.0);
            p.line(1.5, 8.5, 14.5, 8.5);
            match icon {
                I::TableStackup => {
                    p.line(1.5, 11.0, 14.5, 11.0);
                }
                I::TableDrill => {
                    p.line(6.5, 5.0, 6.5, 13.5);
                    p.circle(4.0, 11.0, 1.0);
                }
                I::TableBom => {
                    p.line(4.5, 5.0, 4.5, 13.5);
                    p.line(10.5, 5.0, 10.5, 13.5);
                    p.line(1.5, 11.0, 14.5, 11.0);
                }
                _ => {
                    p.line(8.0, 5.0, 8.0, 13.5);
                    p.line(1.5, 11.0, 14.5, 11.0);
                }
            }
        }
        I::DimLinear => {
            p.line(2.5, 3.0, 2.5, 13.0);
            p.line(13.5, 3.0, 13.5, 13.0);
            p.line(4.0, 8.5, 12.0, 8.5);
            p.head(3.0, 8.5, PI, 3.0);
            p.head(13.0, 8.5, 0.0, 3.0);
        }
        I::DimVertical => {
            p.line(3.0, 2.5, 13.0, 2.5);
            p.line(3.0, 13.5, 13.0, 13.5);
            p.line(8.5, 4.0, 8.5, 12.0);
            p.head(8.5, 3.0, -FRAC_PI_2, 3.0);
            p.head(8.5, 13.0, FRAC_PI_2, 3.0);
        }
        I::DimAligned => {
            p.line(2.0, 9.0, 7.0, 14.0);
            p.line(9.0, 2.0, 14.0, 7.0);
            p.line(4.0, 7.5, 8.0, 3.5);
            p.head(3.0, 8.5, 0.75 * PI, 3.0);
            p.head(9.0, 2.5, -0.25 * PI, 3.0);
        }
        I::DimRadius => {
            p.circle(8.0, 8.0, 6.0);
            p.line(8.0, 8.0, 11.6, 4.4);
            p.head(12.2, 3.8, -0.25 * PI, 3.0);
            p.disc(8.0, 8.0, 1.0);
        }
        I::DimDiameter => {
            p.circle(8.0, 8.0, 6.0);
            p.line(4.4, 11.6, 11.6, 4.4);
            p.head(12.2, 3.8, -0.25 * PI, 3.0);
            p.head(3.8, 12.2, 0.75 * PI, 3.0);
        }
        I::DimOrdinate => {
            p.line(2.5, 2.0, 2.5, 14.0);
            p.line(2.0, 13.5, 14.0, 13.5);
            p.line(6.5, 5.0, 6.5, 13.5);
            p.line(10.5, 8.0, 10.5, 13.5);
            p.line(2.5, 5.5, 6.5, 5.5);
            p.line(2.5, 8.5, 10.5, 8.5);
        }
        I::Callout => {
            p.circle(10.0, 6.0, 4.5);
            p.line(10.0, 4.0, 10.0, 8.5);
            p.line(6.6, 9.0, 3.5, 12.5);
            p.head(2.6, 13.5, 0.75 * PI, 3.5);
        }
        I::Notes => {
            p.rect(2.5, 1.5, 13.5, 14.5);
            p.disc(5.0, 5.0, 0.9);
            p.line(7.0, 5.5, 11.5, 5.5);
            p.disc(5.0, 8.0, 0.9);
            p.line(7.0, 8.5, 11.5, 8.5);
            p.disc(5.0, 11.0, 0.9);
            p.line(7.0, 11.5, 10.0, 11.5);
        }
        I::Text => {
            p.line(3.0, 3.5, 13.0, 3.5);
            p.line(3.5, 3.0, 3.5, 5.5);
            p.line(12.5, 3.0, 12.5, 5.5);
            p.line(8.5, 3.5, 8.5, 13.5);
            p.line(6.0, 13.5, 11.0, 13.5);
        }
        I::Line => {
            p.line(3.0, 13.0, 13.0, 3.0);
            p.disc(3.0, 13.0, 1.4);
            p.disc(13.0, 3.0, 1.4);
        }
        I::Rect => {
            p.rect(2.5, 3.5, 13.5, 12.5);
        }
        I::Graphic => {
            p.rect(2.5, 6.5, 9.5, 13.5);
            p.circle(10.5, 5.5, 3.5);
        }
        I::AlignLeft | I::AlignHCenter | I::AlignRight => {
            let x = match icon {
                I::AlignLeft => 2.5,
                I::AlignHCenter => 8.5,
                _ => 13.5,
            };
            p.line(x, 1.0, x, 15.0);
            let (w1, w2) = (9.0, 5.0);
            let pos = |w: f32| match icon {
                I::AlignLeft => (3.5, 3.5 + w),
                I::AlignHCenter => (8.5 - w / 2.0, 8.5 + w / 2.0),
                _ => (13.5 - w, 13.5),
            };
            let (a0, a1) = pos(w1);
            p.fill_rect(a0, 3.0, a1, 7.0);
            let (b0, b1) = pos(w2);
            p.fill_rect(b0, 9.0, b1, 13.0);
        }
        I::AlignTop | I::AlignVCenter | I::AlignBottom => {
            let y = match icon {
                I::AlignTop => 2.5,
                I::AlignVCenter => 8.5,
                _ => 13.5,
            };
            p.line(1.0, y, 15.0, y);
            let pos = |h: f32| match icon {
                I::AlignTop => (3.5, 3.5 + h),
                I::AlignVCenter => (8.5 - h / 2.0, 8.5 + h / 2.0),
                _ => (13.5 - h, 13.5),
            };
            let (a0, a1) = pos(9.0);
            p.fill_rect(3.0, a0, 7.0, a1);
            let (b0, b1) = pos(5.0);
            p.fill_rect(9.0, b0, 13.0, b1);
        }
        I::DistributeH => {
            p.line(1.5, 2.0, 1.5, 14.0);
            p.line(14.5, 2.0, 14.5, 14.0);
            p.fill_rect(3.5, 4.0, 6.0, 12.0);
            p.fill_rect(10.0, 4.0, 12.5, 12.0);
            p.line(7.5, 8.5, 8.5, 8.5);
        }
        I::DistributeV => {
            p.line(2.0, 1.5, 14.0, 1.5);
            p.line(2.0, 14.5, 14.0, 14.5);
            p.fill_rect(4.0, 3.5, 12.0, 6.0);
            p.fill_rect(4.0, 10.0, 12.0, 12.5);
            p.line(8.5, 7.5, 8.5, 8.5);
        }
        I::MatchWidth => {
            p.rect(2.5, 2.5, 13.5, 6.5);
            p.rect(2.5, 9.5, 13.5, 13.5);
            p.line(5.0, 11.5, 11.0, 11.5);
        }
        I::MatchHeight => {
            p.rect(2.5, 2.5, 6.5, 13.5);
            p.rect(9.5, 2.5, 13.5, 13.5);
            p.line(11.5, 5.0, 11.5, 11.0);
        }
        I::MatchScale => {
            p.rect(1.5, 6.5, 7.5, 12.5);
            p.rect(8.5, 2.5, 14.5, 8.5);
            p.line(5.0, 9.0, 11.0, 5.0);
        }
        I::Center => {
            p.rect(1.5, 1.5, 14.5, 14.5);
            p.fill_rect(5.0, 5.0, 11.0, 11.0);
        }
        I::Undo | I::Redo => {
            let flip = icon == I::Redo;
            let x = |v: f32| if flip { 16.0 - v } else { v };
            let pts: Vec<(f32, f32)> = P::arc_pts(9.0, 9.0, 4.5, -FRAC_PI_2, FRAC_PI_2)
                .into_iter()
                .map(|(a, b)| (x(a), b))
                .collect();
            p.path(&pts, false);
            p.line(x(9.0), 4.5, x(5.0), 4.5);
            p.line(x(9.0), 13.5, x(5.0), 13.5);
            p.head(x(2.5), 4.5, if flip { 0.0 } else { PI }, 3.5);
        }
        I::ZoomIn | I::ZoomOut | I::Search => {
            p.circle(6.5, 6.5, 4.5);
            p.line(10.0, 10.0, 14.0, 14.0);
            if icon != I::Search {
                p.line(4.5, 6.5, 8.5, 6.5);
                if icon == I::ZoomIn {
                    p.line(6.5, 4.5, 6.5, 8.5);
                }
            }
        }
        I::FitPage | I::FitSelection => {
            for (x, y, dx, dy) in [
                (1.5, 1.5, 1.0, 1.0),
                (14.5, 1.5, -1.0, 1.0),
                (1.5, 14.5, 1.0, -1.0),
                (14.5, 14.5, -1.0, -1.0),
            ] {
                p.line(x, y, x + 4.0 * dx, y);
                p.line(x, y, x, y + 4.0 * dy);
            }
            if icon == I::FitPage {
                p.rect(5.5, 4.5, 10.5, 11.5);
            } else {
                p.fill_rect(5.0, 5.0, 11.0, 11.0);
            }
        }
        I::Export => {
            p.poly(
                &[
                    (5.5, 6.5),
                    (2.5, 6.5),
                    (2.5, 14.5),
                    (13.5, 14.5),
                    (13.5, 6.5),
                    (10.5, 6.5),
                ],
                false,
            );
            p.line(8.0, 1.5, 8.0, 10.5);
            p.head(8.0, 1.0, -FRAC_PI_2, 3.5);
        }
        I::Reload => {
            p.arc(8.0, 8.5, 5.5, -0.3 * PI, 1.4 * PI);
            p.head(9.6, 2.8, -0.16, 3.5);
        }
        I::Snap => {
            // Magnet.
            p.path(&P::arc_pts(8.0, 7.5, 4.5, 0.0, PI), false);
            p.line(3.5, 7.5, 3.5, 2.5);
            p.line(12.5, 7.5, 12.5, 2.5);
            p.fill_rect(3.0, 2.0, 6.0, 4.0);
            p.fill_rect(10.0, 2.0, 13.0, 4.0);
            p.line(6.5, 2.5, 6.5, 7.5);
            p.line(9.5, 2.5, 9.5, 7.5);
            p.arc(8.0, 7.5, 1.5, 0.0, PI);
        }
        I::Grid => {
            for v in [2.5, 6.5, 10.5, 14.5] {
                p.line(v, 1.5, v, 14.5);
                p.line(1.5, v, 14.5, v);
            }
        }
        I::Layers => {
            p.poly(&[(8.0, 2.0), (14.5, 5.5), (8.0, 9.0), (1.5, 5.5)], true);
            p.poly(&[(1.5, 8.5), (8.0, 12.0), (14.5, 8.5)], false);
            p.poly(&[(1.5, 11.0), (8.0, 14.5), (14.5, 11.0)], false);
        }
        I::Page | I::PageAdd | I::PageDuplicate | I::PageDelete | I::New => {
            if icon == I::PageDuplicate {
                p.poly(&[(2.5, 11.5), (2.5, 1.5), (9.5, 1.5)], false);
                p.rect(5.5, 4.5, 13.5, 14.5);
                return;
            }
            p.poly(
                &[
                    (3.5, 1.5),
                    (9.5, 1.5),
                    (12.5, 4.5),
                    (12.5, 14.5),
                    (3.5, 14.5),
                ],
                true,
            );
            p.poly(&[(9.5, 1.5), (9.5, 4.5), (12.5, 4.5)], false);
            match icon {
                I::PageAdd | I::New => {
                    p.line(8.0, 7.0, 8.0, 12.0);
                    p.line(5.5, 9.5, 10.5, 9.5);
                }
                I::PageDelete => {
                    p.line(5.5, 9.5, 10.5, 9.5);
                }
                _ => {}
            }
        }
        I::Open => {
            p.poly(
                &[
                    (1.5, 13.5),
                    (1.5, 2.5),
                    (6.0, 2.5),
                    (7.5, 4.5),
                    (12.5, 4.5),
                    (12.5, 6.5),
                ],
                false,
            );
            p.poly(&[(1.5, 13.5), (4.0, 7.5), (15.0, 7.5), (12.5, 13.5)], true);
        }
        I::Save => {
            p.poly(
                &[
                    (2.5, 2.5),
                    (11.5, 2.5),
                    (13.5, 4.5),
                    (13.5, 13.5),
                    (2.5, 13.5),
                ],
                true,
            );
            p.rect(5.5, 2.5, 10.5, 5.5);
            p.rect(4.5, 8.5, 11.5, 13.5);
        }
        I::Eye | I::EyeOff => {
            let top = P::arc_pts(8.0, 13.0, 8.0, -0.83 * PI, -0.17 * PI);
            let bot = P::arc_pts(8.0, 3.0, 8.0, 0.17 * PI, 0.83 * PI);
            p.path(&top, false);
            p.path(&bot, false);
            p.circle(8.0, 8.0, 2.0);
            if icon == I::EyeOff {
                p.line(2.5, 13.5, 13.5, 2.5);
            }
        }
        I::Lock | I::Unlock => {
            p.rect(3.5, 7.5, 12.5, 14.5);
            if icon == I::Lock {
                p.arc(8.0, 5.5, 3.0, PI, TAU);
                p.line(5.0, 5.5, 5.0, 7.5);
                p.line(11.0, 5.5, 11.0, 7.5);
            } else {
                p.arc(10.0, 4.5, 3.0, PI, TAU);
                p.line(13.0, 4.5, 13.0, 6.0);
                p.line(7.0, 4.5, 7.0, 7.5);
            }
            p.line(8.0, 10.0, 8.0, 12.0);
        }
        I::Plus => {
            p.line(8.5, 3.0, 8.5, 14.0);
            p.line(3.0, 8.5, 14.0, 8.5);
        }
        I::Minus => {
            p.line(3.0, 8.5, 14.0, 8.5);
        }
        I::Close => {
            p.line(4.0, 4.0, 12.0, 12.0);
            p.line(12.0, 4.0, 4.0, 12.0);
        }
        I::ChevronDown => {
            p.path(&[(4.5, 6.5), (8.0, 10.0), (11.5, 6.5)], false);
        }
        I::ChevronRight => {
            p.path(&[(6.5, 4.5), (10.0, 8.0), (6.5, 11.5)], false);
        }
        I::Sun => {
            p.circle(8.0, 8.0, 3.0);
            for k in 0..8 {
                let a = k as f32 * PI / 4.0;
                let (c, s) = (a.cos(), a.sin());
                p.path(
                    &[
                        (8.0 + 4.8 * c, 8.0 + 4.8 * s),
                        (8.0 + 6.8 * c, 8.0 + 6.8 * s),
                    ],
                    false,
                );
            }
        }
        I::Moon => {
            // Crescent: the part of circle A outside circle B.
            let (ax, ay, ar) = (8.0f32, 8.5f32, 6.0f32);
            let (bx, by, br) = (11.5f32, 5.0f32, 5.0f32);
            let d = ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt();
            let a = (ar * ar - br * br + d * d) / (2.0 * d);
            let h = (ar * ar - a * a).max(0.0).sqrt();
            let (ux, uy) = ((bx - ax) / d, (by - ay) / d);
            let (mx, my) = (ax + a * ux, ay + a * uy);
            let i1 = (mx - h * uy, my + h * ux);
            let i2 = (mx + h * uy, my - h * ux);
            let ang = |cx: f32, cy: f32, p: (f32, f32)| (p.1 - cy).atan2(p.0 - cx);
            let (a1, a2) = (ang(ax, ay, i1), ang(ax, ay, i2));
            let mut a2b = a2;
            while a2b < a1 {
                a2b += TAU;
            }
            let mut pts = P::arc_pts(ax, ay, ar, a1, a2b);
            let (b1, b2) = (ang(bx, by, i2), ang(bx, by, i1));
            let mut b2b = b2;
            while b2b > b1 {
                b2b -= TAU;
            }
            pts.extend(P::arc_pts(bx, by, br, b1, b2b));
            p.path(&pts, true);
        }
        I::Warning => {
            p.poly(&[(8.0, 1.5), (15.0, 14.5), (1.0, 14.5)], true);
            p.line(8.0, 6.0, 8.0, 10.0);
            p.disc(8.0, 12.2, 0.9);
        }
        I::Error => {
            p.circle(8.0, 8.0, 6.5);
            p.line(5.5, 5.5, 10.5, 10.5);
            p.line(10.5, 5.5, 5.5, 10.5);
        }
        I::Info => {
            p.circle(8.0, 8.0, 6.5);
            p.line(8.0, 7.0, 8.0, 12.0);
            p.disc(8.0, 4.8, 0.9);
        }
        I::Check => {
            p.path(&[(3.0, 8.5), (6.5, 12.0), (13.0, 4.5)], false);
        }
        I::More => {
            for x in [3.5, 8.0, 12.5] {
                p.disc(x, 8.0, 1.3);
            }
        }
        I::Command => {
            p.rect(1.5, 3.5, 14.5, 12.5);
            p.path(&[(4.0, 6.0), (6.5, 8.0), (4.0, 10.0)], false);
            p.line(8.0, 10.5, 12.0, 10.5);
        }
        I::Trash => {
            p.line(2.0, 3.5, 14.0, 3.5);
            p.poly(&[(6.5, 3.5), (6.5, 1.5), (9.5, 1.5), (9.5, 3.5)], false);
            p.poly(&[(3.5, 3.5), (4.5, 14.5), (11.5, 14.5), (12.5, 3.5)], false);
            p.line(6.5, 6.0, 6.5, 12.0);
            p.line(9.5, 6.0, 9.5, 12.0);
        }
        I::Duplicate => {
            p.rect(1.5, 1.5, 10.5, 10.5);
            p.rect(5.5, 5.5, 14.5, 14.5);
        }
        I::Raise | I::Lower => {
            let up = icon == I::Raise;
            p.rect(1.5, 5.5, 10.5, 14.5);
            p.line(13.5, 2.0, 13.5, 13.0);
            if up {
                p.head(13.5, 1.0, -FRAC_PI_2, 3.5);
            } else {
                p.head(13.5, 15.0, FRAC_PI_2, 3.5);
            }
        }
        I::Keyboard => {
            p.rect(1.5, 4.5, 14.5, 12.5);
            for x in [4.0, 7.0, 10.0, 13.0] {
                p.fill_rect(x - 1.0, 6.0, x, 7.0);
            }
            p.line(5.0, 10.5, 11.0, 10.5);
        }
        I::Ruler => {
            p.poly(&[(1.5, 10.5), (10.5, 1.5), (14.5, 5.5), (5.5, 14.5)], true);
            for k in 0..4 {
                let t = 3.5 + 2.0 * k as f32;
                p.path(&[(t, 8.5 - (t - 3.5)), (t + 1.5, 10.0 - (t - 3.5))], false);
            }
        }
        I::Board => {
            p.rect(1.5, 3.5, 14.5, 12.5);
            p.circle(4.5, 6.5, 1.0);
            p.line(7.0, 6.5, 12.0, 6.5);
            p.line(7.0, 9.5, 12.0, 9.5);
        }
        I::Issues => {
            p.circle(8.0, 8.0, 6.5);
            p.line(8.0, 4.5, 8.0, 9.0);
            p.disc(8.0, 11.3, 0.9);
        }
        I::View3d => {
            p.poly(
                &[
                    (8.0, 1.5),
                    (14.0, 4.5),
                    (14.0, 11.5),
                    (8.0, 14.5),
                    (2.0, 11.5),
                    (2.0, 4.5),
                ],
                true,
            );
            p.path(&[(2.0, 4.5), (8.0, 7.5), (14.0, 4.5)], false);
            p.line(8.0, 7.5, 8.0, 14.5);
        }
        I::Polyline => {
            p.path(&[(1.5, 13.5), (5.0, 4.0), (10.0, 11.0), (14.5, 2.5)], false);
        }
        I::Circle => {
            p.circle(8.0, 8.0, 6.0);
        }
        I::Link => {
            p.poly(
                &[
                    (1.5, 7.0),
                    (4.0, 4.5),
                    (8.0, 4.5),
                    (10.5, 7.0),
                    (8.0, 9.5),
                    (6.5, 9.5),
                ],
                false,
            );
            p.poly(
                &[
                    (14.5, 9.0),
                    (12.0, 11.5),
                    (8.0, 11.5),
                    (5.5, 9.0),
                    (8.0, 6.5),
                    (9.5, 6.5),
                ],
                false,
            );
            p.line(1.5, 7.0, 4.0, 9.5);
            p.line(4.0, 9.5, 5.0, 9.5);
            p.line(14.5, 9.0, 12.0, 6.5);
            p.line(12.0, 6.5, 11.0, 6.5);
        }
        I::Unlink => {
            p.poly(
                &[(6.5, 4.5), (4.0, 4.5), (1.5, 7.0), (4.0, 9.5), (6.5, 9.5)],
                false,
            );
            p.poly(
                &[
                    (9.5, 6.5),
                    (12.0, 6.5),
                    (14.5, 9.0),
                    (12.0, 11.5),
                    (9.5, 11.5),
                ],
                false,
            );
            p.line(3.0, 1.5, 13.0, 14.5);
        }
        I::Settings => {
            // Gear: a ring with eight teeth.
            p.circle(8.0, 8.0, 2.2);
            p.circle(8.0, 8.0, 4.6);
            for k in 0..8 {
                let a = k as f32 * PI / 4.0;
                let (c, s) = (a.cos(), a.sin());
                p.path(
                    &[
                        (8.0 + 4.6 * c, 8.0 + 4.6 * s),
                        (8.0 + 6.6 * c, 8.0 + 6.6 * s),
                    ],
                    false,
                );
            }
        }
        I::Log => {
            p.rect(2.5, 1.5, 13.5, 14.5);
            p.line(5.0, 5.5, 11.0, 5.5);
            p.line(5.0, 8.5, 11.0, 8.5);
            p.line(5.0, 11.5, 9.0, 11.5);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for i in Icon::ALL {
            assert!(seen.insert(i.name()), "{}", i.name());
            assert_eq!(Icon::from_name(i.name()), Some(*i));
        }
        assert_eq!(Icon::from_name("nope"), None);
    }
}
