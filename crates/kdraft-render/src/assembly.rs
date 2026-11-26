//! Native assembly views: per footprint its fab graphics (fallback:
//! courtyard graphics, then the pad box), optional pad outlines, holes and
//! pin-1 marks, DNP styles, and reference designators placed by
//! [`kdraft_layout::declutter`].

use kdraft_board::geom::{BBox, Point};
use kdraft_board::{Board, Footprint, FpLayer, PadType, Side};
use kdraft_doc::{AssemblyOptions, DnpStyle, RefFilter};
use kdraft_layout::declutter::{self, Kind, LabelReq, Params, Placed, Stats};
use kdraft_layout::grid::SegmentGrid;

use crate::dl::{Cmd, Color, Fill, HAlign, Item, Path, PathItem, Stroke, TextRun, VAlign};
use crate::font::Fonts;
use crate::native::{hole_path, outline_path, pad_path, OUTLINE_WIDTH};
use crate::view::ViewMap;

pub const BODY_WIDTH: f64 = 0.13;
pub const PAD_OUTLINE_WIDTH: f64 = 0.05;
/// Smallest clearance between neighbouring pad outlines on paper, mm.
/// Below it a footprint's pads are drawn as one outline around the pad
/// field (single outlines would merge into a dark blob) and its body is
/// treated as occupied (the designator goes outside).
pub const MIN_PAD_CLEARANCE: f64 = 0.5;

/// Board box of a pad (rotated rectangle hull).
fn pad_box(p: &kdraft_board::Pad) -> BBox {
    let (hw, hh) = (p.size.w / 2.0, p.size.h / 2.0);
    let (s, c) = p.rotation.to_radians().sin_cos();
    let ex = (hw * c).abs() + (hh * s).abs();
    let ey = (hw * s).abs() + (hh * c).abs();
    BBox {
        min: Point::new(p.position.x - ex, p.position.y - ey),
        max: Point::new(p.position.x + ex, p.position.y + ey),
    }
}

fn box_gap(a: &BBox, b: &BBox) -> f64 {
    let dx = (b.min.x - a.max.x).max(a.min.x - b.max.x).max(0.0);
    let dy = (b.min.y - a.max.y).max(a.min.y - b.max.y).max(0.0);
    if dx == 0.0 && dy == 0.0 {
        // Touching or overlapping.
        0.0
    } else {
        (dx * dx + dy * dy).sqrt()
    }
}

/// Smallest clearance between two copper pads of a footprint, board mm
/// (`None` with fewer than two pads). Large pad counts use a window over
/// pads sorted by x.
pub fn min_pad_clearance(fp: &Footprint) -> Option<f64> {
    let mut boxes: Vec<BBox> = fp
        .pads
        .iter()
        .filter(|p| p.size.w > 0.0 && !matches!(p.kind, PadType::NpThruHole))
        .map(pad_box)
        .collect();
    if boxes.len() < 2 {
        return None;
    }
    boxes.sort_by(|a, b| a.min.x.total_cmp(&b.min.x));
    let mut best = f64::INFINITY;
    for i in 0..boxes.len() {
        for j in i + 1..boxes.len() {
            if boxes[j].min.x - boxes[i].max.x > best {
                break;
            }
            best = best.min(box_gap(&boxes[i], &boxes[j]));
        }
    }
    Some(best)
}

/// Whether the footprint's pads are too close to draw one by one at
/// `scale` (sheet mm per board mm).
pub fn dense_pads(fp: &Footprint, scale: f64) -> bool {
    min_pad_clearance(fp).is_some_and(|c| c * scale < MIN_PAD_CLEARANCE)
}

/// Board box around a footprint's copper pads.
fn pad_field(fp: &Footprint) -> Option<BBox> {
    fp.pads
        .iter()
        .filter(|p| p.size.w > 0.0 && !matches!(p.kind, PadType::NpThruHole))
        .map(pad_box)
        .reduce(|a, b| a.union(&b))
}
pub const LEADER_WIDTH: f64 = 0.08;

/// One designator and where it went.
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub reference: String,
    pub footprint: usize,
    pub placed: Placed,
}

/// The laid-out assembly view.
#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub items: Vec<Item>,
    pub labels: Vec<Label>,
    pub stats: Stats,
    /// Values that did not fit inside their body (values on only).
    pub values_omitted: usize,
}

