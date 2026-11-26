//! Plotted SVG (from `kicad-cli`) into display-list paths. The caller
//! supplies the board-to-SVG transform (from `kdraft_plot::fit`); nothing
//! about the plot's coordinates is assumed here.

use kdraft_board::geom::Affine;
use kdraft_plot::fit::PlotTransform;
use kdraft_plot::path::Seg;
use kdraft_plot::svg::{self, SvgDoc};

use crate::dl::{Color, Fill, Item, LineCap, LineJoin, Path, PathItem, Stroke};
use crate::view::ViewMap;

/// SVG user units to sheet mm: inverse plot transform, then the view.
pub fn plot_to_sheet(fit: &PlotTransform, map: &ViewMap) -> Option<Affine> {
    Some(fit.affine().inverse()?.then(&map.board_to_sheet))
}

fn color(c: svg::Rgb) -> Color {
    Color::rgb(c.r, c.g, c.b)
}

/// Converts the visible content of `doc` with `m` (user units to sheet),
/// white knock-outs included. Fully transparent paints are dropped;
/// partial opacity is not kept.
pub fn import(doc: &SvgDoc, m: &Affine) -> Vec<Item> {
    let scale = m.mean_scale();
    let mut out = Vec::new();
    for s in doc.ink() {
        let mut path = Path::new();
        for sp in &s.subpaths {
            path.move_to(m.apply(sp.start));
            for seg in &sp.segs {
                match *seg {
                    Seg::Line { to } => {
                        path.line_to(m.apply(to));
                    }
                    Seg::Cubic { c1, c2, to } => {
                        path.cubic_to(m.apply(c1), m.apply(c2), m.apply(to));
                    }
                }
            }
            if sp.closed {
                path.close();
            }
        }
        let fill = s.fill.filter(|f| f.opacity > 0.0).map(|f| Fill {
            color: color(f.color),
            even_odd: s.fill_evenodd,
        });
        let stroke = s
            .stroke
            .as_ref()
            .filter(|st| st.paint.opacity > 0.0)
            .map(|st| Stroke {
                width: st.width * scale,
                color: color(st.paint.color),
                cap: match st.cap {
                    svg::LineCap::Butt => LineCap::Butt,
                    svg::LineCap::Round => LineCap::Round,
                    svg::LineCap::Square => LineCap::Square,
                },
                join: match st.join {
                    svg::LineJoin::Miter => LineJoin::Miter,
                    svg::LineJoin::Round => LineJoin::Round,
                    svg::LineJoin::Bevel => LineJoin::Bevel,
                },
                dash: st
                    .dash
                    .as_ref()
                    .map(|d| d.iter().map(|x| x * scale).collect()),
            });
        if fill.is_none() && stroke.is_none() {
            continue;
        }
        out.push(Item::Path(PathItem { path, stroke, fill }));
    }
    out
}

