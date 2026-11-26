//! `.kdraft` reader and canonical writer.
//!
//! The writer emits a fixed field order per node and canonical numbers;
//! optional fields are omitted when unset. Unknown nodes read from a file
//! are kept in the `extra` lists (or as [`Item::Unknown`] inside pages) and
//! written back after the known fields.

use kdraft_board::geom::{BBox, Point};
use kdraft_sexpr::{to_canonical, Node, Tree, Value};

use crate::error::DocError;
use crate::model::*;

type R<T> = Result<T, DocError>;

// ---------------------------------------------------------------- writer

fn num(v: f64) -> R<Value> {
    Value::num(v).map_err(|e| DocError::Write(e.to_string()))
}

fn node(head: &str, items: Vec<Value>) -> Value {
    Value::node(head, items)
}

fn str_node(head: &str, s: &str) -> Value {
    node(head, vec![Value::str(s)])
}

fn point_node(head: &str, p: Point) -> R<Value> {
    Ok(node(head, vec![num(p.x)?, num(p.y)?]))
}

fn push_opt_str(out: &mut Vec<Value>, head: &str, v: &Option<String>) {
    if let Some(s) = v {
        out.push(str_node(head, s));
    }
}

fn anchor_value(a: &Anchor) -> R<Value> {
    Ok(match a {
        Anchor::Pad { reference, number } => {
            node("pad", vec![Value::str(reference), Value::str(number)])
        }
        Anchor::Hole(i) => node("hole", vec![Value::int(*i as i64)]),
        Anchor::OutlineVertex(i) => node("outline-vertex", vec![Value::int(*i as i64)]),
        Anchor::FpOrigin(r) => node("fp-origin", vec![Value::str(r)]),
        Anchor::ViewPoint(p) => point_node("view-point", *p)?,
        Anchor::OutlineEdge(i) => node("outline-edge", vec![Value::int(*i as i64)]),
        Anchor::Unknown(v) => v.clone(),
    })
}

fn dim_style_value(d: &kdraft_layout::dimension::DimStyle) -> R<Option<Value>> {
    let def = kdraft_layout::dimension::DimStyle::default();
    let mut o = Vec::new();
    if d.decimals != def.decimals {
        o.push(node("decimals", vec![Value::int(d.decimals as i64)]));
    }
    if d.trim_zeros != def.trim_zeros {
        o.push(node("trim_zeros", vec![yes_no(d.trim_zeros)]));
    }
    for (k, v, dv) in [
        ("text_height", d.text_height, def.text_height),
        ("arrow_length", d.arrow_length, def.arrow_length),
        ("arrow_width", d.arrow_width, def.arrow_width),
        ("gap", d.gap, def.gap),
        ("overshoot", d.overshoot, def.overshoot),
        ("line_width", d.line_width, def.line_width),
    ] {
        if v != dv {
            o.push(node(k, vec![num(v)?]));
        }
    }
    Ok((!o.is_empty()).then(|| node("dimension", o)))
}

fn read_dim_style(n: Node<'_>) -> R<kdraft_layout::dimension::DimStyle> {
    let mut d = kdraft_layout::dimension::DimStyle::default();
    for c in n.lists() {
        match c.head() {
            Some("decimals") => d.decimals = usize::try_from(c.i64_at(1)?).unwrap_or(2).min(6),
            Some("trim_zeros") => d.trim_zeros = yes(c)?,
            Some("text_height") => d.text_height = f1(c)?,
            Some("arrow_length") => d.arrow_length = f1(c)?,
            Some("arrow_width") => d.arrow_width = f1(c)?,
            Some("gap") => d.gap = f1(c)?,
            Some("overshoot") => d.overshoot = f1(c)?,
            Some("line_width") => d.line_width = f1(c)?,
            _ => {}
        }
    }
    Ok(d)
}

fn yes_no(b: bool) -> Value {
    Value::sym(if b { "yes" } else { "no" })
}

fn assembly_value(a: &AssemblyOptions) -> R<Value> {
    let mut o = Vec::new();
    for (k, v) in [
        ("values", a.values),
        ("references", a.references),
        ("fab_other", a.fab_other),
        ("pads", a.pads),
        ("holes", a.holes),
        ("pin1", a.pin1),
    ] {
        if let Some(b) = v {
            o.push(node(k, vec![yes_no(b)]));
        }
    }
    if let Some(d) = &a.dnp {
        o.push(node("dnp", vec![Value::sym(d.as_str())]));
    }
    if let Some(r) = &a.refs {
        o.push(node("refs", vec![Value::sym(r.as_str())]));
    }
    if let Some(v) = a.text_min {
        o.push(node("text_min", vec![num(v)?]));
    }
    if let Some(v) = a.text_max {
        o.push(node("text_max", vec![num(v)?]));
    }
    o.extend(a.extra.iter().cloned());
    Ok(node("assembly", o))
}

