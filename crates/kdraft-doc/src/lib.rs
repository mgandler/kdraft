//! The `.kdraft` drawing document: model, reader, canonical writer, and
//! pure editing operations.
#![forbid(unsafe_code)]

pub mod anchor;
pub mod callout;
mod error;
mod io;
pub mod layers;
pub mod link;
mod model;
pub mod ops;
pub mod palette;
pub mod render3d;
pub mod style;

use std::collections::HashSet;
use std::fmt;

use kdraft_board::geom::{BBox, Point};
use kdraft_board::Board;
use kdraft_sheet::{PageSpec, TextVars, WksFile};

pub use error::{DocError, OpError};
pub use io::{anchor_text, parse_anchor};
pub use model::*;

/// Standard view scales offered for new views (sheet mm per board mm).
pub const STANDARD_SCALES: [f64; 6] = [0.5, 1.0, 2.0, 4.0, 5.0, 10.0];

/// Scales `doc new` may also use for assembly views of dense boards
/// (between the standard ones), sheet mm per board mm.
pub const DENSE_SCALES: [f64; 9] = [0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 10.0];

/// Below this fraction of designators fitting inside their part at the
/// minimum text height, an assembly view counts as dense.
pub const DENSE_INSIDE_FRACTION: f64 = 0.5;

/// Estimated designator width at 1 mm cap height per character (IBM Plex
/// Sans digits and capitals are about 0.75 cap heights wide).
pub const DESIGNATOR_CHAR_WIDTH: f64 = 0.75;

/// Fraction of designators on `side` that fit inside their part at the
/// default minimum height when drawn at `scale`.
pub fn designator_inside_fraction(board: &Board, side: kdraft_board::Side, scale: f64) -> f64 {
    let bodies: Vec<(BBox, f64)> = board
        .footprints
        .iter()
        .filter(|f| f.side == side && !f.attributes.board_only && f.has_copper_pads())
        .filter_map(|f| {
            let r = f.reference.as_deref()?;
            let (b, _) = f.body()?;
            Some((b, DESIGNATOR_CHAR_WIDTH * r.chars().count() as f64))
        })
        .collect();
    kdraft_layout::declutter::inside_fraction(&bodies, scale, DESIGNATOR_MIN_H)
}

/// Scale for an assembly view: the standard scale that fits, or, when
/// designators would mostly fall below the minimum height inside their
/// parts at that scale, the largest [`DENSE_SCALES`] entry that still fits.
/// Never smaller than the standard choice.
pub fn assembly_scale(board: &Board, side: kdraft_board::Side, aw: f64, ah: f64) -> f64 {
    let Some(b) = board.outline.bbox else {
        return STANDARD_SCALES[0];
    };
    let std = choose_scale(b.width(), b.height(), aw, ah).unwrap_or(STANDARD_SCALES[0]);
    if designator_inside_fraction(board, side, std) >= DENSE_INSIDE_FRACTION {
        return std;
    }
    DENSE_SCALES
        .iter()
        .rev()
        .copied()
        .find(|s| b.width() * s <= aw && b.height() * s <= ah)
        .filter(|s| *s > std)
        .unwrap_or(std)
}

/// Distance of the board-size dimension lines from the outline, sheet mm.
pub const BOARD_SIZE_OFFSET: f64 = 10.0;
/// Room kept for board-size dimensions beside a fabrication view, mm.
pub const BOARD_SIZE_ROOM: f64 = 18.0;

