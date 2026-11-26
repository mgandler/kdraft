//! The built-in commands.

use std::path::PathBuf;

use kdraft_board::geom::{BBox, Point};
use kdraft_doc::ops::{self, Align, Distribute};
use kdraft_doc::{
    DnpStyle, DrillMarks, Graphic, GraphicShape, Id, Item, Page, PageSize, RefFilter, RenderMode,
    Table, TableKind, View, ViewKind, ViewSide,
};
use serde::Deserialize;

use crate::error::{EngineError, R};
use crate::registry::{opt, req, ArgType, CommandKind, CommandSpec, Outcome, Registry};
use crate::session::{Selection, Session};

use ArgType as T;

const ALIGN: &[&str] = &["left", "hcenter", "right", "top", "vcenter", "bottom"];
const VIEW_KINDS: &[&str] = &[
    "fabrication",
    "assembly",
    "drill",
    "detail",
    "layer",
    "render3d",
];
const TABLE_KINDS: &[&str] = &["drill", "stackup", "bom", "custom"];
const SIDES: &[&str] = &["front", "back"];
const MARKS: &[&str] = &["none", "small", "actual", "default"];
const RENDER_MODES: &[&str] = &["plot", "native", "default"];
const DNP_STYLES: &[&str] = &["none", "crossed", "greyed", "hidden"];
const REF_FILTERS: &[&str] = &["placed", "pos_files", "all"];

/// Default offset of duplicated items, mm.
pub const DUPLICATE_OFFSET: f64 = 5.0;

pub(crate) fn spec(
    name: &'static str,
    title: &'static str,
    kind: CommandKind,
    description: &'static str,
    args: Vec<crate::registry::ArgSpec>,
) -> CommandSpec {
    CommandSpec {
        name,
        title,
        description,
        kind,
        args,
        category: crate::meta::Category::Edit,
        icon: None,
        shortcut: None,
        menu: false,
        variants: &[],
    }
}

use CommandKind::{Edit, File, Selection as Sel};

