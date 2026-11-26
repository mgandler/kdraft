//! Native, schematic rendering of the board model: outline, holes, pads,
//! footprint boxes and reference designators. Independent of `kicad-cli`;
//! it is not a plot of the copper or fab layers.

use kdraft_board::geom::{Affine, BBox, Point};
use kdraft_board::outline::{EdgeShape, PolyItem};
use kdraft_board::{Board, DrillShape, Footprint, Hole, Pad, Side};

use crate::dl::{Color, HAlign, Item, Path, Stroke, TextRun, VAlign};
use crate::font::Fonts;
use crate::view::ViewMap;

/// What a native view draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Content {
    pub outline: bool,
    pub holes: bool,
    pub pads: bool,
    pub footprint_boxes: bool,
    pub designators: bool,
    /// Drill symbols per drill-table row at every hole.
    pub symbols: bool,
}

impl Content {
    pub const FABRICATION: Content = Content {
        outline: true,
        holes: true,
        pads: false,
        footprint_boxes: false,
        designators: false,
        symbols: false,
    };
    pub const ASSEMBLY: Content = Content {
        outline: true,
        holes: false,
        pads: true,
        footprint_boxes: true,
        designators: true,
        symbols: false,
    };
    pub const DRILL: Content = Content {
        outline: true,
        holes: true,
        pads: false,
        footprint_boxes: false,
        designators: false,
        symbols: true,
    };
    pub const LAYER: Content = Content {
        outline: true,
        holes: false,
        pads: true,
        footprint_boxes: false,
        designators: false,
        symbols: false,
    };
}

pub const OUTLINE_WIDTH: f64 = 0.25;
pub const HOLE_WIDTH: f64 = 0.12;
pub const PAD_WIDTH: f64 = 0.06;
pub const BOX_WIDTH: f64 = 0.1;
/// Designator cap height limits, sheet mm.
pub const DESIGNATOR_MAX: f64 = 2.5;
pub const DESIGNATOR_MIN: f64 = 0.35;

pub fn outline_path(shape: &EdgeShape) -> Path {
    match shape {
        EdgeShape::Line { start, end } => Path::line(*start, *end),
        EdgeShape::Arc { start, mid, end } => Path::arc3(*start, *mid, *end),
        EdgeShape::Circle { center, radius } => Path::circle(*center, *radius),
        EdgeShape::Rect { start, end } => BBox::from_points([*start, *end])
            .map(|b| Path::rect(&b))
            .unwrap_or_default(),
        EdgeShape::Poly { items } => {
            let mut p = Path::new();
            let mut started = false;
            for it in items {
                match it {
                    PolyItem::Point { at } => {
                        if started {
                            p.line_to(*at);
                        } else {
                            p.move_to(*at);
                            started = true;
                        }
                    }
                    PolyItem::Arc { start, mid, end } => {
                        if started {
                            p.line_to(*start);
                        } else {
                            p.move_to(*start);
                            started = true;
                        }
                        p.arc3_to(*start, *mid, *end);
                    }
                }
            }
            if started {
                p.close();
            }
            p
        }
        EdgeShape::Curve { points } => {
            let mut p = Path::new();
            p.move_to(points[0])
                .cubic_to(points[1], points[2], points[3]);
            p
        }
    }
}

/// Stadium (slot) of size `w` x `h` centred at the origin.
fn stadium(w: f64, h: f64) -> Path {
    Path::round_rect_centered(w, h, w.min(h) / 2.0)
}

/// Pad outline in board coordinates.
pub fn pad_path(pad: &Pad) -> Path {
    let (w, h) = (pad.size.w, pad.size.h);
    let local = match pad.shape.as_str() {
        "circle" => Path::circle(Point::ORIGIN, w / 2.0),
        "oval" => stadium(w, h),
        "roundrect" => {
            let r = pad.roundrect_rratio.unwrap_or(0.25) * w.min(h);
            Path::round_rect_centered(w, h, r)
        }
        // rect, trapezoid, custom, chamfered: the size box.
        _ => Path::round_rect_centered(w, h, 0.0),
    };
    let m =
        Affine::rotate_deg(-pad.rotation).then(&Affine::translate(pad.position.x, pad.position.y));
    local.transform(&m)
}