/// Overall width (above the outline) and height (right of it) dimensions
/// of the outer outline contour, between its extreme vertices, with their
/// current positions stored. Empty if the outline has no vertices (e.g. a
/// single circle).
pub fn board_size_dimensions(board: &Board, view: &Id) -> Vec<Dimension> {
    let Some(outer) = board.outline.outer() else {
        return Vec::new();
    };
    let v = &outer.vertices;
    if v.len() < 2 {
        return Vec::new();
    }
    let pick = |better: &dyn Fn(Point, Point) -> bool| {
        (0..v.len()).fold(0, |best, i| if better(v[i], v[best]) { i } else { best })
    };
    let left = pick(&|a, b| a.x < b.x - 1e-9 || ((a.x - b.x).abs() <= 1e-9 && a.y < b.y));
    let right = pick(&|a, b| a.x > b.x + 1e-9 || ((a.x - b.x).abs() <= 1e-9 && a.y < b.y));
    let top = pick(&|a, b| a.y < b.y - 1e-9 || ((a.y - b.y).abs() <= 1e-9 && a.x > b.x));
    let bottom = pick(&|a, b| a.y > b.y + 1e-9 || ((a.y - b.y).abs() <= 1e-9 && a.x > b.x));
    let mut out = vec![
        Dimension::new(
            DimensionKind::Horizontal,
            view.clone(),
            Anchor::OutlineVertex(left),
            Some(Anchor::OutlineVertex(right)),
            BOARD_SIZE_OFFSET,
        ),
        Dimension::new(
            DimensionKind::Vertical,
            view.clone(),
            Anchor::OutlineVertex(top),
            Some(Anchor::OutlineVertex(bottom)),
            BOARD_SIZE_OFFSET,
        ),
    ];
    let holes = board.holes();
    for d in &mut out {
        if let Ok(p) = anchor::dimension_positions(d, board, &holes) {
            d.last = p;
        }
    }
    out
}

/// Height of the band kept for the fab notes on the fabrication page, mm.
pub const FAB_NOTES_BAND: f64 = 42.0;
/// Smallest strip left of the title block used for the fab notes, mm.
pub const FAB_NOTES_MIN_STRIP: f64 = 26.0;
/// Wrap width of the default fab notes, mm.
pub const FAB_NOTES_WIDTH: f64 = 230.0;

/// Width reserved for tables next to a view on generated pages, mm.
pub const TABLE_COLUMN_WIDTH: f64 = 135.0;
/// Width assumed for a drill table when placing tables side by side, mm.
pub const DRILL_TABLE_WIDTH: f64 = 95.0;

/// Clearance kept free inside the sheet's drawing area (room for scale
/// labels), mm.
pub const AREA_CLEARANCE: f64 = 5.0;

/// Height of one table row for a text height, mm. The renderer lays tables
/// out with the same value.
pub fn table_row_height(text_height: f64) -> f64 {
    text_height * 2.0
}

/// Extra height per additional text line in a table cell, mm.
pub fn table_line_height(text_height: f64) -> f64 {
    text_height * 1.5
}

/// Stored BOM item numbers of the document's BOM tables for the BOM
/// source `source` (`schematic` or `pcb`), by group key. Numbers stored
/// for another source are ignored: a BOM from another source is numbered
/// from scratch, in table order. Tables without a source tag (written
/// before it was recorded) hold PCB-fallback numbers.
pub fn bom_numbers(doc: &Document, source: &str) -> std::collections::BTreeMap<String, usize> {
    let mut m = std::collections::BTreeMap::new();
    for p in &doc.pages {
        for it in &p.items {
            if let Item::Table(t) = it {
                if t.bom_source.as_deref().unwrap_or("pcb") != source {
                    continue;
                }
                for (k, n) in &t.bom_items {
                    m.entry(k.clone()).or_insert(*n);
                }
            }
        }
    }
    m
}

/// Stores the item numbers of `bom` in the document: kept numbers for its
/// source plus the next free numbers for new groups (sequential in table
/// order when none are stored for that source). The numbers go to the
/// first BOM table; other BOM tables' stored numbers are cleared. Returns
/// whether the document changed (false without BOM tables).
pub fn store_bom_numbers(doc: &mut Document, bom: &kdraft_board::bom::Bom) -> bool {
    let tag = bom.source.tag();
    let stored = bom_numbers(doc, tag);
    let nums = kdraft_board::bom::number_lines(&bom.lines, &stored);
    let mut items: Vec<(String, usize)> = bom
        .lines
        .iter()
        .zip(&nums)
        .map(|(l, n)| (l.key(), *n))
        .collect();
    // Keep stored numbers of groups not in this BOM (they may come back).
    for (k, n) in &stored {
        if !items.iter().any(|(x, _)| x == k) {
            items.push((k.clone(), *n));
        }
    }
    let before = doc.clone();
    let mut first = true;
    for p in &mut doc.pages {
        for it in &mut p.items {
            if let Item::Table(t) = it {
                if t.kind != TableKind::Bom {
                    continue;
                }
                if first {
                    t.bom_items = items.clone();
                    t.bom_source = Some(tag.to_string());
                    first = false;
                } else {
                    t.bom_items.clear();
                    t.bom_source = None;
                }
            }
        }
    }
    *doc != before
}