pub(crate) fn register(r: &mut Registry) {
    crate::commands_style::register(r);
    crate::commands_geom::register(r);
    crate::commands_link::register(r);
    register_dimensions(r);
    register_notes(r);
    r.add(
        spec(
            "document.check",
            "Check drawing",
            Sel,
            "Runs the drawing checks (broken anchors, stale board, items outside the drawing area or overlapping, table overflow, small text, unplaced designators, unknown variables, BOM callouts); one message per finding.",
            vec![],
        ),
        document_check,
    );
    // Files and document settings.
    r.add(
        spec("document.new", "New document", File, "Creates a default drawing document for a board (three A3 pages) and makes it the session's document; not written until saved.",
            vec![req("board", T::String, "board file (.kicad_pcb)"), opt("output", T::String, "where the document will be saved; the board path is stored relative to it")]),
        document_new,
    );
    r.add(
        spec(
            "document.open",
            "Open document",
            File,
            "Opens a .kdraft file and its board; clears undo history.",
            vec![req("path", T::String, ".kdraft file")],
        ),
        document_open,
    );
    r.add(
        spec(
            "document.save",
            "Save document",
            File,
            "Writes the document to its file, or to `path` (which becomes its file).",
            vec![opt("path", T::String, "target .kdraft file")],
        ),
        document_save,
    );
    r.add(
        spec(
            "document.set_snap_grid",
            "Set snap grid",
            Edit,
            "Sets the snap grid used for nudging and snapping, mm.",
            vec![req("grid", T::Number, "grid step in mm, > 0")],
        ),
        document_set_snap_grid,
    );
    r.add(
        spec("document.set_sheet", "Set document sheet", Edit, "Sets the document-wide drawing sheet (.kicad_wks path relative to the document, or `default`); null clears it.",
            vec![opt("sheet", T::String, "sheet path or `default`; omit or null to clear")]),
        document_set_sheet,
    );

    // Pages.
    r.add(
        spec(
            "page.add",
            "Add page",
            Edit,
            "Adds an empty page and selects it.",
            vec![
                opt("title", T::String, "page title"),
                opt("paper", T::String, "paper name (A4, A3, ...; default A3)"),
                opt(
                    "portrait",
                    T::Bool,
                    "portrait orientation (default landscape)",
                ),
                opt(
                    "index",
                    T::Integer,
                    "insert position (default: after the current page)",
                ),
            ],
        ),
        page_add,
    );
    r.add(
        spec(
            "page.remove",
            "Remove page",
            Edit,
            "Removes a page (default: the current page).",
            vec![opt("page", T::Id, "page id")],
        ),
        page_remove,
    );
    r.add(
        spec(
            "page.duplicate",
            "Duplicate page",
            Edit,
            "Copies a page with new ids right after it and selects the copy.",
            vec![opt("page", T::Id, "page id (default: current)")],
        ),
        page_duplicate,
    );
    r.add(
        spec(
            "page.move",
            "Move page",
            Edit,
            "Moves the page at index `from` to index `to` (0-based).",
            vec![
                req("from", T::Integer, "current index"),
                req("to", T::Integer, "new index"),
            ],
        ),
        page_move,
    );
    r.add(
        spec(
            "page.rename",
            "Rename page",
            Edit,
            "Sets a page's title.",
            vec![
                opt("page", T::Id, "page id (default: current)"),
                req("title", T::String, "new title"),
            ],
        ),
        page_rename,
    );
    r.add(
        spec(
            "page.set_sheet",
            "Set page sheet",
            Edit,
            "Sets a page's drawing sheet override; null clears it.",
            vec![
                opt("page", T::Id, "page id (default: current)"),
                opt("sheet", T::String, "sheet path or `default`; omit to clear"),
            ],
        ),
        page_set_sheet,
    );
    r.add(
        spec("page.set_paper", "Set paper size", Edit, "Sets a page's paper: a named size (A4, A3, ...) in landscape or portrait, or `User` with width and height.",
            vec![opt("page", T::Id, "page id (default: current)"), req("paper", T::String, "paper name or User"),
                 opt("portrait", T::Bool, "portrait (named sizes)"), opt("width", T::Number, "width mm (User)"), opt("height", T::Number, "height mm (User)")]),
        page_set_paper,
    );
    r.add(
        spec(
            "page.select",
            "Select page",
            Sel,
            "Makes a page current and clears the item selection.",
            vec![req("page", T::Id, "page id")],
        ),
        page_select,
    );

    // Adding items.
    r.add(
        spec(
            "item.add_view",
            "Add view",
            Edit,
            "Adds a board view and selects it.",
            vec![
                opt("page", T::Id, "page id (default: current)"),
                req("kind", T::Enum { values: VIEW_KINDS }, "view kind"),
                opt(
                    "side",
                    T::Enum { values: SIDES },
                    "board side (default front)",
                ),
                opt("layers", T::StringList, "layers (default by kind and side)"),
                opt(
                    "scale",
                    T::Number,
                    "scale (default: largest standard scale that fits the drawing area)",
                ),
                opt(
                    "at",
                    T::Point,
                    "centre on the sheet (default: drawing-area centre)",
                ),
                opt(
                    "drill_marks",
                    T::Enum { values: MARKS },
                    "drill marks (default by kind)",
                ),
            ],
        ),
        item_add_view,
    );
    r.add(
        spec(
            "item.add_table",
            "Add table",
            Edit,
            "Adds a table and selects it.",
            vec![
                opt("page", T::Id, "page id (default: current)"),
                req(
                    "kind",
                    T::Enum {
                        values: TABLE_KINDS,
                    },
                    "table kind",
                ),
                opt(
                    "at",
                    T::Point,
                    "top-left corner (default: drawing-area top-left)",
                ),
            ],
        ),
        item_add_table,
    );
    r.add(
        spec(
            "item.add_text",
            "Add text",
            Edit,
            "Adds a text and selects it.",
            vec![
                opt("page", T::Id, "page id (default: current)"),
                req("text", T::String, "text"),
                req("at", T::Point, "baseline start"),
                opt(
                    "height",
                    T::Number,
                    "text height mm (default: document style)",
                ),
            ],
        ),
        item_add_text,
    );
    r.add(
        spec(
            "item.add_line",
            "Add line",
            Edit,
            "Adds a line and selects it.",
            vec![
                opt("page", T::Id, "page id (default: current)"),
                req("start", T::Point, "start"),
                req("end", T::Point, "end"),
            ],
        ),
        item_add_line,
    );
    r.add(
        spec(
            "item.add_rect",
            "Add rectangle",
            Edit,
            "Adds a rectangle and selects it.",
            vec![
                opt("page", T::Id, "page id (default: current)"),
                req("start", T::Point, "corner"),
                req("end", T::Point, "opposite corner"),
            ],
        ),
        item_add_rect,
    );

    // Editing items. `ids` defaults to the selection.
    let ids = || opt("ids", T::IdList, "item ids (default: the selection)");
    r.add(
        spec("item.remove", "Delete", Edit, "Removes items.", vec![ids()]),
        item_remove,
    );
    r.add(
        spec(
            "item.duplicate",
            "Duplicate",
            Edit,
            "Copies items with new ids, offset, and selects the copies.",
            vec![ids(), opt("offset", T::Point, "offset mm (default [5, 5])")],
        ),
        item_duplicate,
    );
    r.add(
        spec(
            "item.move",
            "Move",
            Edit,
            "Moves items by a delta.",
            vec![ids(), req("delta", T::Point, "delta mm")],
        ),
        item_move,
    );
    r.add(
        spec("item.set_position", "Set position", Edit, "Moves an item so its reference point (view centre, table top-left, graphic first point) is at `at`.",
            vec![opt("id", T::Id, "item id (default: first selected)"), req("at", T::Point, "position mm")]),
        item_set_position,
    );
    r.add(
        spec(
            "item.set_scale",
            "Set scale",
            Edit,
            "Sets the scale of views.",
            vec![ids(), req("scale", T::Number, "sheet mm per board mm, > 0")],
        ),
        item_set_scale,
    );
    r.add(
        spec(
            "item.set_rotation",
            "Set rotation",
            Edit,
            "Sets the rotation of views, degrees counter-clockwise.",
            vec![ids(), req("rotation", T::Number, "degrees")],
        ),
        item_set_rotation,
    );
    r.add(
        spec(
            "item.set_side",
            "Set side",
            Edit,
            "Sets the board side of views.",
            vec![ids(), req("side", T::Enum { values: SIDES }, "side")],
        ),
        item_set_side,
    );
    r.add(
        spec(
            "item.set_layers",
            "Set layers",
            Edit,
            "Sets the layer list of views.",
            vec![ids(), req("layers", T::StringList, "layer names")],
        ),
        item_set_layers,
    );
    r.add(
        spec(
            "item.set_kind",
            "Set kind",
            Edit,
            "Sets the kind of views (fabrication, assembly, ...) or tables (drill, stackup, ...).",
            vec![ids(), req("kind", T::String, "view or table kind")],
        ),
        item_set_kind,
    );
    r.add(
        spec(
            "item.set_render",
            "Set render mode",
            Edit,
            "Draws views from kicad-cli plots or natively from the board model (`default`: native for assembly and drill views, plot otherwise).",
            vec![
                ids(),
                req("mode", T::Enum { values: RENDER_MODES }, "render mode"),
            ],
        ),
        item_set_render,
    );
    r.add(
        spec(
            "item.set_assembly",
            "Set assembly options",
            Edit,
            "Native assembly view settings; omitted fields stay, null resets a field to its default.",
            vec![
                ids(),
                opt("values", T::Bool, "show values: native inside parts, plot views the fab-layer Value texts (default off)"),
                opt("references", T::Bool, "show reference designators: native designators, plot views the fab-layer Reference and ${REFERENCE} texts (default on)"),
                opt("fab_other", T::Bool, "plot views: other footprint texts on fab layers (default on)"),
                opt("pads", T::Bool, "pad outlines (default on)"),
                opt("holes", T::Bool, "hole outlines (default off)"),
                opt("pin1", T::Bool, "pin-1 marks on polarised parts (default on)"),
                opt("dnp", T::Enum { values: DNP_STYLES }, "DNP parts: none, crossed (default), greyed, hidden"),
                opt("refs", T::Enum { values: REF_FILTERS }, "which parts get designators: placed (default: no board-only parts or parts without copper pads), pos_files (also not excluded from position files), all"),
                opt("text_min", T::Number, "minimum designator cap height on paper, mm (default 0.9)"),
                opt("text_max", T::Number, "maximum designator cap height on paper, mm (default 2.5)"),
            ],
        ),
        item_set_assembly,
    );
    r.add(
        spec(
            "item.set_drill_marks",
            "Set drill marks",
            Edit,
            "Sets the drill marks of views' plotted layers (`default`: by view kind).",
            vec![
                ids(),
                req("marks", T::Enum { values: MARKS }, "drill marks"),
            ],
        ),
        item_set_drill_marks,
    );
    r.add(
        spec(
            "bom.renumber",
            "Renumber BOM",
            Edit,
            "Numbers the BOM items from scratch (1, 2, ... in table order) for the BOM in use (schematic once fetched, else from the PCB) and stores the numbers; callouts follow.",
            vec![],
        ),
        bom_renumber,
    );
    r.add(
        spec(
            "item.rename",
            "Rename item",
            Edit,
            "Sets the name shown in the item tree (empty: the default name).",
            vec![
                opt("id", T::Id, "item id (default: first selected)"),
                req("name", T::String, "new name"),
            ],
        ),
        item_rename,
    );
    r.add(
        spec(
            "item.set_hidden",
            "Show or hide",
            Edit,
            "Hidden items are not drawn on the sheet or in exports.",
            vec![ids(), req("hidden", T::Bool, "hidden")],
        ),
        item_set_hidden,
    );
    r.add(
        spec(
            "item.set_locked",
            "Lock or unlock",
            Edit,
            "Locked items cannot be picked or moved on the canvas (the item tree and properties still edit them).",
            vec![ids(), req("locked", T::Bool, "locked")],
        ),
        item_set_locked,
    );
    r.add(
        spec(
            "item.reorder",
            "Change stacking order",
            Edit,
            "Moves an item to position `to` in its page's z-order (0 is the bottom).",
            vec![
                req("id", T::Id, "item id"),
                req("to", T::Integer, "new index, 0 = bottom"),
            ],
        ),
        item_reorder,
    );
    r.add(
        spec(
            "item.set_frame",
            "Set view frame",
            Edit,
            "Shows or hides the frame of views.",
            vec![ids(), req("frame", T::Bool, "frame on")],
        ),
        item_set_frame,
    );
    r.add(
        spec("item.set_geometry", "Edit geometry", Edit, "Changes a graphic: line/rect start and end, circle centre and radius, text position, content and height.",
            vec![opt("id", T::Id, "item id (default: first selected)"), opt("start", T::Point, "start"), opt("end", T::Point, "end"),
                 opt("center", T::Point, "circle centre"), opt("radius", T::Number, "circle radius, or rectangle corner radius (0: square)"), opt("at", T::Point, "text position"),
                 opt("text", T::String, "text"), opt("height", T::Number, "text height")]),
        item_set_geometry,
    );
    r.add(
        spec(
            "item.move_to_page",
            "Move to page",
            Edit,
            "Moves items to another page (same sheet position).",
            vec![ids(), req("page", T::Id, "target page")],
        ),
        item_move_to_page,
    );
    r.add(
        spec(
            "item.copy_to_page",
            "Copy to page",
            Edit,
            "Copies items with new ids to another page (same sheet position); with linked, each copy's placement is linked to its original (\"paste in place, linked\").",
            vec![
                ids(),
                req("page", T::Id, "target page"),
                opt("linked", T::Bool, "link each copy's placement to its original"),
            ],
        ),
        item_copy_to_page,
    );
    r.add(
        spec(
            "item.raise",
            "Bring forward",
            Edit,
            "Moves items one step up the z-order, or to the top.",
            vec![ids(), opt("to_top", T::Bool, "to the top")],
        ),
        item_raise,
    );
    r.add(
        spec(
            "item.lower",
            "Send backward",
            Edit,
            "Moves items one step down the z-order, or to the bottom.",
            vec![ids(), opt("to_bottom", T::Bool, "to the bottom")],
        ),
        item_lower,
    );

    // Selection.
    r.add(
        spec(
            "select.set",
            "Select",
            Sel,
            "Replaces the selection (items on one page, in this order).",
            vec![
                opt(
                    "page",
                    T::Id,
                    "page (default: the page of the first item, else current)",
                ),
                req("ids", T::IdList, "item ids"),
            ],
        ),
        select_set,
    );
    r.add(
        spec(
            "select.add",
            "Add to selection",
            Sel,
            "Adds items of the current page to the selection.",
            vec![req("ids", T::IdList, "item ids")],
        ),
        select_add,
    );
    r.add(
        spec(
            "select.toggle",
            "Toggle selection",
            Sel,
            "Adds an item to or removes it from the selection.",
            vec![req("id", T::Id, "item id")],
        ),
        select_toggle,
    );
    r.add(
        spec(
            "select.clear",
            "Clear selection",
            Sel,
            "Deselects all items.",
            vec![],
        ),
        select_clear,
    );
    r.add(
        spec(
            "select.all",
            "Select all",
            Sel,
            "Selects all items with an id on a page.",
            vec![opt("page", T::Id, "page (default: current)")],
        ),
        select_all,
    );

    // Layout.
    r.add(
        spec("layout.align", "Align", Edit, "Aligns items: to the first selected item, or with one item to the page's drawing area.",
            vec![ids(), req("mode", T::Enum { values: ALIGN }, "edge or centre")]),
        layout_align,
    );
    r.add(
        spec(
            "layout.distribute",
            "Distribute",
            Edit,
            "Spaces three or more items evenly; the outer two stay in place.",
            vec![
                ids(),
                req(
                    "direction",
                    T::Enum {
                        values: &["horizontal", "vertical"],
                    },
                    "direction",
                ),
                opt(
                    "spacing",
                    T::Enum {
                        values: &["gaps", "centres"],
                    },
                    "equal gaps (default) or equal centre distances",
                ),
            ],
        ),
        layout_distribute,
    );
    r.add(
        spec("layout.match", "Match size", Edit, "Makes items match the first selected item: width or height (views by scale, rectangles, lines and circles by geometry) or view scale.",
            vec![ids(), req("property", T::Enum { values: &["width", "height", "scale"] }, "what to match")]),
        layout_match,
    );
    r.add(
        spec(
            "layout.center",
            "Centre",
            Edit,
            "Centres the items' combined box on the drawing area or the page.",
            vec![
                ids(),
                opt(
                    "target",
                    T::Enum {
                        values: &["area", "page"],
                    },
                    "drawing area (default) or page",
                ),
                opt(
                    "axis",
                    T::Enum {
                        values: &["both", "horizontal", "vertical"],
                    },
                    "axis (default both)",
                ),
            ],
        ),
        layout_center,
    );
    r.add(
        spec(
            "layout.nudge",
            "Nudge",
            Edit,
            "Moves items by whole snap-grid steps.",
            vec![
                ids(),
                req("dx", T::Number, "steps in x"),
                req("dy", T::Number, "steps in y"),
            ],
        ),
        layout_nudge,
    );
}

// ---------------------------------------------------------------- helpers

pub(crate) type P2 = [f64; 2];

pub(crate) fn p(v: P2) -> R<Point> {
    if v[0].is_finite() && v[1].is_finite() {
        Ok(Point::new(v[0], v[1]))
    } else {
        Err(EngineError::invalid("coordinates must be finite"))
    }
}

pub(crate) fn positive(v: f64, what: &str) -> R<f64> {
    if v.is_finite() && v > 0.0 {
        Ok(v)
    } else {
        Err(EngineError::invalid(format!(
            "{what} must be a positive number, got {v}"
        )))
    }
}

pub(crate) fn page_arg(s: &Session, page: Option<&str>) -> R<usize> {
    match page {
        Some(id) => s
            .page_index(&Id(id.to_string()))
            .ok_or_else(|| kdraft_doc::OpError::NoSuchPage(id.to_string()).into()),
        None => s
            .current_page_index()
            .ok_or_else(|| EngineError::invalid("the document has no pages")),
    }
}

/// Items to act on: given ids (all on the page of the first) or the
/// selection. Returns the page index and ids.
pub(crate) fn ids_arg(s: &Session, ids: Option<Vec<String>>) -> R<(usize, Vec<Id>)> {
    let ids: Vec<Id> = match ids {
        Some(v) => v.into_iter().map(Id).collect(),
        None => s.selection.items.clone(),
    };
    let first = ids
        .first()
        .ok_or_else(|| EngineError::invalid("nothing selected"))?;
    let (pi, _) = s
        .find_item(first)
        .ok_or_else(|| kdraft_doc::OpError::NoSuchItem(first.to_string()))?;
    for id in &ids {
        if s.doc().pages[pi].item(id).is_none() {
            return Err(EngineError::invalid(format!(
                "item {id} is not on the page of {first}"
            )));
        }
    }
    Ok((pi, ids))
}