/// Plotted ink of a layer in `color` at `opacity`. KiCad plots a layer in
/// black with white knock-outs (drill marks inside pads, slots): black
/// becomes the layer colour; white knock-outs are returned separately so
/// the caller can draw them once, above all layers of a view, as a
/// multi-layer plot does. Other colours are kept.
pub fn recolor(items: Vec<Item>, color: Color, opacity: f64) -> (Vec<Item>, Vec<Item>) {
    let mut ink = Vec::new();
    let mut knock = Vec::new();
    let is_white = |c: Color| c.r == 255 && c.g == 255 && c.b == 255;
    for it in items {
        let Item::Path(mut p) = it else {
            ink.push(it);
            continue;
        };
        let white_fill = p.fill.is_some_and(|f| is_white(f.color));
        let white_stroke = p.stroke.as_ref().is_some_and(|s| is_white(s.color));
        if (white_fill || p.fill.is_none()) && (white_stroke || p.stroke.is_none()) {
            knock.push(Item::Path(p));
            continue;
        }
        if let Some(f) = &mut p.fill {
            if f.color == Color::BLACK {
                f.color = color;
            }
            f.color = f.color.faded(opacity);
        }
        if let Some(st) = &mut p.stroke {
            if st.color == Color::BLACK {
                st.color = color;
            }
            st.color = st.color.faded(opacity);
        }
        ink.push(Item::Path(p));
    }
    (ink, knock)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kdraft_board::geom::{BBox, Point};
    use kdraft_doc::{Id, View, ViewKind, ViewSide};

    #[test]
    fn plot_lands_where_the_board_maps() {
        // A plot of a 10 x 5 board drawn at 100 units per mm, offset.
        let doc = svg::parse(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="20mm" height="10mm" viewBox="0 0 2000 1000">
                 <rect x="500" y="250" width="1000" height="500" fill="none" stroke="#000" stroke-width="10"/>
               </svg>"##,
        )
        .unwrap();
        let fit = PlotTransform {
            sx: 100.0,
            ox: 500.0,
            sy: 100.0,
            oy: 250.0,
        };
        let v = View {
            id: Id("v".into()),
            kind: ViewKind::Fabrication,
            side: ViewSide::Front,
            scale: 2.0,
            rotation: 0.0,
            at: Point::new(100.0, 100.0),
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
        let board = BBox {
            min: Point::new(0.0, 0.0),
            max: Point::new(10.0, 5.0),
        };
        let map = ViewMap::new(&v, board);
        let m = plot_to_sheet(&fit, &map).unwrap();
        let items = import(&doc, &m);
        assert_eq!(items.len(), 1);
        let Item::Path(p) = &items[0] else { panic!() };
        let bb = p.path.control_bbox().unwrap();
        assert!(bb.min.approx_eq(Point::new(90.0, 95.0), 1e-9), "{bb:?}");
        assert!(bb.max.approx_eq(Point::new(110.0, 105.0), 1e-9), "{bb:?}");
        // 10 units = 0.1 board mm = 0.2 sheet mm.
        assert!((p.stroke.as_ref().unwrap().width - 0.2).abs() < 1e-12);
    }

    fn fixture(name: &str) -> SvgDoc {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/kicad-cli-svg")
            .join(name);
        svg::parse(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    /// Real kicad-cli 10.0.8 plots: copper is black with white drill
    /// knock-outs (filled circles and a stroked slot); edge cuts, fab and
    /// courtyard have none.
    #[test]
    fn real_plots_recolour_and_keep_knockouts() {
        let red = Color::rgb(200, 52, 52);
        for (name, knockouts) in [
            ("F_Cu-mode0.svg", 4),
            ("B_Cu-mode0.svg", 4),
            ("F_Cu-mode0-mirror.svg", 4),
            ("Edge_Cuts-mode0.svg", 0),
            ("F_Fab-mode0.svg", 0),
            ("F_Courtyard-mode0.svg", 0),
        ] {
            let d = fixture(name);
            let items = import(&d, &Affine::IDENTITY);
            assert!(!items.is_empty(), "{name}");
            let (ink, knock) = recolor(items, red, 0.5);
            assert_eq!(knock.len(), knockouts, "{name}");
            for it in &ink {
                let Item::Path(p) = it else { continue };
                for c in p
                    .fill
                    .map(|f| f.color)
                    .into_iter()
                    .chain(p.stroke.as_ref().map(|s| s.color))
                {
                    assert_eq!((c.r, c.g, c.b, c.a), (200, 52, 52, 128), "{name}");
                }
            }
            for it in &knock {
                let Item::Path(p) = it else { continue };
                for c in p
                    .fill
                    .map(|f| f.color)
                    .into_iter()
                    .chain(p.stroke.as_ref().map(|s| s.color))
                {
                    assert_eq!(c, Color::WHITE, "{name}");
                }
            }
        }
        // Fab text is plotted as stroked outlines: it recolours too.
        let fab = import(&fixture("F_Fab-mode0.svg"), &Affine::IDENTITY);
        assert!(fab.len() > 10);
    }
}
