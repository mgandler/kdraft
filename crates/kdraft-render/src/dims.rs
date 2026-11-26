//! Dimensions: anchors resolved against the board through their view's
//! transform, laid out by [`kdraft_layout::dimension`], drawn as lines,
//! filled arrowheads and upright text. A dimension whose anchors do not
//! resolve is drawn at its last resolved position in a broken style (red,
//! dashed, `?` after the value).

use kdraft_board::geom::{BBox, Point};
use kdraft_board::{Board, Hole};
use kdraft_doc::{Dimension, DimensionKind, Document, View};
use kdraft_layout::dimension::{self as geom, Direction, Layout};

use crate::dl::{Color, Fill, HAlign, Item, Path, PathItem, Stroke, TextRun, VAlign};
use crate::font::Fonts;
use crate::view::ViewMap;

pub const BROKEN: Color = Color::rgb(200, 30, 30);

/// Anchors resolved in board coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    /// from, to (linear, ordinate); centre and a circle point (radial).
    pub points: Vec<Point>,
    /// Why the anchors did not resolve (then `points` are the stored last
    /// positions, possibly empty).
    pub broken: Option<String>,
}

/// Resolves a dimension's anchors.
pub fn resolve_dimension(d: &Dimension, board: &Board, holes: &[Hole]) -> Resolved {
    match kdraft_doc::anchor::dimension_positions(d, board, holes) {
        Ok(points) => Resolved {
            points,
            broken: None,
        },
        Err(e) => Resolved {
            points: d.last.clone(),
            broken: Some(e.to_string()),
        },
    }
}

/// The view a dimension belongs to.
pub fn view_of<'a>(doc: &'a Document, d: &Dimension) -> Option<&'a View> {
    let id = d.view.as_ref()?;
    doc.pages
        .iter()
        .flat_map(|p| p.views())
        .find(|v| &v.id == id)
}

/// Lays a dimension out on the sheet. `None` if it has no view or no
/// positions at all.
pub fn layout(
    doc: &Document,
    d: &Dimension,
    board: &Board,
    board_box: BBox,
    holes: &[Hole],
    fonts: &Fonts,
) -> Option<(Layout, Resolved)> {
    let view = view_of(doc, d)?;
    let map = ViewMap::new(view, board_box);
    let res = resolve_dimension(d, board, holes);
    if res.points.len() < 2 {
        return None;
    }
    let st = kdraft_doc::style::dim_style_for(&doc.styles, d);
    let tw = |s: &str, h: f64| fonts.width(s, h, false);
    let fmt = |v: f64| {
        let mut s = geom::format_value(v, st.decimals, st.trim_zeros);
        if let Some(t) = &d.tolerance {
            s.push(' ');
            s.push_str(t);
        }
        if res.broken.is_some() {
            s.push_str(" ?");
        }
        s
    };
    let (b1, b2) = (res.points[0], res.points[1]);
    let (s1, s2) = (map.point(b1), map.point(b2));
    let l = match d.kind {
        DimensionKind::Radius | DimensionKind::Radial | DimensionKind::Diameter => {
            let r_board = b1.dist(b2);
            let dia = d.kind == DimensionKind::Diameter;
            let text = if dia {
                format!("\u{d8}{}", fmt(2.0 * r_board))
            } else {
                format!("R{}", fmt(r_board))
            };
            geom::radial(
                s1,
                r_board * map.scale,
                d.angle.unwrap_or(45.0),
                dia,
                &text,
                &st,
                &tw,
            )
        }
        DimensionKind::Ordinate => {
            let x_axis = d.axis.as_deref() != Some("y");
            // Board values relative to the datum, Y up.
            let v = if x_axis { b2.x - b1.x } else { b1.y - b2.y };
            let level = if x_axis {
                s1.y + d.offset
            } else {
                s1.x - d.offset
            };
            geom::ordinate(s2, x_axis, level, &fmt(v), &st, &tw)
        }
        _ => {
            let dir = match d.kind {
                DimensionKind::Horizontal => Direction::Horizontal,
                DimensionKind::Vertical => Direction::Vertical,
                DimensionKind::Aligned => Direction::Aligned,
                _ => {
                    if (s2.x - s1.x).abs() >= (s2.y - s1.y).abs() {
                        Direction::Horizontal
                    } else {
                        Direction::Vertical
                    }
                }
            };
            let v = geom::measured(s1, s2, dir, map.scale);
            let mut l = geom::linear(s1, s2, dir, d.offset, &fmt(v), &st, &tw);
            let w = tw(&l.text, st.text_height);
            shift_text(&mut l, d.text_shift(), w);
            l
        }
    };
    Some((l, res))
}