/// Numbers the BOM from scratch (1, 2, ... in table order) and stores the
/// numbers; returns whether the document changed.
pub fn renumber_bom(doc: &mut Document, bom: &kdraft_board::bom::Bom) -> bool {
    let before = doc.clone();
    for p in &mut doc.pages {
        for it in &mut p.items {
            if let Item::Table(t) = it {
                t.bom_items.clear();
                t.bom_source = None;
            }
        }
    }
    store_bom_numbers(doc, bom);
    *doc != before
}

/// Largest standard scale at which a `w` x `h` mm board fits into an area of
/// `aw` x `ah` mm; `None` if it does not fit even at the smallest scale.
pub fn choose_scale(w: f64, h: f64, aw: f64, ah: f64) -> Option<f64> {
    STANDARD_SCALES
        .iter()
        .rev()
        .copied()
        .find(|s| w * s <= aw && h * s <= ah)
}

/// Drawing area of a page with a drawing sheet: inside the frame, clear of
/// the title block, minus [`AREA_CLEARANCE`].
pub fn drawing_area(sheet: &WksFile, size: &PageSize) -> BBox {
    let r = sheet.resolve(
        &PageSpec {
            width: size.width,
            height: size.height,
            page_number: 1,
        },
        &TextVars::new(),
    );
    r.drawing_area(AREA_CLEARANCE)
}

/// Estimated width of a text of `chars` characters at cap height `th`
/// (IBM Plex Sans averages about 0.8 cap heights per character, a little
/// more than digits and capitals).
fn est_width(chars: usize, th: f64) -> f64 {
    0.8 * th * chars as f64 + 1.6 * th
}

/// Height of BOM rows (title and header rows not included).
fn bom_row_heights(bom: &kdraft_board::bom::Bom, th: f64) -> Vec<f64> {
    bom.lines
        .iter()
        .map(|l| {
            let n = kdraft_board::bom::wrap_refs(&l.refs, kdraft_board::bom::REF_WRAP_CHARS).len();
            table_row_height(th) + (n.max(1) - 1) as f64 * table_line_height(th)
        })
        .collect()
}

/// BOM pages for `doc new`: the PCB-derived BOM (the schematic one needs
/// kicad-cli and is only fetched at export) split into chunks that fit the
/// drawing area, side by side when there is room, then on continuation
/// pages. Nothing is dropped; if the exported BOM is longer, `kdraft check`
/// reports the rows not shown. The first chunk stores the item numbers.
pub fn bom_pages(board: &Board, size: &PageSize, area: &BBox, styles: &Styles) -> Vec<Page> {
    bom_pages_for(&kdraft_board::bom::from_board(board), size, area, styles)
}

/// BOM pages for a given BOM (from the schematic or the PCB).
pub fn bom_pages_for(
    bom: &kdraft_board::bom::Bom,
    size: &PageSize,
    area: &BBox,
    styles: &Styles,
) -> Vec<Page> {
    if bom.lines.is_empty() {
        return Vec::new();
    }
    let th = styles.text_height;
    let row = table_row_height(th);
    let heights = bom_row_heights(bom, th);
    let maxlen = |f: &dyn Fn(&kdraft_board::bom::BomLine) -> usize| {
        bom.lines.iter().map(f).max().unwrap_or(0)
    };
    let width = est_width(5, th)
        + est_width(4, th)
        + est_width(kdraft_board::bom::REF_WRAP_CHARS, th)
        + est_width(maxlen(&|l| l.value.chars().count()).clamp(6, 40), th)
        + if bom.lines.iter().any(|l| l.dnp) {
            est_width(3, th)
        } else {
            0.0
        }
        + est_width(
            maxlen(&|l| {
                l.footprint
                    .rsplit(':')
                    .next()
                    .map_or(0, |s| s.chars().count())
            })
            .clamp(9, 48),
            th,
        );
    let gap = 8.0;
    let cols = (((area.width() + gap) / (width + gap)).floor() as usize).max(1);
    let mut pages = Vec::new();
    let mut next = 0;
    let n = heights.len();
    let numbers: Vec<(String, usize)> = bom
        .lines
        .iter()
        .enumerate()
        .map(|(i, l)| (l.key(), i + 1))
        .collect();
    while next < n {
        let title = if pages.is_empty() {
            "Bill of materials".to_string()
        } else {
            format!("Bill of materials ({})", pages.len() + 1)
        };
        let mut page = Page::new(&title, size.clone());
        for c in 0..cols {
            if next >= n {
                break;
            }
            // Title, header and (on the last chunk) the total row.
            let mut used = 3.0 * row;
            let start = next;
            while next < n && (next == start || used + heights[next] <= area.height()) {
                used += heights[next];
                next += 1;
            }
            let mut t = Table::new(
                TableKind::Bom,
                Point::new(area.min.x + c as f64 * (width + gap), area.min.y),
            );
            t.rows = Some((start, next - start));
            if start == 0 {
                t.bom_items = numbers.clone();
                t.bom_source = Some(bom.source.tag().to_string());
            }
            page.items.push(Item::Table(t));
        }
        pages.push(page);
    }
    pages
}