impl Layout {
    pub fn unplaced(&self) -> Vec<&str> {
        self.labels
            .iter()
            .filter(|l| l.placed.kind == Kind::Unplaced)
            .map(|l| l.reference.as_str())
            .collect()
    }

    /// Sheet box of all drawn content.
    pub fn bbox(&self) -> Option<BBox> {
        let mut b = crate::dl::paths_bbox(&self.items);
        for l in &self.labels {
            if l.placed.kind != Kind::Unplaced {
                b = Some(match b {
                    Some(b) => b.union(&l.placed.rect),
                    None => l.placed.rect,
                });
            }
        }
        b
    }
}

/// Footprints whose designator the filter keeps.
pub fn wants_designator(fp: &Footprint, f: &RefFilter) -> bool {
    match f {
        RefFilter::All => true,
        RefFilter::Placed | RefFilter::Other(_) => {
            !fp.attributes.board_only && fp.has_copper_pads()
        }
        RefFilter::PosFiles => {
            !fp.attributes.board_only
                && fp.has_copper_pads()
                && !fp.attributes.exclude_from_pos_files
        }
    }
}

/// Body box: see [`Footprint::body`].
pub fn body(fp: &Footprint) -> Option<(BBox, FpLayer)> {
    fp.body()
}

fn polarised(fp: &Footprint) -> bool {
    let r = fp.reference.as_deref().unwrap_or("");
    let prefix: String = r.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let lib = fp.lib_id.to_ascii_lowercase();
    fp.pads.len() >= 3
        || matches!(prefix.as_str(), "D" | "LED" | "CR" | "ZD" | "BT")
        || lib.contains("cp_")
        || lib.contains("polarized")
        || lib.contains("tantal")
        || lib.contains("led")
        || lib.contains("diode")
}

/// Flattened outline in sheet coordinates.
fn outline_segments(board: &Board, map: &ViewMap) -> Vec<(Point, Point)> {
    let mut segs = Vec::new();
    for prim in &board.outline.primitives {
        let p = outline_path(&prim.shape).transform(&map.board_to_sheet);
        let mut cur = None;
        let mut start = None;
        for c in &p.cmds {
            match *c {
                Cmd::Move(q) => {
                    cur = Some(q);
                    start = Some(q);
                }
                Cmd::Line(q) => {
                    if let Some(a) = cur {
                        segs.push((a, q));
                    }
                    cur = Some(q);
                }
                Cmd::Cubic(c1, c2, q) => {
                    if let Some(a) = cur {
                        let mut prev = a;
                        for k in 1..=8 {
                            let t = f64::from(k) / 8.0;
                            let u = 1.0 - t;
                            let pt = Point::new(
                                u * u * u * a.x
                                    + 3.0 * u * u * t * c1.x
                                    + 3.0 * u * t * t * c2.x
                                    + t * t * t * q.x,
                                u * u * u * a.y
                                    + 3.0 * u * u * t * c1.y
                                    + 3.0 * u * t * t * c2.y
                                    + t * t * t * q.y,
                            );
                            segs.push((prev, pt));
                            prev = pt;
                        }
                    }
                    cur = Some(q);
                }
                Cmd::Close => {
                    if let (Some(a), Some(s)) = (cur, start) {
                        segs.push((a, s));
                    }
                    cur = start;
                }
            }
        }
    }
    segs
}

fn text(s: &str, at: Point, size: f64, rotation: f64, color: Color) -> Item {
    Item::Text(TextRun {
        text: s.to_string(),
        at,
        size,
        h_align: HAlign::Center,
        v_align: VAlign::Center,
        rotation,
        bold: false,
        italic: false,
        color,
        invisible: false,
    })
}

/// Lays out and draws a native assembly view of `side`.
pub fn render(
    board: &Board,
    map: &ViewMap,
    side: Side,
    opts: &AssemblyOptions,
    fonts: &Fonts,
) -> Layout {
    render_with(
        board,
        map,
        side,
        opts,
        fonts,
        &crate::native::Palette::default(),
    )
}

