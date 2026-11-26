//! Note lists (numbered, wrapped, variables resolved from the board and
//! the document context) and callouts (leader from an anchor to a balloon
//! with a note or BOM item number).

use kdraft_board::geom::{BBox, Point};
use kdraft_board::{Board, DrillShape};
use kdraft_doc::anchor::resolve;
use kdraft_doc::style::BalloonShape;
use kdraft_doc::{Callout, Document, Item as DocItem, NoteList};
use kdraft_sheet::{PageInfo, TextVars};

use crate::dl::{Color, HAlign, Item, Path, Stroke, TextRun, VAlign};
use crate::font::Fonts;
use crate::page::SheetContext;
use crate::table::fmt_num;
use crate::view::ViewMap;

/// Line spacing as a multiple of the text height (as in `bounds`).
pub const LINE_FACTOR: f64 = 1.8;

/// Callout line width relative to the document line width.
pub const CALLOUT_LINE: f64 = 0.7;

/// Board variables for notes: `BOARD_THICKNESS`, `LAYER_COUNT`,
/// `COPPER_FINISH`, `BOARD_WIDTH`, `BOARD_HEIGHT`, `MIN_DRILL`,
/// `HOLE_COUNT`.
pub fn board_vars(board: &Board) -> Vec<(&'static str, String)> {
    let thickness = board
        .stackup
        .as_ref()
        .map(|s| s.total_thickness())
        .filter(|t| *t > 0.0)
        .or(board.thickness);
    let finish = board
        .stackup
        .as_ref()
        .and_then(|s| s.copper_finish.clone())
        .filter(|f| !f.trim().is_empty() && !f.trim().eq_ignore_ascii_case("none"))
        .unwrap_or_else(|| "not specified".to_string());
    let holes = board.holes();
    let min_drill = holes
        .iter()
        .map(|h| match h.shape {
            DrillShape::Round { diameter } => diameter,
            DrillShape::Oval { width, height } => width.min(height),
        })
        .filter(|d| *d > 0.0)
        .fold(f64::INFINITY, f64::min);
    let bb = board.outline.bbox;
    let mut v = vec![
        (
            "BOARD_THICKNESS",
            thickness.map_or("not specified".into(), |t| fmt_num(t, 2, 3)),
        ),
        ("LAYER_COUNT", board.copper_layer_count().to_string()),
        ("COPPER_FINISH", finish),
        ("HOLE_COUNT", holes.len().to_string()),
        (
            "MIN_DRILL",
            if min_drill.is_finite() {
                fmt_num(min_drill, 2, 3)
            } else {
                "-".into()
            },
        ),
    ];
    if let Some(b) = bb {
        v.push(("BOARD_WIDTH", fmt_num(b.width(), 2, 2)));
        v.push(("BOARD_HEIGHT", fmt_num(b.height(), 2, 2)));
    }
    v
}

/// Variables for notes: board variables, title block and project
/// variables (through the sheet's expander).
pub fn note_vars(board: &Board, ctx: &SheetContext) -> TextVars {
    let mut v = TextVars::from_title_block(
        &board.title_block,
        &PageInfo {
            file_name: ctx.file_name.clone(),
            project_name: ctx.project_name.clone(),
            kicad_version: ctx.kicad_version.clone(),
            ..Default::default()
        },
    );
    v.set_project(&ctx.project_vars);
    for (k, val) in board_vars(board) {
        v.set(k, val);
    }
    v
}

/// `${NAME}` references left after expansion.
pub fn unknown_vars(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("${") {
        let after = &rest[i + 2..];
        match after.find('}') {
            Some(j) => {
                out.push(after[..j].to_string());
                rest = &after[j + 1..];
            }
            None => break,
        }
    }
    out
}

/// Line breaking for notes: newlines, then greedy word wrap to `width`
/// (mm) at cap height `h` (see [`kdraft_layout::text::break_lines`]).
pub fn wrap(text: &str, width: Option<f64>, h: f64, fonts: &Fonts) -> Vec<String> {
    kdraft_layout::text::break_lines(text, width, &|s| fonts.width(s, h, false))
}

