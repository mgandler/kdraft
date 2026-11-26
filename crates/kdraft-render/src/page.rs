//! Composes document pages: drawing sheet, views, tables, graphics.

use std::collections::BTreeMap;

use kdraft_board::geom::{Affine, BBox, Point};
use kdraft_board::{Board, Side};
use kdraft_doc::{
    Document, GraphicShape, Item as DocItem, Page, TableKind, View, ViewKind, ViewSide,
};
use kdraft_plot::fit::PlotTransform;
use kdraft_plot::svg::SvgDoc;
use kdraft_sheet::{PageInfo, PageSpec, TextVars, WksFile};

use crate::assembly;
use crate::dl::{Color, DocList, Group, HAlign, Item, PageList, Path, Stroke, TextRun, VAlign};
use crate::font::Fonts;
use crate::import;
use crate::native::{self, Content};
use crate::sheet;
use crate::table::{self, TableStyle};
use crate::view::{scale_label, ViewMap};

/// Label put on views drawn by the native renderer.
pub const NATIVE_NOTE: &str = "NATIVE PREVIEW (board model, not kicad-cli plot output)";

/// Margin around plotted content when clipping a view, sheet mm.
pub const PLOT_CLIP_MARGIN: f64 = 1.0;

/// Plotted layers of one view with the transform fitted for them; for 3D
/// views the rendered picture instead.
pub struct PlotContent {
    pub layers: Vec<SvgDoc>,
    pub fit: PlotTransform,
    /// The picture from `kicad-cli pcb render` or its progress (3D views).
    pub image: Option<crate::render3d::State>,
}

/// Placeholder text of a 3D view without kicad-cli.
pub const NO_KICAD_3D: &str = "3D view needs kicad-cli";
/// Placeholder text of a 3D view waiting for its picture.
pub const PENDING_3D: &str = "Rendering 3D view...";

/// Per-page sheet inputs.
pub struct PageSheet<'a> {
    pub wks: &'a WksFile,
    /// A kicad-cli rendering of the same sheet, if available.
    pub svg: Option<&'a SvgDoc>,
}

/// Values for sheet text variables that do not come from the board, and
/// the schematic BOM when one was exported.
#[derive(Debug, Clone, Default)]
pub struct SheetContext {
    pub file_name: Option<String>,
    pub project_name: Option<String>,
    pub kicad_version: Option<String>,
    pub project_vars: BTreeMap<String, String>,
    /// BOM from `kicad-cli sch export bom`; `None`: derived from the PCB.
    pub bom: Option<std::sync::Arc<kdraft_board::bom::Bom>>,
}

/// The BOM to show: the schematic's if exported, else the PCB fallback.
pub fn bom_for(board: &Board, ctx: &SheetContext) -> kdraft_board::bom::Bom {
    match &ctx.bom {
        Some(b) => (**b).clone(),
        None => kdraft_board::bom::from_board(board),
    }
}

/// Renders every page. `sheet_for` gives each page's sheet, `plot_for`
/// plotted content for a view (or `None` for native rendering).
pub fn render_document<'s>(
    doc: &Document,
    board: &Board,
    ctx: &SheetContext,
    sheet_for: &dyn Fn(usize, &Page) -> PageSheet<'s>,
    plot_for: &dyn Fn(&View) -> Option<PlotContent>,
    fonts: &Fonts,
) -> (DocList, Vec<String>) {
    let mut warnings = Vec::new();
    let mut pages = Vec::new();
    for (i, p) in doc.pages.iter().enumerate() {
        let sheet = sheet_for(i, p);
        let (pl, w) = render_page(doc, p, i, board, ctx, &sheet, plot_for, fonts);
        warnings.extend(w.into_iter().map(|w| format!("page {}: {w}", i + 1)));
        pages.push(pl);
    }
    let title = board
        .title_block
        .title
        .clone()
        .unwrap_or_else(|| "kdraft drawing".to_string());
    (DocList { title, pages }, warnings)
}

/// The board's box as views map it (outline box, else footprint
/// positions, else 10 mm).
pub fn board_box_of(board: &Board) -> BBox {
    board_box(board)
}

fn board_box(board: &Board) -> BBox {
    board
        .outline
        .bbox
        .or_else(|| BBox::from_points(board.footprints.iter().map(|f| f.position)))
        .unwrap_or(BBox {
            min: Point::ORIGIN,
            max: Point::new(10.0, 10.0),
        })
}

