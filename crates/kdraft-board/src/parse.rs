//! Reads the board model from a parsed `.kicad_pcb` tree.
//!
//! Only the subset kdraft needs is read; unknown nodes are skipped. Values
//! kdraft uses must be well formed, otherwise a positioned error is returned.

use std::collections::{BTreeMap, HashMap};

use kdraft_sexpr::{Node, NodeKind, Tree};

use crate::error::BoardError;
use crate::geom::{normalize_deg, union_opt, BBox, Placement, Point};
use crate::model::*;
use crate::outline::{
    EdgePrimitive, EdgeShape, EdgeSource, Outline, PolyItem, DEFAULT_CHAIN_TOLERANCE,
};

type R<T> = Result<T, BoardError>;

pub(crate) fn read_board(tree: &Tree, hash: BoardHash) -> R<Board> {
    let root = tree.require_root()?;
    if root.head() != Some("kicad_pcb") {
        return Err(BoardError::NotABoard {
            found: root.head().map(str::to_string),
        });
    }
    let mut warnings = Vec::new();
    let version = match root.find("version") {
        Some(v) => v.i64_at(1)?,
        None => {
            warnings.push("file has no (version ...)".to_string());
            0
        }
    };
    let nets = read_net_table(root)?;

    let mut footprints = Vec::new();
    let mut vias = Vec::new();
    let mut edges = Vec::new();
    for item in root.lists() {
        match item.head() {
            Some("footprint") | Some("module") => {
                let index = footprints.len();
                let (fp, fp_edges) = read_footprint(item, &nets, index, &mut warnings)?;
                footprints.push(fp);
                edges.extend(fp_edges);
            }
            Some("via") => vias.push(read_via(item, &nets, &mut warnings)?),
            Some(h) if h.starts_with("gr_") && layer_of(item)?.as_deref() == Some("Edge.Cuts") => {
                if let Some(shape) = read_shape(item, &h[3..], None, &mut warnings)? {
                    edges.push(EdgePrimitive {
                        shape,
                        source: EdgeSource::Board,
                        uuid: opt_str(item, "uuid")?,
                    });
                }
            }
            _ => {}
        }
    }

    let general = root.find("general");
    Ok(Board {
        version,
        generator: opt_str(root, "generator")?,
        generator_version: opt_str(root, "generator_version")?,
        thickness: match general {
            Some(g) => opt_f64(g, "thickness")?,
            None => None,
        },
        paper: opt_str(root, "paper")?,
        layers: read_layers(root)?,
        title_block: read_title_block(root)?,
        stackup: match root.find("setup").and_then(|s| s.find("stackup")) {
            Some(s) => Some(read_stackup(s)?),
            None => None,
        },
        footprints,
        vias,
        outline: Outline::build(edges, DEFAULT_CHAIN_TOLERANCE),
        hash,
        warnings,
    })
}

fn opt_str(node: Node<'_>, head: &str) -> R<Option<String>> {
    match node.find(head) {
        Some(n) if n.len() > 1 => Ok(Some(n.str_at(1)?.into_owned())),
        _ => Ok(None),
    }
}

fn opt_f64(node: Node<'_>, head: &str) -> R<Option<f64>> {
    match node.find(head) {
        Some(n) if n.len() > 1 => Ok(Some(n.f64_at(1)?)),
        _ => Ok(None),
    }
}

fn opt_bool(node: Node<'_>, head: &str) -> R<Option<bool>> {
    Ok(opt_str(node, head)?.map(|s| matches!(s.as_str(), "yes" | "true")))
}

/// `(head x y)` as a point.
fn point(n: Node<'_>) -> R<Point> {
    Ok(Point::new(n.f64_at(1)?, n.f64_at(2)?))
}

fn req_point(node: Node<'_>, head: &str) -> R<Point> {
    point(node.require(head)?)
}