fn item_value(item: &Item) -> R<Value> {
    let mut o = Vec::new();
    let head = item.kind_name();
    match item {
        Item::Unknown(v) => return Ok(v.clone()),
        Item::View(v) => {
            o.push(str_node("id", v.id.as_str()));
            o.push(node("kind", vec![Value::sym(v.kind.as_str())]));
            o.push(node("side", vec![Value::sym(v.side.as_str())]));
            o.push(node("scale", vec![num(v.scale)?]));
            if v.rotation != 0.0 {
                o.push(node("rotation", vec![num(v.rotation)?]));
            }
            o.push(point_node("at", v.at)?);
            o.push(node("layers", v.layers.iter().map(Value::sym).collect()));
            if let Some(c) = v.crop {
                o.push(node(
                    "crop",
                    vec![num(c.min.x)?, num(c.min.y)?, num(c.max.x)?, num(c.max.y)?],
                ));
            }
            if let Some(p) = &v.parent {
                o.push(str_node("parent", p.as_str()));
            }
            push_opt_str(&mut o, "style", &v.style);
            if v.frame {
                o.push(node("frame", vec![Value::sym("yes")]));
            }
            if let Some(d) = &v.drill_marks {
                o.push(node("drill_marks", vec![Value::sym(d.as_str())]));
            }
            if let Some(r) = &v.render {
                o.push(node("render", vec![Value::sym(r.as_str())]));
            }
            if !v.assembly.is_empty() {
                o.push(assembly_value(&v.assembly)?);
            }
            o.extend(v.extra.iter().cloned());
        }
        Item::Table(t) => {
            o.push(str_node("id", t.id.as_str()));
            o.push(node("kind", vec![Value::sym(t.kind.as_str())]));
            o.push(point_node("at", t.at)?);
            if !t.columns.is_empty() {
                o.push(node("columns", t.columns.iter().map(Value::str).collect()));
            }
            push_opt_str(&mut o, "style", &t.style);
            if let Some((a, n)) = t.rows {
                o.push(node(
                    "rows",
                    vec![Value::int(a as i64), Value::int(n as i64)],
                ));
            }
            if !t.bom_items.is_empty() {
                let mut items: Vec<Value> = Vec::new();
                if let Some(src) = &t.bom_source {
                    items.push(node("source", vec![Value::sym(src)]));
                }
                items.extend(
                    t.bom_items
                        .iter()
                        .map(|(k, n)| node("item", vec![Value::str(k), Value::int(*n as i64)])),
                );
                o.push(node("items", items));
            }
            o.extend(t.extra.iter().cloned());
        }
        Item::Dimension(d) => {
            o.push(str_node("id", d.id.as_str()));
            o.push(node("kind", vec![Value::sym(d.kind.as_str())]));
            if let Some(v) = &d.view {
                o.push(str_node("view", v.as_str()));
            }
            if let Some(a) = &d.from {
                o.push(node("from", vec![anchor_value(a)?]));
            }
            if let Some(a) = &d.to {
                o.push(node("to", vec![anchor_value(a)?]));
            }
            o.push(node("offset", vec![num(d.offset)?]));
            push_opt_str(&mut o, "style", &d.style);
            push_opt_str(&mut o, "tolerance", &d.tolerance);
            if let Some(a) = &d.axis {
                o.push(node("axis", vec![Value::sym(a)]));
            }
            if let Some(a) = d.angle {
                o.push(node("angle", vec![num(a)?]));
            }
            if !d.last.is_empty() {
                let pts = d
                    .last
                    .iter()
                    .map(|p| point_node("xy", *p))
                    .collect::<R<Vec<_>>>()?;
                o.push(node("last", pts));
            }
            o.extend(d.extra.iter().cloned());
        }
        Item::Callout(c) => {
            o.push(str_node("id", c.id.as_str()));
            o.push(point_node("at", c.at)?);
            if let Some(v) = &c.view {
                o.push(str_node("view", v.as_str()));
            }
            if let Some(a) = &c.anchor {
                o.push(node("anchor", vec![anchor_value(a)?]));
            }
            // Commands keep one of them; a file with both keeps both.
            push_opt_str(&mut o, "label", &c.label);
            push_opt_str(&mut o, "target", &c.target);
            if !c.leader.is_empty() {
                let pts = c
                    .leader
                    .iter()
                    .map(|p| point_node("xy", *p))
                    .collect::<R<Vec<_>>>()?;
                o.push(node("leader", pts));
            }
            o.extend(c.extra.iter().cloned());
        }
        Item::NoteList(n) => {
            o.push(str_node("id", n.id.as_str()));
            o.push(point_node("at", n.at)?);
            push_opt_str(&mut o, "title", &n.title);
            if let Some(w) = n.width {
                o.push(node("width", vec![num(w)?]));
            }
            for note in &n.notes {
                o.push(node(
                    "note",
                    vec![Value::str(&note.id), Value::str(&note.text)],
                ));
            }
            o.extend(n.extra.iter().cloned());
        }
        Item::Graphic(g) => {
            o.push(str_node("id", g.id.as_str()));
            o.push(shape_value(&g.shape)?);
            push_opt_str(&mut o, "style", &g.style);
            o.extend(g.extra.iter().cloned());
        }
    }
    Ok(node(head, o))
}

