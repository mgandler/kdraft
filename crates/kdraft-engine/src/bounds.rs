//! Sheet bounding boxes of document items, used by align, distribute,
//! snapping and hit testing.

use kdraft_board::geom::{BBox, Point};
use kdraft_board::Board;
use kdraft_doc::{Dimension, DimensionKind, Document, GraphicShape, Item, Styles, TableKind};
use kdraft_render::table::{self, TableStyle};
use kdraft_render::view::ViewMap;
use kdraft_render::Fonts;

/// What a view's box covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoundsMode {
    /// The board outline box (or crop) through the view transform.
    #[default]
    Content,
    /// The view frame: content plus the frame margin.
    Frame,
}

pub struct BoundsCtx<'a> {
    /// The schematic BOM if known (else BOM tables use the PCB fallback).
    pub bom: Option<&'a kdraft_board::bom::Bom>,
    /// The document (dimensions find their view in it).
    pub doc: Option<&'a Document>,
    pub board: Option<&'a Board>,
    pub fonts: &'a Fonts,
    pub styles: &'a Styles,
    pub mode: BoundsMode,
    /// 3D view pictures (their trimmed frames are the views' bounds).
    pub renders: Option<&'a kdraft_render::render3d::Renders>,
}

/// The board box views are placed by: the outline box, else the box of the
/// footprint origins, else 10 x 10 mm at the origin (as the renderer does).
pub fn board_box(board: Option<&Board>) -> BBox {
    board
        .and_then(|b| {
            b.outline
                .bbox
                .or_else(|| BBox::from_points(b.footprints.iter().map(|f| f.position)))
        })
        .unwrap_or(BBox {
            min: Point::ORIGIN,
            max: Point::new(10.0, 10.0),
        })
}

/// Line spacing of note lists, as a multiple of the text height (the
/// renderer uses the same).
pub const NOTE_LINE_FACTOR: f64 = 1.8;

/// The sheet box of a 3D view: its trimmed picture once rendered, else the
/// view frame.
pub fn render3d_bounds(
    v: &kdraft_doc::View,
    bb: BBox,
    renders: Option<&kdraft_render::render3d::Renders>,
) -> BBox {
    let frame = kdraft_doc::ops::view_bounds(v, bb);
    let st = renders.and_then(|r| r.get(&crate::pipeline::render_key(v, bb)));
    kdraft_render::render3d::sheet_rect(st, &frame)
}

/// Layout of a text graphic as drawn (variables expanded with the board's
/// values when a board is loaded).
pub fn text_layout(
    g: &kdraft_doc::Graphic,
    ctx: &BoundsCtx<'_>,
) -> Option<kdraft_render::graphic::TextLayout> {
    let GraphicShape::Text {
        at,
        text,
        height,
        rotation,
        wrap,
    } = &g.shape
    else {
        return None;
    };
    let mut eff = Item::Graphic(g.clone()).effective_style(ctx.styles);
    if let Some(h) = height {
        eff.text_height = Some(*h);
    }
    let vars = ctx
        .board
        .map(|b| kdraft_render::notes::note_vars(b, &kdraft_render::SheetContext::default()));
    Some(kdraft_render::graphic::layout_text(
        text,
        *at,
        *rotation,
        *wrap,
        &eff,
        vars.as_ref(),
        ctx.fonts,
    ))
}