/// Moves a linear dimension's text by `shift` along its line, extending
/// the line under text moved past an end.
fn shift_text(l: &mut geom::Layout, shift: f64, text_w: f64) {
    if shift == 0.0 {
        return;
    }
    let (a, b) = l.dim_line;
    let len = a.dist(b);
    if len < 1e-9 {
        return;
    }
    let u = Point::new((b.x - a.x) / len, (b.y - a.y) / len);
    let d = Point::new(u.x * shift, u.y * shift);
    l.text_at = Point::new(l.text_at.x + d.x, l.text_at.y + d.y);
    l.text_box = BBox {
        min: Point::new(l.text_box.min.x + d.x, l.text_box.min.y + d.y),
        max: Point::new(l.text_box.max.x + d.x, l.text_box.max.y + d.y),
    };
    // Where the text's far edge projects onto the line.
    let mid = Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
    let t = (l.text_at.x - mid.x) * u.x + (l.text_at.y - mid.y) * u.y;
    let half = len / 2.0;
    let reach = t.abs() + text_w / 2.0 + 0.5;
    if reach > half {
        let (end, sign) = if t > 0.0 { (b, 1.0) } else { (a, -1.0) };
        let far = Point::new(mid.x + u.x * sign * reach, mid.y + u.y * sign * reach);
        l.lines.push((end, far));
    }
}

fn arrow(tip: Point, dir: Point, len: f64, w: f64) -> Path {
    let base = Point::new(tip.x - dir.x * len, tip.y - dir.y * len);
    let n = Point::new(-dir.y * w / 2.0, dir.x * w / 2.0);
    Path::polyline(
        &[
            tip,
            Point::new(base.x + n.x, base.y + n.y),
            Point::new(base.x - n.x, base.y - n.y),
        ],
        true,
    )
}

/// Display items of a laid-out dimension.
pub fn draw(l: &Layout, broken: bool, st: &geom::DimStyle) -> Vec<Item> {
    let color = if broken { BROKEN } else { Color::BLACK };
    let mut stroke = Stroke::solid(st.line_width, color);
    if broken {
        stroke.dash = Some(vec![1.0, 0.6]);
    }
    let mut out: Vec<Item> = l
        .lines
        .iter()
        .map(|(a, b)| Item::stroked(Path::line(*a, *b), stroke.clone()))
        .collect();
    for (tip, dir) in &l.arrows {
        out.push(Item::Path(PathItem {
            path: arrow(*tip, *dir, st.arrow_length, st.arrow_width),
            stroke: None,
            fill: Some(Fill {
                color,
                even_odd: false,
            }),
        }));
    }
    out.push(Item::Text(TextRun {
        text: l.text.clone(),
        at: l.text_at,
        size: st.text_height,
        h_align: HAlign::Center,
        v_align: VAlign::Center,
        rotation: l.text_rotation,
        bold: false,
        italic: false,
        color,
        invisible: false,
    }));
    out
}