/// [`render`] with a palette.
pub fn render_with(
    board: &Board,
    map: &ViewMap,
    side: Side,
    opts: &AssemblyOptions,
    fonts: &Fonts,
    pal: &crate::native::Palette,
) -> Layout {
    let black = pal.designator == Color::BLACK;
    let m = &map.board_to_sheet;
    let dnp = opts.dnp();
    let mut out = Layout::default();
    let parts: Vec<(usize, &Footprint)> = board
        .footprints
        .iter()
        .enumerate()
        .filter(|(_, f)| f.side == side)
        .filter(|(_, f)| !(f.attributes.dnp && dnp == DnpStyle::Hidden))
        .collect();

    // Pads first (lowest), then holes, outline, bodies, marks, labels.
    if opts.pads() {
        let st = Stroke::solid(
            PAD_OUTLINE_WIDTH,
            if black { Color::gray(150) } else { pal.pads },
        );
        for (_, fp) in &parts {
            if dense_pads(fp, map.scale) {
                if let Some(f) = pad_field(fp) {
                    out.items
                        .push(Item::stroked(Path::rect(&f).transform(m), st.clone()));
                }
                continue;
            }
            for pad in fp.pads.iter().filter(|p| p.size.w > 0.0) {
                if matches!(pad.kind, PadType::NpThruHole) {
                    continue;
                }
                out.items
                    .push(Item::stroked(pad_path(pad).transform(m), st.clone()));
            }
        }
    }
    if opts.holes() {
        let st = Stroke::solid(
            PAD_OUTLINE_WIDTH,
            if black { Color::gray(90) } else { pal.holes },
        );
        for h in board.holes() {
            out.items
                .push(Item::stroked(hole_path(&h).transform(m), st.clone()));
        }
    }
    for prim in &board.outline.primitives {
        out.items.push(Item::stroked(
            outline_path(&prim.shape).transform(m),
            Stroke::solid(OUTLINE_WIDTH, pal.outline),
        ));
    }

    let mut reqs = Vec::new();
    let mut occupied = Vec::new();
    let mut req_fp = Vec::new();
    let mut marks = Vec::new();
    for &(fi, fp) in &parts {
        let grey = fp.attributes.dnp && dnp == DnpStyle::Greyed;
        let color = if grey { Color::gray(170) } else { pal.body };
        let Some((bb, kind)) = body(fp) else { continue };
        let wants = wants_designator(fp, &opts.refs());
        let label = wants && opts.references();
        let sb = m.apply_bbox(&bb);
        let st = Stroke::solid(BODY_WIDTH, color);
        match kind {
            FpLayer::Fab | FpLayer::Courtyard => {
                for g in fp
                    .graphics
                    .iter()
                    .filter(|g| g.kind == kind && g.side == fp.side)
                {
                    let path = outline_path(&g.shape).transform(m);
                    if g.filled {
                        out.items.push(Item::Path(PathItem {
                            path,
                            stroke: Some(st.clone()),
                            fill: Some(Fill {
                                color,
                                even_odd: false,
                            }),
                        }));
                    } else {
                        out.items.push(Item::stroked(path, st.clone()));
                    }
                }
            }
            // Pad box only: drawn for parts that get a designator, not for
            // stitching vias and similar board-only footprints.
            FpLayer::Silk if wants => out.items.push(Item::stroked(Path::rect(&sb), st.clone())),
            FpLayer::Silk => {}
        }
        if fp.attributes.dnp && dnp == DnpStyle::Crossed {
            let x = Stroke::solid(BODY_WIDTH, pal.dnp);
            out.items
                .push(Item::stroked(Path::line(sb.min, sb.max), x.clone()));
            out.items.push(Item::stroked(
                Path::line(
                    Point::new(sb.min.x, sb.max.y),
                    Point::new(sb.max.x, sb.min.y),
                ),
                x,
            ));
        }
        if opts.pin1() && polarised(fp) {
            if let Some(p1) = fp.pads.iter().find(|p| p.number == "1" || p.number == "A1") {
                let r = (0.3 * p1.size.w.min(p1.size.h) * map.scale).clamp(0.15, 0.45);
                let c = m.apply(p1.position);
                out.items.push(Item::filled(Path::circle(c, r), color));
                marks.push(BBox {
                    min: Point::new(c.x - r, c.y - r),
                    max: Point::new(c.x + r, c.y + r),
                });
            }
        }
        if let Some(r) = fp.reference.as_deref().filter(|r| !r.is_empty() && label) {
            // A dense pad field counts as part of the body and fills it.
            let dense = dense_pads(fp, map.scale);
            let body = match pad_field(fp).filter(|_| dense) {
                Some(f) => sb.union(&m.apply_bbox(&f)),
                None => sb,
            };
            reqs.push(LabelReq {
                body,
                unit_width: fonts.width(r, 1.0, false),
            });
            occupied.push(dense);
            req_fp.push((fi, r.to_string(), grey));
        }
    }

    let min_h = opts.text_min();
    let grow = 6.0 * min_h;
    let area = BBox {
        min: Point::new(map.frame.min.x - grow, map.frame.min.y - grow),
        max: Point::new(map.frame.max.x + grow, map.frame.max.y + grow),
    };
    let params = Params::new(min_h, opts.text_max(), area);
    let segs = SegmentGrid::new((4.0 * min_h).max(1.0), outline_segments(board, map));
    let placed = declutter::place_with_options(&reqs, &occupied, &marks, &segs, &params);
    out.stats = declutter::stats(&placed);
    for (pl, (fi, r, grey)) in placed.into_iter().zip(req_fp) {
        let color = if grey {
            Color::gray(150)
        } else {
            pal.designator
        };
        if pl.kind != Kind::Unplaced {
            if let Some((a, b)) = pl.leader {
                out.items.push(Item::stroked(
                    Path::line(a, b),
                    Stroke::solid(LEADER_WIDTH, pal.leader),
                ));
                out.items
                    .push(Item::filled(Path::circle(a, 0.12), pal.leader));
            }
            out.items.push(text(&r, pl.at, pl.size, pl.rotation, color));
            if opts.values() {
                let fp = &board.footprints[fi];
                if !place_value(&mut out, fp, &pl, map, min_h, fonts, pal.value) {
                    out.values_omitted += 1;
                }
            }
        }
        out.labels.push(Label {
            reference: r,
            footprint: fi,
            placed: pl,
        });
    }
    out
}

