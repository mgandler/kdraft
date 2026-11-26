//! Geometry editing commands: text content and layout, points of lines
//! and polylines, sizes, and re-anchoring of dimensions and callouts.

use kdraft_board::geom::Point;
use kdraft_doc::{GraphicShape, Id, Item};
use serde::Deserialize;

use crate::commands::{
    add_item, graphic, ids_arg, item_mut, one_id, p, page_arg, positive, spec, P2,
};
use crate::error::{EngineError, R};
use crate::registry::{opt, req, ArgType as T, CommandKind, Outcome, Registry};
use crate::session::Session;

pub(crate) fn register(r: &mut Registry) {
    register_3d(r);
    r.add(
        spec(
            "text.set",
            "Edit text",
            CommandKind::Edit,
            "Changes a text: content (lines separated by \\n), wrap width (0 or null: no wrapping), rotation and position.",
            vec![
                opt("id", T::Id, "item id (default: first selected)"),
                opt("text", T::String, "text; \\n starts a new line"),
                opt("wrap", T::Number, "wrap width, mm (0: none)"),
                opt("rotation", T::Number, "degrees, counter-clockwise"),
                opt("at", T::Point, "anchor: first baseline at the alignment edge"),
            ],
        ),
        text_set,
    );
    let page = || opt("page", T::Id, "page (default: current)");
    let id = || opt("id", T::Id, "item id (default: first selected)");
    r.add(
        spec(
            "item.add_polyline",
            "Add polyline",
            CommandKind::Edit,
            "Adds an open polyline (or a closed polygon) through points.",
            vec![
                page(),
                req(
                    "points",
                    T::PointList,
                    "points as [[x, y], ...], sheet mm (at least 2)",
                ),
                opt("closed", T::Bool, "closed polygon (at least 3 points)"),
            ],
        ),
        add_polyline,
    );
    r.add(
        spec(
            "item.add_circle",
            "Add circle",
            CommandKind::Edit,
            "Adds a circle.",
            vec![
                page(),
                req("center", T::Point, "centre, sheet mm"),
                req("radius", T::Number, "radius, mm"),
            ],
        ),
        add_circle,
    );
    r.add(
        spec(
            "graphic.set_points",
            "Edit points",
            CommandKind::Edit,
            "Sets the defining points of a graphic: line start and end, rectangle corners, polyline or polygon vertices, arc start/mid/end, circle centre and a point on the circle, text anchor.",
            vec![id(), req("points", T::PointList, "points as [[x, y], ...], sheet mm")],
        ),
        set_points,
    );
    r.add(
        spec(
            "graphic.insert_point",
            "Insert vertex",
            CommandKind::Edit,
            "Inserts a vertex into a polyline or polygon before index (a line becomes a polyline).",
            vec![
                id(),
                req("index", T::Integer, "position of the new vertex"),
                req("at", T::Point, "vertex, sheet mm"),
            ],
        ),
        insert_point,
    );
    r.add(
        spec(
            "graphic.remove_point",
            "Remove vertex",
            CommandKind::Edit,
            "Removes a vertex of a polyline (at least 2 remain) or polygon (at least 3).",
            vec![id(), req("index", T::Integer, "vertex index")],
        ),
        remove_point,
    );
    r.add(
        spec(
            "item.set_size",
            "Set size",
            CommandKind::Edit,
            "Sets the size of rectangles (width, height from the first corner), circles (width: diameter), texts (width: wrap width) and lines (width/height: extent from the start).",
            vec![
                opt("ids", T::IdList, "item ids (default: the selection)"),
                opt("width", T::Number, "width, mm"),
                opt("height", T::Number, "height, mm"),
            ],
        ),
        set_size,
    );
    r.add(
        spec(
            "dimension.set_anchor",
            "Re-anchor dimension",
            CommandKind::Edit,
            "Points one end of a dimension at another anchor of its view.",
            vec![
                id(),
                req(
                    "end",
                    T::Enum {
                        values: &["from", "to"],
                    },
                    "which end",
                ),
                req(
                    "anchor",
                    T::String,
                    "anchor S-expression, e.g. (pad \"U1\" \"1\")",
                ),
            ],
        ),
        dimension_set_anchor,
    );
    r.add(
        spec(
            "dimension.set_text_shift",
            "Move dimension text",
            CommandKind::Edit,
            "Moves the text of linear dimensions along the dimension line, sheet mm from the middle (0: centred).",
            vec![
                opt("ids", T::IdList, "dimension ids (default: the selection)"),
                req("shift", T::Number, "shift, sheet mm"),
            ],
        ),
        dimension_set_text_shift,
    );
    r.add(
        spec(
            "callout.set_anchor",
            "Re-anchor callout",
            CommandKind::Edit,
            "Points the leader of a callout at another anchor (of the same or another view on the page).",
            vec![
                id(),
                req("anchor", T::String, "anchor S-expression"),
                opt("view", T::Id, "view of the anchor (default: the callout's)"),
            ],
        ),
        callout_set_anchor,
    );
}