/// `(at x y [angle])`; KiCad defaults a missing `at` to the origin.
fn at(node: Node<'_>) -> R<(Point, f64)> {
    match node.find("at") {
        Some(a) => {
            let angle = if a.len() > 3 { a.f64_at(3)? } else { 0.0 };
            Ok((point(a)?, angle))
        }
        None => Ok((Point::ORIGIN, 0.0)),
    }
}

fn layer_of(node: Node<'_>) -> R<Option<String>> {
    opt_str(node, "layer")
}

/// Atoms after the head, decoded.
fn atoms(node: Node<'_>) -> Vec<String> {
    node.children()
        .skip(1)
        .filter_map(|c| c.atom().map(|a| a.into_owned()))
        .collect()
}

fn read_net_table(root: Node<'_>) -> R<HashMap<i64, String>> {
    let mut nets = HashMap::new();
    for n in root.find_all("net") {
        if n.len() >= 3 {
            nets.insert(n.i64_at(1)?, n.str_at(2)?.into_owned());
        }
    }
    Ok(nets)
}

/// `(net 3 "GND")` (KiCad 9), `(net "GND")` (KiCad 10) or `(net 3)`.
fn net_of(node: Node<'_>, nets: &HashMap<i64, String>) -> R<Option<String>> {
    let Some(n) = node.find("net") else {
        return Ok(None);
    };
    let name = match n.len() {
        0 | 1 => None,
        2 => {
            let c = n.child(1);
            if c.is_some_and(|c| c.kind() == NodeKind::String) {
                Some(n.str_at(1)?.into_owned())
            } else {
                nets.get(&n.i64_at(1)?).cloned()
            }
        }
        _ => Some(n.str_at(2)?.into_owned()),
    };
    Ok(name.filter(|s| !s.is_empty()))
}

fn read_layers(root: Node<'_>) -> R<Vec<Layer>> {
    let Some(layers) = root.find("layers") else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for l in layers.lists() {
        out.push(Layer {
            ordinal: l.i64_at(0)?,
            name: l.str_at(1)?.into_owned(),
            kind: l.str_at(2)?.into_owned(),
            user_name: if l.len() > 3 {
                Some(l.str_at(3)?.into_owned())
            } else {
                None
            },
        });
    }
    Ok(out)
}

fn read_title_block(root: Node<'_>) -> R<TitleBlock> {
    let Some(tb) = root.find("title_block") else {
        return Ok(TitleBlock::default());
    };
    let mut comments = BTreeMap::new();
    for c in tb.find_all("comment") {
        let k = c.i64_at(1)?;
        comments.insert(u32::try_from(k).unwrap_or(0), c.str_at(2)?.into_owned());
    }
    Ok(TitleBlock {
        title: opt_str(tb, "title")?,
        date: opt_str(tb, "date")?,
        rev: opt_str(tb, "rev")?,
        company: opt_str(tb, "company")?,
        comments,
    })
}

fn read_stackup(s: Node<'_>) -> R<Stackup> {
    let mut layers = Vec::new();
    for l in s.find_all("layer") {
        let name = l.str_at(1)?.into_owned();
        // Properties up to each `addsublayer` atom belong to one sublayer.
        let mut current = StackupLayer {
            name: name.clone(),
            ..Default::default()
        };
        let mut sub = 0u32;
        for c in l.children().skip(2) {
            if c.kind() == NodeKind::Symbol && c.raw() == "addsublayer" {
                let kind = current.kind.clone();
                layers.push(std::mem::take(&mut current));
                sub += 1;
                current = StackupLayer {
                    name: name.clone(),
                    kind,
                    sublayer: sub,
                    ..Default::default()
                };
                continue;
            }
            if !c.is_list() || c.len() < 2 {
                continue;
            }
            match c.head() {
                Some("type") => current.kind = c.str_at(1)?.into_owned(),
                Some("thickness") => current.thickness = Some(c.f64_at(1)?),
                Some("material") => current.material = Some(c.str_at(1)?.into_owned()),
                Some("epsilon_r") => current.epsilon_r = Some(c.f64_at(1)?),
                Some("loss_tangent") => current.loss_tangent = Some(c.f64_at(1)?),
                Some("color") => current.color = Some(c.str_at(1)?.into_owned()),
                _ => {}
            }
        }
        layers.push(current);
    }
    Ok(Stackup {
        layers,
        copper_finish: opt_str(s, "copper_finish")?,
        dielectric_constraints: opt_bool(s, "dielectric_constraints")?,
        edge_connector: opt_str(s, "edge_connector")?,
        castellated_pads: opt_bool(s, "castellated_pads")?,
        edge_plating: opt_bool(s, "edge_plating")?,
    })
}

