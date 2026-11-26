//! Simple tables: column widths from measured content, header row, borders;
//! and the drill and layer stack tables generated from the board model.

use kdraft_board::geom::{BBox, Point};
use kdraft_board::{Board, DrillShape, HoleKind};
use kdraft_doc::table_row_height;

use crate::dl::{Color, HAlign, Item, Path, Stroke, TextRun, VAlign};
use crate::font::Fonts;

#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    /// Key used by document column selections.
    pub key: String,
    pub title: String,
    pub align: HAlign,
    /// Cells hold drill-symbol row indices: drawn as the symbol (with the
    /// letter kept as invisible, searchable text).
    pub symbols: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableData {
    pub title: String,
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<String>>,
    /// Bold last row (totals).
    pub footer: Option<Vec<String>>,
}

impl TableData {
    /// Keeps only the columns named in `keys`, in that order; all columns
    /// if `keys` is empty or names none of them.
    pub fn select(&self, keys: &[String]) -> TableData {
        let idx: Vec<usize> = keys
            .iter()
            .filter_map(|k| self.columns.iter().position(|c| &c.key == k))
            .collect();
        if idx.is_empty() {
            return self.clone();
        }
        let pick = |r: &Vec<String>| {
            idx.iter()
                .map(|&i| r.get(i).cloned().unwrap_or_default())
                .collect()
        };
        TableData {
            title: self.title.clone(),
            columns: idx.iter().map(|&i| self.columns[i].clone()).collect(),
            rows: self.rows.iter().map(pick).collect(),
            footer: self.footer.as_ref().map(pick),
        }
    }
}

/// Number with up to `max_dec` decimals, trailing zeros removed, but at
/// least `min_dec`.
pub fn fmt_num(v: f64, min_dec: usize, max_dec: usize) -> String {
    let mut s = format!("{v:.max_dec$}");
    if s.contains('.') {
        let dot = s.find('.').unwrap_or(s.len());
        while s.len() > dot + 1 + min_dec && s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" {
        s = "0".to_string();
    }
    s
}

fn col(key: &str, title: &str, align: HAlign) -> Column {
    Column {
        key: key.to_string(),
        title: title.to_string(),
        align,
        symbols: false,
    }
}

/// Symbol letters A..Z, AA.. for drill rows (placeholders for drill
/// symbols).
pub fn symbol(i: usize) -> String {
    let mut n = i;
    let mut s = Vec::new();
    loop {
        s.push((b'A' + (n % 26) as u8) as char);
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    s.iter().rev().collect()
}

pub fn drill_table(board: &Board) -> TableData {
    let rows: Vec<Vec<String>> = board
        .drill_table()
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let (size, slot) = match r.shape {
                DrillShape::Round { diameter } => (fmt_num(diameter, 2, 3), false),
                DrillShape::Oval { width, height } => (
                    format!("{} x {}", fmt_num(width, 2, 3), fmt_num(height, 2, 3)),
                    true,
                ),
            };
            let kind = match (r.kind, slot) {
                (HoleKind::Via, _) => "via",
                (HoleKind::Pad, true) => "slot",
                (HoleKind::Pad, false) => "pad",
            };
            vec![
                symbol(i),
                size,
                if r.plated { "PTH" } else { "NPTH" }.to_string(),
                kind.to_string(),
                r.count.to_string(),
            ]
        })
        .collect();
    let total: usize = board.drill_table().iter().map(|r| r.count).sum();
    TableData {
        title: "DRILL TABLE".to_string(),
        columns: vec![
            Column {
                symbols: true,
                ..col("symbol", "SYM", HAlign::Center)
            },
            col("diameter", "SIZE (mm)", HAlign::Right),
            col("plated", "PLATING", HAlign::Center),
            col("type", "TYPE", HAlign::Left),
            col("count", "QTY", HAlign::Right),
        ],
        rows,
        footer: Some(vec![
            String::new(),
            "TOTAL".to_string(),
            String::new(),
            String::new(),
            total.to_string(),
        ]),
    }
}

