//! The item tree of the current page: rows (top-most item first), their
//! default names and icons, click selection with Shift (range) and Ctrl
//! (toggle), and the z-order target of a row drag. Plain logic; cached
//! per document revision by the app.

use kdraft_doc::{DimensionKind, GraphicShape, Id, Item, Page, TableKind, ViewKind, ViewSide};

use crate::icons::Icon;

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: Id,
    /// Index in the page's item list (z-order, 0 = bottom).
    pub z: usize,
    pub name: String,
    /// Kind shown dimmed after the name ("view", "table").
    pub kind: &'static str,
    pub icon: Icon,
    pub hidden: bool,
    pub locked: bool,
}

/// Default display name of an item.
pub fn default_name(it: &Item) -> String {
    match it {
        Item::View(v) => match (&v.kind, v.side) {
            (ViewKind::Fabrication, _) => "Fabrication view".into(),
            (ViewKind::Assembly, ViewSide::Front) => "Assembly top".into(),
            (ViewKind::Assembly, ViewSide::Back) => "Assembly bottom".into(),
            (ViewKind::Drill, _) => "Drill drawing".into(),
            (ViewKind::Layer, _) => format!("Layer view {}", v.layers.join(", ")),
            (ViewKind::Render3d, _) => "3D view".into(),
            (k, _) => format!("{} view", k.as_str()),
        },
        Item::Table(t) => {
            let base = match t.kind {
                TableKind::Drill => "Drill table",
                TableKind::Stackup => "Layer stack",
                TableKind::Bom => "BOM",
                _ => "Table",
            };
            match t.rows {
                Some((start, n)) => format!("{base} rows {}-{}", start + 1, start + n),
                None => base.into(),
            }
        }
        Item::Dimension(d) => match d.kind {
            DimensionKind::Radius | DimensionKind::Radial => "Radius".into(),
            DimensionKind::Diameter => "Diameter".into(),
            DimensionKind::Ordinate => format!("Ordinate {}", d.axis.as_deref().unwrap_or("x")),
            DimensionKind::Horizontal => "Horizontal dimension".into(),
            DimensionKind::Vertical => "Vertical dimension".into(),
            DimensionKind::Aligned => "Aligned dimension".into(),
            _ => "Dimension".into(),
        },
        Item::Callout(c) => match (c.kind(), &c.target, &c.label) {
            (kdraft_doc::CalloutKind::Label, _, Some(l)) => format!("Callout {l}"),
            (kdraft_doc::CalloutKind::Refdes, _, _) => {
                format!("Callout {}", c.anchor_reference().unwrap_or("?"))
            }
            (_, Some(t), _) => format!("Callout {}", t.split_once(':').map_or(t.as_str(), |x| x.1)),
            _ => "Callout".into(),
        },
        Item::NoteList(n) => n.title.clone().unwrap_or_else(|| "Notes".into()),
        Item::Graphic(g) => match &g.shape {
            GraphicShape::Text { text, .. } => {
                let first = text.lines().next().unwrap_or("");
                if text.contains('\n') {
                    format!("Text \"{first}...\"")
                } else {
                    format!("Text \"{first}\"")
                }
            }
            GraphicShape::Line { .. } => "Line".into(),
            GraphicShape::Rect { .. } => "Rectangle".into(),
            GraphicShape::Circle { .. } => "Circle".into(),
            GraphicShape::Arc { .. } => "Arc".into(),
            GraphicShape::Polygon { .. } => "Polygon".into(),
            GraphicShape::Polyline { .. } => "Polyline".into(),
            GraphicShape::Image { .. } => "Image".into(),
        },
        Item::Unknown(_) => "Unknown item".into(),
    }
}

pub fn display_name(it: &Item) -> String {
    it.name().map_or_else(|| default_name(it), str::to_string)
}

pub fn icon_for(it: &Item) -> Icon {
    match it {
        Item::View(v) => match v.kind {
            ViewKind::Assembly => Icon::ViewAssembly,
            ViewKind::Drill => Icon::ViewDrill,
            ViewKind::Render3d => Icon::View3d,
            _ => Icon::ViewFab,
        },
        Item::Table(t) => match t.kind {
            TableKind::Drill => Icon::TableDrill,
            TableKind::Stackup => Icon::TableStackup,
            TableKind::Bom => Icon::TableBom,
            _ => Icon::Table,
        },
        Item::Dimension(d) => match d.kind {
            DimensionKind::Radius | DimensionKind::Radial => Icon::DimRadius,
            DimensionKind::Diameter => Icon::DimDiameter,
            DimensionKind::Ordinate => Icon::DimOrdinate,
            DimensionKind::Vertical => Icon::DimVertical,
            DimensionKind::Aligned => Icon::DimAligned,
            _ => Icon::DimLinear,
        },
        Item::Callout(_) => Icon::Callout,
        Item::NoteList(_) => Icon::Notes,
        Item::Graphic(g) => match g.shape {
            GraphicShape::Text { .. } => Icon::Text,
            GraphicShape::Line { .. } => Icon::Line,
            GraphicShape::Rect { .. } => Icon::Rect,
            GraphicShape::Polyline { .. } => Icon::Polyline,
            GraphicShape::Circle { .. } => Icon::Circle,
            _ => Icon::Graphic,
        },
        Item::Unknown(_) => Icon::Info,
    }
}