/// Hole outline in board coordinates.
pub fn hole_path(h: &Hole) -> Path {
    match h.shape {
        DrillShape::Round { diameter } => Path::circle(h.position, diameter / 2.0),
        DrillShape::Oval { width, height } => {
            let m = Affine::rotate_deg(-h.rotation)
                .then(&Affine::translate(h.position.x, h.position.y));
            stadium(width, height).transform(&m)
        }
    }
}

fn on_side(pad: &Pad, side: Side) -> bool {
    let want = match side {
        Side::Front => "F.Cu",
        Side::Back => "B.Cu",
    };
    pad.layers
        .iter()
        .any(|l| l == want || l == "*.Cu" || l == "F&B.Cu")
}

/// Designator that fits `w` x `h` (sheet mm): cap height and rotation.
fn fit_designator(fonts: &Fonts, text: &str, w: f64, h: f64) -> Option<(f64, f64)> {
    let unit = fonts.width(text, 1.0, false).max(1e-9);
    let horiz = (0.8 * w / unit).min(0.6 * h).min(DESIGNATOR_MAX);
    let vert = (0.8 * h / unit).min(0.6 * w).min(DESIGNATOR_MAX);
    let (size, rot) = if vert > horiz * 1.2 {
        (vert, 90.0)
    } else {
        (horiz, 0.0)
    };
    (size >= DESIGNATOR_MIN).then_some((size, rot))
}

/// Colours of the native renderer's roles (from a view's layer colours).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub pads: Color,
    pub pad_fill: Color,
    pub holes: Color,
    pub outline: Color,
    /// Part bodies (fab or courtyard outlines, pad boxes).
    pub body: Color,
    pub designator: Color,
    pub value: Color,
    pub dnp: Color,
    pub leader: Color,
    pub symbols: Color,
}

impl Default for Palette {
    /// The black-and-grey look (preset `black`).
    fn default() -> Self {
        Palette {
            pads: Color::gray(90),
            pad_fill: Color::gray(225),
            holes: Color::BLACK,
            outline: Color::BLACK,
            body: Color::gray(40),
            designator: Color::BLACK,
            value: Color::gray(70),
            dnp: Color::gray(60),
            leader: Color::gray(80),
            symbols: Color::BLACK,
        }
    }
}

fn mix_white(c: Color, k: f64) -> Color {
    let m = |v: u8| (f64::from(v) + (255.0 - f64::from(v)) * k).round() as u8;
    Color::rgba(m(c.r), m(c.g), m(c.b), c.a)
}

impl Palette {
    /// The palette of a view: black presets keep the default look; other
    /// presets and overrides colour each role with its layer's colour.
    pub fn for_view(v: &kdraft_doc::View, st: &kdraft_doc::Styles) -> Palette {
        if v.plain_black(st) {
            return Palette::default();
        }
        let f = v.side == kdraft_doc::ViewSide::Front;
        let side = |front: &'static str, back: &'static str| if f { front } else { back };
        // Every role through the view's one resolution order: its own
        // override, the palette's role colour, the role's layer colour.
        let role = |r: &str, layer: &str| {
            let (c, o) = v.role_color(st, r, layer);
            crate::graphic::color(c, o)
        };
        // A layer view's pads in the colour of the copper layer it shows.
        let copper = v
            .layers
            .iter()
            .find(|l| l.ends_with(".Cu"))
            .map_or(side("F.Cu", "B.Cu"), String::as_str);
        let pads = role("pads", copper);
        let body = role("bodies", side("F.Fab", "B.Fab"));
        let designator = role("designators", side("F.SilkS", "B.SilkS"));
        Palette {
            pads,
            pad_fill: mix_white(pads, 0.8),
            // Holes and drill symbols stay grey unless a role colour is set.
            holes: v
                .role_own(st, "holes")
                .map_or(Color::gray(60), |(c, o)| crate::graphic::color(c, o)),
            outline: role("outline", "Edge.Cuts"),
            body,
            designator,
            value: body,
            dnp: role("dnp", side("F.CrtYd", "B.CrtYd")),
            leader: designator,
            symbols: v
                .role_own(st, "holes")
                .map_or(Color::gray(40), |(c, o)| crate::graphic::color(c, o)),
        }
    }
}