pub(crate) fn one_id(s: &Session, id: Option<String>) -> R<(usize, Id)> {
    let (pi, ids) = ids_arg(s, id.map(|i| vec![i]))?;
    Ok((pi, ids[0].clone()))
}

pub(crate) fn select_items(s: &mut Session, pi: usize, ids: Vec<Id>) {
    s.selection = Selection {
        page: Some(s.doc().pages[pi].id.clone()),
        items: ids,
    };
}

pub(crate) fn item_mut<'a>(s: &'a mut Session, pi: usize, id: &Id) -> R<&'a mut Item> {
    s.doc_mut().pages[pi]
        .items
        .iter_mut()
        .find(|i| i.id() == Some(id))
        .ok_or_else(|| kdraft_doc::OpError::NoSuchItem(id.to_string()).into())
}

/// Applies `f` to every selected view; error if none is a view.
fn each_view(
    s: &mut Session,
    ids: Option<Vec<String>>,
    f: &dyn Fn(&mut View) -> R<()>,
) -> R<Outcome> {
    let (pi, ids) = ids_arg(s, ids)?;
    let mut changed = Vec::new();
    let mut skipped = 0;
    for id in &ids {
        match item_mut(s, pi, id)? {
            Item::View(v) => {
                f(v)?;
                changed.push(id.clone());
            }
            _ => skipped += 1,
        }
    }
    if changed.is_empty() {
        return Err(EngineError::invalid("no view among the items"));
    }
    let mut o = Outcome::changed(&changed);
    if skipped > 0 {
        o = o.msg(format!("{skipped} non-view item(s) left unchanged"));
    }
    Ok(o)
}

fn parse_side(s: &str) -> R<ViewSide> {
    match s {
        "front" => Ok(ViewSide::Front),
        "back" => Ok(ViewSide::Back),
        o => Err(EngineError::invalid(format!(
            "side must be front or back, got {o:?}"
        ))),
    }
}

fn parse_marks(s: &str) -> R<Option<DrillMarks>> {
    Ok(match s {
        "default" => None,
        "none" | "small" | "actual" => Some(DrillMarks::parse(s)),
        o => {
            return Err(EngineError::invalid(format!(
                "drill marks must be one of {MARKS:?}, got {o:?}"
            )))
        }
    })
}

/// Paper size in mm for a name and orientation.
pub fn paper(name: &str, portrait: bool, w: Option<f64>, h: Option<f64>) -> R<PageSize> {
    if name == "User" {
        let (w, h) = (
            positive(w.unwrap_or(f64::NAN), "width")?,
            positive(h.unwrap_or(f64::NAN), "height")?,
        );
        return Ok(PageSize {
            name: "User".into(),
            width: w,
            height: h,
        });
    }
    let (a, b) = kdraft_sheet::paper_size(name)
        .ok_or_else(|| EngineError::invalid(format!("unknown paper size {name:?}")))?;
    let (width, height) = if portrait { (a, b) } else { (b, a) };
    Ok(PageSize {
        name: name.to_string(),
        width,
        height,
    })
}

/// Default layers for a new view.
pub fn default_layers(kind: &ViewKind, side: ViewSide) -> Vec<String> {
    let f = side == ViewSide::Front;
    let l: &[&str] = match kind {
        ViewKind::Fabrication => &["Edge.Cuts", "Dwgs.User"],
        ViewKind::Assembly if f => &["F.Fab", "F.CrtYd", "Edge.Cuts"],
        ViewKind::Assembly => &["B.Fab", "B.CrtYd", "Edge.Cuts"],
        ViewKind::Drill => &["Edge.Cuts"],
        ViewKind::Render3d => &[],
        _ if f => &["F.Cu", "Edge.Cuts"],
        _ => &["B.Cu", "Edge.Cuts"],
    };
    l.iter().map(|s| s.to_string()).collect()
}

pub(crate) fn add_item(s: &mut Session, pi: usize, item: Item) -> R<Outcome> {
    let id = item
        .id()
        .cloned()
        .ok_or_else(|| EngineError::invalid("item has no id"))?;
    s.doc_mut().pages[pi].items.push(item);
    select_items(s, pi, vec![id.clone()]);
    Ok(Outcome::created([id]))
}

/// Boxes of items, in input order.
fn boxes(s: &Session, pi: usize, ids: &[Id]) -> R<Vec<BBox>> {
    ids.iter()
        .map(|id| {
            let it = s.doc().pages[pi]
                .item(id)
                .ok_or_else(|| kdraft_doc::OpError::NoSuchItem(id.to_string()))?;
            s.item_bounds(it)
                .ok_or_else(|| kdraft_doc::OpError::NoBounds(id.to_string()).into())
        })
        .collect()
}

/// The ids without dimensions (they follow their view's anchors and
/// have no sheet position of their own); error if nothing is left.
fn movable(s: &Session, pi: usize, ids: Vec<Id>) -> R<Vec<Id>> {
    let page = &s.doc().pages[pi];
    let first = ids.first().cloned();
    let out: Vec<Id> = ids
        .into_iter()
        .filter(|id| !matches!(page.item(id), Some(Item::Dimension(_) | Item::Unknown(_))))
        .collect();
    if out.is_empty() {
        return Err(kdraft_doc::OpError::NotMovable(
            first.map(|i| i.to_string()).unwrap_or_default(),
        )
        .into());
    }
    Ok(out)
}

/// `ids` plus the dimensions and callouts anchored to views among them
/// (on any page), which go with their view when it is deleted, copied or
/// moved to another page.
pub(crate) fn with_dependents(doc: &kdraft_doc::Document, ids: &[Id]) -> Vec<Id> {
    let views: Vec<&Id> = ids
        .iter()
        .filter(|id| {
            doc.pages
                .iter()
                .any(|p| matches!(p.item(id), Some(Item::View(_))))
        })
        .collect();
    let mut out = ids.to_vec();
    for p in &doc.pages {
        for it in &p.items {
            let v = match it {
                Item::Dimension(d) => d.view.as_ref(),
                Item::Callout(c) => c.view.as_ref(),
                _ => None,
            };
            if let (Some(v), Some(id)) = (v, it.id()) {
                if views.contains(&v) && !out.contains(id) {
                    out.push(id.clone());
                }
            }
        }
    }
    out
}

fn translate_all(s: &mut Session, pi: usize, ids: &[Id], d: Point) -> R<Outcome> {
    if !(d.x.is_finite() && d.y.is_finite()) {
        return Err(EngineError::invalid("delta must be finite"));
    }
    let mut moved = Vec::new();
    let mut fixed = 0;
    for id in ids {
        if item_mut(s, pi, id)?.translate(d.x, d.y) {
            moved.push(id.clone());
        } else {
            fixed += 1;
        }
    }
    if moved.is_empty() {
        return Err(kdraft_doc::OpError::NotMovable(ids[0].to_string()).into());
    }
    let mut o = Outcome::changed(&moved);
    if fixed > 0 {
        o = o.msg(format!(
            "{fixed} item(s) without a sheet position not moved"
        ));
    }
    Ok(o)
}

// ---------------------------------------------------------------- documents

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewArgs {
    board: String,
    output: Option<String>,
}

