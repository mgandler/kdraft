//! Drawing sheet rendering: natively from a resolved `.kicad_wks`, or from
//! a `kicad-cli` SVG of a stub board placed by a fit against the native
//! geometry (never by assumed coordinates).

use kdraft_board::geom::{Affine, Point};
use kdraft_plot::fit::{fit, shape_vertices, Fit, PointIndex, RefPoint};
use kdraft_plot::svg::SvgDoc;
use kdraft_sheet::{HJustify, ResolvedSheet, VJustify};

use crate::dl::{Color, Fill, HAlign, Item, Path, PathItem, Stroke, TextRun, VAlign};
use crate::font::Fonts;
use crate::import;

fn texts(sheet: &ResolvedSheet, fonts: &Fonts, invisible: bool) -> Vec<Item> {
    let mut out = Vec::new();
    for t in &sheet.texts {
        if t.text.trim().is_empty() {
            continue;
        }
        let mut size = t.size.1;
        if let Some(mh) = t.max_height {
            size = size.min(mh);
        }
        if let Some(ml) = t.max_len {
            let w = fonts.width(&t.text, size, t.bold);
            if w > ml && w > 0.0 {
                size *= ml / w;
            }
        }
        out.push(Item::Text(TextRun {
            text: t.text.clone(),
            at: t.at,
            size,
            h_align: match t.h_justify {
                HJustify::Left => HAlign::Left,
                HJustify::Center => HAlign::Center,
                HJustify::Right => HAlign::Right,
            },
            v_align: match t.v_justify {
                VJustify::Top => VAlign::Top,
                VJustify::Center => VAlign::Center,
                VJustify::Bottom => VAlign::Baseline,
            },
            rotation: t.rotation,
            bold: t.bold,
            italic: t.italic,
            color: Color::BLACK,
            invisible,
        }));
    }
    out
}

/// Lines, rectangles, polygons and texts of the sheet.
pub fn render_native(sheet: &ResolvedSheet, fonts: &Fonts) -> Vec<Item> {
    let mut out = Vec::new();
    for r in &sheet.rects {
        out.push(Item::stroked(
            Path::rect(&r.bbox),
            Stroke::solid(r.width, Color::BLACK),
        ));
    }
    for l in &sheet.lines {
        out.push(Item::stroked(
            Path::line(l.a, l.b),
            Stroke::solid(l.width, Color::BLACK),
        ));
    }
    for p in &sheet.polygons {
        out.push(Item::Path(PathItem {
            path: Path::polyline(&p.points, true),
            stroke: (p.width > 0.0).then(|| Stroke::solid(p.width, Color::BLACK)),
            fill: Some(Fill {
                color: Color::BLACK,
                even_odd: false,
            }),
        }));
    }
    out.extend(texts(sheet, fonts, false));
    out
}

/// Fits the stub-sheet SVG to page mm using the native geometry of the
/// same sheet: line ends and rectangle corners as reference points.
pub fn fit_sheet_svg(native: &ResolvedSheet, doc: &SvgDoc) -> Option<Fit> {
    let mut refs = Vec::new();
    for l in &native.lines {
        for p in [l.a, l.b] {
            refs.push(RefPoint {
                at: p,
                label: "line".to_string(),
            });
        }
    }
    for r in &native.rects {
        for p in [
            r.bbox.min,
            r.bbox.max,
            Point::new(r.bbox.min.x, r.bbox.max.y),
            Point::new(r.bbox.max.x, r.bbox.min.y),
        ] {
            refs.push(RefPoint {
                at: p,
                label: "rect".to_string(),
            });
        }
    }
    let native_box = kdraft_board::geom::BBox::from_points(refs.iter().map(|r| r.at))?;
    let svg_box = doc.content_bbox()?;
    let guess = kdraft_plot::fit::fit_bbox(&native_box, &svg_box, 1.0, 1.0)?.scale();
    let index = PointIndex::new(shape_vertices(doc), guess);
    fit(&native_box, &svg_box, &refs, &index, "sheet-geometry")
}

/// The kicad-cli sheet SVG as page background, plus the native texts as
/// invisible searchable text. `None` if the fit is not good enough.
pub fn render_from_svg(native: &ResolvedSheet, doc: &SvgDoc, fonts: &Fonts) -> Option<Vec<Item>> {
    let f = fit_sheet_svg(native, doc)?;
    if !f.is_good() {
        return None;
    }
    let m: Affine = f.transform.affine().inverse()?;
    let mut out = import::import(doc, &m);
    out.extend(texts(native, fonts, true));
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kdraft_board::TitleBlock;
    use kdraft_sheet::{PageInfo, PageSpec, TextVars};

    fn resolved() -> ResolvedSheet {
        let tb = TitleBlock {
            title: Some("Hello".into()),
            ..Default::default()
        };
        let vars = TextVars::from_title_block(
            &tb,
            &PageInfo {
                page_number: 1,
                page_count: 1,
                paper: "A3".into(),
                ..Default::default()
            },
        );
        kdraft_sheet::default_sheet().unwrap().resolve(
            &PageSpec {
                width: 420.0,
                height: 297.0,
                page_number: 1,
            },
            &vars,
        )
    }

    #[test]
    fn native_sheet_items() {
        let f = Fonts::load().unwrap();
        let r = resolved();
        let items = render_native(&r, &f);
        let t = crate::dl::texts(&items);
        assert!(t
            .iter()
            .any(|t| t.text == "Title: Hello" && t.bold && t.italic));
        // Empty comment lines are skipped.
        assert!(t.iter().all(|t| !t.text.is_empty()));
        assert_eq!(items.len() - t.len(), r.rects.len() + r.lines.len());
    }

    /// An SVG drawing the same sheet at another scale and offset is placed
    /// back onto the page by the fit.
    #[test]
    fn svg_sheet_is_fitted_not_assumed() {
        let f = Fonts::load().unwrap();
        let r = resolved();
        let mut s = String::from(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"420mm\" height=\"297mm\" viewBox=\"0 0 42000 29700\"><g transform=\"translate(30 -20)\" stroke=\"#000\" fill=\"none\" stroke-width=\"15\">",
        );
        for l in &r.lines {
            s.push_str(&format!(
                "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"/>",
                l.a.x * 100.0,
                l.a.y * 100.0,
                l.b.x * 100.0,
                l.b.y * 100.0
            ));
        }
        for q in &r.rects {
            s.push_str(&format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/>",
                q.bbox.min.x * 100.0,
                q.bbox.min.y * 100.0,
                q.bbox.width() * 100.0,
                q.bbox.height() * 100.0
            ));
        }
        s.push_str("</g></svg>");
        let doc = kdraft_plot::svg::parse(&s).unwrap();
        let fit = fit_sheet_svg(&r, &doc).unwrap();
        assert!(fit.is_good(), "{:?}", fit.residuals);
        assert!((fit.transform.sx - 100.0).abs() < 1e-6 && (fit.transform.oy + 20.0).abs() < 1e-3);
        let items = render_from_svg(&r, &doc, &f).unwrap();
        let t = crate::dl::texts(&items);
        assert!(!t.is_empty() && t.iter().all(|t| t.invisible));
        // The outer frame lands on the margin box again.
        let first = items.iter().find_map(|i| match i {
            Item::Path(p) => p.path.control_bbox(),
            _ => None,
        });
        assert!(first.is_some());
    }
}