/// Sheet box of an item; `None` for dimensions and unknown items.
pub fn item_bounds(item: &Item, ctx: &BoundsCtx<'_>) -> Option<BBox> {
    let th = ctx.styles.text_height;
    match item {
        Item::View(v) => {
            let bb = board_box(ctx.board);
            Some(match ctx.mode {
                BoundsMode::Content if v.kind == kdraft_doc::ViewKind::Render3d => {
                    render3d_bounds(v, bb, ctx.renders)
                }
                BoundsMode::Content => kdraft_doc::ops::view_bounds(v, bb),
                BoundsMode::Frame => ViewMap::new(v, bb).frame,
            })
        }
        Item::Table(t) => {
            let data = match (ctx.board, &t.kind) {
                (Some(b), TableKind::Drill) => table::drill_table(b),
                (Some(b), TableKind::Stackup) => table::stackup_table(b),
                // The PCB-derived BOM (the schematic one is only known at
                // export time).
                (Some(b), TableKind::Bom) => {
                    let bom = match ctx.bom {
                        Some(x) => x.clone(),
                        None => kdraft_board::bom::from_board(b),
                    };
                    let nums = kdraft_board::bom::number_lines(&bom.lines, &Default::default());
                    let keys: Vec<String> = if t.columns.is_empty() {
                        table::bom_default_columns(&bom)
                    } else {
                        t.columns.clone()
                    };
                    let style = TableStyle::from_style(&item.effective_style(ctx.styles));
                    let (_, bx) = table::layout(
                        &table::bom_table(&bom, &nums, t.rows).select(&keys),
                        t.at,
                        style,
                        ctx.fonts,
                    );
                    return Some(bx);
                }
                // Not laid out (no board, or BOM/custom): the anchor only.
                _ => return Some(BBox::from_point(t.at)),
            };
            let style = TableStyle::from_style(&item.effective_style(ctx.styles));
            let (_, b) = table::layout(&data.select(&t.columns), t.at, style, ctx.fonts);
            Some(b)
        }
        Item::Graphic(g) => match &g.shape {
            GraphicShape::Text { .. } => {
                let l = text_layout(g, ctx)?;
                BBox::from_points(l.corners)
            }
            // Geometry (what the position and size fields show), without
            // the stroke width.
            _ => g.shape.bbox(),
        },
        Item::NoteList(n) => match ctx.board {
            Some(b) => Some(
                kdraft_render::notes::render_notes_styled(
                    n,
                    b,
                    &kdraft_render::SheetContext::default(),
                    &item.effective_style(ctx.styles),
                    ctx.fonts,
                )
                .1,
            ),
            None => Some(BBox::from_point(n.at)),
        },
        Item::Callout(c) => {
            // The balloon; derived numbers estimated with two digits.
            let eff = item.effective_style(ctx.styles);
            let th = eff.text_height.unwrap_or(th);
            let label = match (c.kind(), &c.label) {
                (kdraft_doc::CalloutKind::Label, Some(l)) => l.clone(),
                (kdraft_doc::CalloutKind::Refdes, _) => {
                    c.anchor_reference().unwrap_or("00").to_string()
                }
                _ => "00".to_string(),
            };
            let b = kdraft_render::notes::Balloon::new(
                &label,
                th,
                eff.shape.unwrap_or(kdraft_doc::style::BalloonShape::Circle),
                ctx.fonts,
            );
            Some(b.bbox(c.at))
        }
        Item::Dimension(d) => dimension_geometry(d, ctx).map(|g| g.bbox),
        Item::Unknown(_) => None,
    }
}

/// Sheet geometry of a dimension as drawn: extension and dimension lines,
/// leaders, arrowheads and the text box.
#[derive(Debug, Clone, PartialEq)]
pub struct DimGeom {
    pub lines: Vec<(Point, Point)>,
    /// Arrowhead triangles.
    pub arrows: Vec<[Point; 3]>,
    pub text_box: BBox,
    pub text_at: Point,
    pub text: String,
    pub dim_line: (Point, Point),
    /// The measured anchor points on the sheet (from, to; centre and a
    /// circle point for radius/diameter). Empty when nothing resolves.
    pub anchors: Vec<Point>,
    /// Anchors do not resolve (drawn at the last known place, or nowhere).
    pub broken: bool,
    /// Union of all of the above.
    pub bbox: BBox,
}

/// Size of the placeholder box of a broken dimension without any stored
/// position, sheet mm.
const PLACEHOLDER: f64 = 4.0;