fn register_3d(r: &mut Registry) {
    r.add(
        spec(
            "view.set_render3d",
            "Set 3D view options",
            CommandKind::Edit,
            "Sets the options of 3D views (kicad-cli pcb render): side, isometric preset, rotation, perspective, zoom, quality, background, resolution. Omitted options stay.",
            vec![
                opt("ids", T::IdList, "view ids (default: the selection)"),
                opt("side", T::Enum { values: &kdraft_doc::render3d::SIDES }, "side looked at"),
                opt("preset", T::Enum { values: &["top", "bottom", "isometric", "isometric_bottom"] }, "camera preset (sets side and rotation)"),
                opt("rotate", T::String, "rotation 'X,Y,Z' degrees around the board centre; empty clears"),
                opt("perspective", T::Bool, "perspective instead of orthographic"),
                opt("zoom", T::Integer, "camera zoom, 1 or more"),
                opt("quality", T::Enum { values: &["basic", "high"] }, "render quality"),
                opt("background", T::Enum { values: &["transparent", "white"] }, "background"),
                opt("dpi", T::Integer, "resolution on paper, 10..1200 (default 300; at most 4096 px a side)"),
                opt("appearance", T::String, "kicad-cli appearance preset: follow_plot_settings (its default), follow_pcb_editor or a user preset name; empty: default"),
                opt("stackup_colors", T::Bool, "use the board stackup colours"),
                opt("floor", T::Bool, "floor, shadows and post-processing"),
            ],
        ),
        set_render3d,
    );
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Render3dArgs {
    ids: Option<Vec<String>>,
    side: Option<String>,
    preset: Option<String>,
    rotate: Option<String>,
    perspective: Option<bool>,
    zoom: Option<i64>,
    quality: Option<String>,
    background: Option<String>,
    dpi: Option<i64>,
    appearance: Option<String>,
    stackup_colors: Option<bool>,
    floor: Option<bool>,
}

fn set_render3d(s: &mut Session, a: Render3dArgs) -> R<Outcome> {
    use kdraft_doc::render3d::{ISOMETRIC, SIDES};
    let (pi, ids) = ids_arg(s, a.ids)?;
    let rotate = match a.rotate.as_deref().map(str::trim) {
        None => None,
        Some("") => Some(None),
        Some(t) => {
            let v: Vec<f64> = t
                .split(',')
                .map(|x| x.trim().parse::<f64>())
                .collect::<Result<_, _>>()
                .map_err(|_| EngineError::invalid("rotate must be 'X,Y,Z' degrees"))?;
            match v.as_slice() {
                [x, y, z] if v.iter().all(|q| q.is_finite()) => Some(Some([*x, *y, *z])),
                _ => return Err(EngineError::invalid("rotate must be 'X,Y,Z' degrees")),
            }
        }
    };
    if let Some(sd) = &a.side {
        if !SIDES.contains(&sd.as_str()) {
            return Err(EngineError::invalid(format!("unknown side {sd:?}")));
        }
    }
    if a.zoom.is_some_and(|z| !(1..=100).contains(&z)) {
        return Err(EngineError::invalid("zoom must be 1..100"));
    }
    if a.quality
        .as_deref()
        .is_some_and(|q| q != "basic" && q != "high")
    {
        return Err(EngineError::invalid("quality must be basic or high"));
    }
    if a.background
        .as_deref()
        .is_some_and(|b| b != "transparent" && b != "white")
    {
        return Err(EngineError::invalid(
            "background must be transparent or white",
        ));
    }
    if a.dpi.is_some_and(|d| !(10..=1200).contains(&d)) {
        return Err(EngineError::invalid("dpi must be 10..1200"));
    }
    let mut changed = Vec::new();
    for id in &ids {
        let Item::View(v) = item_mut(s, pi, id)? else {
            continue;
        };
        if v.kind != kdraft_doc::ViewKind::Render3d {
            continue;
        }
        let mut o = v.render3d();
        match a.preset.as_deref() {
            Some("top") => {
                o.side = "top".into();
                o.rotate = None;
            }
            Some("bottom") => {
                o.side = "bottom".into();
                o.rotate = None;
            }
            Some("isometric") => {
                o.side = "top".into();
                o.rotate = Some(ISOMETRIC);
            }
            Some("isometric_bottom") => {
                o.side = "bottom".into();
                o.rotate = Some(ISOMETRIC);
            }
            Some(p) => return Err(EngineError::invalid(format!("unknown preset {p:?}"))),
            None => {}
        }
        if let Some(sd) = &a.side {
            o.side = sd.clone();
        }
        if let Some(r) = rotate {
            o.rotate = r;
        }
        if let Some(p) = a.perspective {
            o.perspective = p;
        }
        if let Some(z) = a.zoom {
            o.zoom = z as u32;
        }
        if let Some(q) = &a.quality {
            o.quality = q.clone();
        }
        if let Some(b) = &a.background {
            o.background = b.clone();
        }
        if let Some(d) = a.dpi {
            o.dpi = d as u32;
        }
        if let Some(p) = &a.appearance {
            o.appearance = Some(p.trim().to_string()).filter(|p| !p.is_empty());
        }
        if let Some(b) = a.stackup_colors {
            o.stackup_colors = b;
        }
        if let Some(b) = a.floor {
            o.floor = b;
        }
        v.set_render3d(&o);
        changed.push(id.clone());
    }
    if changed.is_empty() {
        return Err(EngineError::invalid("no 3D view among the items"));
    }
    Ok(Outcome::changed(&changed))
}

fn pts(v: &[serde_json::Value]) -> R<Vec<Point>> {
    v.iter()
        .map(|x| {
            let a: P2 = serde_json::from_value(x.clone())
                .map_err(|_| EngineError::invalid("points must be [[x, y], ...]"))?;
            p(a)
        })
        .collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PolylineArgs {
    page: Option<String>,
    points: Vec<serde_json::Value>,
    closed: Option<bool>,
}

fn add_polyline(s: &mut Session, a: PolylineArgs) -> R<Outcome> {
    let pi = page_arg(s, a.page.as_deref())?;
    let points = pts(&a.points)?;
    let closed = a.closed.unwrap_or(false);
    let min = if closed { 3 } else { 2 };
    if points.len() < min {
        return Err(EngineError::invalid(format!(
            "at least {min} points needed"
        )));
    }
    let shape = if closed {
        GraphicShape::Polygon { points }
    } else {
        GraphicShape::Polyline { points }
    };
    add_item(s, pi, graphic(shape))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CircleArgs {
    page: Option<String>,
    center: P2,
    radius: f64,
}

fn add_circle(s: &mut Session, a: CircleArgs) -> R<Outcome> {
    let pi = page_arg(s, a.page.as_deref())?;
    add_item(
        s,
        pi,
        graphic(GraphicShape::Circle {
            center: p(a.center)?,
            radius: positive(a.radius, "radius")?,
        }),
    )
}

fn graphic_mut(s: &mut Session, id: Option<String>) -> R<(Id, &mut GraphicShape)> {
    let (pi, id) = one_id(s, id)?;
    match item_mut(s, pi, &id)? {
        Item::Graphic(g) => Ok((id, &mut g.shape)),
        _ => Err(EngineError::invalid(format!("item {id} is not a graphic"))),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PointsArgs {
    id: Option<String>,
    points: Vec<serde_json::Value>,
}

fn set_points(s: &mut Session, a: PointsArgs) -> R<Outcome> {
    let v = pts(&a.points)?;
    let (id, shape) = graphic_mut(s, a.id)?;
    shape.set_points(&v).map_err(EngineError::invalid)?;
    Ok(Outcome::changed([id]))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InsertArgs {
    id: Option<String>,
    index: usize,
    at: P2,
}

fn insert_point(s: &mut Session, a: InsertArgs) -> R<Outcome> {
    let at = p(a.at)?;
    let (id, shape) = graphic_mut(s, a.id)?;
    if let GraphicShape::Line { start, end } = shape {
        *shape = GraphicShape::Polyline {
            points: vec![*start, *end],
        };
    }
    match shape {
        GraphicShape::Polyline { points } | GraphicShape::Polygon { points } => {
            if a.index > points.len() {
                return Err(EngineError::invalid(format!(
                    "index {} past the end ({} points)",
                    a.index,
                    points.len()
                )));
            }
            points.insert(a.index, at);
        }
        _ => {
            return Err(EngineError::invalid(
                "only lines, polylines and polygons have vertices",
            ))
        }
    }
    Ok(Outcome::changed([id]))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoveArgs {
    id: Option<String>,
    index: usize,
}

fn remove_point(s: &mut Session, a: RemoveArgs) -> R<Outcome> {
    let (id, shape) = graphic_mut(s, a.id)?;
    match shape {
        GraphicShape::Polyline { points } if points.len() > 2 && a.index < points.len() => {
            points.remove(a.index);
            if points.len() == 2 {
                let (st, en) = (points[0], points[1]);
                *shape = GraphicShape::Line { start: st, end: en };
            }
        }
        GraphicShape::Polygon { points } if points.len() > 3 && a.index < points.len() => {
            points.remove(a.index);
        }
        GraphicShape::Polyline { .. } | GraphicShape::Polygon { .. } => {
            return Err(EngineError::invalid(
                "cannot remove that vertex (index out of range, or too few would remain)",
            ))
        }
        _ => {
            return Err(EngineError::invalid(
                "only polylines and polygons have removable vertices",
            ))
        }
    }
    Ok(Outcome::changed([id]))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SizeArgs {
    ids: Option<Vec<String>>,
    width: Option<f64>,
    height: Option<f64>,
}

fn set_size(s: &mut Session, a: SizeArgs) -> R<Outcome> {
    let (pi, ids) = ids_arg(s, a.ids)?;
    let w = a.width.map(|v| positive(v, "width")).transpose()?;
    let h = a.height.map(|v| positive(v, "height")).transpose()?;
    if w.is_none() && h.is_none() {
        return Err(EngineError::invalid("give width or height"));
    }
    let mut changed = Vec::new();
    for id in &ids {
        let Item::Graphic(g) = item_mut(s, pi, id)? else {
            continue;
        };
        let used = match &mut g.shape {
            GraphicShape::Rect { start, end, .. } | GraphicShape::Line { start, end } => {
                let sx = if end.x >= start.x { 1.0 } else { -1.0 };
                let sy = if end.y >= start.y { 1.0 } else { -1.0 };
                if let Some(w) = w {
                    end.x = start.x + sx * w;
                }
                if let Some(h) = h {
                    end.y = start.y + sy * h;
                }
                true
            }
            GraphicShape::Circle { radius, .. } => match w.or(h) {
                Some(d) => {
                    *radius = d / 2.0;
                    true
                }
                None => false,
            },
            GraphicShape::Text { wrap, .. } => match w {
                Some(v) => {
                    *wrap = Some(v);
                    true
                }
                None => false,
            },
            _ => false,
        };
        if used {
            changed.push(id.clone());
        }
    }
    if changed.is_empty() {
        return Err(EngineError::invalid("none of the items can take this size"));
    }
    Ok(Outcome::changed(&changed))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DimAnchorArgs {
    id: Option<String>,
    end: String,
    anchor: String,
}

fn dimension_set_anchor(s: &mut Session, a: DimAnchorArgs) -> R<Outcome> {
    let anchor =
        kdraft_doc::parse_anchor(&a.anchor).map_err(|e| EngineError::invalid(e.to_string()))?;
    let board = s.board_arc();
    let (pi, id) = one_id(s, a.id)?;
    let Item::Dimension(d) = item_mut(s, pi, &id)? else {
        return Err(EngineError::invalid(format!(
            "item {id} is not a dimension"
        )));
    };
    match a.end.as_str() {
        "from" => d.from = Some(anchor),
        "to" => {
            if d.to.is_none() {
                return Err(EngineError::invalid(
                    "radius and diameter dimensions have one anchor (end: from)",
                ));
            }
            d.to = Some(anchor)
        }
        o => {
            return Err(EngineError::invalid(format!(
                "end must be from or to, got {o:?}"
            )))
        }
    }
    if let Some(b) = board {
        let holes = b.holes();
        d.last = kdraft_doc::anchor::dimension_positions(d, &b, &holes)
            .map_err(|e| EngineError::invalid(format!("anchor does not resolve: {e}")))?;
    }
    Ok(Outcome::changed([id]))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextShiftArgs {
    ids: Option<Vec<String>>,
    shift: f64,
}

fn dimension_set_text_shift(s: &mut Session, a: TextShiftArgs) -> R<Outcome> {
    if !a.shift.is_finite() {
        return Err(EngineError::invalid("shift must be finite"));
    }
    let (pi, ids) = ids_arg(s, a.ids)?;
    let mut changed = Vec::new();
    for id in &ids {
        if let Item::Dimension(d) = item_mut(s, pi, id)? {
            d.set_text_shift(a.shift);
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
struct CalloutAnchorArgs {
    id: Option<String>,
    anchor: String,
    view: Option<String>,
}

fn callout_set_anchor(s: &mut Session, a: CalloutAnchorArgs) -> R<Outcome> {
    let anchor =
        kdraft_doc::parse_anchor(&a.anchor).map_err(|e| EngineError::invalid(e.to_string()))?;
    if let Some(b) = s.board() {
        kdraft_doc::anchor::resolve(&anchor, b, &b.holes())
            .map_err(|e| EngineError::invalid(format!("anchor does not resolve: {e}")))?;
    }
    let (pi, id) = one_id(s, a.id)?;
    if let Some(v) = &a.view {
        let ok = s.doc().pages[pi]
            .item(&Id(v.clone()))
            .is_some_and(|i| matches!(i, Item::View(_)));
        if !ok {
            return Err(EngineError::invalid(format!(
                "no view {v} on the callout's page"
            )));
        }
    }
    let Item::Callout(c) = item_mut(s, pi, &id)? else {
        return Err(EngineError::invalid(format!("item {id} is not a callout")));
    };
    c.anchor = Some(anchor);
    if let Some(v) = a.view {
        c.view = Some(Id(v));
    }
    Ok(Outcome::changed([id]))
}

fn some<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextSetArgs {
    id: Option<String>,
    text: Option<String>,
    /// `Some(None)`: explicit null.
    #[serde(default, deserialize_with = "some")]
    wrap: Option<Option<f64>>,
    rotation: Option<f64>,
    at: Option<P2>,
}

/// Rotation normalised to (-180, 180].
pub(crate) fn norm_angle(a: f64) -> f64 {
    let mut r = a % 360.0;
    if r > 180.0 {
        r -= 360.0;
    } else if r <= -180.0 {
        r += 360.0;
    }
    if r.abs() < 1e-9 {
        0.0
    } else {
        r
    }
}

fn text_set(s: &mut Session, a: TextSetArgs) -> R<Outcome> {
    let (pi, id) = one_id(s, a.id)?;
    let at = a.at.map(p).transpose()?;
    let wrap = match a.wrap {
        None => None,
        Some(None) => Some(None),
        Some(Some(0.0)) => Some(None),
        Some(Some(w)) => Some(Some(positive(w, "wrap")?)),
    };
    if let Some(r) = a.rotation {
        if !r.is_finite() {
            return Err(EngineError::invalid("rotation must be finite"));
        }
    }
    if a.text.is_none() && wrap.is_none() && a.rotation.is_none() && at.is_none() {
        return Err(EngineError::invalid("nothing to change"));
    }
    let Item::Graphic(g) = item_mut(s, pi, &id)? else {
        return Err(EngineError::invalid(format!("item {id} is not a text")));
    };
    let GraphicShape::Text {
        at: a0,
        text: t0,
        rotation: r0,
        wrap: w0,
        ..
    } = &mut g.shape
    else {
        return Err(EngineError::invalid(format!("item {id} is not a text")));
    };
    if let Some(t) = a.text {
        // Newlines are stored as \n only.
        *t0 = t.replace("\r\n", "\n").replace('\r', "\n");
    }
    if let Some(w) = wrap {
        *w0 = w;
    }
    if let Some(r) = a.rotation {
        *r0 = norm_angle(r);
    }
    if let Some(v) = at {
        *a0 = v;
    }
    Ok(Outcome::changed([id]))
}