pub fn stackup_table(board: &Board) -> TableData {
    let columns = vec![
        col("layer", "LAYER", HAlign::Left),
        col("type", "TYPE", HAlign::Left),
        col("material", "MATERIAL", HAlign::Left),
        col("thickness", "THICKNESS (mm)", HAlign::Right),
    ];
    let Some(st) = &board.stackup else {
        let thick = board
            .thickness
            .map_or("-".to_string(), |t| fmt_num(t, 2, 4));
        return TableData {
            title: "LAYER STACK".to_string(),
            columns,
            rows: vec![vec![
                "(no stackup in board file)".to_string(),
                String::new(),
                String::new(),
                String::new(),
            ]],
            footer: Some(vec![
                "BOARD THICKNESS".to_string(),
                String::new(),
                String::new(),
                thick,
            ]),
        };
    };
    let rows = st
        .layers
        .iter()
        .map(|l| {
            vec![
                if l.sublayer > 0 {
                    format!("{} ({})", l.name, l.sublayer + 1)
                } else {
                    l.name.clone()
                },
                l.kind.clone(),
                l.material.clone().unwrap_or_default(),
                l.thickness.map_or(String::new(), |t| fmt_num(t, 3, 4)),
            ]
        })
        .collect();
    TableData {
        title: "LAYER STACK".to_string(),
        columns,
        rows,
        footer: Some(vec![
            "TOTAL".to_string(),
            String::new(),
            st.copper_finish
                .as_ref()
                .map(|f| f.trim())
                .filter(|f| !f.is_empty() && !f.eq_ignore_ascii_case("none"))
                .map_or(String::new(), |f| format!("finish {f}")),
            fmt_num(st.total_thickness(), 3, 4),
        ]),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TableStyle {
    pub text_height: f64,
    pub line_width: f64,
    /// Fill behind the column header row.
    pub header_fill: Option<Color>,
}

impl TableStyle {
    /// From an effective table style (border and text colours are applied
    /// afterwards with [`crate::graphic::Recolor`]).
    pub fn from_style(s: &kdraft_doc::style::ItemStyle) -> TableStyle {
        TableStyle {
            text_height: s.text_height.unwrap_or(2.5),
            line_width: s.width.unwrap_or(0.25),
            header_fill: s
                .header_fill
                .filter(|c| !c.is_none())
                .map(|c| crate::graphic::color(c, 1.0)),
        }
    }
}

/// Lays the table out with its top-left corner at `at`. Cells may hold
/// several lines separated by `\n`; such rows grow by
/// [`kdraft_doc::table_line_height`] per extra line. Returns the items and
/// the table's box.
pub fn layout(t: &TableData, at: Point, style: TableStyle, fonts: &Fonts) -> (Vec<Item>, BBox) {
    let th = style.text_height;
    let row_h = table_row_height(th);
    let line_h = kdraft_doc::table_line_height(th);
    let pad = 0.8 * th;
    let n = t.columns.len().max(1);
    let mut widths = vec![0.0f64; n];
    let mut all_rows: Vec<(&Vec<String>, bool)> = t.rows.iter().map(|r| (r, false)).collect();
    if let Some(f) = &t.footer {
        all_rows.push((f, true));
    }
    for (i, c) in t.columns.iter().enumerate() {
        widths[i] = fonts.width(&c.title, th, true);
    }
    for (r, bold) in &all_rows {
        for (i, cell) in r.iter().enumerate().take(n) {
            for l in cell.split('\n') {
                widths[i] = widths[i].max(fonts.width(l, th, *bold));
            }
        }
    }
    for w in &mut widths {
        *w += 2.0 * pad;
    }
    let mut total_w: f64 = widths.iter().sum();
    let title_w = fonts.width(&t.title, th, true) + 2.0 * pad;
    if title_w > total_w {
        let extra = (title_w - total_w) / n as f64;
        for w in &mut widths {
            *w += extra;
        }
        total_w = title_w;
    }
    // Row tops: title, header, then data rows of varying height.
    let lines_of = |r: &Vec<String>| r.iter().map(|c| c.split('\n').count()).max().unwrap_or(1);
    let mut tops = vec![at.y, at.y + row_h];
    let mut y = at.y + 2.0 * row_h;
    for (r, _) in &all_rows {
        tops.push(y);
        y += row_h + (lines_of(r) - 1) as f64 * line_h;
    }
    let total_h = y - at.y;
    let bbox = BBox {
        min: at,
        max: Point::new(at.x + total_w, at.y + total_h),
    };
    let mut items = Vec::new();
    if let Some(c) = style.header_fill {
        items.push(Item::filled(
            Path::rect(&BBox {
                min: Point::new(at.x, at.y + row_h),
                max: Point::new(at.x + total_w, at.y + 2.0 * row_h),
            }),
            c,
        ));
    }
    let thin = Stroke::solid(style.line_width * 0.6, Color::BLACK);
    let thick = Stroke::solid(style.line_width, Color::BLACK);
    items.push(Item::stroked(Path::rect(&bbox), thick.clone()));
    let n_rows = tops.len();
    for (k, &y) in tops.iter().enumerate().skip(1) {
        let s = if k <= 2 || (t.footer.is_some() && k == n_rows - 1) {
            thick.clone()
        } else {
            thin.clone()
        };
        items.push(Item::stroked(
            Path::line(Point::new(at.x, y), Point::new(at.x + total_w, y)),
            s,
        ));
    }
    let mut x = at.x;
    for w in &widths[..n - 1] {
        x += w;
        items.push(Item::stroked(
            Path::line(Point::new(x, at.y + row_h), Point::new(x, at.y + total_h)),
            thin.clone(),
        ));
    }
    let text = |s: &str, x: f64, y: f64, align: HAlign, bold: bool| {
        Item::Text(TextRun {
            text: s.to_string(),
            at: Point::new(x, y),
            size: th,
            h_align: align,
            v_align: VAlign::Center,
            rotation: 0.0,
            bold,
            italic: false,
            color: Color::BLACK,
            invisible: false,
        })
    };
    items.push(text(
        &t.title,
        at.x + total_w / 2.0,
        at.y + row_h / 2.0,
        HAlign::Center,
        true,
    ));
    let cell_x = |i: usize, align: HAlign| {
        let x0 = at.x + widths[..i].iter().sum::<f64>();
        match align {
            HAlign::Left => x0 + pad,
            HAlign::Center => x0 + widths[i] / 2.0,
            HAlign::Right => x0 + widths[i] - pad,
        }
    };
    let hy = at.y + 1.5 * row_h;
    for (i, c) in t.columns.iter().enumerate() {
        items.push(text(
            &c.title,
            cell_x(i, HAlign::Center),
            hy,
            HAlign::Center,
            true,
        ));
    }
    for (k, (r, bold)) in all_rows.iter().enumerate() {
        let y = tops[k + 2] + 0.5 * row_h;
        for (i, cell) in r.iter().enumerate().take(n) {
            if cell.is_empty() {
                continue;
            }
            let a = t.columns.get(i).map_or(HAlign::Left, |c| c.align);
            if t.columns.get(i).is_some_and(|c| c.symbols) && !*bold {
                let c = Point::new(cell_x(i, HAlign::Center), y);
                items.extend(crate::symbols::draw(
                    &crate::symbols::symbol(k),
                    c,
                    1.3 * th,
                    0.6 * style.line_width,
                    Color::BLACK,
                ));
                if let Item::Text(mut tr) = text(cell, c.x, y, HAlign::Center, false) {
                    tr.invisible = true;
                    items.push(Item::Text(tr));
                }
                continue;
            }
            for (li, l) in cell.split('\n').enumerate() {
                items.push(text(l, cell_x(i, a), y + li as f64 * line_h, a, *bold));
            }
        }
    }
    (items, bbox)
}

/// Column keys of a BOM table shown when the document selects none.
pub const BOM_DEFAULT_COLUMNS: [&str; 5] = ["item", "qty", "refs", "value", "footprint"];

/// The default columns for `bom`: [`BOM_DEFAULT_COLUMNS`], plus a DNP
/// column when any line is not populated.
pub fn bom_default_columns(bom: &kdraft_board::bom::Bom) -> Vec<String> {
    let mut v: Vec<String> = BOM_DEFAULT_COLUMNS.iter().map(|s| s.to_string()).collect();
    if bom.lines.iter().any(|l| l.dnp) {
        v.push("dnp".into());
    }
    v
}

/// A BOM table: rows `rows` (a chunk) of `bom`, numbered by `numbers`.
/// Extra exported fields are columns `field:<label>`.
pub fn bom_table(
    bom: &kdraft_board::bom::Bom,
    numbers: &[usize],
    rows: Option<(usize, usize)>,
) -> TableData {
    use kdraft_board::bom::{wrap_refs, BomSource, REF_WRAP_CHARS};
    let n = bom.lines.len();
    let (start, count) = rows.unwrap_or((0, n));
    let end = start.saturating_add(count).min(n);
    let mut columns = vec![
        col("item", "ITEM", HAlign::Right),
        col("qty", "QTY", HAlign::Right),
        col("refs", "REFERENCES", HAlign::Left),
        col("value", "VALUE", HAlign::Left),
        col("footprint", "FOOTPRINT", HAlign::Left),
        col("dnp", "DNP", HAlign::Center),
    ];
    for c in &bom.extra_columns {
        columns.push(col(&format!("field:{c}"), &c.to_uppercase(), HAlign::Left));
    }
    let rows: Vec<Vec<String>> = (start.min(end)..end)
        .map(|i| {
            let l = &bom.lines[i];
            let fp = l.footprint.rsplit(':').next().unwrap_or(&l.footprint);
            let mut r = vec![
                numbers.get(i).map_or(String::new(), |n| n.to_string()),
                l.qty.to_string(),
                wrap_refs(&l.refs, REF_WRAP_CHARS).join("\n"),
                l.value.clone(),
                fp.to_string(),
                if l.dnp { "DNP".into() } else { String::new() },
            ];
            for c in &bom.extra_columns {
                r.push(l.fields.get(c).cloned().unwrap_or_default());
            }
            r
        })
        .collect();
    let mut title = match bom.source {
        BomSource::Schematic => "BILL OF MATERIALS".to_string(),
        BomSource::Pcb => "BILL OF MATERIALS (FROM PCB)".to_string(),
    };
    if start > 0 {
        title.push_str(" (CONT.)");
    }
    let total: usize = bom.lines.iter().filter(|l| !l.dnp).map(|l| l.qty).sum();
    let footer = (end == n).then(|| {
        let mut f = vec![String::new(); columns.len()];
        f[1] = total.to_string();
        f[2] = "TOTAL (fitted)".to_string();
        f
    });
    TableData {
        title,
        columns,
        rows,
        footer,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board() -> Board {
        Board::load(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/kicad9-minimal.kicad_pcb"),
        )
        .unwrap()
    }

    #[test]
    fn dnp_in_its_own_column_value_untouched() {
        use kdraft_board::bom::{Bom, BomLine, BomSource};
        let line = |r: &str, v: &str, dnp: bool| BomLine {
            refs: vec![r.into()],
            qty: 1,
            value: v.into(),
            footprint: "Lib:FP".into(),
            dnp,
            fields: Default::default(),
        };
        let bom = Bom {
            source: BomSource::Schematic,
            lines: vec![line("R1", "10k", false), line("J12", "ST_STDC14", true)],
            extra_columns: Vec::new(),
        };
        let cols = bom_default_columns(&bom);
        assert_eq!(cols.last().map(String::as_str), Some("dnp"));
        let t = bom_table(&bom, &[1, 2], None).select(&cols);
        let vi = t.columns.iter().position(|c| c.key == "value").unwrap();
        let di = t.columns.iter().position(|c| c.key == "dnp").unwrap();
        assert_eq!(t.rows[1][vi], "ST_STDC14");
        assert_eq!(t.rows[1][di], "DNP");
        assert_eq!(t.rows[0][di], "");
        let fitted = Bom {
            lines: vec![line("R1", "10k", false)],
            ..bom
        };
        assert!(!bom_default_columns(&fitted).contains(&"dnp".to_string()));
    }

    #[test]
    fn drill_and_stackup_tables() {
        let b = board();
        let d = drill_table(&b);
        assert_eq!(d.rows.len(), 4);
        assert_eq!(d.rows[0][0], "A");
        assert_eq!(d.rows[0][1], "0.30");
        assert_eq!(d.rows[0][2], "PTH");
        assert_eq!(d.rows[0][3], "via");
        assert!(d
            .rows
            .iter()
            .any(|r| r[3] == "slot" && r[1].contains(" x ")));
        assert!(d.rows.iter().any(|r| r[2] == "NPTH"));
        assert_eq!(d.footer.as_ref().unwrap()[4], "5");
        let s = stackup_table(&b);
        assert_eq!(s.rows.len(), 7);
        assert_eq!(s.footer.as_ref().unwrap()[3], "1.600");
        assert_eq!(s.footer.as_ref().unwrap()[2], "finish ENIG");
        for none in ["None", "", " none "] {
            let mut b2 = b.clone();
            if let Some(st) = b2.stackup.as_mut() {
                st.copper_finish = Some(none.to_string());
            }
            assert_eq!(stackup_table(&b2).footer.unwrap()[2], "", "{none:?}");
        }
        let sel = d.select(&[
            "count".to_string(),
            "symbol".to_string(),
            "nope".to_string(),
        ]);
        assert_eq!(sel.columns.len(), 2);
        assert_eq!(sel.rows[0], vec!["2".to_string(), "A".to_string()]);
    }

    #[test]
    fn layout_sizes_columns_to_content() {
        let f = Fonts::load().unwrap();
        let t = TableData {
            title: "T".into(),
            columns: vec![col("a", "A", HAlign::Left), col("b", "B", HAlign::Right)],
            rows: vec![
                vec!["short".into(), "1".into()],
                vec!["a much longer cell".into(), "22".into()],
            ],
            footer: None,
        };
        let style = TableStyle {
            text_height: 2.5,
            line_width: 0.25,
            header_fill: None,
        };
        let (items, bb) = layout(&t, Point::new(10.0, 20.0), style, &f);
        assert!((bb.height() - 4.0 * 5.0).abs() < 1e-9);
        let long = f.width("a much longer cell", 2.5, false);
        assert!(bb.width() > long + f.width("22", 2.5, false));
        // Texts stay inside the box.
        for tr in crate::dl::texts(&items) {
            assert!(tr.at.x > bb.min.x && tr.at.x < bb.max.x);
            assert!(tr.at.y > bb.min.y && tr.at.y < bb.max.y);
        }
    }

    #[test]
    fn helpers() {
        assert_eq!(fmt_num(0.3, 2, 3), "0.30");
        assert_eq!(fmt_num(1.6, 0, 4), "1.6");
        assert_eq!(fmt_num(1.0, 0, 4), "1");
        assert_eq!(fmt_num(0.035, 3, 4), "0.035");
        assert_eq!(symbol(0), "A");
        assert_eq!(symbol(25), "Z");
        assert_eq!(symbol(26), "AA");
        assert_eq!(symbol(27), "AB");
    }
}