fn document_new(s: &mut Session, a: NewArgs) -> R<Outcome> {
    let out = a.output.map(PathBuf::from);
    let n = Session::new_for_board(std::path::Path::new(&a.board), out.as_deref())?;
    let pages = n.doc().pages.len();
    s.replace_with(n);
    Ok(Outcome::default().msg(format!("new document with {pages} pages")))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathArgs {
    path: String,
}

fn document_open(s: &mut Session, a: PathArgs) -> R<Outcome> {
    let n = Session::open(std::path::Path::new(&a.path))?;
    let mut o = Outcome::default().msg(format!("opened {}", a.path));
    if let Some(e) = &n.board_state().error {
        o = o.msg(format!("board not loaded: {e}"));
    }
    s.replace_with(n);
    Ok(o)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveArgs {
    path: Option<String>,
}

fn document_save(s: &mut Session, a: SaveArgs) -> R<Outcome> {
    let p = s.save(a.path.as_deref().map(std::path::Path::new))?;
    Ok(Outcome::default().msg(format!("saved {}", p.display())))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GridArgs {
    grid: f64,
}

fn document_set_snap_grid(s: &mut Session, a: GridArgs) -> R<Outcome> {
    let g = positive(a.grid, "grid")?;
    s.doc_mut().styles.snap_grid = g;
    Ok(Outcome::default())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DocSheetArgs {
    sheet: Option<String>,
}

fn document_set_sheet(s: &mut Session, a: DocSheetArgs) -> R<Outcome> {
    s.doc_mut().sheet = a.sheet.filter(|x| !x.is_empty());
    Ok(Outcome::default())
}

// ---------------------------------------------------------------- pages

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageAddArgs {
    title: Option<String>,
    paper: Option<String>,
    portrait: Option<bool>,
    index: Option<usize>,
}

fn page_add(s: &mut Session, a: PageAddArgs) -> R<Outcome> {
    // Unset: the user's default paper.
    let size = paper(
        a.paper.as_deref().unwrap_or(&s.defaults.paper),
        a.portrait.unwrap_or(s.defaults.portrait),
        None,
        None,
    )?;
    let n = s.doc().pages.len();
    let index = a
        .index
        .unwrap_or_else(|| s.current_page_index().map_or(n, |i| i + 1))
        .min(n);
    // Named after where it goes, as the page list numbers it.
    let title = a.title.unwrap_or_else(|| format!("Page {}", index + 1));
    let page = Page::new(&title, size);
    let id = page.id.clone();
    let d = ops::add_page(s.doc(), page, Some(index))?;
    s.set_doc(d);
    s.selection = Selection {
        page: Some(id.clone()),
        items: vec![],
    };
    Ok(Outcome::created([id]))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageArg {
    page: Option<String>,
}

fn page_remove(s: &mut Session, a: PageArg) -> R<Outcome> {
    let pi = page_arg(s, a.page.as_deref())?;
    let id = s.doc().pages[pi].id.clone();
    let d = ops::remove_page(s.doc(), &id)?;
    s.set_doc(d);
    let n = s.doc().pages.len();
    s.selection = Selection {
        page: (n > 0).then(|| s.doc().pages[pi.min(n - 1)].id.clone()),
        items: vec![],
    };
    Ok(Outcome::changed([id]))
}

fn page_duplicate(s: &mut Session, a: PageArg) -> R<Outcome> {
    let pi = page_arg(s, a.page.as_deref())?;
    let id = s.doc().pages[pi].id.clone();
    let (d, new) = ops::duplicate_page(s.doc(), &id, &mut Id::new_v4)?;
    s.set_doc(d);
    s.selection = Selection {
        page: Some(new.clone()),
        items: vec![],
    };
    Ok(Outcome::created([new]))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveArgs {
    from: usize,
    to: usize,
}

fn page_move(s: &mut Session, a: MoveArgs) -> R<Outcome> {
    let d = ops::move_page(s.doc(), a.from, a.to)?;
    let id = d.pages[a.to].id.clone();
    s.set_doc(d);
    Ok(Outcome::changed([id]))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameArgs {
    page: Option<String>,
    title: String,
}

fn page_rename(s: &mut Session, a: RenameArgs) -> R<Outcome> {
    let pi = page_arg(s, a.page.as_deref())?;
    let p = &mut s.doc_mut().pages[pi];
    p.title = Some(a.title);
    Ok(Outcome::changed([p.id.clone()]))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageSheetArgs {
    page: Option<String>,
    sheet: Option<String>,
}

fn page_set_sheet(s: &mut Session, a: PageSheetArgs) -> R<Outcome> {
    let pi = page_arg(s, a.page.as_deref())?;
    let p = &mut s.doc_mut().pages[pi];
    p.sheet = a.sheet.filter(|x| !x.is_empty());
    Ok(Outcome::changed([p.id.clone()]))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PaperArgs {
    page: Option<String>,
    paper: String,
    portrait: Option<bool>,
    width: Option<f64>,
    height: Option<f64>,
}

fn page_set_paper(s: &mut Session, a: PaperArgs) -> R<Outcome> {
    let pi = page_arg(s, a.page.as_deref())?;
    let size = paper(&a.paper, a.portrait.unwrap_or(false), a.width, a.height)?;
    let p = &mut s.doc_mut().pages[pi];
    p.size = size;
    Ok(Outcome::changed([p.id.clone()]))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageReq {
    page: String,
}

fn page_select(s: &mut Session, a: PageReq) -> R<Outcome> {
    let pi = page_arg(s, Some(&a.page))?;
    select_items(s, pi, vec![]);
    Ok(Outcome::default())
}

// ---------------------------------------------------------------- adding

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddViewArgs {
    page: Option<String>,
    kind: String,
    side: Option<String>,
    layers: Option<Vec<String>>,
    scale: Option<f64>,
    at: Option<P2>,
    drill_marks: Option<String>,
}

fn item_add_view(s: &mut Session, a: AddViewArgs) -> R<Outcome> {
    let pi = page_arg(s, a.page.as_deref())?;
    let kind = ViewKind::parse(&a.kind);
    let side = parse_side(a.side.as_deref().unwrap_or("front"))?;
    let area = s.drawing_area(pi);
    let bb = s.board_box();
    let scale = match a.scale {
        Some(v) => positive(v, "scale")?,
        None => area
            .and_then(|r| kdraft_doc::choose_scale(bb.width(), bb.height(), r.width(), r.height()))
            .unwrap_or(kdraft_doc::STANDARD_SCALES[0]),
    };
    let at = match a.at {
        Some(v) => p(v)?,
        None => area.map_or(Point::new(100.0, 100.0), |r| r.center()),
    };
    let drill_marks = match a.drill_marks.as_deref() {
        Some(m) => parse_marks(m)?,
        None => Some(DrillMarks::default_for(&kind)),
    };
    let layers = a.layers.unwrap_or_else(|| default_layers(&kind, side));
    let v = View {
        id: Id::new_v4(),
        kind,
        side,
        scale,
        rotation: 0.0,
        at,
        layers,
        crop: None,
        parent: None,
        style: None,
        frame: false,
        drill_marks,
        render: None,
        assembly: Default::default(),
        extra: vec![],
    };
    let mut v = v;
    if v.kind == ViewKind::Render3d {
        // The user's default resolution and quality.
        let mut r = v.render3d();
        r.dpi = s.defaults.render_dpi;
        r.quality = s.defaults.render_quality.clone();
        v.set_render3d(&r);
    }
    add_item(s, pi, Item::View(v))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddTableArgs {
    page: Option<String>,
    kind: String,
    at: Option<P2>,
}

fn item_add_table(s: &mut Session, a: AddTableArgs) -> R<Outcome> {
    let pi = page_arg(s, a.page.as_deref())?;
    if !TABLE_KINDS.contains(&a.kind.as_str()) {
        return Err(EngineError::invalid(format!(
            "table kind must be one of {TABLE_KINDS:?}"
        )));
    }
    let at = match a.at {
        Some(v) => p(v)?,
        None => s.drawing_area(pi).map_or(Point::new(20.0, 20.0), |r| r.min),
    };
    let t = Table {
        id: Id::new_v4(),
        kind: TableKind::parse(&a.kind),
        at,
        columns: vec![],
        style: None,
        bom_items: Vec::new(),
        bom_source: None,
        rows: None,
        extra: vec![],
    };
    add_item(s, pi, Item::Table(t))
}

pub(crate) fn graphic(shape: GraphicShape) -> Item {
    Item::Graphic(Graphic {
        id: Id::new_v4(),
        shape,
        style: None,
        extra: vec![],
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddTextArgs {
    page: Option<String>,
    text: String,
    at: P2,
    height: Option<f64>,
}

fn item_add_text(s: &mut Session, a: AddTextArgs) -> R<Outcome> {
    let pi = page_arg(s, a.page.as_deref())?;
    let height = a.height.map(|h| positive(h, "height")).transpose()?;
    add_item(
        s,
        pi,
        graphic(GraphicShape::Text {
            at: p(a.at)?,
            text: a.text,
            height,
            rotation: 0.0,
            wrap: None,
        }),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TwoPointArgs {
    page: Option<String>,
    start: P2,
    end: P2,
}

fn item_add_line(s: &mut Session, a: TwoPointArgs) -> R<Outcome> {
    let pi = page_arg(s, a.page.as_deref())?;
    add_item(
        s,
        pi,
        graphic(GraphicShape::Line {
            start: p(a.start)?,
            end: p(a.end)?,
        }),
    )
}

fn item_add_rect(s: &mut Session, a: TwoPointArgs) -> R<Outcome> {
    let pi = page_arg(s, a.page.as_deref())?;
    add_item(
        s,
        pi,
        graphic(GraphicShape::Rect {
            start: p(a.start)?,
            end: p(a.end)?,
            radius: 0.0,
        }),
    )
}

// ---------------------------------------------------------------- editing

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IdsArgs {
    pub(crate) ids: Option<Vec<String>>,
}

fn item_remove(s: &mut Session, a: IdsArgs) -> R<Outcome> {
    let (_, ids) = ids_arg(s, a.ids)?;
    let ids = with_dependents(s.doc(), &ids);
    let d = ops::remove_items(s.doc(), &ids)?;
    s.set_doc(d);
    s.selection.items.retain(|i| !ids.contains(i));
    Ok(Outcome::changed(&ids))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DupArgs {
    ids: Option<Vec<String>>,
    offset: Option<P2>,
}

fn item_duplicate(s: &mut Session, a: DupArgs) -> R<Outcome> {
    let (pi, ids) = ids_arg(s, a.ids)?;
    let off = p(a.offset.unwrap_or([DUPLICATE_OFFSET, DUPLICATE_OFFSET]))?;
    let page = s.doc().pages[pi].id.clone();
    let all = with_dependents(s.doc(), &ids);
    let (d, new) = ops::copy_items(s.doc(), &all, &page, off, &mut Id::new_v4)?;
    s.set_doc(d);
    select_items(s, pi, new[..ids.len()].to_vec());
    Ok(Outcome::created(&new))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeltaArgs {
    ids: Option<Vec<String>>,
    delta: P2,
}

fn item_move(s: &mut Session, a: DeltaArgs) -> R<Outcome> {
    let (pi, ids) = ids_arg(s, a.ids)?;
    translate_all(s, pi, &ids, p(a.delta)?)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PosArgs {
    id: Option<String>,
    at: P2,
}

fn item_set_position(s: &mut Session, a: PosArgs) -> R<Outcome> {
    let (pi, id) = one_id(s, a.id)?;
    let target = p(a.at)?;
    let cur = s.doc().pages[pi]
        .item(&id)
        .and_then(Item::position)
        .ok_or_else(|| kdraft_doc::OpError::NotMovable(id.to_string()))?;
    translate_all(s, pi, &[id], target - cur)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScaleArgs {
    ids: Option<Vec<String>>,
    scale: f64,
}

fn item_set_scale(s: &mut Session, a: ScaleArgs) -> R<Outcome> {
    let v = positive(a.scale, "scale")?;
    each_view(s, a.ids, &|view| {
        view.scale = v;
        Ok(())
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RotArgs {
    ids: Option<Vec<String>>,
    rotation: f64,
}

fn item_set_rotation(s: &mut Session, a: RotArgs) -> R<Outcome> {
    if !a.rotation.is_finite() {
        return Err(EngineError::invalid("rotation must be finite"));
    }
    let r = kdraft_board::geom::normalize_deg(a.rotation);
    each_view(s, a.ids, &|view| {
        view.rotation = r;
        Ok(())
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SideArgs {
    ids: Option<Vec<String>>,
    side: String,
}

fn item_set_side(s: &mut Session, a: SideArgs) -> R<Outcome> {
    let side = parse_side(&a.side)?;
    each_view(s, a.ids, &|view| {
        view.side = side;
        Ok(())
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LayersArgs {
    ids: Option<Vec<String>>,
    layers: Vec<String>,
}

fn item_set_layers(s: &mut Session, a: LayersArgs) -> R<Outcome> {
    if a.layers.is_empty() {
        return Err(EngineError::invalid("a view needs at least one layer"));
    }
    let layers = a.layers;
    each_view(s, a.ids, &|view| {
        view.layers = layers.clone();
        Ok(())
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KindArgs {
    ids: Option<Vec<String>>,
    kind: String,
}

fn item_set_kind(s: &mut Session, a: KindArgs) -> R<Outcome> {
    let (pi, ids) = ids_arg(s, a.ids)?;
    let mut changed = Vec::new();
    for id in &ids {
        match item_mut(s, pi, id)? {
            Item::View(v) if VIEW_KINDS.contains(&a.kind.as_str()) => {
                v.kind = ViewKind::parse(&a.kind);
                changed.push(id.clone());
            }
            Item::Table(t) if TABLE_KINDS.contains(&a.kind.as_str()) => {
                t.kind = TableKind::parse(&a.kind);
                changed.push(id.clone());
            }
            _ => {}
        }
    }
    if changed.is_empty() {
        return Err(EngineError::invalid(format!(
            "kind {:?} applies to none of the items (views: {VIEW_KINDS:?}, tables: {TABLE_KINDS:?})",
            a.kind
        )));
    }
    Ok(Outcome::changed(&changed))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MarksArgs {
    ids: Option<Vec<String>>,
    marks: String,
}

fn item_set_drill_marks(s: &mut Session, a: MarksArgs) -> R<Outcome> {
    let m = parse_marks(&a.marks)?;
    each_view(s, a.ids, &|view| {
        view.drill_marks = m.clone();
        Ok(())
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenderArgs {
    ids: Option<Vec<String>>,
    mode: String,
}

fn item_set_render(s: &mut Session, a: RenderArgs) -> R<Outcome> {
    let m = match a.mode.as_str() {
        "default" => None,
        "plot" | "native" => Some(RenderMode::parse(&a.mode)),
        o => {
            return Err(EngineError::invalid(format!(
                "render mode must be one of {RENDER_MODES:?}, got {o:?}"
            )))
        }
    };
    each_view(s, a.ids, &|view| {
        view.render = m.clone();
        Ok(())
    })
}

/// A field that may be absent (keep), null (reset) or a value (set).
fn tri<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssemblyArgs {
    ids: Option<Vec<String>>,
    #[serde(default, deserialize_with = "tri")]
    values: Option<Option<bool>>,
    #[serde(default, deserialize_with = "tri")]
    references: Option<Option<bool>>,
    #[serde(default, deserialize_with = "tri")]
    fab_other: Option<Option<bool>>,
    #[serde(default, deserialize_with = "tri")]
    pads: Option<Option<bool>>,
    #[serde(default, deserialize_with = "tri")]
    holes: Option<Option<bool>>,
    #[serde(default, deserialize_with = "tri")]
    pin1: Option<Option<bool>>,
    #[serde(default, deserialize_with = "tri")]
    dnp: Option<Option<String>>,
    #[serde(default, deserialize_with = "tri")]
    refs: Option<Option<String>>,
    #[serde(default, deserialize_with = "tri")]
    text_min: Option<Option<f64>>,
    #[serde(default, deserialize_with = "tri")]
    text_max: Option<Option<f64>>,
}

fn item_set_assembly(s: &mut Session, a: AssemblyArgs) -> R<Outcome> {
    let check = |v: &Option<Option<String>>, allowed: &[&str], what: &str| -> R<()> {
        if let Some(Some(x)) = v {
            if !allowed.contains(&x.as_str()) {
                return Err(EngineError::invalid(format!(
                    "{what} must be one of {allowed:?}, got {x:?}"
                )));
            }
        }
        Ok(())
    };
    check(&a.dnp, DNP_STYLES, "dnp")?;
    check(&a.refs, REF_FILTERS, "refs")?;
    for h in [a.text_min, a.text_max].into_iter().flatten().flatten() {
        positive(h, "text height")?;
    }
    each_view(s, a.ids.clone(), &|view| {
        let o = &mut view.assembly;
        if let Some(v) = a.values {
            o.values = v;
        }
        if let Some(v) = a.references {
            o.references = v;
        }
        if let Some(v) = a.fab_other {
            o.fab_other = v;
        }
        if let Some(v) = a.pads {
            o.pads = v;
        }
        if let Some(v) = a.holes {
            o.holes = v;
        }
        if let Some(v) = a.pin1 {
            o.pin1 = v;
        }
        if let Some(v) = &a.dnp {
            o.dnp = v.as_deref().map(DnpStyle::parse);
        }
        if let Some(v) = &a.refs {
            o.refs = v.as_deref().map(RefFilter::parse);
        }
        if let Some(v) = a.text_min {
            o.text_min = v;
        }
        if let Some(v) = a.text_max {
            o.text_max = v;
        }
        Ok(())
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameArgs {
    ids: Option<Vec<String>>,
    frame: bool,
}

fn bom_renumber(s: &mut Session, _a: NoArgs) -> R<Outcome> {
    let bom = s.effective_bom();
    let has = s.doc().pages.iter().any(|p| {
        p.items
            .iter()
            .any(|i| matches!(i, Item::Table(t) if t.kind == TableKind::Bom))
    });
    if !has {
        return Err(EngineError::invalid(
            "the document has no BOM table to number: place one first (Insert > Place BOM table)",
        ));
    }
    let mut d = s.doc().clone();
    kdraft_doc::renumber_bom(&mut d, &bom);
    s.set_doc(d);
    Ok(Outcome::default().msg(format!(
        "BOM renumbered: {} items from the {} BOM",
        bom.lines.len(),
        bom.source.tag()
    )))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemRenameArgs {
    id: Option<String>,
    name: String,
}

fn item_rename(s: &mut Session, a: ItemRenameArgs) -> R<Outcome> {
    let (pi, id) = one_id(s, a.id)?;
    let it = item_mut(s, pi, &id)?;
    if !it.set_name(Some(&a.name)) {
        return Err(EngineError::invalid("this item cannot be renamed"));
    }
    Ok(Outcome::changed([&id]))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HiddenArgs {
    ids: Option<Vec<String>>,
    hidden: bool,
}

fn item_set_hidden(s: &mut Session, a: HiddenArgs) -> R<Outcome> {
    let (pi, ids) = ids_arg(s, a.ids)?;
    for id in &ids {
        item_mut(s, pi, id)?.set_hidden(a.hidden);
    }
    Ok(Outcome::changed(&ids))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LockedArgs {
    ids: Option<Vec<String>>,
    locked: bool,
}

fn item_set_locked(s: &mut Session, a: LockedArgs) -> R<Outcome> {
    let (pi, ids) = ids_arg(s, a.ids)?;
    for id in &ids {
        item_mut(s, pi, id)?.set_locked(a.locked);
    }
    Ok(Outcome::changed(&ids))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReorderArgs {
    id: String,
    to: i64,
}

fn item_reorder(s: &mut Session, a: ReorderArgs) -> R<Outcome> {
    let (pi, id) = one_id(s, Some(a.id))?;
    let items = &mut s.doc_mut().pages[pi].items;
    let from = items
        .iter()
        .position(|i| i.id() == Some(&id))
        .ok_or_else(|| kdraft_doc::OpError::NoSuchItem(id.to_string()))?;
    let to = usize::try_from(a.to.max(0))
        .unwrap_or(0)
        .min(items.len() - 1);
    let it = items.remove(from);
    items.insert(to, it);
    Ok(Outcome::changed([&id]))
}

fn item_set_frame(s: &mut Session, a: FrameArgs) -> R<Outcome> {
    let f = a.frame;
    each_view(s, a.ids, &|view| {
        view.frame = f;
        Ok(())
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GeomArgs {
    id: Option<String>,
    start: Option<P2>,
    end: Option<P2>,
    center: Option<P2>,
    radius: Option<f64>,
    at: Option<P2>,
    text: Option<String>,
    height: Option<f64>,
}

fn item_set_geometry(s: &mut Session, a: GeomArgs) -> R<Outcome> {
    let (pi, id) = one_id(s, a.id)?;
    let start = a.start.map(p).transpose()?;
    let end = a.end.map(p).transpose()?;
    let center = a.center.map(p).transpose()?;
    let at = a.at.map(p).transpose()?;
    let corner = match a.radius {
        Some(r) if r.is_finite() && r >= 0.0 => Some(r),
        Some(r) => {
            return Err(EngineError::invalid(format!(
                "radius must be 0 or more, got {r}"
            )))
        }
        None => None,
    };
    let radius = a
        .radius
        .filter(|r| *r != 0.0)
        .map(|r| positive(r, "radius"))
        .transpose()?;
    let height = a.height.map(|h| positive(h, "height")).transpose()?;
    let Item::Graphic(g) = item_mut(s, pi, &id)? else {
        return Err(EngineError::invalid(format!("item {id} is not a graphic")));
    };
    let mut used = false;
    if let (GraphicShape::Rect { radius: r0, .. }, Some(v)) = (&mut g.shape, corner) {
        *r0 = v;
        used = true;
    }
    match &mut g.shape {
        GraphicShape::Line { start: s0, end: e0 }
        | GraphicShape::Rect {
            start: s0, end: e0, ..
        } => {
            if let Some(v) = start {
                *s0 = v;
                used = true;
            }
            if let Some(v) = end {
                *e0 = v;
                used = true;
            }
        }
        GraphicShape::Circle {
            center: c0,
            radius: r0,
        } => {
            if let Some(v) = center {
                *c0 = v;
                used = true;
            }
            if let Some(v) = radius {
                *r0 = v;
                used = true;
            }
        }
        GraphicShape::Text {
            at: a0,
            text: t0,
            height: h0,
            ..
        } => {
            if let Some(v) = at {
                *a0 = v;
                used = true;
            }
            if let Some(v) = a.text {
                *t0 = v;
                used = true;
            }
            if let Some(v) = height {
                *h0 = Some(v);
                used = true;
            }
        }
        _ => {}
    }
    if !used {
        return Err(EngineError::invalid(
            "none of the given fields applies to this graphic",
        ));
    }
    Ok(Outcome::changed([id]))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToPageArgs {
    ids: Option<Vec<String>>,
    page: String,
}

fn item_move_to_page(s: &mut Session, a: ToPageArgs) -> R<Outcome> {
    let (_, ids) = ids_arg(s, a.ids)?;
    let target = page_arg(s, Some(&a.page))?;
    let tid = s.doc().pages[target].id.clone();
    let mut d = s.doc().clone();
    for id in &with_dependents(s.doc(), &ids) {
        d = ops::move_item(&d, id, &tid, None)?;
    }
    s.set_doc(d);
    select_items(s, target, ids.clone());
    Ok(Outcome::changed(&ids))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CopyToPageArgs {
    ids: Option<Vec<String>>,
    page: String,
    linked: Option<bool>,
}

fn item_copy_to_page(s: &mut Session, a: CopyToPageArgs) -> R<Outcome> {
    let (_, ids) = ids_arg(s, a.ids)?;
    let target = page_arg(s, Some(&a.page))?;
    let tid = s.doc().pages[target].id.clone();
    let all = with_dependents(s.doc(), &ids);
    let (mut d, new) = ops::copy_items(s.doc(), &all, &tid, Point::ORIGIN, &mut Id::new_v4)?;
    if a.linked.unwrap_or(false) {
        for (orig, copy) in all.iter().zip(&new) {
            let placeable = d
                .pages
                .iter()
                .find_map(|p| p.item(copy))
                .is_some_and(|i| i.position().is_some());
            if placeable {
                kdraft_doc::link::link(&mut d, copy, orig, None, true)
                    .map_err(|e| EngineError::invalid(e.to_string()))?;
            }
        }
    }
    s.set_doc(d);
    Ok(Outcome::created(&new))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RaiseArgs {
    ids: Option<Vec<String>>,
    to_top: Option<bool>,
}

fn item_raise(s: &mut Session, a: RaiseArgs) -> R<Outcome> {
    let (pi, ids) = ids_arg(s, a.ids)?;
    let page = s.doc().pages[pi].id.clone();
    let d = ops::raise(s.doc(), &page, &ids, a.to_top.unwrap_or(false))?;
    s.set_doc(d);
    Ok(Outcome::changed(&ids))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LowerArgs {
    ids: Option<Vec<String>>,
    to_bottom: Option<bool>,
}

fn item_lower(s: &mut Session, a: LowerArgs) -> R<Outcome> {
    let (pi, ids) = ids_arg(s, a.ids)?;
    let page = s.doc().pages[pi].id.clone();
    let d = ops::lower(s.doc(), &page, &ids, a.to_bottom.unwrap_or(false))?;
    s.set_doc(d);
    Ok(Outcome::changed(&ids))
}

// ---------------------------------------------------------------- selection

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelSetArgs {
    page: Option<String>,
    ids: Vec<String>,
}

fn select_set(s: &mut Session, a: SelSetArgs) -> R<Outcome> {
    let pi = match (&a.page, a.ids.first()) {
        (Some(p), _) => page_arg(s, Some(p))?,
        (None, Some(first)) => {
            s.find_item(&Id(first.clone()))
                .ok_or_else(|| kdraft_doc::OpError::NoSuchItem(first.clone()))?
                .0
        }
        (None, None) => page_arg(s, None)?,
    };
    let mut ids: Vec<Id> = Vec::new();
    for i in a.ids {
        let id = Id(i);
        if s.doc().pages[pi].item(&id).is_none() {
            return Err(kdraft_doc::OpError::NoSuchItem(id.to_string()).into());
        }
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    select_items(s, pi, ids);
    Ok(Outcome::default())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelAddArgs {
    ids: Vec<String>,
}

fn select_add(s: &mut Session, a: SelAddArgs) -> R<Outcome> {
    let pi = page_arg(s, None)?;
    for i in a.ids {
        let id = Id(i);
        if s.doc().pages[pi].item(&id).is_none() {
            return Err(EngineError::invalid(format!(
                "item {id} is not on the current page"
            )));
        }
        if !s.selection.contains(&id) {
            s.selection.items.push(id);
        }
    }
    s.selection.page = Some(s.doc().pages[pi].id.clone());
    Ok(Outcome::default())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelToggleArgs {
    id: String,
}

fn select_toggle(s: &mut Session, a: SelToggleArgs) -> R<Outcome> {
    let id = Id(a.id);
    let (pi, _) = s
        .find_item(&id)
        .ok_or_else(|| kdraft_doc::OpError::NoSuchItem(id.to_string()))?;
    if s.current_page_index() != Some(pi) {
        select_items(s, pi, vec![id]);
    } else if s.selection.contains(&id) {
        s.selection.items.retain(|i| *i != id);
    } else {
        s.selection.items.push(id);
    }
    Ok(Outcome::default())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

fn select_clear(s: &mut Session, _: NoArgs) -> R<Outcome> {
    s.selection.items.clear();
    Ok(Outcome::default())
}

fn select_all(s: &mut Session, a: PageArg) -> R<Outcome> {
    let pi = page_arg(s, a.page.as_deref())?;
    let ids: Vec<Id> = s.doc().pages[pi]
        .items
        .iter()
        .filter_map(|i| i.id().cloned())
        .collect();
    select_items(s, pi, ids);
    Ok(Outcome::default())
}

// ---------------------------------------------------------------- layout

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AlignArgs {
    ids: Option<Vec<String>>,
    mode: String,
}

fn parse_align(m: &str) -> R<Align> {
    Ok(match m {
        "left" => Align::Left,
        "hcenter" => Align::HCenter,
        "right" => Align::Right,
        "top" => Align::Top,
        "vcenter" => Align::VCenter,
        "bottom" => Align::Bottom,
        o => {
            return Err(EngineError::invalid(format!(
                "align mode must be one of {ALIGN:?}, got {o:?}"
            )))
        }
    })
}

fn layout_align(s: &mut Session, a: AlignArgs) -> R<Outcome> {
    let mode = parse_align(&a.mode)?;
    let (pi, ids) = ids_arg(s, a.ids)?;
    let ids = movable(s, pi, ids)?;
    let reference = if ids.len() == 1 {
        s.drawing_area(pi)
            .ok_or_else(|| EngineError::invalid("page has no drawing area"))?
    } else {
        boxes(s, pi, &ids[..1])?[0]
    };
    let page = s.doc().pages[pi].id.clone();
    let session: &Session = s;
    let bounds = |it: &Item| session.item_bounds(it);
    let d = ops::align_to(session.doc(), &page, &ids, mode, reference, &bounds)?;
    s.set_doc(d);
    Ok(Outcome::changed(&ids))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DistArgs {
    ids: Option<Vec<String>>,
    direction: String,
    spacing: Option<String>,
}

fn layout_distribute(s: &mut Session, a: DistArgs) -> R<Outcome> {
    let dir = match a.direction.as_str() {
        "horizontal" => Distribute::Horizontal,
        "vertical" => Distribute::Vertical,
        o => {
            return Err(EngineError::invalid(format!(
                "direction must be horizontal or vertical, got {o:?}"
            )))
        }
    };
    let (pi, ids) = ids_arg(s, a.ids)?;
    let ids = movable(s, pi, ids)?;
    crate::enable::check_items("layout.distribute", ids.len()).map_err(EngineError::invalid)?;
    let page = s.doc().pages[pi].id.clone();
    let session: &Session = s;
    let bounds = |it: &Item| session.item_bounds(it);
    let d = match a.spacing.as_deref().unwrap_or("gaps") {
        "gaps" => ops::distribute(session.doc(), &page, &ids, dir, &bounds)?,
        "centres" => ops::distribute_centres(session.doc(), &page, &ids, dir, &bounds)?,
        o => {
            return Err(EngineError::invalid(format!(
                "spacing must be gaps or centres, got {o:?}"
            )))
        }
    };
    s.set_doc(d);
    Ok(Outcome::changed(&ids))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MatchArgs {
    ids: Option<Vec<String>>,
    property: String,
}

fn layout_match(s: &mut Session, a: MatchArgs) -> R<Outcome> {
    let (pi, ids) = ids_arg(s, a.ids)?;
    let ids = movable(s, pi, ids)?;
    crate::enable::check_items("layout.match", ids.len()).map_err(EngineError::invalid)?;
    let bx = boxes(s, pi, &ids)?;
    let reference = bx[0];
    let ref_scale = match s.doc().pages[pi].item(&ids[0]) {
        Some(Item::View(v)) => Some(v.scale),
        _ => None,
    };
    let mut changed = Vec::new();
    let mut skipped = 0;
    for (id, b) in ids.iter().zip(&bx).skip(1) {
        let item = item_mut(s, pi, id)?;
        let done = match a.property.as_str() {
            "scale" => match (item, ref_scale) {
                (Item::View(v), Some(rs)) => {
                    v.scale = rs;
                    true
                }
                _ => false,
            },
            "width" | "height" => {
                let w = a.property == "width";
                let (want, have) = if w {
                    (reference.width(), b.width())
                } else {
                    (reference.height(), b.height())
                };
                resize(item, w, want, have)
            }
            o => {
                return Err(EngineError::invalid(format!(
                    "property must be width, height or scale, got {o:?}"
                )))
            }
        };
        if done {
            changed.push(id.clone());
        } else {
            skipped += 1;
        }
    }
    if changed.is_empty() {
        return Err(EngineError::invalid(format!(
            "none of the items can match the reference's {}",
            a.property
        )));
    }
    let mut o = Outcome::changed(&changed);
    if skipped > 0 {
        o = o.msg(format!("{skipped} item(s) cannot be resized this way"));
    }
    Ok(o)
}

/// Resizes an item so its box extent along one axis becomes `want`.
fn resize(item: &mut Item, horizontal: bool, want: f64, have: f64) -> bool {
    if !(want > 0.0 && have > 0.0) {
        return false;
    }
    let f = want / have;
    match item {
        Item::View(v) => {
            v.scale *= f;
            true
        }
        Item::Graphic(g) => match &mut g.shape {
            GraphicShape::Rect { start, end, .. } | GraphicShape::Line { start, end } => {
                if horizontal {
                    end.x = start.x + (end.x - start.x) * f;
                } else {
                    end.y = start.y + (end.y - start.y) * f;
                }
                true
            }
            GraphicShape::Circle { radius, .. } => {
                *radius *= f;
                true
            }
            _ => false,
        },
        _ => false,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CenterArgs {
    ids: Option<Vec<String>>,
    target: Option<String>,
    axis: Option<String>,
}

fn layout_center(s: &mut Session, a: CenterArgs) -> R<Outcome> {
    let (pi, ids) = ids_arg(s, a.ids)?;
    let ids = movable(s, pi, ids)?;
    let target = match a.target.as_deref().unwrap_or("area") {
        "area" => s
            .drawing_area(pi)
            .ok_or_else(|| EngineError::invalid("page has no drawing area"))?,
        "page" => {
            let sz = &s.doc().pages[pi].size;
            BBox {
                min: Point::ORIGIN,
                max: Point::new(sz.width, sz.height),
            }
        }
        o => {
            return Err(EngineError::invalid(format!(
                "target must be area or page, got {o:?}"
            )))
        }
    };
    let all = boxes(s, pi, &ids)?
        .into_iter()
        .reduce(|x, y| x.union(&y))
        .ok_or_else(|| EngineError::invalid("nothing selected"))?;
    let d = target.center() - all.center();
    let d = match a.axis.as_deref().unwrap_or("both") {
        "both" => d,
        "horizontal" => Point::new(d.x, 0.0),
        "vertical" => Point::new(0.0, d.y),
        o => {
            return Err(EngineError::invalid(format!(
                "axis must be both, horizontal or vertical, got {o:?}"
            )))
        }
    };
    translate_all(s, pi, &ids, d)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NudgeArgs {
    ids: Option<Vec<String>>,
    dx: f64,
    dy: f64,
}

fn layout_nudge(s: &mut Session, a: NudgeArgs) -> R<Outcome> {
    let (pi, ids) = ids_arg(s, a.ids)?;
    let g = s.doc().styles.snap_grid;
    let g = if g > 0.0 && g.is_finite() {
        g
    } else {
        kdraft_doc::DEFAULT_SNAP_GRID
    };
    translate_all(s, pi, &ids, Point::new(a.dx * g, a.dy * g))
}

// ---------------------------------------------------------------- dimensions

pub(crate) fn register_dimensions(r: &mut Registry) {
    let view = || {
        opt(
            "view",
            T::Id,
            "view id (default: the selected view, else the first view on the current page)",
        )
    };
    let anchor = |n: &'static str, d: &'static str| req(n, T::String, d);
    let ids = || opt("ids", T::IdList, "item ids (default: the selection)");
    r.add(
        spec("dimension.add_linear", "Add linear dimension", Edit, "Adds a horizontal, vertical or aligned dimension between two anchors, e.g. \"(outline-vertex 0)\" and \"(pad \\\"U1\\\" \\\"1\\\")\".",
            vec![view(), anchor("from", "first anchor (S-expression)"), anchor("to", "second anchor (S-expression)"),
                 opt("kind", T::Enum { values: &["horizontal", "vertical", "aligned", "linear"] }, "direction (default linear: the larger of horizontal and vertical)"),
                 opt("offset", T::Number, "dimension line distance beyond the outermost point, sheet mm (default 8; negative: other side)")]),
        dimension_add_linear,
    );
    r.add(
        spec("dimension.add_radius", "Add radius or diameter dimension", Edit, "Adds a radius (or diameter) dimension on a hole or an outline arc/circle (\"(hole N)\", \"(outline-edge N)\").",
            vec![view(), anchor("anchor", "circular anchor"), opt("diameter", T::Bool, "diameter instead of radius"),
                 opt("angle", T::Number, "leader direction, degrees CCW (default 45)")]),
        dimension_add_radius,
    );
    r.add(
        spec("dimension.add_ordinate", "Add ordinate dimension", Edit, "Adds an ordinate dimension of an anchor from a datum (default: the lower-left vertex of the outline). Values are board millimetres, Y up.",
            vec![view(), opt("datum", T::String, "datum anchor (default: lower-left outline vertex)"), anchor("to", "measured anchor"),
                 req("axis", T::Enum { values: &["x", "y"] }, "coordinate shown"),
                 opt("offset", T::Number, "leader end distance from the datum, sheet mm (default 8 below for x, left for y)")]),
        dimension_add_ordinate,
    );
    r.add(
        spec(
            "dimension.set_offset",
            "Set dimension offset",
            Edit,
            "Sets the offset of dimensions, sheet mm.",
            vec![ids(), req("offset", T::Number, "offset, sheet mm")],
        ),
        dimension_set_offset,
    );
    r.add(
        spec(
            "dimension.set_tolerance",
            "Set dimension tolerance",
            Edit,
            "Sets the tolerance text shown after the value (e.g. \"±0.1\"); null clears it.",
            vec![ids(), opt("tolerance", T::String, "tolerance text")],
        ),
        dimension_set_tolerance,
    );
    r.add(
        spec(
            "dimension.set_style",
            "Set dimension style",
            Edit,
            "Sets the document's dimension style; omitted fields stay.",
            vec![
                opt("decimals", T::Integer, "decimals (0..6, default 2)"),
                opt("trim_zeros", T::Bool, "trim trailing zeros (default off)"),
                opt(
                    "text_height",
                    T::Number,
                    "text cap height, mm (default 2.5)",
                ),
                opt("arrow_length", T::Number, "arrow length, mm (default 2.5)"),
                opt("gap", T::Number, "extension line gap, mm (default 1)"),
                opt(
                    "overshoot",
                    T::Number,
                    "extension line overshoot, mm (default 1.5)",
                ),
            ],
        ),
        dimension_set_style,
    );
    r.add(
        spec("dimension.board_size", "Add board size dimensions", Edit, "Adds overall width (above) and height (right) dimensions of the board outline to a view.",
            vec![view()]),
        dimension_board_size,
    );
}

/// Page index and id of the view a dimension command applies to.
pub(crate) fn target_view(s: &Session, view: Option<String>) -> R<(usize, Id)> {
    if let Some(v) = view {
        let id = Id(v);
        for (pi, p) in s.doc().pages.iter().enumerate() {
            if p.views().any(|x| x.id == id) {
                return Ok((pi, id));
            }
        }
        return Err(EngineError::invalid(format!("no view {id}")));
    }
    let pi = s
        .current_page_index()
        .ok_or_else(|| EngineError::invalid("no page"))?;
    let page = &s.doc().pages[pi];
    let board_views = || page.views().filter(|v| v.kind != ViewKind::Render3d);
    let selected = s
        .selection
        .items
        .iter()
        .find(|id| board_views().any(|v| &v.id == *id))
        .cloned();
    let id = selected
        .or_else(|| board_views().next().map(|v| v.id.clone()))
        .ok_or_else(|| {
            EngineError::invalid(
                "no board view on this page: place a fabrication view first (Insert > Place fabrication view)",
            )
        })?;
    Ok((pi, id))
}

pub(crate) fn anchor_arg(text: &str) -> R<kdraft_doc::Anchor> {
    kdraft_doc::parse_anchor(text).map_err(|e| EngineError::invalid(e.to_string()))
}

fn add_dimension(s: &mut Session, pi: usize, mut d: kdraft_doc::Dimension) -> R<Outcome> {
    let board = s
        .board_arc()
        .ok_or_else(|| EngineError::invalid("no board loaded"))?;
    let holes = board.holes();
    d.last = kdraft_doc::anchor::dimension_positions(&d, &board, &holes)
        .map_err(|e| EngineError::invalid(format!("anchor does not resolve: {e}")))?;
    add_item(s, pi, Item::Dimension(d))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DimLinearArgs {
    view: Option<String>,
    from: String,
    to: String,
    kind: Option<String>,
    offset: Option<f64>,
}

fn dimension_add_linear(s: &mut Session, a: DimLinearArgs) -> R<Outcome> {
    use kdraft_doc::DimensionKind as K;
    let (pi, vid) = target_view(s, a.view)?;
    let kind = match a.kind.as_deref().unwrap_or("linear") {
        "horizontal" => K::Horizontal,
        "vertical" => K::Vertical,
        "aligned" => K::Aligned,
        "linear" => K::Linear,
        o => return Err(EngineError::invalid(format!("unknown kind {o:?}"))),
    };
    let d = kdraft_doc::Dimension::new(
        kind,
        vid,
        anchor_arg(&a.from)?,
        Some(anchor_arg(&a.to)?),
        finite(a.offset.unwrap_or(8.0))?,
    );
    add_dimension(s, pi, d)
}

fn finite(v: f64) -> R<f64> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(EngineError::invalid("value must be finite"))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DimRadiusArgs {
    view: Option<String>,
    anchor: String,
    diameter: Option<bool>,
    angle: Option<f64>,
}

fn dimension_add_radius(s: &mut Session, a: DimRadiusArgs) -> R<Outcome> {
    use kdraft_doc::DimensionKind as K;
    let (pi, vid) = target_view(s, a.view)?;
    let kind = if a.diameter.unwrap_or(false) {
        K::Diameter
    } else {
        K::Radius
    };
    let mut d = kdraft_doc::Dimension::new(kind, vid, anchor_arg(&a.anchor)?, None, 0.0);
    d.angle = a.angle.map(finite).transpose()?;
    add_dimension(s, pi, d)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DimOrdinateArgs {
    view: Option<String>,
    datum: Option<String>,
    to: String,
    axis: String,
    offset: Option<f64>,
}

/// Lower-left vertex of the outer outline (largest y, then smallest x).
fn lower_left_vertex(s: &Session) -> R<kdraft_doc::Anchor> {
    let b = s
        .board()
        .ok_or_else(|| EngineError::invalid("no board loaded"))?;
    let outer = b
        .outline
        .outer()
        .ok_or_else(|| EngineError::invalid("board has no closed outline"))?;
    let v = &outer.vertices;
    let i = (0..v.len())
        .min_by(|&i, &j| {
            (v[i].x - v[i].y)
                .total_cmp(&(v[j].x - v[j].y))
                .then(i.cmp(&j))
        })
        .ok_or_else(|| EngineError::invalid("outline has no vertices"))?;
    Ok(kdraft_doc::Anchor::OutlineVertex(i))
}

fn dimension_add_ordinate(s: &mut Session, a: DimOrdinateArgs) -> R<Outcome> {
    if a.axis != "x" && a.axis != "y" {
        return Err(EngineError::invalid("axis must be x or y"));
    }
    let (pi, vid) = target_view(s, a.view)?;
    let datum = match &a.datum {
        Some(t) => anchor_arg(t)?,
        None => lower_left_vertex(s)?,
    };
    let mut d = kdraft_doc::Dimension::new(
        kdraft_doc::DimensionKind::Ordinate,
        vid,
        datum,
        Some(anchor_arg(&a.to)?),
        finite(a.offset.unwrap_or(8.0))?,
    );
    d.axis = Some(a.axis);
    add_dimension(s, pi, d)
}

fn each_dimension(
    s: &mut Session,
    ids: Option<Vec<String>>,
    f: &dyn Fn(&mut kdraft_doc::Dimension),
) -> R<Outcome> {
    let (pi, ids) = ids_arg(s, ids)?;
    let mut changed = Vec::new();
    for id in &ids {
        if let Item::Dimension(d) = item_mut(s, pi, id)? {
            f(d);
            changed.push(id.clone());
        }
    }
    if changed.is_empty() {
        return Err(EngineError::invalid("no dimension among the items"));
    }
    Ok(Outcome::changed(&changed))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DimOffsetArgs {
    ids: Option<Vec<String>>,
    offset: f64,
}

fn dimension_set_offset(s: &mut Session, a: DimOffsetArgs) -> R<Outcome> {
    let o = finite(a.offset)?;
    each_dimension(s, a.ids, &|d| d.offset = o)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DimTolArgs {
    ids: Option<Vec<String>>,
    tolerance: Option<String>,
}

fn dimension_set_tolerance(s: &mut Session, a: DimTolArgs) -> R<Outcome> {
    let t = a.tolerance.filter(|t| !t.is_empty());
    each_dimension(s, a.ids, &|d| d.tolerance = t.clone())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DimStyleArgs {
    decimals: Option<i64>,
    trim_zeros: Option<bool>,
    text_height: Option<f64>,
    arrow_length: Option<f64>,
    gap: Option<f64>,
    overshoot: Option<f64>,
}

fn dimension_set_style(s: &mut Session, a: DimStyleArgs) -> R<Outcome> {
    let mut st = s.doc().styles.dimension;
    if let Some(d) = a.decimals {
        if !(0..=6).contains(&d) {
            return Err(EngineError::invalid("decimals must be 0..6"));
        }
        st.decimals = d as usize;
    }
    if let Some(t) = a.trim_zeros {
        st.trim_zeros = t;
    }
    if let Some(v) = a.text_height {
        st.text_height = positive(v, "text_height")?;
    }
    if let Some(v) = a.arrow_length {
        st.arrow_length = positive(v, "arrow_length")?;
    }
    if let Some(v) = a.gap {
        st.gap = finite(v)?.max(0.0);
    }
    if let Some(v) = a.overshoot {
        st.overshoot = finite(v)?.max(0.0);
    }
    s.doc_mut().styles.dimension = st;
    Ok(Outcome::default())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ViewArg {
    view: Option<String>,
}

fn dimension_board_size(s: &mut Session, a: ViewArg) -> R<Outcome> {
    let given = a.view.is_some();
    let mut made = None;
    let (pi, vid) = match target_view(s, a.view) {
        Ok(t) => t,
        // No board view on the page: place a fabrication view (centred,
        // largest standard scale that leaves room for the dimensions).
        Err(_) if !given && s.board().is_some() => {
            let pi = page_arg(s, None)?;
            let bb = s.board_box();
            let area = s.drawing_area(pi);
            let room = 2.0 * kdraft_doc::BOARD_SIZE_ROOM;
            let scale = area
                .and_then(|r| {
                    kdraft_doc::choose_scale(
                        bb.width(),
                        bb.height(),
                        r.width() - room,
                        r.height() - room,
                    )
                })
                .unwrap_or(kdraft_doc::STANDARD_SCALES[0]);
            let o = item_add_view(
                s,
                AddViewArgs {
                    page: Some(s.doc().pages[pi].id.0.clone()),
                    kind: "fabrication".into(),
                    side: None,
                    layers: None,
                    scale: Some(scale),
                    at: None,
                    drill_marks: None,
                },
            )?;
            let id = o
                .created
                .first()
                .cloned()
                .ok_or_else(|| EngineError::invalid("no view placed"))?;
            made = Some(id.clone());
            (pi, Id(id))
        }
        Err(e) => return Err(e),
    };
    let board = s
        .board_arc()
        .ok_or_else(|| EngineError::invalid("no board loaded"))?;
    let dims = kdraft_doc::board_size_dimensions(&board, &vid);
    if dims.is_empty() {
        return Err(EngineError::invalid(
            "the outline has no vertices to dimension",
        ));
    }
    let mut ids: Vec<Id> = dims.iter().map(|d| d.id.clone()).collect();
    for d in dims {
        s.doc_mut().pages[pi].items.push(Item::Dimension(d));
    }
    select_items(s, pi, ids.clone());
    match made {
        Some(v) => {
            ids.insert(0, Id(v));
            Ok(Outcome::created(ids)
                .msg("the page had no board view: added a fabrication view for the dimensions"))
        }
        None => Ok(Outcome::created(ids)),
    }
}

fn document_check(s: &mut Session, _: NoArgs) -> R<Outcome> {
    let rep = crate::check::check(s);
    let mut o = Outcome::default();
    for line in rep.text().lines() {
        o = o.msg(line.to_string());
    }
    Ok(o)
}

// ---------------------------------------------------------------- notes and callouts

pub(crate) fn register_notes(r: &mut Registry) {
    let ids = || {
        opt(
            "id",
            T::Id,
            "note list id (default: the selected note list)",
        )
    };
    r.add(
        spec("item.add_notes", "Add note list", Edit, "Adds a numbered note list; texts may use ${BOARD_THICKNESS}, ${LAYER_COUNT}, ${COPPER_FINISH}, ${BOARD_WIDTH}, ${BOARD_HEIGHT}, ${MIN_DRILL}, ${HOLE_COUNT} and title-block/project variables.",
            vec![opt("at", T::Point, "top-left, sheet mm (default: top left of the drawing area)"), opt("width", T::Number, "wrap width, mm"), opt("title", T::String, "heading (default NOTES)"),
                 opt("notes", T::StringList, "note texts")]),
        notes_add_list,
    );
    r.add(
        spec(
            "notes.add",
            "Add note",
            Edit,
            "Appends (or inserts) a note with a new stable id.",
            vec![
                ids(),
                req("text", T::String, "note text"),
                opt("index", T::Integer, "0-based position (default: end)"),
            ],
        ),
        notes_add,
    );
    r.add(
        spec(
            "notes.edit",
            "Edit note",
            Edit,
            "Replaces the text of a note.",
            vec![
                ids(),
                req("note", T::String, "note id, e.g. N2"),
                req("text", T::String, "new text"),
            ],
        ),
        notes_edit,
    );
    r.add(
        spec(
            "notes.remove",
            "Remove note",
            Edit,
            "Removes a note; the others renumber, ids stay.",
            vec![ids(), req("note", T::String, "note id")],
        ),
        notes_remove,
    );
    r.add(
        spec(
            "notes.move",
            "Move note",
            Edit,
            "Moves a note to a new 0-based position; callouts follow their note's new number.",
            vec![
                ids(),
                req("note", T::String, "note id"),
                req("to", T::Integer, "0-based position"),
            ],
        ),
        notes_move,
    );
    crate::commands_callout::register(r);
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AddNotesArgs {
    pub(crate) at: Option<P2>,
    pub(crate) width: Option<f64>,
    pub(crate) title: Option<String>,
    pub(crate) notes: Option<Vec<String>>,
}

pub(crate) fn notes_add_list(s: &mut Session, a: AddNotesArgs) -> R<Outcome> {
    let pi = page_arg(s, None)?;
    let notes = a
        .notes
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .map(|(i, t)| kdraft_doc::Note {
            id: format!("N{}", i + 1),
            text: t,
        })
        .collect();
    let at = match a.at {
        Some(v) => p(v)?,
        None => notes_default_at(s, pi),
    };
    let n = kdraft_doc::NoteList {
        id: Id::new_v4(),
        at,
        title: Some(a.title.unwrap_or_else(|| "NOTES".to_string())),
        width: a.width.map(|w| positive(w, "width")).transpose()?,
        notes,
        extra: Vec::new(),
    };
    add_item(s, pi, Item::NoteList(n))
}

/// Default place of a new note list: the drawing area's top left, 10 mm in.
fn notes_default_at(s: &mut Session, pi: usize) -> Point {
    s.drawing_area(pi).map_or(Point::new(20.0, 20.0), |r| {
        Point::new(r.min.x + 10.0, r.min.y + 10.0)
    })
}

/// The note list a notes command works on when none is given: the
/// selected one, else the first on the current page.
pub(crate) fn default_note_list(s: &Session) -> R<Option<(usize, Id)>> {
    let pi = page_arg(s, None)?;
    let page = &s.doc().pages[pi];
    let id = s
        .selection
        .items
        .iter()
        .find(|i| matches!(page.item(i), Some(Item::NoteList(_))))
        .cloned()
        .or_else(|| {
            page.items.iter().find_map(|i| match i {
                Item::NoteList(n) => Some(n.id.clone()),
                _ => None,
            })
        });
    Ok(id.map(|id| (pi, id)))
}

pub(crate) fn note_list_mut(
    s: &mut Session,
    id: Option<String>,
) -> R<(Id, &mut kdraft_doc::NoteList)> {
    let (pi, id) = match id {
        Some(i) => {
            let id = Id(i);
            let pi = s
                .doc()
                .pages
                .iter()
                .position(|p| p.item(&id).is_some())
                .ok_or_else(|| EngineError::invalid(format!("no item {id}")))?;
            (pi, id)
        }
        None => default_note_list(s)?.ok_or_else(|| {
            EngineError::invalid("no note list on this page: add one with Insert > Note list")
        })?,
    };
    match item_mut(s, pi, &id)? {
        Item::NoteList(n) => Ok((id, n)),
        _ => Err(EngineError::invalid(format!(
            "item {id} is not a note list"
        ))),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoteAddArgs {
    id: Option<String>,
    text: String,
    index: Option<i64>,
}

fn notes_add(s: &mut Session, a: NoteAddArgs) -> R<Outcome> {
    // No list given and none on the page: start one.
    if a.id.is_none() && default_note_list(s)?.is_none() {
        let o = notes_add_list(
            s,
            AddNotesArgs {
                at: None,
                width: None,
                title: None,
                notes: Some(vec![a.text]),
            },
        )?;
        return Ok(o.msg("added a note list with note N1"));
    }
    let (id, n) = note_list_mut(s, a.id)?;
    let nid = n.next_id();
    let at = a
        .index
        .map_or(n.notes.len(), |i| (i.max(0) as usize).min(n.notes.len()));
    n.notes.insert(
        at,
        kdraft_doc::Note {
            id: nid.clone(),
            text: a.text,
        },
    );
    Ok(Outcome::changed([id]).msg(format!("note {nid}")))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoteEditArgs {
    id: Option<String>,
    note: String,
    text: String,
}

fn notes_edit(s: &mut Session, a: NoteEditArgs) -> R<Outcome> {
    let (id, n) = note_list_mut(s, a.id)?;
    let note = n
        .notes
        .iter_mut()
        .find(|x| x.id == a.note)
        .ok_or_else(|| EngineError::invalid(format!("no note {}", a.note)))?;
    note.text = a.text;
    Ok(Outcome::changed([id]))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoteRef {
    id: Option<String>,
    note: String,
}

fn notes_remove(s: &mut Session, a: NoteRef) -> R<Outcome> {
    let (id, n) = note_list_mut(s, a.id)?;
    let before = n.notes.len();
    n.notes.retain(|x| x.id != a.note);
    if n.notes.len() == before {
        return Err(EngineError::invalid(format!("no note {}", a.note)));
    }
    Ok(Outcome::changed([id]))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoteMoveArgs {
    id: Option<String>,
    note: String,
    to: i64,
}

fn notes_move(s: &mut Session, a: NoteMoveArgs) -> R<Outcome> {
    let (id, n) = note_list_mut(s, a.id)?;
    let from = n
        .notes
        .iter()
        .position(|x| x.id == a.note)
        .ok_or_else(|| EngineError::invalid(format!("no note {}", a.note)))?;
    let note = n.notes.remove(from);
    let to = (a.to.max(0) as usize).min(n.notes.len());
    n.notes.insert(to, note);
    Ok(Outcome::changed([id]))
}