fn shape_value(s: &GraphicShape) -> R<Value> {
    Ok(match s {
        GraphicShape::Line { start, end } => node(
            "line",
            vec![point_node("start", *start)?, point_node("end", *end)?],
        ),
        GraphicShape::Rect { start, end, radius } => {
            let mut v = vec![point_node("start", *start)?, point_node("end", *end)?];
            if *radius > 0.0 {
                v.push(node("radius", vec![num(*radius)?]));
            }
            node("rect", v)
        }
        GraphicShape::Circle { center, radius } => node(
            "circle",
            vec![
                point_node("center", *center)?,
                node("radius", vec![num(*radius)?]),
            ],
        ),
        GraphicShape::Arc { start, mid, end } => node(
            "arc",
            vec![
                point_node("start", *start)?,
                point_node("mid", *mid)?,
                point_node("end", *end)?,
            ],
        ),
        GraphicShape::Polyline { points } => node(
            "polyline",
            vec![node(
                "pts",
                points
                    .iter()
                    .map(|p| point_node("xy", *p))
                    .collect::<R<Vec<_>>>()?,
            )],
        ),
        GraphicShape::Polygon { points } => node(
            "polygon",
            vec![node(
                "pts",
                points
                    .iter()
                    .map(|p| point_node("xy", *p))
                    .collect::<R<Vec<_>>>()?,
            )],
        ),
        GraphicShape::Text {
            at,
            text,
            height,
            rotation,
            wrap,
        } => {
            let mut v = vec![Value::str(text), point_node("at", *at)?];
            if let Some(h) = height {
                v.push(node("height", vec![num(*h)?]));
            }
            if *rotation != 0.0 {
                v.push(node("rotation", vec![num(*rotation)?]));
            }
            if let Some(w) = wrap {
                v.push(node("wrap", vec![num(*w)?]));
            }
            node("text", v)
        }
        GraphicShape::Image {
            at,
            path,
            width,
            height,
        } => node(
            "image",
            vec![
                Value::str(path),
                point_node("at", *at)?,
                node("size", vec![num(*width)?, num(*height)?]),
            ],
        ),
    })
}

pub(crate) fn to_value(doc: &Document) -> R<Value> {
    let mut o = vec![node("version", vec![Value::int(i64::from(doc.version))])];

    let s = &doc.source;
    let mut src = vec![str_node("board", &s.board)];
    push_opt_str(&mut src, "variant", &s.variant);
    push_opt_str(&mut src, "kicad_cli", &s.kicad_cli);
    push_opt_str(&mut src, "board_hash", &s.board_hash);
    src.extend(s.extra.iter().cloned());
    o.push(node("source", src));

    let st = &doc.styles;
    let mut styles = vec![
        node("text_height", vec![num(st.text_height)?]),
        node("line_width", vec![num(st.line_width)?]),
        node("snap_grid", vec![num(st.snap_grid)?]),
    ];
    if let Some(d) = dim_style_value(&st.dimension)? {
        styles.push(d);
    }
    if let Some(a) = st.appearance.to_node() {
        styles.push(a);
    }
    if st.callouts_per_document {
        styles.push(node("callout_numbering", vec![Value::sym("document")]));
    }
    if let Some(p) = &st.layer_palette {
        styles.push(p.to_value());
    }
    if st.strict_callout_numbering {
        styles.push(node("strict_callout_numbering", vec![Value::sym("yes")]));
    }
    styles.extend(st.extra.iter().cloned());
    o.push(node("styles", styles));
    push_opt_str(&mut o, "sheet", &doc.sheet);

    for p in &doc.pages {
        let mut pv = vec![str_node("id", p.id.as_str())];
        push_opt_str(&mut pv, "title", &p.title);
        push_opt_str(&mut pv, "sheet", &p.sheet);
        pv.push(node(
            "size",
            vec![
                Value::sym(&p.size.name),
                num(p.size.width)?,
                num(p.size.height)?,
            ],
        ));
        for it in &p.items {
            pv.push(item_value(it)?);
        }
        o.push(node("page", pv));
    }
    o.extend(doc.extra.iter().cloned());
    Ok(node("kdraft", o))
}