#[allow(clippy::too_many_arguments)]
pub fn render_page(
    doc: &Document,
    page: &Page,
    index: usize,
    board: &Board,
    ctx: &SheetContext,
    sheet: &PageSheet<'_>,
    plot_for: &dyn Fn(&View) -> Option<PlotContent>,
    fonts: &Fonts,
) -> (PageList, Vec<String>) {
    let (mut items, mut warnings) = render_sheet(doc, page, index, board, ctx, sheet, fonts);
    for it in page.items.iter().filter(|i| !i.hidden()) {
        let (v, w) = render_item(doc, it, board, ctx, plot_for, fonts);
        items.extend(v);
        warnings.extend(w);
    }
    (
        PageList {
            width: page.size.width,
            height: page.size.height,
            title: page
                .title
                .clone()
                .unwrap_or_else(|| format!("Page {}", index + 1)),
            items,
        },
        warnings,
    )
}

/// The drawing sheet of a page (frame, title block) as display items.
pub fn render_sheet(
    doc: &Document,
    page: &Page,
    index: usize,
    board: &Board,
    ctx: &SheetContext,
    sheet: &PageSheet<'_>,
    fonts: &Fonts,
) -> (Vec<Item>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut items = Vec::new();
    let mut vars = TextVars::from_title_block(
        &board.title_block,
        &PageInfo {
            page_number: index + 1,
            page_count: doc.pages.len(),
            paper: page.size.name.clone(),
            sheet_name: page.title.clone(),
            file_name: ctx.file_name.clone(),
            project_name: ctx.project_name.clone(),
            kicad_version: ctx.kicad_version.clone(),
        },
    );
    vars.set_project(&ctx.project_vars);
    let resolved = sheet.wks.resolve(
        &PageSpec {
            width: page.size.width,
            height: page.size.height,
            page_number: index + 1,
        },
        &vars,
    );
    let from_svg = sheet
        .svg
        .and_then(|svg| sheet::render_from_svg(&resolved, svg, fonts));
    match from_svg {
        Some(v) => items.extend(v),
        None => {
            if sheet.svg.is_some() {
                warnings.push(
                    "kicad-cli sheet SVG could not be fitted to the sheet geometry; drawn natively"
                        .to_string(),
                );
            }
            items.extend(sheet::render_native(&resolved, fonts));
        }
    }
    (items, warnings)
}

/// One document item as display items (sheet mm), with warnings for
/// content that is not rendered.
pub fn render_item(
    doc: &Document,
    it: &DocItem,
    board: &Board,
    ctx: &SheetContext,
    plot_for: &dyn Fn(&View) -> Option<PlotContent>,
    fonts: &Fonts,
) -> (Vec<Item>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut items = Vec::new();
    let bbox = board_box(board);
    let eff = it.effective_style(&doc.styles);
    let style = TableStyle::from_style(&eff);
    let recolor = crate::graphic::Recolor::from_style(&eff);
    {
        match it {
            DocItem::View(v) => {
                let (vi, w) = render_view(v, board, bbox, doc, plot_for, fonts);
                items.extend(vi);
                warnings.extend(w);
            }
            DocItem::Table(t) => {
                let data = match t.kind {
                    TableKind::Drill => table::drill_table(board),
                    TableKind::Stackup => table::stackup_table(board),
                    TableKind::Bom => {
                        let bom = bom_for(board, ctx);
                        let numbers = kdraft_board::bom::number_lines(
                            &bom.lines,
                            &kdraft_doc::bom_numbers(doc, bom.source.tag()),
                        );
                        let data = table::bom_table(&bom, &numbers, t.rows);
                        let keys: Vec<String> = if t.columns.is_empty() {
                            table::bom_default_columns(&bom)
                        } else {
                            t.columns.clone()
                        };
                        let (mut v, _) = table::layout(&data.select(&keys), t.at, style, fonts);
                        recolor.apply(&mut v);
                        items.extend(v);
                        return (items, warnings);
                    }
                    _ => {
                        warnings.push(format!(
                            "table {} of kind {} is not rendered (not implemented)",
                            t.id,
                            t.kind.as_str()
                        ));
                        return (items, warnings);
                    }
                };
                let (mut v, _) = table::layout(&data.select(&t.columns), t.at, style, fonts);
                recolor.apply(&mut v);
                items.extend(v);
            }
            DocItem::Graphic(g) => {
                if let GraphicShape::Image { .. } = g.shape {
                    warnings.push(format!("graphic {} (image) is not rendered", g.id));
                } else {
                    let vars = crate::notes::note_vars(board, ctx);
                    items.extend(crate::graphic::render(g, &eff, Some(&vars), fonts));
                }
            }
            DocItem::Dimension(d) => {
                let (mut v, w) = crate::dims::render(doc, d, board, bbox, fonts);
                recolor.apply(&mut v);
                items.extend(v);
                warnings.extend(w);
            }
            DocItem::NoteList(n) => {
                let (mut v, _, w) = crate::notes::render_notes_styled(n, board, ctx, &eff, fonts);
                recolor.apply(&mut v);
                items.extend(v);
                warnings.extend(w);
            }
            DocItem::Callout(c) => {
                let bom = bom_for(board, ctx);
                let numbers = kdraft_board::bom::number_lines(
                    &bom.lines,
                    &kdraft_doc::bom_numbers(doc, bom.source.tag()),
                );
                let lookup = |key: &str| {
                    bom.lines
                        .iter()
                        .position(|l| l.key() == key)
                        .and_then(|i| numbers.get(i).copied())
                };
                let (mut v, w) = crate::notes::render_callout(doc, c, board, bbox, &lookup, fonts);
                recolor.apply(&mut v);
                items.extend(v);
                warnings.extend(w);
            }
            other => warnings.push(format!(
                "{} {} is not rendered (not implemented)",
                other.kind_name(),
                other.id().map_or("-".to_string(), |i| i.to_string())
            )),
        }
    }
    (items, warnings)
}