fn read_footprint(
    fp: Node<'_>,
    nets: &HashMap<i64, String>,
    index: usize,
    warnings: &mut Vec<String>,
) -> R<(Footprint, Vec<EdgePrimitive>)> {
    let lib_id = fp.str_at(1)?.into_owned();
    let layer = layer_of(fp)?.unwrap_or_else(|| "F.Cu".to_string());
    let (position, rotation) = at(fp)?;
    let place = Placement {
        origin: position,
        rotation,
    };

    let mut properties = BTreeMap::new();
    for p in fp.find_all("property") {
        if p.len() >= 3 {
            properties.insert(p.str_at(1)?.into_owned(), p.str_at(2)?.into_owned());
        }
    }
    let mut legacy_ref = None;
    let mut legacy_val = None;
    for t in fp.find_all("fp_text") {
        if t.len() < 3 {
            continue;
        }
        match t.str_at(1)?.as_ref() {
            "reference" => legacy_ref = Some(t.str_at(2)?.into_owned()),
            "value" => legacy_val = Some(t.str_at(2)?.into_owned()),
            _ => {}
        }
    }
    let reference = properties.get("Reference").cloned().or(legacy_ref);
    let value = properties.get("Value").cloned().or(legacy_val);

    let mut attributes = Attributes::default();
    if let Some(a) = fp.find("attr") {
        for flag in atoms(a) {
            match flag.as_str() {
                "smd" => attributes.mount = MountType::Smd,
                "through_hole" => attributes.mount = MountType::ThroughHole,
                "exclude_from_pos_files" => attributes.exclude_from_pos_files = true,
                "exclude_from_bom" => attributes.exclude_from_bom = true,
                "dnp" => attributes.dnp = true,
                "board_only" => attributes.board_only = true,
                _ => {}
            }
        }
    }
    let locked = fp.has_symbol("locked") || opt_bool(fp, "locked")?.unwrap_or(false);

    let mut pads = Vec::new();
    for p in fp.find_all("pad") {
        pads.push(read_pad(p, &place, nets)?);
    }

    let mut courtyard: Option<BBox> = None;
    let mut fab: Option<BBox> = None;
    let mut edges = Vec::new();
    let mut graphics = Vec::new();
    let mut texts = Vec::new();
    for g in fp.lists() {
        let Some(head) = g.head() else { continue };
        if head == "property" || head == "fp_text" {
            if let Some(t) = read_fp_text(g, &place)? {
                texts.push(t);
            }
            continue;
        }
        let Some(kind) = head.strip_prefix("fp_") else {
            continue;
        };
        let Some(gl) = layer_of(g)? else { continue };
        let class = FpLayer::classify(&gl);
        let is_edge = gl == "Edge.Cuts";
        if class.is_none() && !is_edge {
            continue;
        }
        let Some(shape) = read_shape(g, kind, Some(&place), warnings)? else {
            continue;
        };
        match class {
            Some((family, side)) => {
                match family {
                    FpLayer::Courtyard => courtyard = union_opt(courtyard, shape.bbox()),
                    FpLayer::Fab => fab = union_opt(fab, shape.bbox()),
                    FpLayer::Silk => {}
                }
                let mut ignore = Vec::new();
                let local = read_shape(g, kind, None, &mut ignore)?.unwrap_or(shape.clone());
                graphics.push(FpGraphic {
                    layer: gl,
                    kind: family,
                    side,
                    shape,
                    local,
                    // A real demo file has -0.000001: treat as zero.
                    width: stroke_width(g)?.max(0.0),
                    filled: is_filled(g)?,
                });
            }
            None => edges.push(EdgePrimitive {
                shape,
                source: EdgeSource::Footprint { index },
                uuid: opt_str(g, "uuid")?,
            }),
        }
    }

    Ok((
        Footprint {
            lib_id,
            uuid: opt_str(fp, "uuid")?.or(opt_str(fp, "tstamp")?),
            reference,
            value,
            side: Side::from_layer(&layer),
            layer,
            position,
            rotation,
            locked,
            attributes,
            properties,
            pads,
            courtyard,
            fab,
            graphics,
            texts,
        },
        edges,
    ))
}