/// Value text under an inside designator, if the body has room for it.
fn place_value(
    out: &mut Layout,
    fp: &Footprint,
    pl: &Placed,
    map: &ViewMap,
    min_h: f64,
    fonts: &Fonts,
    color: Color,
) -> bool {
    let Some(v) = fp.value.as_deref().filter(|v| !v.is_empty()) else {
        return true;
    };
    if pl.kind != Kind::Inside {
        return false;
    }
    let Some((bb, _)) = body(fp) else {
        return false;
    };
    let sb = map.board_to_sheet.apply_bbox(&bb);
    let h = min_h.max(0.7 * pl.size).min(pl.size);
    let w = fonts.width(v, h, false);
    if pl.rotation == 0.0 {
        let y = pl.rect.max.y + 0.6 * h;
        if w <= 0.9 * sb.width() && y + 0.6 * h <= sb.max.y {
            out.items
                .push(text(v, Point::new(pl.at.x, y), h, 0.0, color));
            return true;
        }
    } else {
        let x = pl.rect.max.x + 0.6 * h;
        if w <= 0.9 * sb.height() && x + 0.6 * h <= sb.max.x {
            out.items
                .push(text(v, Point::new(x, pl.at.y), h, 90.0, color));
            return true;
        }
    }
    false
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

    fn map(b: &Board, side: ViewSide, scale: f64) -> ViewMap {
        let v = View {
            id: Id("v".into()),
            kind: ViewKind::Assembly,
            side,
            scale,
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

    /// User board A: a 2x7 1.27 mm header next to another connector at
    /// 1.5:1 drew as a dark blob with its designator against the
    /// neighbour's. Its pads become one outline and its label goes
    /// outside, clear of the neighbour's body and label.
    #[test]
    fn dense_header_is_one_outline_with_the_label_outside() {
        let b = Board::load(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/dense-header.kicad_pcb"),
        )
        .unwrap();
        let j12 = b.footprint_by_reference("J12").unwrap();
        let j11 = b.footprint_by_reference("J11").unwrap();
        assert!((min_pad_clearance(j12).unwrap() - 0.27).abs() < 1e-9);
        assert!(dense_pads(j12, 1.5));
        assert!(!dense_pads(j12, 4.0));
        assert!(!dense_pads(j11, 1.5));
        let f = Fonts::load().unwrap();
        let o = AssemblyOptions::default();
        let count_strokes = |l: &Layout| {
            l.items
                .iter()
                .filter(|i| matches!(i, Item::Path(p) if p.stroke.as_ref().is_some_and(|s| (s.width - PAD_OUTLINE_WIDTH).abs() < 1e-12)))
                .count()
        };
        let m15 = map(&b, ViewSide::Front, 1.5);
        let l = render(&b, &m15, Side::Front, &o, &f);
        // One outline for J12's 14 pads plus J11's two.
        assert_eq!(count_strokes(&l), 3);
        let label = |r: &str| {
            l.labels
                .iter()
                .find(|x| x.reference == r)
                .unwrap()
                .placed
                .clone()
        };
        let (a, c) = (label("J12"), label("J11"));
        assert_ne!(a.kind, Kind::Inside);
        assert_ne!(a.kind, Kind::Unplaced);
        assert!(!kdraft_layout::grid::overlaps(&a.rect, &c.rect));
        let j11_body = m15.board_to_sheet.apply_bbox(&body(j11).unwrap().0);
        let j12_field = m15.board_to_sheet.apply_bbox(&pad_field(j12).unwrap());
        assert!(!kdraft_layout::grid::overlaps(&a.rect, &j11_body));
        assert!(!kdraft_layout::grid::overlaps(&a.rect, &j12_field));
        // At 4:1 the pads are drawn one by one again.
        let l4 = render(&b, &map(&b, ViewSide::Front, 4.0), Side::Front, &o, &f);
        assert_eq!(count_strokes(&l4), 16);
        // Look at it: KDRAFT_SAMPLES_DIR=dir cargo test -p kdraft-render dense_header
        if let Some(dir) = std::env::var_os("KDRAFT_SAMPLES_DIR") {
            {
                let (name, lay, mm) = ("dense-header-1.5", &l, &m15);
                let region = BBox {
                    min: Point::new(mm.frame.min.x - 8.0, mm.frame.min.y - 8.0),
                    max: Point::new(mm.frame.max.x + 8.0, mm.frame.max.y + 8.0),
                };
                let px = crate::raster::render(
                    &lay.items,
                    &crate::raster::Target {
                        region,
                        px_per_mm: 20.0,
                        background: Some(Color::WHITE),
                    },
                    &f,
                )
                .unwrap();
                std::fs::write(
                    std::path::Path::new(&dir).join(format!("{name}.png")),
                    px.encode_png().unwrap(),
                )
                .unwrap();
            }
        }
    }

    #[test]
    fn fixture_front_and_back() {
        let b = board();
        let f = Fonts::load().unwrap();
        let o = AssemblyOptions::default();
        let front = render(&b, &map(&b, ViewSide::Front, 5.0), Side::Front, &o, &f);
        let refs: Vec<&str> = front.labels.iter().map(|l| l.reference.as_str()).collect();
        assert!(refs.contains(&"R1") && refs.contains(&"U9"), "{refs:?}");
        // The mounting hole has no copper pads: no designator by default.
        assert!(!refs.contains(&"H1"), "{refs:?}");
        assert!(front.unplaced().is_empty());
        assert!(declutter::overlapping_pairs(
            &front
                .labels
                .iter()
                .map(|l| l.placed.clone())
                .collect::<Vec<_>>()
        )
        .is_empty());
        let back = render(&b, &map(&b, ViewSide::Back, 5.0), Side::Back, &o, &f);
        for t in crate::dl::texts(&back.items) {
            assert!(t.rotation == 0.0 || t.rotation == 90.0);
        }
        assert!(!back.labels.iter().any(|l| l.reference == "R1"));
        let all = AssemblyOptions {
            refs: Some(RefFilter::All),
            ..Default::default()
        };
        let front = render(&b, &map(&b, ViewSide::Front, 5.0), Side::Front, &all, &f);
        assert!(front.labels.iter().any(|l| l.reference == "H1"));
    }

    #[test]
    fn small_scale_moves_labels_outside() {
        let b = board();
        let f = Fonts::load().unwrap();
        let o = AssemblyOptions::default();
        let l = render(&b, &map(&b, ViewSide::Front, 1.0), Side::Front, &o, &f);
        assert!(l.stats.outside + l.stats.leader > 0, "{:?}", l.stats);
        for lab in &l.labels {
            assert!(lab.placed.size >= o.text_min() - 1e-9);
        }
    }
}