/// Draws the board in sheet coordinates through `map`. `side` selects
/// footprints and pads; text stays upright and unmirrored.
pub fn render(board: &Board, map: &ViewMap, side: Side, what: Content, fonts: &Fonts) -> Vec<Item> {
    render_with(board, map, side, what, fonts, &Palette::default())
}

/// [`render`] with a palette.
pub fn render_with(
    board: &Board,
    map: &ViewMap,
    side: Side,
    what: Content,
    fonts: &Fonts,
    pal: &Palette,
) -> Vec<Item> {
    let m = &map.board_to_sheet;
    let mut out = Vec::new();
    if what.pads {
        let gray = pal.pads;
        // All copper on this side, including through-hole pads of parts
        // mounted on the other side.
        for fp in &board.footprints {
            for pad in fp
                .pads
                .iter()
                .filter(|p| on_side(p, side) && p.size.w > 0.0)
            {
                out.push(Item::Path(crate::dl::PathItem {
                    path: pad_path(pad).transform(m),
                    stroke: Some(Stroke::solid(PAD_WIDTH, gray)),
                    fill: Some(crate::dl::Fill {
                        color: pal.pad_fill,
                        even_odd: false,
                    }),
                }));
            }
        }
    }
    if what.holes {
        for h in board.holes() {
            let p = hole_path(&h).transform(m);
            out.push(Item::stroked(p, Stroke::solid(HOLE_WIDTH, pal.holes)));
            if !h.plated {
                // Non-plated: a cross through the hole.
                let r = h.shape.tool_diameter() / 2.0 * std::f64::consts::FRAC_1_SQRT_2;
                let c = h.position;
                for (a, b) in [
                    (Point::new(c.x - r, c.y - r), Point::new(c.x + r, c.y + r)),
                    (Point::new(c.x - r, c.y + r), Point::new(c.x + r, c.y - r)),
                ] {
                    out.push(Item::stroked(
                        Path::line(m.apply(a), m.apply(b)),
                        Stroke::solid(HOLE_WIDTH, pal.holes),
                    ));
                }
            }
        }
    }
    if what.symbols {
        out.extend(drill_symbols_in(board, m, pal.symbols));
    }
    if what.outline {
        for prim in &board.outline.primitives {
            out.push(Item::stroked(
                outline_path(&prim.shape).transform(m),
                Stroke::solid(OUTLINE_WIDTH, pal.outline),
            ));
        }
    }
    // Boxes and designators last, so pad fills do not hide them.
    if what.footprint_boxes || what.designators {
        for fp in board.footprints.iter().filter(|f| f.side == side) {
            footprint(&mut out, fp, map, what, fonts, pal);
        }
    }
    out
}

/// Drill symbol target size, minimum and fraction of the hole pitch, sheet
/// mm.
pub const SYMBOL_SIZE: f64 = 2.0;
pub const SYMBOL_MIN: f64 = 0.6;
pub const SYMBOL_PITCH_FRACTION: f64 = 0.7;
pub const SYMBOL_WIDTH: f64 = 0.1;

/// The drill symbol of each hole's drill-table row at the hole's centre.
pub fn drill_symbols(board: &Board, m: &Affine) -> Vec<Item> {
    drill_symbols_in(board, m, Color::BLACK)
}

/// [`drill_symbols`] in `color`.
pub fn drill_symbols_in(board: &Board, m: &Affine, color: Color) -> Vec<Item> {
    let holes = board.holes();
    let rows = kdraft_board::drill_row_index(&holes);
    let centres: Vec<Point> = holes.iter().map(|h| m.apply(h.position)).collect();
    let sizes = crate::symbols::sizes(&centres, SYMBOL_SIZE, SYMBOL_MIN, SYMBOL_PITCH_FRACTION);
    let mut out = Vec::new();
    for ((c, s), r) in centres.iter().zip(&sizes).zip(&rows) {
        out.extend(crate::symbols::draw(
            &crate::symbols::symbol(*r),
            *c,
            *s,
            SYMBOL_WIDTH,
            color,
        ));
    }
    out
}