/// Geometry of a dimension. A dimension without a view or a board, or
/// broken without stored positions, gets a small placeholder at its view
/// (so it can still be selected and deleted).
pub fn dimension_geometry(d: &Dimension, ctx: &BoundsCtx<'_>) -> Option<DimGeom> {
    let doc = ctx.doc?;
    let bb = board_box(ctx.board);
    let laid = ctx.board.and_then(|b| {
        let holes = b.holes();
        kdraft_render::dims::layout(doc, d, b, bb, &holes, ctx.fonts)
    });
    let view = kdraft_render::dims::view_of(doc, d);
    let Some((l, res)) = laid else {
        let c = view.map(|v| v.at)?;
        let h = PLACEHOLDER / 2.0;
        let b = BBox {
            min: Point::new(c.x - h, c.y - h),
            max: Point::new(c.x + h, c.y + h),
        };
        return Some(DimGeom {
            lines: Vec::new(),
            arrows: Vec::new(),
            text_box: b,
            text_at: c,
            text: "?".into(),
            dim_line: (c, c),
            anchors: Vec::new(),
            broken: true,
            bbox: b,
        });
    };
    let st = kdraft_doc::style::dim_style_for(&doc.styles, d);
    let arrows: Vec<[Point; 3]> = l
        .arrows
        .iter()
        .map(|(tip, dir)| {
            let base = Point::new(
                tip.x - dir.x * st.arrow_length,
                tip.y - dir.y * st.arrow_length,
            );
            let n = Point::new(-dir.y * st.arrow_width / 2.0, dir.x * st.arrow_width / 2.0);
            [
                *tip,
                Point::new(base.x + n.x, base.y + n.y),
                Point::new(base.x - n.x, base.y - n.y),
            ]
        })
        .collect();
    let anchors: Vec<Point> = match view {
        Some(v) => {
            let map = ViewMap::new(v, bb);
            res.points.iter().map(|p| map.point(*p)).collect()
        }
        None => Vec::new(),
    };
    let pts = l
        .lines
        .iter()
        .flat_map(|(a, b)| [*a, *b])
        .chain(arrows.iter().flatten().copied())
        .chain([l.text_box.min, l.text_box.max]);
    let bbox = BBox::from_points(pts).unwrap_or(l.text_box);
    Some(DimGeom {
        lines: l.lines.clone(),
        arrows,
        text_box: l.text_box,
        text_at: l.text_at,
        text: l.text.clone(),
        dim_line: l.dim_line,
        anchors,
        broken: res.broken.is_some(),
        bbox,
    })
}

/// Distance from `p` to the segment `a`-`b`.
pub fn segment_dist(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let l2 = dx * dx + dy * dy;
    let t = if l2 < 1e-18 {
        0.0
    } else {
        (((p.x - a.x) * dx + (p.y - a.y) * dy) / l2).clamp(0.0, 1.0)
    };
    p.dist(Point::new(a.x + t * dx, a.y + t * dy))
}

fn in_box(b: &BBox, p: Point, tol: f64) -> bool {
    p.x >= b.min.x - tol && p.x <= b.max.x + tol && p.y >= b.min.y - tol && p.y <= b.max.y + tol
}

impl DimGeom {
    /// Whether `p` is within `tol` of a line or arrowhead, or inside the
    /// text box (grown by `tol`). The empty space inside the bounding box
    /// does not count.
    pub fn hit(&self, p: Point, tol: f64) -> bool {
        in_box(&self.text_box, p, tol)
            || self
                .lines
                .iter()
                .any(|(a, b)| segment_dist(p, *a, *b) <= tol)
            || self
                .arrows
                .iter()
                .any(|t| (0..3).any(|k| segment_dist(p, t[k], t[(k + 1) % 3]) <= tol))
    }
}

/// Precise hit test of an item: dimensions by their lines and text, free
/// lines by distance to the line, everything else by its box.
pub fn item_hit(item: &Item, ctx: &BoundsCtx<'_>, p: Point, tol: f64) -> bool {
    match item {
        Item::Dimension(d) => dimension_geometry(d, ctx).is_some_and(|g| g.hit(p, tol)),
        Item::Graphic(g) => match &g.shape {
            GraphicShape::Line { start, end } => segment_dist(p, *start, *end) <= tol,
            _ => item_bounds(item, ctx).is_some_and(|b| in_box(&b, p, tol)),
        },
        _ => item_bounds(item, ctx).is_some_and(|b| in_box(&b, p, tol)),
    }
}