pub(crate) fn to_text(doc: &Document) -> R<String> {
    Ok(to_canonical(&[to_value(doc)?]))
}

// ---------------------------------------------------------------- reader

fn s1(n: Node<'_>) -> R<String> {
    Ok(n.str_at(1)?.into_owned())
}

fn f1(n: Node<'_>) -> R<f64> {
    Ok(n.f64_at(1)?)
}

fn pt(n: Node<'_>) -> R<Point> {
    Ok(Point::new(n.f64_at(1)?, n.f64_at(2)?))
}

fn yes(n: Node<'_>) -> R<bool> {
    Ok(matches!(n.str_at(1)?.as_ref(), "yes" | "true"))
}

/// Children after the head, with a "seen" set so that only the first
/// occurrence of a known field is read; repeats and unknowns go to `extra`.
struct Fields<'a> {
    node: Node<'a>,
}

impl<'a> Fields<'a> {
    fn new(node: Node<'a>) -> Self {
        Fields { node }
    }

    fn each(
        &self,
        known: &[&str],
        mut f: impl FnMut(&str, Node<'a>) -> R<()>,
        extra: &mut Vec<Value>,
    ) -> R<()> {
        let mut seen: Vec<&str> = Vec::new();
        for c in self.node.children().skip(1) {
            match c.head() {
                Some(h) if c.is_list() && known.contains(&h) && !seen.contains(&h) => {
                    seen.push(h);
                    f(h, c)?;
                }
                _ => extra.push(c.to_value()),
            }
        }
        Ok(())
    }
}

fn require_id(id: Option<String>, what: &str, n: Node<'_>) -> R<Id> {
    match id {
        Some(s) if !s.is_empty() => Ok(Id(s)),
        _ => {
            let (line, column) = n.line_col();
            Err(DocError::Missing {
                what: format!("{what} (id ...)"),
                line,
                column,
            })
        }
    }
}

fn read_anchor(holder: Node<'_>) -> R<Option<Anchor>> {
    let Some(a) = holder.lists().next() else {
        return Ok(None);
    };
    Ok(Some(match a.head() {
        Some("pad") => Anchor::Pad {
            reference: a.str_at(1)?.into_owned(),
            number: a.str_at(2)?.into_owned(),
        },
        Some("hole") => Anchor::Hole(usize::try_from(a.i64_at(1)?).unwrap_or(usize::MAX)),
        Some("outline-vertex") => {
            Anchor::OutlineVertex(usize::try_from(a.i64_at(1)?).unwrap_or(usize::MAX))
        }
        Some("fp-origin") => Anchor::FpOrigin(a.str_at(1)?.into_owned()),
        Some("view-point") => Anchor::ViewPoint(pt(a)?),
        Some("outline-edge") => {
            Anchor::OutlineEdge(usize::try_from(a.i64_at(1)?).unwrap_or(usize::MAX))
        }
        _ => Anchor::Unknown(a.to_value()),
    }))
}

/// Parses one anchor written as an S-expression, e.g. `(pad "U7" "3")`.
pub fn parse_anchor(text: &str) -> R<Anchor> {
    let wrapped = format!("(a {text})");
    let tree = Tree::parse(wrapped.as_str())?;
    let root = tree.require_root()?;
    match read_anchor(root)? {
        Some(Anchor::Unknown(_)) | None => Err(DocError::Write(format!(
            "unknown anchor {text:?}; expected (pad \"REF\" \"N\"), (hole N), (outline-vertex N), (outline-edge N), (fp-origin \"REF\") or (view-point X Y)"
        ))),
        Some(a) => Ok(a),
    }
}

/// An anchor as S-expression text.
pub fn anchor_text(a: &Anchor) -> R<String> {
    Ok(to_canonical(&[anchor_value(a)?]).trim().to_string())
}

fn read_view(n: Node<'_>) -> R<View> {
    let mut id = None;
    let mut v = View {
        id: Id(String::new()),
        kind: ViewKind::Fabrication,
        side: ViewSide::Front,
        scale: 1.0,
        rotation: 0.0,
        at: Point::ORIGIN,
        layers: Vec::new(),
        crop: None,
        parent: None,
        style: None,
        frame: false,
        drill_marks: None,
        render: None,
        assembly: AssemblyOptions::default(),
        extra: Vec::new(),
    };
    let mut extra = Vec::new();
    let known = [
        "id",
        "kind",
        "side",
        "scale",
        "rotation",
        "at",
        "layers",
        "crop",
        "parent",
        "style",
        "frame",
        "drill_marks",
        "render",
        "assembly",
    ];
    Fields::new(n).each(
        &known,
        |h, c| {
            match h {
                "id" => id = Some(s1(c)?),
                "kind" => v.kind = ViewKind::parse(&s1(c)?),
                "side" => {
                    v.side = if s1(c)? == "back" {
                        ViewSide::Back
                    } else {
                        ViewSide::Front
                    }
                }
                "scale" => v.scale = f1(c)?,
                "rotation" => v.rotation = f1(c)?,
                "at" => v.at = pt(c)?,
                "layers" => {
                    v.layers = c
                        .children()
                        .skip(1)
                        .filter_map(|a| a.atom().map(|s| s.into_owned()))
                        .collect()
                }
                "crop" => {
                    v.crop = Some(BBox {
                        min: Point::new(c.f64_at(1)?, c.f64_at(2)?),
                        max: Point::new(c.f64_at(3)?, c.f64_at(4)?),
                    })
                }
                "parent" => v.parent = Some(Id(s1(c)?)),
                "style" => v.style = Some(s1(c)?),
                "frame" => v.frame = yes(c)?,
                "drill_marks" => v.drill_marks = Some(DrillMarks::parse(&s1(c)?)),
                "render" => v.render = Some(RenderMode::parse(&s1(c)?)),
                "assembly" => v.assembly = read_assembly(c)?,
                _ => {}
            }
            Ok(())
        },
        &mut extra,
    )?;
    v.id = require_id(id, "view", n)?;
    v.extra = extra;
    Ok(v)
}

fn read_assembly(n: Node<'_>) -> R<AssemblyOptions> {
    let mut a = AssemblyOptions::default();
    let mut extra = Vec::new();
    Fields::new(n).each(
        &[
            "values",
            "references",
            "fab_other",
            "pads",
            "holes",
            "pin1",
            "dnp",
            "refs",
            "text_min",
            "text_max",
        ],
        |h, c| {
            match h {
                "values" => a.values = Some(yes(c)?),
                "references" => a.references = Some(yes(c)?),
                "fab_other" => a.fab_other = Some(yes(c)?),
                "pads" => a.pads = Some(yes(c)?),
                "holes" => a.holes = Some(yes(c)?),
                "pin1" => a.pin1 = Some(yes(c)?),
                "dnp" => a.dnp = Some(DnpStyle::parse(&s1(c)?)),
                "refs" => a.refs = Some(RefFilter::parse(&s1(c)?)),
                "text_min" => a.text_min = Some(f1(c)?),
                "text_max" => a.text_max = Some(f1(c)?),
                _ => {}
            }
            Ok(())
        },
        &mut extra,
    )?;
    a.extra = extra;
    Ok(a)
}

fn read_table(n: Node<'_>) -> R<Table> {
    let mut id = None;
    let mut t = Table {
        id: Id(String::new()),
        kind: TableKind::Custom,
        at: Point::ORIGIN,
        columns: Vec::new(),
        style: None,
        bom_items: Vec::new(),
        bom_source: None,
        rows: None,
        extra: Vec::new(),
    };
    let mut extra = Vec::new();
    Fields::new(n).each(
        &["id", "kind", "at", "columns", "style", "rows", "items"],
        |h, c| {
            match h {
                "id" => id = Some(s1(c)?),
                "kind" => t.kind = TableKind::parse(&s1(c)?),
                "at" => t.at = pt(c)?,
                "columns" => {
                    t.columns = c
                        .children()
                        .skip(1)
                        .filter_map(|a| a.atom().map(|s| s.into_owned()))
                        .collect()
                }
                "style" => t.style = Some(s1(c)?),
                "rows" => {
                    let a = usize::try_from(c.i64_at(1)?).unwrap_or(0);
                    let k = usize::try_from(c.i64_at(2)?).unwrap_or(0);
                    t.rows = Some((a, k));
                }
                "items" => {
                    if let Some(src) = c.find("source") {
                        t.bom_source = Some(s1(src)?);
                    }
                    for it in c.find_all("item") {
                        t.bom_items.push((
                            it.str_at(1)?.into_owned(),
                            usize::try_from(it.i64_at(2)?).unwrap_or(0),
                        ));
                    }
                }
                _ => {}
            }
            Ok(())
        },
        &mut extra,
    )?;
    t.id = require_id(id, "table", n)?;
    t.extra = extra;
    Ok(t)
}

fn read_dimension(n: Node<'_>) -> R<Dimension> {
    let mut id = None;
    let mut d = Dimension {
        id: Id(String::new()),
        kind: DimensionKind::Linear,
        view: None,
        from: None,
        to: None,
        offset: 0.0,
        style: None,
        tolerance: None,
        axis: None,
        angle: None,
        last: Vec::new(),
        extra: Vec::new(),
    };
    let mut extra = Vec::new();
    Fields::new(n).each(
        &[
            "id",
            "kind",
            "view",
            "from",
            "to",
            "offset",
            "style",
            "tolerance",
            "axis",
            "angle",
            "last",
        ],
        |h, c| {
            match h {
                "id" => id = Some(s1(c)?),
                "kind" => d.kind = DimensionKind::parse(&s1(c)?),
                "view" => d.view = Some(Id(s1(c)?)),
                "from" => d.from = read_anchor(c)?,
                "to" => d.to = read_anchor(c)?,
                "offset" => d.offset = f1(c)?,
                "style" => d.style = Some(s1(c)?),
                "tolerance" => d.tolerance = Some(s1(c)?),
                "axis" => d.axis = Some(s1(c)?),
                "angle" => d.angle = Some(f1(c)?),
                "last" => d.last = c.find_all("xy").map(pt).collect::<R<Vec<_>>>()?,
                _ => {}
            }
            Ok(())
        },
        &mut extra,
    )?;
    d.id = require_id(id, "dimension", n)?;
    d.extra = extra;
    Ok(d)
}

fn read_callout(n: Node<'_>) -> R<Callout> {
    let mut id = None;
    let mut co = Callout {
        id: Id(String::new()),
        at: Point::ORIGIN,
        view: None,
        anchor: None,
        label: None,
        target: None,
        leader: Vec::new(),
        extra: Vec::new(),
    };
    let mut extra = Vec::new();
    Fields::new(n).each(
        &["id", "at", "view", "anchor", "label", "target", "leader"],
        |h, c| {
            match h {
                "id" => id = Some(s1(c)?),
                "at" => co.at = pt(c)?,
                "view" => co.view = Some(Id(s1(c)?)),
                "anchor" => co.anchor = read_anchor(c)?,
                "label" => co.label = Some(s1(c)?),
                "target" => co.target = Some(s1(c)?),
                "leader" => {
                    co.leader = c.find_all("xy").map(pt).collect::<R<Vec<_>>>()?;
                }
                _ => {}
            }
            Ok(())
        },
        &mut extra,
    )?;
    co.id = require_id(id, "callout", n)?;
    co.extra = extra;
    Ok(co)
}

fn read_notes(n: Node<'_>) -> R<NoteList> {
    let mut id = None;
    let mut at = Point::ORIGIN;
    let mut title = None;
    let mut width = None;
    let mut notes = Vec::new();
    let mut extra = Vec::new();
    for c in n.children().skip(1) {
        match c.head() {
            Some("id") if c.is_list() && id.is_none() => id = Some(s1(c)?),
            Some("at") if c.is_list() => at = pt(c)?,
            Some("title") if c.is_list() && title.is_none() => title = Some(s1(c)?),
            Some("width") if c.is_list() && width.is_none() => width = Some(f1(c)?),
            Some("note") if c.is_list() => notes.push(Note {
                id: c.str_at(1)?.into_owned(),
                text: c.str_at(2)?.into_owned(),
            }),
            _ => extra.push(c.to_value()),
        }
    }
    Ok(NoteList {
        id: require_id(id, "notes", n)?,
        at,
        title,
        width,
        notes,
        extra,
    })
}

fn read_shape(c: Node<'_>) -> R<Option<GraphicShape>> {
    let p = |h: &str| -> R<Point> { pt(c.require(h)?) };
    Ok(Some(match c.head() {
        Some("line") => GraphicShape::Line {
            start: p("start")?,
            end: p("end")?,
        },
        Some("rect") => GraphicShape::Rect {
            start: p("start")?,
            end: p("end")?,
            radius: match c.find("radius") {
                Some(r) => f1(r)?.max(0.0),
                None => 0.0,
            },
        },
        Some("circle") => GraphicShape::Circle {
            center: p("center")?,
            radius: f1(c.require("radius")?)?,
        },
        Some("arc") => GraphicShape::Arc {
            start: p("start")?,
            mid: p("mid")?,
            end: p("end")?,
        },
        Some("polyline") => GraphicShape::Polyline {
            points: c
                .require("pts")?
                .find_all("xy")
                .map(pt)
                .collect::<R<Vec<_>>>()?,
        },
        Some("polygon") => GraphicShape::Polygon {
            points: c
                .require("pts")?
                .find_all("xy")
                .map(pt)
                .collect::<R<Vec<_>>>()?,
        },
        Some("text") => GraphicShape::Text {
            text: c.str_at(1)?.into_owned(),
            at: p("at")?,
            height: match c.find("height") {
                Some(h) => Some(f1(h)?),
                None => None,
            },
            rotation: match c.find("rotation") {
                Some(r) => f1(r)?,
                None => 0.0,
            },
            wrap: match c.find("wrap") {
                Some(w) => Some(f1(w)?).filter(|w| *w > 0.0),
                None => None,
            },
        },
        Some("image") => {
            let size = c.require("size")?;
            GraphicShape::Image {
                path: c.str_at(1)?.into_owned(),
                at: p("at")?,
                width: size.f64_at(1)?,
                height: size.f64_at(2)?,
            }
        }
        _ => return Ok(None),
    }))
}

const SHAPES: [&str; 8] = [
    "line", "rect", "circle", "arc", "polygon", "polyline", "text", "image",
];

fn read_graphic(n: Node<'_>) -> R<Graphic> {
    let mut id = None;
    let mut shape = None;
    let mut style = None;
    let mut extra = Vec::new();
    for c in n.children().skip(1) {
        match c.head() {
            Some("id") if c.is_list() && id.is_none() => id = Some(s1(c)?),
            Some("style") if c.is_list() && style.is_none() => style = Some(s1(c)?),
            Some(h) if c.is_list() && shape.is_none() && SHAPES.contains(&h) => {
                shape = read_shape(c)?
            }
            _ => extra.push(c.to_value()),
        }
    }
    let id = require_id(id, "graphic", n)?;
    let Some(shape) = shape else {
        let (line, column) = n.line_col();
        return Err(DocError::Missing {
            what: "graphic shape".to_string(),
            line,
            column,
        });
    };
    Ok(Graphic {
        id,
        shape,
        style,
        extra,
    })
}

fn read_item(c: Node<'_>) -> R<Item> {
    if !c.is_list() {
        return Ok(Item::Unknown(c.to_value()));
    }
    let mut it = match c.head() {
        Some("view") => Item::View(read_view(c)?),
        Some("table") => Item::Table(read_table(c)?),
        Some("dimension") => Item::Dimension(read_dimension(c)?),
        Some("callout") => Item::Callout(read_callout(c)?),
        Some("notes") => Item::NoteList(read_notes(c)?),
        Some("graphic") => Item::Graphic(read_graphic(c)?),
        _ => Item::Unknown(c.to_value()),
    };
    // Tolerant read, canonical write: one appearance node in canonical
    // attribute order.
    let over = it.style_override();
    if !over.is_empty()
        || it
            .extra_values()
            .is_some_and(|e| e.iter().any(|v| v.is(crate::style::APPEARANCE)))
    {
        it.set_style_override(&over);
    }
    if let Some(l) = it.link() {
        it.set_link(Some(&l));
    }
    if let Item::View(v) = &mut it {
        if v.extra.iter().any(|x| x.is(crate::layers::LAYER_COLORS)) {
            let lc = v.layer_colors();
            v.set_layer_colors(&lc);
        }
        if v.extra.iter().any(|x| x.is(crate::render3d::RENDER3D)) {
            let r = v.render3d();
            v.set_render3d(&r);
        }
    }
    Ok(it)
}

fn read_page(n: Node<'_>) -> R<Page> {
    let mut id = None;
    let mut title = None;
    let mut sheet = None;
    let mut size = None;
    let mut items = Vec::new();
    for c in n.children().skip(1) {
        match c.head() {
            Some("id") if c.is_list() && id.is_none() => id = Some(s1(c)?),
            Some("title") if c.is_list() && title.is_none() => title = Some(s1(c)?),
            Some("sheet") if c.is_list() && sheet.is_none() => sheet = Some(s1(c)?),
            Some("size") if c.is_list() && size.is_none() => {
                size = Some(PageSize {
                    name: s1(c)?,
                    width: c.f64_at(2)?,
                    height: c.f64_at(3)?,
                })
            }
            _ => items.push(read_item(c)?),
        }
    }
    Ok(Page {
        id: require_id(id, "page", n)?,
        sheet,
        size: size.unwrap_or_else(PageSize::a3_landscape),
        title,
        items,
    })
}

pub(crate) fn parse(text: &str) -> R<Document> {
    let tree = Tree::parse(text)?;
    let root = tree.require_root()?;
    if root.head() != Some("kdraft") {
        return Err(DocError::NotADocument {
            found: root.head().map(str::to_string),
        });
    }
    let version = match root.find("version") {
        Some(v) => v.i64_at(1)?,
        None => {
            let (line, column) = root.line_col();
            return Err(DocError::Missing {
                what: "(version ...)".to_string(),
                line,
                column,
            });
        }
    };
    let version = u32::try_from(version).unwrap_or(u32::MAX);
    if version == 0 || version > CURRENT_VERSION {
        return Err(DocError::UnsupportedVersion {
            found: version,
            supported: CURRENT_VERSION,
        });
    }
    // Migrations from older versions would run here, on the tree, before
    // reading. Version 1 is the first format, so there are none yet.

    let mut doc = Document {
        version,
        source: Source::default(),
        styles: Styles::default(),
        sheet: None,
        pages: Vec::new(),
        extra: Vec::new(),
    };
    let mut have_sheet = false;
    let mut have_version = false;
    let mut have_source = false;
    let mut have_styles = false;
    for c in root.children().skip(1) {
        match c.head() {
            Some("version") if c.is_list() && !have_version => have_version = true,
            Some("source") if c.is_list() && !have_source => {
                have_source = true;
                let mut board = None;
                let mut extra = Vec::new();
                let s = &mut doc.source;
                Fields::new(c).each(
                    &["board", "variant", "kicad_cli", "board_hash"],
                    |h, f| {
                        match h {
                            "board_hash" => s.board_hash = Some(s1(f)?),
                            "board" => board = Some(s1(f)?),
                            "variant" => s.variant = Some(s1(f)?),
                            "kicad_cli" => s.kicad_cli = Some(s1(f)?),
                            _ => {}
                        }
                        Ok(())
                    },
                    &mut extra,
                )?;
                s.extra = extra;
                s.board = match board {
                    Some(b) => b,
                    None => {
                        let (line, column) = c.line_col();
                        return Err(DocError::Missing {
                            what: "(source (board ...))".to_string(),
                            line,
                            column,
                        });
                    }
                };
            }
            Some("styles") if c.is_list() && !have_styles => {
                have_styles = true;
                let mut extra = Vec::new();
                let st = &mut doc.styles;
                Fields::new(c).each(
                    &[
                        "text_height",
                        "line_width",
                        "snap_grid",
                        "dimension",
                        crate::style::APPEARANCE,
                        "callout_numbering",
                        "strict_callout_numbering",
                        crate::palette::LAYER_PALETTE,
                    ],
                    |h, f| {
                        match h {
                            "text_height" => st.text_height = f1(f)?,
                            "line_width" => st.line_width = f1(f)?,
                            "snap_grid" => st.snap_grid = f1(f)?,
                            "dimension" => st.dimension = read_dim_style(f)?,
                            "callout_numbering" => st.callouts_per_document = s1(f)? == "document",
                            crate::palette::LAYER_PALETTE => {
                                st.layer_palette =
                                    Some(crate::palette::Palette::from_value(&f.to_value()))
                            }
                            "strict_callout_numbering" => {
                                st.strict_callout_numbering = s1(f)? == "yes"
                            }
                            crate::style::APPEARANCE => {
                                st.appearance = crate::style::StyleSet::from_value(&f.to_value())
                            }
                            _ => {}
                        }
                        Ok(())
                    },
                    &mut extra,
                )?;
                st.extra = extra;
                crate::layers::migrate_doc_preset(st);
            }
            Some("sheet") if c.is_list() && !have_sheet => {
                have_sheet = true;
                doc.sheet = Some(s1(c)?);
            }
            Some("page") if c.is_list() => doc.pages.push(read_page(c)?),
            _ => doc.extra.push(c.to_value()),
        }
    }
    if !have_source {
        return Err(DocError::Missing {
            what: "(source ...)".to_string(),
            line: 1,
            column: 1,
        });
    }
    Ok(doc)
}