/// `(stroke (width w) ...)` or legacy `(width w)`.
fn stroke_width(g: Node<'_>) -> R<f64> {
    if let Some(st) = g.find("stroke") {
        if let Some(w) = opt_f64(st, "width")? {
            return Ok(w);
        }
    }
    Ok(opt_f64(g, "width")?.unwrap_or(0.0))
}

/// `(fill yes|solid)` or `(fill (type solid))`; `none`/`no` otherwise.
fn is_filled(g: Node<'_>) -> R<bool> {
    let Some(f) = g.find("fill") else {
        return Ok(false);
    };
    let v = match f.child(1) {
        Some(c) if c.is_list() => match f.find("type") {
            Some(t) if t.len() > 1 => t.str_at(1)?.into_owned(),
            _ => String::new(),
        },
        Some(_) => f.str_at(1)?.into_owned(),
        None => String::new(),
    };
    Ok(matches!(v.as_str(), "yes" | "solid" | "true"))
}

/// A `property` (only with a position) or `fp_text` of a footprint.
fn read_fp_text(t: Node<'_>, place: &Placement) -> R<Option<FpText>> {
    let is_prop = t.head() == Some("property");
    if t.len() < 3 {
        return Ok(None);
    }
    let Some(at_node) = t.find("at") else {
        return Ok(None);
    };
    let (kind, text) = if is_prop {
        let name = t.str_at(1)?.into_owned();
        let kind = match name.as_str() {
            "Reference" => FpTextKind::Reference,
            "Value" => FpTextKind::Value,
            _ => FpTextKind::Property(name),
        };
        (kind, t.str_at(2)?.into_owned())
    } else {
        let kind = match t.str_at(1)?.as_ref() {
            "reference" => FpTextKind::Reference,
            "value" => FpTextKind::Value,
            _ => FpTextKind::User,
        };
        (kind, t.str_at(2)?.into_owned())
    };
    let local = point(at_node)?;
    // `(at x y [angle] [unlocked])`
    let angle = at_node
        .child(3)
        .and_then(|c| c.atom())
        .and_then(|a| kdraft_sexpr::parse_f64(&a))
        .unwrap_or(0.0);
    // Legacy: `unlocked` inside `(at ...)`; newer: `(unlocked yes)`.
    let mut keep_upright = !at_node.has_symbol("unlocked");
    if let Some(u) = opt_bool(t, "unlocked")? {
        keep_upright = !u;
    }
    let mut hidden = t.has_symbol("hide") || opt_bool(t, "hide")?.unwrap_or(false);
    let mut size = Size { w: 1.524, h: 1.524 };
    let mut thickness = None;
    let mut mirrored = false;
    let mut justify = Vec::new();
    if let Some(e) = t.find("effects") {
        if let Some(f) = e.find("font") {
            if let Some(sz) = f.find("size") {
                // `(size h w)`: KiCad writes height first.
                size = Size {
                    h: sz.f64_at(1)?,
                    w: if sz.len() > 2 {
                        sz.f64_at(2)?
                    } else {
                        sz.f64_at(1)?
                    },
                };
            }
            thickness = opt_f64(f, "thickness")?;
        }
        if let Some(j) = e.find("justify") {
            for a in atoms(j) {
                if a == "mirror" {
                    mirrored = true;
                } else {
                    justify.push(a);
                }
            }
        }
        if e.has_symbol("hide") || opt_bool(e, "hide")?.unwrap_or(false) {
            hidden = true;
        }
    }
    Ok(Some(FpText {
        kind,
        text,
        layer: layer_of(t)?.unwrap_or_default(),
        local,
        position: place.apply(local),
        angle,
        size,
        thickness,
        hidden,
        keep_upright,
        mirrored,
        justify,
    }))
}