fn text(s: &str, at: Point, size: f64, bold: bool, h: HAlign, v: VAlign) -> Item {
    Item::Text(TextRun {
        text: s.to_string(),
        at,
        size,
        h_align: h,
        v_align: v,
        rotation: 0.0,
        bold,
        italic: false,
        color: Color::BLACK,
        invisible: false,
    })
}

/// Lays a note list out: items, box and warnings (unknown variables).
pub fn render_notes(
    n: &NoteList,
    board: &Board,
    ctx: &SheetContext,
    th: f64,
    fonts: &Fonts,
) -> (Vec<Item>, BBox, Vec<String>) {
    let s = kdraft_doc::style::ItemStyle {
        text_height: Some(th),
        line_spacing: Some(LINE_FACTOR),
        ..Default::default()
    };
    render_notes_styled(n, board, ctx, &s, fonts)
}

/// [`render_notes`] with an effective notes style (text height and line
/// spacing; colours are applied by the caller).
pub fn render_notes_styled(
    n: &NoteList,
    board: &Board,
    ctx: &SheetContext,
    style: &kdraft_doc::style::ItemStyle,
    fonts: &Fonts,
) -> (Vec<Item>, BBox, Vec<String>) {
    let th = style.text_height.unwrap_or(2.5);
    let vars = note_vars(board, ctx);
    let mut items = Vec::new();
    let mut warnings = Vec::new();
    let line = th * style.line_spacing.unwrap_or(LINE_FACTOR);
    let mut y = n.at.y;
    let mut bbox = BBox::from_point(n.at);
    if let Some(t) = &n.title {
        items.push(text(t, n.at, th, true, HAlign::Left, VAlign::Top));
        bbox.include(Point::new(n.at.x + fonts.width(t, th, true), y + th));
        y += line;
    }
    let num_w = fonts.width(&format!("{}. ", n.notes.len().max(1)), th, false);
    for (k, note) in n.notes.iter().enumerate() {
        let resolved = vars.expand(&note.text);
        for u in unknown_vars(&resolved) {
            warnings.push(format!(
                "note {} ({}): unknown variable ${{{u}}}",
                k + 1,
                note.id
            ));
        }
        let body_w = n.width.map(|w| (w - num_w).max(10.0));
        items.push(text(
            &format!("{}.", k + 1),
            Point::new(n.at.x, y),
            th,
            false,
            HAlign::Left,
            VAlign::Top,
        ));
        for l in wrap(&resolved, body_w, th, fonts) {
            let x = n.at.x + num_w;
            bbox.include(Point::new(x + fonts.width(&l, th, false), y + th));
            items.push(text(
                &l,
                Point::new(x, y),
                th,
                false,
                HAlign::Left,
                VAlign::Top,
            ));
            y += line;
        }
    }
    (items, bbox, warnings)
}

/// The balloon text of a callout: its label, the number of its note
/// (`note:N1` or a bare note id), its BOM item (`bom:<group key>`,
/// numbers from `bom_item`) or the designator of its anchor (`refdes`).
pub fn callout_label(
    doc: &Document,
    c: &Callout,
    bom_item: &dyn Fn(&str) -> Option<usize>,
) -> Option<String> {
    let Some(t) = c.target.as_deref() else {
        return c.label.clone().filter(|l| !l.is_empty());
    };
    if t == kdraft_doc::REFDES_TARGET {
        return c.anchor_reference().map(str::to_string);
    }
    if let Some(key) = t.strip_prefix("bom:") {
        return bom_item(key).map(|n| n.to_string());
    }
    let id = t.strip_prefix("note:").unwrap_or(t);
    // Note ids are per list: the callout's own page first, then the rest.
    let own = doc
        .pages
        .iter()
        .position(|p| p.item(&c.id).is_some())
        .unwrap_or(0);
    let order = std::iter::once(own).chain((0..doc.pages.len()).filter(move |&i| i != own));
    order
        .filter_map(|i| doc.pages.get(i))
        .flat_map(|p| p.items.iter())
        .find_map(|it| match it {
            DocItem::NoteList(n) => n.number_of(id),
            _ => None,
        })
        .map(|n| n.to_string())
}