fn footprint(
    out: &mut Vec<Item>,
    fp: &Footprint,
    map: &ViewMap,
    what: Content,
    fonts: &Fonts,
    pal: &Palette,
) {
    let Some(b) = fp.fab.or(fp.courtyard) else {
        return;
    };
    let sb = map.board_to_sheet.apply_bbox(&b);
    if what.footprint_boxes {
        out.push(Item::stroked(
            Path::rect(&sb),
            Stroke::solid(BOX_WIDTH, pal.body),
        ));
    }
    if !what.designators {
        return;
    }
    let Some(r) = fp.reference.as_deref().filter(|r| !r.is_empty()) else {
        return;
    };
    if let Some((size, rotation)) = fit_designator(fonts, r, sb.width(), sb.height()) {
        out.push(Item::Text(TextRun {
            text: r.to_string(),
            at: sb.center(),
            size,
            h_align: HAlign::Center,
            v_align: VAlign::Center,
            rotation,
            bold: false,
            italic: false,
            color: pal.designator,
            invisible: false,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kdraft_doc::{Id, View, ViewKind, ViewSide};

    fn board() -> Board {
        Board::load(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/kicad9-minimal.kicad_pcb"),
        )
        .unwrap()
    }

    fn map(b: &Board, side: ViewSide) -> ViewMap {
        let v = View {
            id: Id("v".into()),
            kind: ViewKind::Assembly,
            side,
            scale: 5.0,
            rotation: 0.0,
            at: Point::new(200.0, 140.0),
            layers: vec![],
            crop: None,
            parent: None,
            style: None,
            frame: false,
            drill_marks: None,
            render: None,
            assembly: Default::default(),
            extra: vec![],
        };
        ViewMap::new(&v, b.outline.bbox.unwrap())
    }

    #[test]
    fn assembly_front_and_back() {
        let b = board();
        let f = Fonts::load().unwrap();
        let front = render(
            &b,
            &map(&b, ViewSide::Front),
            Side::Front,
            Content::ASSEMBLY,
            &f,
        );
        let back = render(
            &b,
            &map(&b, ViewSide::Back),
            Side::Back,
            Content::ASSEMBLY,
            &f,
        );
        let refs = |items: &[Item]| -> Vec<String> {
            crate::dl::texts(items)
                .iter()
                .map(|t| t.text.clone())
                .collect()
        };
        let fr = refs(&front);
        let br = refs(&back);
        assert!(fr.contains(&"R1".to_string()), "{fr:?}");
        assert!(!br.contains(&"R1".to_string()));
        assert_eq!(br.len(), 1, "{br:?}");
        // Back-side text is not mirrored or rotated by the view.
        for t in crate::dl::texts(&back) {
            assert!(t.rotation == 0.0 || t.rotation == 90.0);
        }
        // Everything lies inside the frame.
        let frame = map(&b, ViewSide::Front).frame;
        for it in &front {
            if let Item::Path(p) = it {
                let bb = p.path.control_bbox().unwrap();
                assert!(bb.min.x >= frame.min.x && bb.max.y <= frame.max.y, "{bb:?}");
            }
        }
    }

    #[test]
    fn fabrication_has_outline_and_holes() {
        let b = board();
        let f = Fonts::load().unwrap();
        let items = render(
            &b,
            &map(&b, ViewSide::Front),
            Side::Front,
            Content::FABRICATION,
            &f,
        );
        // 5 holes (+2 cross lines for the NPTH) and the outline primitives.
        assert_eq!(items.len(), 5 + 2 + b.outline.primitives.len());
        assert!(crate::dl::texts(&items).is_empty());
    }

    #[test]
    fn rotated_pad_outline() {
        let b = board();
        let fp = b
            .footprint_by_reference("U1")
            .or(b.footprints.last())
            .unwrap();
        let pad = &fp.pads[0];
        let bb = pad_path(pad).control_bbox().unwrap();
        assert!(
            bb.center().approx_eq(pad.position, 1e-9),
            "{bb:?} {:?}",
            pad.position
        );
    }

    #[test]
    fn designator_fit() {
        let f = Fonts::load().unwrap();
        let (s, r) = fit_designator(&f, "R1", 10.0, 4.0).unwrap();
        assert!(s <= 2.4 + 1e-9 && r == 0.0);
        let (_, r) = fit_designator(&f, "U10", 2.0, 12.0).unwrap();
        assert_eq!(r, 90.0);
        assert!(fit_designator(&f, "R1", 0.2, 0.2).is_none());
    }
}