fn read_pad(p: Node<'_>, place: &Placement, nets: &HashMap<i64, String>) -> R<Pad> {
    let number = p.str_at(1)?.into_owned();
    let kind = match p.str_at(2)?.as_ref() {
        "smd" => PadType::Smd,
        "thru_hole" => PadType::ThruHole,
        "np_thru_hole" => PadType::NpThruHole,
        "connect" => PadType::Connect,
        other => PadType::Other(other.to_string()),
    };
    let shape = p.str_at(3)?.into_owned();
    let (rel_position, file_angle) = at(p)?;
    let size = match p.find("size") {
        Some(s) => Size {
            w: s.f64_at(1)?,
            h: s.f64_at(2)?,
        },
        None => Size { w: 0.0, h: 0.0 },
    };
    let drill = match p.find("drill") {
        Some(d) => read_drill(d)?,
        None => None,
    };
    Ok(Pad {
        number,
        kind,
        shape,
        rel_position,
        rel_rotation: normalize_deg(file_angle - place.rotation),
        position: place.apply(rel_position),
        rotation: file_angle,
        size,
        roundrect_rratio: match p.find("roundrect_rratio") {
            Some(r) => Some(r.f64_at(1)?),
            None => None,
        },
        drill,
        layers: p.find("layers").map(atoms).unwrap_or_default(),
        net: net_of(p, nets)?,
        uuid: opt_str(p, "uuid")?.or(opt_str(p, "tstamp")?),
    })
}

/// `(drill D)`, `(drill oval W H)`, each with optional `(offset x y)`.
/// Returns `None` for a drill without a size.
fn read_drill(d: Node<'_>) -> R<Option<Drill>> {
    let mut oval = false;
    let mut nums = Vec::new();
    for (i, c) in d.children().enumerate().skip(1) {
        if c.is_list() {
            continue;
        }
        if c.raw() == "oval" {
            oval = true;
        } else {
            nums.push(d.f64_at(i)?);
        }
    }
    let offset = match d.find("offset") {
        Some(o) => point(o)?,
        None => Point::ORIGIN,
    };
    let shape = match (oval, nums.as_slice()) {
        (_, []) => return Ok(None),
        (false, [dia, ..]) => DrillShape::Round { diameter: *dia },
        // An oval with equal sides is a round hole (seen on a real board as
        // `(drill oval 0.4 0.4)` and `(drill oval 0.4)`).
        (true, [w]) => DrillShape::Round { diameter: *w },
        (true, [w, h, ..]) if (w - h).abs() < 1e-9 => DrillShape::Round { diameter: *w },
        (true, [w, h, ..]) => DrillShape::Oval {
            width: *w,
            height: *h,
        },
    };
    Ok(Some(Drill { shape, offset }))
}