/// A balloon's outline around its centre: its shape and half extents
/// (they grow with the label so long labels fit).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Balloon {
    pub shape: BalloonShape,
    pub half_w: f64,
    pub half_h: f64,
}

impl Balloon {
    /// The balloon for `label` at cap height `th`.
    pub fn new(label: &str, th: f64, shape: BalloonShape, fonts: &Fonts) -> Balloon {
        let tw = fonts.width(label, th, false);
        let (half_w, half_h) = match shape {
            BalloonShape::Circle => {
                let r = (0.5 * tw + 0.45 * th).max(0.9 * th);
                (r, r)
            }
            BalloonShape::Square => {
                let h = 0.9 * th;
                ((0.5 * tw + 0.45 * th).max(h), h)
            }
            BalloonShape::Hexagon => {
                let h = 0.9 * th;
                // The pointed ends take half the height each.
                ((0.5 * tw + 0.35 * th + 0.5 * h).max(1.15 * h), h)
            }
            BalloonShape::None => (0.5 * tw + 0.2 * th, 0.6 * th),
        };
        Balloon {
            shape,
            half_w,
            half_h,
        }
    }

    /// Outline polygon (corners), `None` for circles and text only.
    fn corners(&self, c: Point) -> Option<Vec<Point>> {
        let (w, h) = (self.half_w, self.half_h);
        match self.shape {
            BalloonShape::Square | BalloonShape::None => Some(vec![
                Point::new(c.x - w, c.y - h),
                Point::new(c.x + w, c.y - h),
                Point::new(c.x + w, c.y + h),
                Point::new(c.x - w, c.y + h),
            ]),
            BalloonShape::Hexagon => {
                let k = 0.5 * h;
                Some(vec![
                    Point::new(c.x - w, c.y),
                    Point::new(c.x - w + k, c.y - h),
                    Point::new(c.x + w - k, c.y - h),
                    Point::new(c.x + w, c.y),
                    Point::new(c.x + w - k, c.y + h),
                    Point::new(c.x - w + k, c.y + h),
                ])
            }
            BalloonShape::Circle => None,
        }
    }

    /// The drawn outline at centre `c` (nothing for text only).
    pub fn path(&self, c: Point) -> Option<Path> {
        match self.shape {
            BalloonShape::Circle => Some(Path::circle(c, self.half_w)),
            BalloonShape::None => None,
            _ => self.corners(c).map(|p| Path::polyline(&p, true)),
        }
    }

    pub fn bbox(&self, c: Point) -> BBox {
        BBox {
            min: Point::new(c.x - self.half_w, c.y - self.half_h),
            max: Point::new(c.x + self.half_w, c.y + self.half_h),
        }
    }

    /// Where a leader from `from` meets the outline (centre `c`); `None`
    /// when `from` is inside.
    pub fn edge(&self, c: Point, from: Point) -> Option<Point> {
        let d = Point::new(from.x - c.x, from.y - c.y);
        let len = d.x.hypot(d.y);
        if len < 1e-9 {
            return None;
        }
        let u = Point::new(d.x / len, d.y / len);
        let t = match self.corners(c) {
            None => self.half_w,
            Some(poly) => {
                // Nearest crossing of the ray c + t*u with the polygon.
                let mut best = f64::INFINITY;
                for i in 0..poly.len() {
                    let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                    let e = Point::new(b.x - a.x, b.y - a.y);
                    let den = u.x * e.y - u.y * e.x;
                    if den.abs() < 1e-12 {
                        continue;
                    }
                    let w = Point::new(a.x - c.x, a.y - c.y);
                    let t = (w.x * e.y - w.y * e.x) / den;
                    let s = (w.x * u.y - w.y * u.x) / den;
                    if t > 0.0 && (-1e-9..=1.0 + 1e-9).contains(&s) {
                        best = best.min(t);
                    }
                }
                best
            }
        };
        (t.is_finite() && len > t).then(|| Point::new(c.x + u.x * t, c.y + u.y * t))
    }
}