/// Rows of a page, top-most item first.
pub fn rows(page: &Page) -> Vec<Row> {
    page.items
        .iter()
        .enumerate()
        .rev()
        .filter_map(|(z, it)| {
            Some(Row {
                id: it.id()?.clone(),
                z,
                name: display_name(it),
                kind: it.kind_name(),
                icon: icon_for(it),
                hidden: it.hidden(),
                locked: it.locked(),
            })
        })
        .collect()
}

/// Selection after clicking row `clicked` with the modifiers; `anchor`
/// is the row of the last plain click (Shift ranges from it).
pub fn click_select(
    rows: &[Row],
    current: &[Id],
    clicked: usize,
    anchor: Option<usize>,
    shift: bool,
    ctrl: bool,
) -> Vec<Id> {
    let Some(row) = rows.get(clicked) else {
        return current.to_vec();
    };
    if shift {
        let a = anchor.filter(|a| *a < rows.len()).unwrap_or(clicked);
        let (lo, hi) = (a.min(clicked), a.max(clicked));
        // The anchor row first: it is the alignment reference.
        let mut out = vec![rows[a].id.clone()];
        out.extend(
            rows[lo..=hi]
                .iter()
                .map(|r| r.id.clone())
                .filter(|i| *i != rows[a].id),
        );
        return out;
    }
    if ctrl {
        let mut out = current.to_vec();
        if let Some(p) = out.iter().position(|i| *i == row.id) {
            out.remove(p);
        } else {
            out.push(row.id.clone());
        }
        return out;
    }
    vec![row.id.clone()]
}

/// z-order index for dropping the row `from` at insertion index `insert`
/// (0..=rows.len(), in display order); `None` when nothing moves.
pub fn drop_z(rows: &[Row], from: usize, insert: usize) -> Option<usize> {
    let to = crate::pages::move_target(from, insert, rows.len())?;
    // The moved item takes the place (page index) of the item now shown
    // at its target row.
    rows.get(to).map(|r| r.z)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kdraft_board::geom::Point;
    use kdraft_doc::{PageSize, Table};

    fn page() -> Page {
        let mut p = Page::new("p", PageSize::a3_landscape());
        for k in [TableKind::Drill, TableKind::Stackup, TableKind::Bom] {
            p.items.push(Item::Table(Table::new(k, Point::ORIGIN)));
        }
        p
    }

    #[test]
    fn rows_top_first_with_names_and_flags() {
        let mut p = page();
        p.items[0].set_name(Some("Holes"));
        p.items[1].set_hidden(true);
        let r = rows(&p);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].name, "BOM");
        assert_eq!(r[0].z, 2);
        assert!(r[1].hidden);
        assert_eq!(r[2].name, "Holes");
        assert_eq!(r[2].icon, Icon::TableDrill);
    }

    #[test]
    fn click_shift_ctrl() {
        let r = rows(&page());
        let ids: Vec<Id> = r.iter().map(|x| x.id.clone()).collect();
        assert_eq!(
            click_select(&r, &[], 1, None, false, false),
            vec![ids[1].clone()]
        );
        let s = click_select(&r, &[ids[0].clone()], 2, Some(0), true, false);
        assert_eq!(s, ids);
        let s = click_select(&r, &ids, 1, Some(0), false, true);
        assert_eq!(s, vec![ids[0].clone(), ids[2].clone()]);
        let s = click_select(&r, &s, 1, Some(0), false, true);
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn drag_targets_in_z_order() {
        let r = rows(&page());
        // Top row (z 2) dropped below the last row: to the bottom (z 0).
        assert_eq!(drop_z(&r, 0, 3), Some(0));
        // Bottom row (z 0) dropped above the first: to the top.
        assert_eq!(drop_z(&r, 2, 0), Some(2));
        assert_eq!(drop_z(&r, 1, 1), None);
        assert_eq!(drop_z(&r, 1, 2), None);
    }
}