fn read_via(v: Node<'_>, nets: &HashMap<i64, String>, warnings: &mut Vec<String>) -> R<Via> {
    let mut kind = ViaType::Through;
    for c in v.children().skip(1) {
        if c.kind() == NodeKind::Symbol {
            match c.raw() {
                "blind" | "buried" => kind = ViaType::BlindBuried,
                "micro" => kind = ViaType::Micro,
                _ => {}
            }
        }
    }
    // UNVERIFIED: a `(type ...)` child is accepted in case a later format
    // moves the via type there; no KiCad 9/10 file seen uses it.
    if let Some(t) = opt_str(v, "type")? {
        match t.as_str() {
            "blind" | "buried" => kind = ViaType::BlindBuried,
            "micro" => kind = ViaType::Micro,
            _ => {}
        }
    }
    let position = req_point(v, "at")?;
    let size = match opt_f64(v, "size")? {
        Some(s) => s,
        None => {
            let (line, col) = v.line_col();
            warnings.push(format!("via at {line}:{col} has no (size ...)"));
            0.0
        }
    };
    let drill = opt_f64(v, "drill")?.unwrap_or(0.0);
    Ok(Via {
        position,
        size,
        drill,
        layers: v.find("layers").map(atoms).unwrap_or_default(),
        kind,
        net: net_of(v, nets)?,
        uuid: opt_str(v, "uuid")?.or(opt_str(v, "tstamp")?),
    })
}

/// Reads a graphic shape (`line`, `arc`, `circle`, `rect`, `poly`, `curve`,
/// `bezier`). `place` maps footprint-local coordinates to board coordinates.
fn read_shape(
    g: Node<'_>,
    kind: &str,
    place: Option<&Placement>,
    warnings: &mut Vec<String>,
) -> R<Option<EdgeShape>> {
    let tf = |p: Point| place.map_or(p, |pl| pl.apply(p));
    let rotated = place.is_some_and(|pl| pl.rotation != 0.0);
    let shape = match kind {
        "line" => EdgeShape::Line {
            start: tf(req_point(g, "start")?),
            end: tf(req_point(g, "end")?),
        },
        "arc" => {
            let Some(mid) = g.find("mid") else {
                let (line, col) = g.line_col();
                warnings.push(format!(
                    "arc at {line}:{col} has no (mid ...); legacy arc format is not supported"
                ));
                return Ok(None);
            };
            EdgeShape::Arc {
                start: tf(req_point(g, "start")?),
                mid: tf(point(mid)?),
                end: tf(req_point(g, "end")?),
            }
        }
        "circle" => {
            let c = req_point(g, "center")?;
            let e = req_point(g, "end")?;
            EdgeShape::Circle {
                center: tf(c),
                radius: c.dist(e),
            }
        }
        "rect" => {
            let s = req_point(g, "start")?;
            let e = req_point(g, "end")?;
            if rotated {
                let corners = [s, Point::new(e.x, s.y), e, Point::new(s.x, e.y)];
                EdgeShape::Poly {
                    items: corners
                        .into_iter()
                        .map(|p| PolyItem::Point { at: tf(p) })
                        .collect(),
                }
            } else {
                EdgeShape::Rect {
                    start: tf(s),
                    end: tf(e),
                }
            }
        }
        "poly" => {
            let mut items = Vec::new();
            if let Some(pts) = g.find("pts") {
                for it in pts.lists() {
                    match it.head() {
                        Some("xy") => items.push(PolyItem::Point { at: tf(point(it)?) }),
                        Some("arc") => items.push(PolyItem::Arc {
                            start: tf(req_point(it, "start")?),
                            mid: tf(req_point(it, "mid")?),
                            end: tf(req_point(it, "end")?),
                        }),
                        _ => {}
                    }
                }
            }
            if items.is_empty() {
                return Ok(None);
            }
            EdgeShape::Poly { items }
        }
        "curve" | "bezier" => {
            let Some(pts) = g.find("pts") else {
                return Ok(None);
            };
            let xy: Vec<Point> = pts.find_all("xy").map(point).collect::<R<Vec<_>>>()?;
            let [a, b, c, d] = xy.as_slice() else {
                let (line, col) = g.line_col();
                warnings.push(format!("curve at {line}:{col} does not have 4 points"));
                return Ok(None);
            };
            EdgeShape::Curve {
                points: [tf(*a), tf(*b), tf(*c), tf(*d)],
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(shape))
}