/// Free strip left of the title block (below the drawing area), if the
/// sheet has one at least `min_h` mm high: room for notes.
pub fn notes_strip(sheet: &WksFile, size: &PageSize, min_h: f64) -> Option<BBox> {
    let r = sheet.resolve(
        &PageSpec {
            width: size.width,
            height: size.height,
            page_number: 1,
        },
        &TextVars::new(),
    );
    let frame = r.frame();
    let tb = r.title_block()?;
    let area = r.drawing_area(AREA_CLEARANCE);
    let strip = BBox {
        min: Point::new(area.min.x, area.max.y + 1.0),
        max: Point::new(tb.min.x - AREA_CLEARANCE, frame.max.y - AREA_CLEARANCE),
    };
    (strip.height() >= min_h && strip.width() >= 100.0).then_some(strip)
}

/// A problem found by [`Document::validate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub page: Option<String>,
    pub item: Option<String>,
    pub message: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.page, &self.item) {
            (Some(p), Some(i)) => write!(f, "page {p}, item {i}: {}", self.message),
            (Some(p), None) => write!(f, "page {p}: {}", self.message),
            _ => write!(f, "{}", self.message),
        }
    }
}

impl Document {
    /// An empty document for a board.
    pub fn new(board_path: &str) -> Document {
        Document {
            version: CURRENT_VERSION,
            source: Source {
                board: board_path.to_string(),
                ..Default::default()
            },
            styles: Styles::default(),
            sheet: None,
            pages: Vec::new(),
            extra: Vec::new(),
        }
    }

    /// Default drawing set for a board on A3 landscape with the given
    /// drawing sheet: fabrication, assembly top, and drill and stackup
    /// pages. Views use the largest standard scale that fits the sheet's
    /// drawing area (1:2 if nothing fits). No sheet override is written:
    /// the sheet is chosen at render time (project sheet, else default).
    pub fn new_for_board(board: &Board, board_path: &str, sheet: &WksFile) -> Document {
        Document::new_for_board_bom(board, board_path, sheet, None)
    }

    /// [`Document::new_for_board`] with the BOM pages made from `bom` (the
    /// schematic BOM when known; else the PCB fallback).
    pub fn new_for_board_bom(
        board: &Board,
        board_path: &str,
        sheet: &WksFile,
        bom: Option<&kdraft_board::bom::Bom>,
    ) -> Document {
        Document::new_for_board_sized(board, board_path, sheet, bom, PageSize::a3_landscape())
    }