/// Renders one dimension: items and warnings.
pub fn render(
    doc: &Document,
    d: &Dimension,
    board: &Board,
    board_box: BBox,
    fonts: &Fonts,
) -> (Vec<Item>, Vec<String>) {
    let holes = board.holes();
    match layout(doc, d, board, board_box, &holes, fonts) {
        Some((l, res)) => {
            let mut w = Vec::new();
            if let Some(e) = &res.broken {
                w.push(format!("dimension {}: broken anchor ({e})", d.id));
            }
            (
                draw(
                    &l,
                    res.broken.is_some(),
                    &kdraft_doc::style::dim_style_for(&doc.styles, d),
                ),
                w,
            )
        }
        None => (
            Vec::new(),
            vec![format!(
                "dimension {}: not drawn (no view, or anchors unresolved without a last position)",
                d.id
            )],
        ),
    }
}

/// The board positions to store as a dimension's `last` (None when the
/// anchors do not resolve now).
pub fn current_positions(d: &Dimension, board: &Board, holes: &[Hole]) -> Option<Vec<Point>> {
    let r = resolve_dimension(d, board, holes);
    r.broken.is_none().then_some(r.points)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kdraft_doc::anchor::resolve;
    use kdraft_doc::{Anchor, Id, Item as DocItem};

    fn setup() -> (Board, Document) {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/kicad9-minimal.kicad_pcb");
        let b = Board::load(&p).unwrap();
        let sheet = kdraft_sheet::default_sheet().unwrap();
        let d = Document::new_for_board(&b, "b.kicad_pcb", &sheet);
        (b, d)
    }

    #[test]
    fn linear_radius_ordinate_and_broken() {
        let (b, mut doc) = setup();
        let f = Fonts::load().unwrap();
        let bb = b.outline.bbox.unwrap();
        let vid = doc.pages[0].views().next().unwrap().id.clone();
        let mut dim = Dimension::new(
            DimensionKind::Horizontal,
            vid.clone(),
            Anchor::OutlineVertex(0),
            Some(Anchor::OutlineVertex(1)),
            8.0,
        );
        let holes = b.holes();
        let (l, r) = layout(&doc, &dim, &b, bb, &holes, &f).unwrap();
        assert!(r.broken.is_none());
        let p0 = resolve(&Anchor::OutlineVertex(0), &b, &holes).unwrap();
        let p1 = resolve(&Anchor::OutlineVertex(1), &b, &holes).unwrap();
        let want = geom::format_value((p1.x - p0.x).abs(), 2, false);
        assert!(l.text.starts_with(&want), "{} vs {want}", l.text);
        // Independent of the view scale.
        if let DocItem::View(v) = &mut doc.pages[0].items[0] {
            v.scale = 2.0;
        }
        let (l2, _) = layout(&doc, &dim, &b, bb, &holes, &f).unwrap();
        assert_eq!(l2.text, l.text);
        // Broken anchor: drawn at the last position, flagged.
        dim.last = current_positions(&dim, &b, &holes).unwrap();
        dim.to = Some(Anchor::OutlineVertex(999));
        let (lb, rb) = layout(&doc, &dim, &b, bb, &holes, &f).unwrap();
        assert!(rb.broken.is_some());
        assert!(lb.text.ends_with('?'));
        let items = draw(&lb, true, &doc.styles.dimension);
        assert!(crate::dl::texts(&items).iter().all(|t| t.color == BROKEN));
        // Radius of the NPTH (hole 2 in the fixture is 3.2 mm).
        let rad = Dimension::new(
            DimensionKind::Diameter,
            vid.clone(),
            Anchor::Hole(2),
            None,
            0.0,
        );
        let (lr, _) = layout(&doc, &rad, &b, bb, &holes, &f).unwrap();
        assert!(lr.text.contains("3.20"), "{}", lr.text);
        let mut ord = Dimension::new(
            DimensionKind::Ordinate,
            vid,
            Anchor::OutlineVertex(0),
            Some(Anchor::Hole(2)),
            -5.0,
        );
        ord.axis = Some("y".into());
        assert!(layout(&doc, &ord, &b, bb, &holes, &f).is_some());
        let _ = Id::new_v4();
    }
}