/// Draws a callout: leader from the anchor (through its view) via the
/// intermediate points to the balloon at `c.at`.
pub fn render_callout(
    doc: &Document,
    c: &Callout,
    board: &Board,
    board_box: BBox,
    bom_item: &dyn Fn(&str) -> Option<usize>,
    fonts: &Fonts,
) -> (Vec<Item>, Vec<String>) {
    let eff = DocItem::Callout(c.clone()).effective_style(&doc.styles);
    let th = eff.text_height.unwrap_or(doc.styles.text_height);
    let mut warnings = Vec::new();
    if let (Some(kdraft_doc::REFDES_TARGET), Some(r)) = (c.target.as_deref(), c.anchor_reference())
    {
        if !board
            .footprints
            .iter()
            .any(|f| f.reference.as_deref() == Some(r))
        {
            warnings.push(format!("callout {}: the board has no part {r}", c.id));
        }
    }
    let label = callout_label(doc, c, bom_item).unwrap_or_else(|| {
        warnings.push(format!(
            "callout {}: target {:?} not found",
            c.id,
            c.target.as_deref().unwrap_or("")
        ));
        "?".to_string()
    });
    let balloon = Balloon::new(&label, th, eff.shape.unwrap_or(BalloonShape::Circle), fonts);
    let st = Stroke::solid(
        eff.width.unwrap_or(doc.styles.line_width * CALLOUT_LINE),
        Color::BLACK,
    );
    let mut items = Vec::new();
    if let Some(p) = balloon.path(c.at) {
        items.push(Item::stroked(p, st.clone()));
    }
    items.push(text(
        &label,
        c.at,
        th,
        false,
        HAlign::Center,
        VAlign::Center,
    ));
    let start = match (&c.anchor, &c.view) {
        (Some(a), Some(vid)) => {
            let view = doc
                .pages
                .iter()
                .flat_map(|p| p.views())
                .find(|v| &v.id == vid);
            let holes = board.holes();
            match (view, resolve(a, board, &holes)) {
                (Some(v), Ok(p)) => Some(ViewMap::new(v, board_box).point(p)),
                (_, Err(e)) => {
                    warnings.push(format!("callout {}: broken anchor ({e})", c.id));
                    None
                }
                (None, _) => None,
            }
        }
        _ => None,
    };
    if let Some(s) = start {
        let mut pts = vec![s];
        pts.extend(c.leader.iter().copied());
        // End on the balloon's edge.
        let last = *pts.last().unwrap_or(&s);
        if let Some(e) = balloon.edge(c.at, last) {
            pts.push(e);
        }
        items.push(Item::stroked(Path::polyline(&pts, false), st));
        items.push(Item::filled(Path::circle(s, 0.25), Color::BLACK));
    }
    (items, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kdraft_doc::{Id, Note};

    fn board() -> Board {
        Board::load(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/kicad9-minimal.kicad_pcb"),
        )
        .unwrap()
    }

    #[test]
    fn variables_wrapping_and_unknowns() {
        let b = board();
        let f = Fonts::load().unwrap();
        let n = NoteList {
            id: Id("n".into()),
            at: Point::new(20.0, 20.0),
            title: Some("NOTES".into()),
            width: Some(60.0),
            notes: kdraft_doc::DEFAULT_FAB_NOTES
                .iter()
                .enumerate()
                .map(|(i, t)| Note {
                    id: format!("N{}", i + 1),
                    text: t.to_string(),
                })
                .chain(std::iter::once(Note {
                    id: "N9".into(),
                    text: "Mystery ${NOPE} and ${TITLE}.".into(),
                }))
                .collect(),
            extra: vec![],
        };
        let (items, bbox, warnings) = render_notes(&n, &b, &SheetContext::default(), 2.5, &f);
        let texts: Vec<String> = crate::dl::texts(&items)
            .iter()
            .map(|t| t.text.clone())
            .collect();
        let all = texts.join(" ");
        assert!(all.contains("50.00 x 30.00 mm"), "{all}");
        assert!(all.contains("1.6"), "{all}");
        assert!(all.contains("ENIG"), "{all}");
        assert!(all.contains("kdraft test board"), "{all}");
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("${NOPE}"));
        assert!(bbox.width() <= 60.0 + 1e-6, "{bbox:?}");
        assert_eq!(n.number_of("N3"), Some(3));
        assert_eq!(n.next_id(), "N6");
    }
}