/// The offset a linear or ordinate dimension gets when its dimension line
/// is dragged by `delta` (sheet mm) from where it was with `offset`.
/// `None` for kinds whose offset is not a perpendicular distance
/// (radius, diameter) or without resolved anchors.
pub fn dragged_offset(d: &Dimension, g: &DimGeom, offset: f64, delta: Point) -> Option<f64> {
    let (s1, s2) = match g.anchors.as_slice() {
        [a, b, ..] => (*a, *b),
        _ => return None,
    };
    let dot = |a: Point, b: Point| a.x * b.x + a.y * b.y;
    let n0 = match d.kind {
        DimensionKind::Radius | DimensionKind::Radial | DimensionKind::Diameter => return None,
        DimensionKind::Ordinate => {
            let x_axis = d.axis.as_deref() != Some("y");
            return Some(if x_axis {
                offset + delta.y
            } else {
                offset - delta.x
            });
        }
        DimensionKind::Horizontal => Point::new(0.0, -1.0),
        DimensionKind::Vertical => Point::new(1.0, 0.0),
        DimensionKind::Aligned => {
            let v = Point::new(s2.x - s1.x, s2.y - s1.y);
            let l = (v.x * v.x + v.y * v.y).sqrt();
            if l < 1e-9 {
                return None;
            }
            Point::new(v.y / l, -v.x / l)
        }
        _ => {
            if (s2.x - s1.x).abs() >= (s2.y - s1.y).abs() {
                Point::new(0.0, -1.0)
            } else {
                Point::new(1.0, 0.0)
            }
        }
    };
    let hi = dot(s1, n0).max(dot(s2, n0));
    let lo = dot(s1, n0).min(dot(s2, n0));
    // Signed position of the dimension line along n0.
    let level = if offset >= 0.0 {
        hi + offset
    } else {
        lo + offset
    };
    let new = level + dot(delta, n0);
    Some(if new >= hi {
        new - hi
    } else if new <= lo {
        new - lo
    } else if new - lo < hi - new {
        // Between the anchors: onto the nearer side, flush.
        -0.01
    } else {
        0.0
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kdraft_doc::{Graphic, Id, Table};

    #[test]
    fn text_and_table_bounds() {
        let fonts = Fonts::load().unwrap();
        let styles = Styles::default();
        let ctx = BoundsCtx {
            bom: None,
            doc: None,
            board: None,
            fonts: &fonts,
            styles: &styles,
            mode: BoundsMode::Content,
            renders: None,
        };
        let t = Item::Graphic(Graphic {
            id: Id("t".into()),
            shape: GraphicShape::Text {
                at: Point::new(10.0, 20.0),
                text: "ABC".into(),
                height: Some(2.0),
                rotation: 0.0,
                wrap: None,
            },
            style: None,
            extra: vec![],
        });
        let b = item_bounds(&t, &ctx).unwrap();
        assert_eq!(b.min, Point::new(10.0, 18.0));
        assert!(b.width() > 2.0 && b.width() < 6.0);
        let tab = Item::Table(Table {
            id: Id("x".into()),
            kind: TableKind::Drill,
            at: Point::new(5.0, 5.0),
            columns: vec![],
            style: None,
            bom_items: Vec::new(),
            bom_source: None,
            rows: None,
            extra: vec![],
        });
        // No board: anchor only.
        assert_eq!(item_bounds(&tab, &ctx).unwrap().width(), 0.0);
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/kicad9-minimal.kicad_pcb");
        let board = Board::load(path).unwrap();
        let ctx = BoundsCtx {
            board: Some(&board),
            ..ctx
        };
        let b = item_bounds(&tab, &ctx).unwrap();
        assert_eq!(b.min, Point::new(5.0, 5.0));
        assert!(b.width() > 20.0 && b.height() > 10.0, "{b:?}");
    }
}