    /// [`Document::new_for_board_bom`] on paper `size`.
    pub fn new_for_board_sized(
        board: &Board,
        board_path: &str,
        sheet: &WksFile,
        bom: Option<&kdraft_board::bom::Bom>,
        size: PageSize,
    ) -> Document {
        let area = drawing_area(sheet, &size);
        let bbox = board.outline.bbox;
        let fit = |a: &BBox| {
            bbox.and_then(|b| choose_scale(b.width(), b.height(), a.width(), a.height()))
                .unwrap_or(STANDARD_SCALES[0])
        };
        let view = |kind: ViewKind, layers: &[&str], a: &BBox| View {
            id: Id::new_v4(),
            drill_marks: Some(DrillMarks::default_for(&kind)),
            render: None,
            assembly: AssemblyOptions::default(),
            kind,
            side: ViewSide::Front,
            scale: fit(a),
            rotation: 0.0,
            at: a.center(),
            layers: layers.iter().map(|s| s.to_string()).collect(),
            crop: None,
            parent: None,
            style: None,
            frame: false,
            extra: Vec::new(),
        };
        let mut fab = Page::new("Fabrication", size.clone());
        // Canonical layer names: kicad-cli 10.0.8 plotted Edge.Cuts,
        // Dwgs.User, F.Fab and F.CrtYd as spelled here (first real export,
        // docs/STATUS.md). UNVERIFIED on KiCad 9.0.x.
        // Room above and to the right for the board-size dimensions, and
        // a band at the bottom for the fab notes.
        // Notes go left of the title block when the sheet has room there,
        // else into a band at the bottom of the drawing area.
        let strip = notes_strip(sheet, &size, FAB_NOTES_MIN_STRIP);
        let notes_h = if strip.is_some() { 0.0 } else { FAB_NOTES_BAND };
        let fab_area = BBox {
            min: Point::new(area.min.x, area.min.y + BOARD_SIZE_ROOM),
            max: Point::new(area.max.x - BOARD_SIZE_ROOM, area.max.y - notes_h),
        };
        let fv = view(
            ViewKind::Fabrication,
            &["Edge.Cuts", "Dwgs.User"],
            &fab_area,
        );
        let fid = fv.id.clone();
        fab.items.push(Item::View(fv));
        for d in board_size_dimensions(board, &fid) {
            fab.items.push(Item::Dimension(d));
        }
        let (notes_at, notes_w) = match strip {
            Some(s) => (Point::new(s.min.x, s.min.y + 1.0), s.width()),
            None => (
                Point::new(area.min.x, area.max.y - notes_h + 6.0),
                FAB_NOTES_WIDTH.min(area.width()),
            ),
        };
        fab.items.push(Item::NoteList(NoteList {
            id: Id::new_v4(),
            at: notes_at,
            title: Some("NOTES".to_string()),
            width: Some(notes_w),
            notes: DEFAULT_FAB_NOTES
                .iter()
                .enumerate()
                .map(|(i, t)| Note {
                    id: format!("N{}", i + 1),
                    text: t.to_string(),
                })
                .collect(),
            extra: Vec::new(),
        }));
        let mut asm = Page::new("Assembly top", size.clone());
        let mut av = view(
            ViewKind::Assembly,
            &["F.Fab", "F.CrtYd", "Edge.Cuts"],
            &area,
        );
        av.scale = assembly_scale(
            board,
            kdraft_board::Side::Front,
            area.width(),
            area.height(),
        );
        // Written explicitly: values off, designators on (both render modes).
        av.assembly.values = Some(false);
        av.assembly.references = Some(true);
        asm.items.push(Item::View(av));
        // Assembly bottom only when parts are placed on the back.
        let back_parts = board.footprints.iter().any(|f| {
            f.side == kdraft_board::Side::Back && !f.attributes.board_only && f.has_copper_pads()
        });
        let asm_bottom = back_parts.then(|| {
            let mut p = Page::new("Assembly bottom", size.clone());
            let mut v = view(
                ViewKind::Assembly,
                &["B.Fab", "B.CrtYd", "Edge.Cuts"],
                &area,
            );
            v.side = ViewSide::Back;
            v.assembly.values = Some(false);
            v.assembly.references = Some(true);
            v.scale = assembly_scale(board, kdraft_board::Side::Back, area.width(), area.height());
            p.items.push(Item::View(v));
            p
        });

        // Drill and stackup: tables either in a right-hand column or in a
        // band below the drill view, whichever gives the larger view scale.
        let styles = Styles::default();
        let row = table_row_height(styles.text_height);
        let drill_h = (board.drill_table().len() + 3) as f64 * row;
        let stack_rows = board.stackup.as_ref().map_or(1, |s| s.layers.len());
        let stack_h = (stack_rows + 3) as f64 * row;
        // When both tables do not fit above each other in the column, the
        // layer stack table goes to a page of its own.
        let stacked = drill_h + 8.0 + stack_h <= area.height();
        let col_w = TABLE_COLUMN_WIDTH.min(area.width() / 2.0);
        let right = BBox {
            min: area.min,
            max: Point::new(area.max.x - col_w - 5.0, area.max.y),
        };
        let band = drill_h.max(stack_h) + 8.0;
        let below = BBox {
            min: area.min,
            max: Point::new(area.max.x, (area.max.y - band).max(area.min.y + 10.0)),
        };
        let tables_below = stacked && fit(&below) > fit(&right);
        let view_area = if tables_below { below } else { right };
        let mut drill = Page::new("Drill and stackup", size.clone());
        drill.items.push(Item::View(view(
            ViewKind::Drill,
            &["Edge.Cuts"],
            &view_area,
        )));
        let table = |kind: TableKind, at: Point| Table {
            id: Id::new_v4(),
            kind,
            at,
            columns: Vec::new(),
            style: None,
            bom_items: Vec::new(),
            bom_source: None,
            rows: None,
            extra: Vec::new(),
        };
        let (drill_at, stack_at) = if tables_below {
            let y = below.max.y + 8.0;
            (
                Point::new(area.min.x, y),
                Point::new(area.min.x + DRILL_TABLE_WIDTH + 10.0, y),
            )
        } else {
            let x = area.max.x - col_w;
            (
                Point::new(x, area.min.y),
                Point::new(x, area.min.y + drill_h + 8.0),
            )
        };
        drill
            .items
            .push(Item::Table(table(TableKind::Drill, drill_at)));
        let stack_page = if stacked {
            drill
                .items
                .push(Item::Table(table(TableKind::Stackup, stack_at)));
            None
        } else {
            let mut p = Page::new("Layer stack", size.clone());
            p.items
                .push(Item::Table(table(TableKind::Stackup, area.min)));
            Some(p)
        };

        let mut doc = Document::new(board_path);
        doc.pages = vec![fab, asm];
        doc.pages.extend(asm_bottom);
        doc.pages.push(drill);
        doc.pages.extend(stack_page);
        match bom {
            Some(b) => doc.pages.extend(bom_pages_for(b, &size, &area, &styles)),
            None => doc.pages.extend(bom_pages(board, &size, &area, &styles)),
        }
        doc
    }