fn text(s: &str, at: Point, size: f64, h: HAlign, v: VAlign) -> Item {
    Item::Text(TextRun {
        text: s.to_string(),
        at,
        size,
        h_align: h,
        v_align: v,
        rotation: 0.0,
        bold: false,
        italic: false,
        color: Color::BLACK,
        invisible: false,
    })
}

/// A 3D view: the rendered picture fitted into the view frame (keeping
/// its aspect), on white if asked; a placeholder box while there is none.
fn render_3d_view(
    v: &View,
    board_box: BBox,
    doc: &Document,
    plot_for: &dyn Fn(&View) -> Option<PlotContent>,
    _fonts: &Fonts,
) -> (Vec<Item>, Vec<String>) {
    // The board box at the view's scale (no frame margin): the outline
    // box of a picture to scale lands exactly on it.
    let f = kdraft_doc::ops::view_bounds(v, board_box);
    let opts = v.render3d();
    let state = plot_for(v).map(|p| p.image);
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    let pic = match &state {
        Some(Some(crate::render3d::State::Ready(p))) => crate::dl::ImageItem::from_png(
            std::sync::Arc::clone(&p.png),
            crate::render3d::place(p, &f),
        ),
        _ => None,
    };
    let mut label_y = f.max.y;
    match pic {
        Some(img) => {
            if opts.background == "white" {
                out.push(Item::filled(Path::rect(&img.rect), Color::WHITE));
            }
            label_y = img.rect.max.y;
            out.push(Item::Image(img));
        }
        None => {
            out.push(Item::stroked(
                Path::rect(&f),
                Stroke {
                    dash: Some(vec![1.5, 1.0]),
                    ..Stroke::solid(0.25, Color::gray(150))
                },
            ));
            let msg = match &state {
                Some(Some(crate::render3d::State::Pending(m))) => m.clone(),
                Some(Some(crate::render3d::State::Failed(e))) => {
                    warnings.push(format!("view {}: 3D render failed: {e}", v.id));
                    "3D render failed".to_string()
                }
                Some(_) => PENDING_3D.to_string(),
                None => {
                    warnings.push(format!("view {}: {NO_KICAD_3D}", v.id));
                    NO_KICAD_3D.to_string()
                }
            };
            out.push(text(
                &msg,
                f.center(),
                (f.height() / 12.0).clamp(1.5, 4.0),
                HAlign::Center,
                VAlign::Center,
            ));
        }
    }
    if v.frame {
        out.push(Item::stroked(
            Path::rect(&f),
            Stroke::solid(0.1, Color::gray(150)),
        ));
    }
    let what = if opts.to_scale() {
        format!(
            "3D VIEW ({})   {}",
            opts.side.to_uppercase(),
            scale_label(v.scale)
        )
    } else {
        "3D VIEW (NOT TO SCALE)".to_string()
    };
    out.push(text(
        &what,
        Point::new(f.center().x, label_y + 1.5),
        doc.styles.text_height,
        HAlign::Center,
        VAlign::Top,
    ));
    (out, warnings)
}

fn native_content(kind: &ViewKind) -> Content {
    match kind {
        ViewKind::Fabrication => Content::FABRICATION,
        ViewKind::Assembly => Content::ASSEMBLY,
        ViewKind::Drill => Content::DRILL,
        _ => Content::LAYER,
    }
}

fn render_view(
    v: &View,
    board: &Board,
    bbox: BBox,
    doc: &Document,
    plot_for: &dyn Fn(&View) -> Option<PlotContent>,
    fonts: &Fonts,
) -> (Vec<Item>, Vec<String>) {
    let mut warnings = Vec::new();
    let map = ViewMap::new(v, bbox);
    if v.kind == ViewKind::Render3d {
        return render_3d_view(v, bbox, doc, plot_for, fonts);
    }
    let side = match v.side {
        ViewSide::Front => Side::Front,
        ViewSide::Back => Side::Back,
    };
    let plot = if v.wants_plot() { plot_for(v) } else { None };
    let mut content = Vec::new();
    let native = plot.is_none();
    // Native because no plot was available although one was wanted.
    let fell_back = native && v.wants_plot();
    let mut extent = None;
    match plot {
        Some(pc) => {
            if let Some(m) = import::plot_to_sheet(&pc.fit, &map) {
                if v.plain_black(&doc.styles) {
                    for d in &pc.layers {
                        content.extend(import::import(d, &m));
                    }
                } else {
                    // Layers in list order, each in its colour; drill
                    // knock-outs once on top, as in a multi-layer plot.
                    let mut knock = Vec::new();
                    for (d, name) in pc.layers.iter().zip(&v.layers) {
                        let (c, o) = v.layer_color(&doc.styles, name);
                        let (ink, k) = import::recolor(
                            import::import(d, &m),
                            crate::graphic::color(c, 1.0),
                            o,
                        );
                        content.extend(ink);
                        knock.extend(k);
                    }
                    content.extend(knock);
                }
            }
        }
        None if v.kind == ViewKind::Assembly => {
            let pal = native::Palette::for_view(v, &doc.styles);
            let l = assembly::render_with(board, &map, side, &v.assembly, fonts, &pal);
            let s = &l.stats;
            if s.unplaced > 0 {
                warnings.push(format!(
                    "view {}: {} designator(s) unplaced: {}",
                    v.id,
                    s.unplaced,
                    l.unplaced().join(", ")
                ));
            }
            if l.values_omitted > 0 {
                warnings.push(format!(
                    "view {}: {} value(s) did not fit inside their part and are omitted",
                    v.id, l.values_omitted
                ));
            }
            extent = l.bbox();
            content = l.items;
        }
        None => {
            let pal = native::Palette::for_view(v, &doc.styles);
            content = native::render_with(board, &map, side, native_content(&v.kind), fonts, &pal)
        }
    }
    // Plotted content (fab texts, courtyards) and outside designators may
    // extend past the outline: clip to the frame grown to the content plus
    // a small margin.
    let grown = |b: BBox| {
        map.frame.union(&BBox {
            min: Point::new(b.min.x - PLOT_CLIP_MARGIN, b.min.y - PLOT_CLIP_MARGIN),
            max: Point::new(b.max.x + PLOT_CLIP_MARGIN, b.max.y + PLOT_CLIP_MARGIN),
        })
    };
    let clip = if let Some(e) = extent {
        grown(e)
    } else if native {
        map.frame
    } else {
        crate::dl::paths_bbox(&content).map_or(map.frame, grown)
    };
    let mut out = vec![Item::Group(Group {
        transform: Affine::IDENTITY,
        clip: Some(clip),
        items: content,
    })];
    let thin = Stroke::solid(0.1, Color::gray(150));
    if v.frame || fell_back {
        out.push(Item::stroked(Path::rect(&map.frame), thin));
    }
    if fell_back {
        out.push(Item::Text(TextRun {
            text: NATIVE_NOTE.to_string(),
            // Above the frame, clear of the view content.
            at: Point::new(map.frame.min.x, map.frame.min.y - 0.6),
            size: 1.2,
            h_align: HAlign::Left,
            v_align: VAlign::Baseline,
            rotation: 0.0,
            bold: false,
            italic: false,
            color: Color::gray(110),
            invisible: false,
        }));
    }
    let name = match (&v.kind, v.side) {
        (ViewKind::Assembly, ViewSide::Front) => "ASSEMBLY TOP".to_string(),
        (ViewKind::Assembly, ViewSide::Back) => "ASSEMBLY BOTTOM (VIEWED FROM BOTTOM)".to_string(),
        (ViewKind::Drill, _) => "DRILL DRAWING".to_string(),
        (ViewKind::Fabrication, _) => "FABRICATION".to_string(),
        (k, ViewSide::Back) => format!("{} (BOTTOM)", k.as_str().to_uppercase()),
        (k, ViewSide::Front) => k.as_str().to_uppercase(),
    };
    let th = doc.styles.text_height;
    out.push(text(
        &format!("{name}   {}", scale_label(v.scale)),
        Point::new(map.frame.center().x, clip.max.y + 1.5),
        th,
        HAlign::Center,
        VAlign::Top,
    ));
    (out, warnings)
}