    pub fn parse(text: &str) -> Result<Document, DocError> {
        io::parse(text)
    }

    /// Canonical text.
    pub fn to_text(&self) -> Result<String, DocError> {
        io::to_text(self)
    }

    /// Canonical S-expression value.
    pub fn to_value(&self) -> Result<kdraft_sexpr::Value, DocError> {
        io::to_value(self)
    }

    /// Consistency checks: duplicate ids, dangling view references, invalid
    /// scales and sizes.
    pub fn validate(&self) -> Vec<Problem> {
        let mut out = Vec::new();
        let mut seen: HashSet<&Id> = HashSet::new();
        let mut views: HashSet<&Id> = HashSet::new();
        for p in &self.pages {
            if !seen.insert(&p.id) {
                out.push(Problem {
                    page: Some(p.id.to_string()),
                    item: None,
                    message: "duplicate id".to_string(),
                });
            }
            for v in p.views() {
                views.insert(&v.id);
            }
        }
        for p in &self.pages {
            let page = Some(p.id.to_string());
            let mut problem = |item: &Id, message: String| {
                out.push(Problem {
                    page: page.clone(),
                    item: Some(item.to_string()),
                    message,
                })
            };
            if !(p.size.width.is_finite() && p.size.width > 0.0 && p.size.height > 0.0) {
                problem(&p.id, "page size must be positive".to_string());
            }
            for it in &p.items {
                let Some(id) = it.id() else { continue };
                if !seen.insert(id) {
                    problem(id, "duplicate id".to_string());
                }
                let mut check_view = |r: &Option<Id>, what: &str| {
                    if let Some(v) = r {
                        if !views.contains(v) {
                            problem(id, format!("{what} refers to missing view {v}"));
                        }
                    }
                };
                match it {
                    Item::View(v) => {
                        check_view(&v.parent, "parent");
                        if v.scale.is_nan() || v.scale <= 0.0 {
                            problem(id, format!("scale {} must be positive", v.scale));
                        }
                        if v.layers.is_empty() {
                            problem(id, "view has no layers".to_string());
                        }
                    }
                    Item::Dimension(d) => check_view(&d.view, "dimension"),
                    Item::Callout(c) => check_view(&c.view, "callout"),
                    _ => {}
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests;
