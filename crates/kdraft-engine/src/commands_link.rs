//! Placement link commands and `view.pages_per_layer`.

use kdraft_board::geom::Point;
use kdraft_doc::link::{self, Follow, Link};
use kdraft_doc::{Id, Item};
use serde::Deserialize;

use crate::commands::{ids_arg, one_id, p, select_items, spec, P2};
use crate::error::{EngineError, R};
use crate::registry::{opt, req, ArgType as T, CommandKind, Outcome, Registry};
use crate::session::Session;

const FOLLOWS: &[&str] = &["position", "x", "y", "scale", "rotation", "side", "size"];
pub(crate) const LAYER_SETS: &[&str] = &[
    "copper",
    "silkscreen",
    "fab",
    "mask",
    "paste",
    "courtyard",
    "user",
];

pub(crate) fn register(r: &mut Registry) {
    let ids = || opt("ids", T::IdList, "item ids (default: the selection)");
    r.add(
        spec(
            "link.set",
            "Link placement to...",
            CommandKind::Edit,
            "Links items' placement to a reference item (any page). Linking to a follower links to its reference. The items keep their place (the offset is their distance to the reference) unless keep_offset is false.",
            vec![
                ids(),
                req("to", T::Id, "reference item"),
                opt("follow", T::StringList, "followed properties: position, x, y, scale, rotation, side, size (default: position, scale and rotation from view to view, else position)"),
                opt("keep_offset", T::Bool, "keep the items where they are (default true); false puts them on the reference"),
            ],
        ),
        link_set,
    );
    r.add(
        spec(
            "link.clear",
            "Unlink",
            CommandKind::Edit,
            "Removes the placement link of items (they stay where they are).",
            vec![ids()],
        ),
        link_clear,
    );
    r.add(
        spec(
            "link.set_offset",
            "Set link offset",
            CommandKind::Edit,
            "Sets a follower's distance from its reference, sheet mm, or changes the offsets of followers by a delta (moves followers alone).",
            vec![
                opt("ids", T::IdList, "followers (default: the selection)"),
                opt("offset", T::Point, "offset [dx, dy], sheet mm"),
                opt("delta", T::Point, "added to every follower's offset instead"),
            ],
        ),
        link_set_offset,
    );
    r.add(
        spec(
            "link.set_follow",
            "Set followed properties",
            CommandKind::Edit,
            "Sets which properties a follower takes from its reference.",
            vec![
                opt("id", T::Id, "follower (default: first selected)"),
                req(
                    "follow",
                    T::StringList,
                    "position, x, y, scale, rotation, side, size",
                ),
            ],
        ),
        link_set_follow,
    );
    r.add(
        spec(
            "link.select_reference",
            "Select reference",
            CommandKind::Selection,
            "Selects the reference of a linked item (going to its page).",
            vec![opt("id", T::Id, "follower (default: first selected)")],
        ),
        link_select_reference,
    );
    r.add(
        spec(
            "link.match_colours",
            "Match layer colours to reference",
            CommandKind::Edit,
            "Gives linked views the colours of their reference (palette choice and own colours for the layers both draw). A selected follower takes its reference's; a selected reference gives its colours to all its followers.",
            vec![opt("ids", T::IdList, "views (default: the selection)")],
        ),
        link_match_colours,
    );
    r.add(
        spec(
            "link.select_followers",
            "Select linked items",
            CommandKind::Selection,
            "Selects the items linked to a reference (on the current page if any are there, else on the first page that has some).",
            vec![opt("id", T::Id, "reference or follower (default: first selected)")],
        ),
        link_select_followers,
    );
    r.add(
        spec(
            "view.pages_per_layer",
            "Layer pages...",
            CommandKind::Edit,
            "Creates one page per board layer, each with a view of that layer at the same position, scale and rotation. With a reference view (given, or the selected one) the views link to it and take its page's paper and sheet; without one (or with reference \"new\") the first new view is centred in the drawing area at the largest standard scale that fits and the others link to it. Pages go after the current page (or at the end); an empty untitled current page can be used as the first one. Run again with the same reference to add missing layers.",
            vec![
                opt("reference", T::String, "reference view id, or \"new\" (default: the selected view, else new)"),
                opt("view", T::Id, "older name of reference"),
                opt("layers", T::StringList, "layer names (default: from set)"),
                opt("set", T::Enum { values: LAYER_SETS }, "layer set when no layers are given (default copper)"),
                opt("common", T::StringList, "layers added to every page (default [\"Edge.Cuts\"]; [] for none)"),
                opt("at_end", T::Bool, "put the pages at the end (default: after the current page)"),
                opt("use_current_page", T::Bool, "use the current page as the first layer page if it is empty and untitled (default false)"),
            ],
        ),
        crate::layer_pages::run,
    );
}

fn parse_follow(v: &[String]) -> R<Vec<Follow>> {
    v.iter()
        .map(|s| {
            Follow::parse(s).ok_or_else(|| {
                EngineError::invalid(format!(
                    "unknown followed property {s:?} (one of {})",
                    FOLLOWS.join(", ")
                ))
            })
        })
        .collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetArgs {
    ids: Option<Vec<String>>,
    to: String,
    follow: Option<Vec<String>>,
    keep_offset: Option<bool>,
}

fn link_set(s: &mut Session, a: SetArgs) -> R<Outcome> {
    let (_, ids) = ids_arg(s, a.ids)?;
    let to = Id(a.to);
    let follow = a.follow.as_deref().map(parse_follow).transpose()?;
    let mut d = s.doc().clone();
    for id in &ids {
        if id == &to {
            continue;
        }
        link::link(
            &mut d,
            id,
            &to,
            follow.clone(),
            a.keep_offset.unwrap_or(true),
        )
        .map_err(|e| EngineError::invalid(e.to_string()))?;
    }
    s.set_doc(d);
    Ok(Outcome::changed(&ids))
}

fn link_clear(s: &mut Session, a: crate::commands::IdsArgs) -> R<Outcome> {
    let (pi, ids) = ids_arg(s, a.ids)?;
    let mut changed = Vec::new();
    for id in &ids {
        let it = crate::commands::item_mut(s, pi, id)?;
        if it.link().is_some() {
            it.set_link(None);
            changed.push(id.clone());
        }
    }
    if changed.is_empty() {
        return Err(EngineError::invalid("none of the items is linked"));
    }
    Ok(Outcome::changed(&changed))
}

fn follower_link(s: &mut Session, id: Option<String>) -> R<(usize, Id, Link)> {
    let (pi, id) = one_id(s, id)?;
    let l = s.doc().pages[pi]
        .item(&id)
        .and_then(Item::link)
        .ok_or_else(|| EngineError::invalid(format!("item {id} is not linked")))?;
    Ok((pi, id, l))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OffsetArgs {
    ids: Option<Vec<String>>,
    offset: Option<P2>,
    delta: Option<P2>,
}

fn link_set_offset(s: &mut Session, a: OffsetArgs) -> R<Outcome> {
    let (pi, ids) = ids_arg(s, a.ids)?;
    let (offset, delta) = (a.offset.map(p).transpose()?, a.delta.map(p).transpose()?);
    if offset.is_some() == delta.is_some() {
        return Err(EngineError::invalid("give either offset or delta"));
    }
    let mut changed = Vec::new();
    for id in &ids {
        let it = crate::commands::item_mut(s, pi, id)?;
        let Some(mut l) = it.link() else { continue };
        l.offset = match (offset, delta) {
            (Some(o), _) => o,
            (_, Some(d)) => Point::new(l.offset.x + d.x, l.offset.y + d.y),
            _ => l.offset,
        };
        it.set_link(Some(&l));
        changed.push(id.clone());
    }
    if changed.is_empty() {
        return Err(EngineError::invalid("none of the items is linked"));
    }
    let mut d = s.doc().clone();
    link::resolve(&mut d);
    s.set_doc(d);
    Ok(Outcome::changed(&changed))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FollowArgs {
    id: Option<String>,
    follow: Vec<String>,
}

fn link_set_follow(s: &mut Session, a: FollowArgs) -> R<Outcome> {
    let (pi, id, mut l) = follower_link(s, a.id)?;
    let f = parse_follow(&a.follow)?;
    let ix = link::index(s.doc());
    let me = s.doc().pages[pi].item(&id).cloned();
    let r = link::get(s.doc(), &ix, &l.to).cloned();
    if let (Some(me), Some(r)) = (me, r) {
        let allowed = link::allowed_follow(&me, &r);
        if let Some(bad) = f.iter().find(|x| !allowed.contains(x)) {
            return Err(EngineError::invalid(format!(
                "{} cannot be followed between these items",
                bad.as_str()
            )));
        }
        // Keep the follower where it is: the offset is its distance now.
        if let (Some(fp), Some(rp)) = (me.position(), r.position()) {
            l.offset = Point::new(fp.x - rp.x, fp.y - rp.y);
        }
    }
    l.follow = f;
    crate::commands::item_mut(s, pi, &id)?.set_link(Some(&l));
    let mut d = s.doc().clone();
    link::resolve(&mut d);
    s.set_doc(d);
    Ok(Outcome::changed([id]))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OneArg {
    id: Option<String>,
}

fn link_select_reference(s: &mut Session, a: OneArg) -> R<Outcome> {
    let (_, _, l) = follower_link(s, a.id)?;
    let (pi, _) = s
        .find_item(&l.to)
        .ok_or_else(|| EngineError::invalid(format!("the reference {} no longer exists", l.to)))?;
    select_items(s, pi, vec![l.to.clone()]);
    Ok(Outcome::default())
}

fn link_select_followers(s: &mut Session, a: OneArg) -> R<Outcome> {
    let (pi, id) = one_id(s, a.id)?;
    let root = s.doc().pages[pi]
        .item(&id)
        .and_then(Item::link)
        .map_or(id.clone(), |l| l.to);
    let fs = link::followers(s.doc(), &root);
    if fs.is_empty() {
        return Err(EngineError::invalid("no items are linked to this one"));
    }
    let pages: Vec<usize> = fs
        .iter()
        .filter_map(|f| s.find_item(f).map(|x| x.0))
        .collect();
    let target = if pages.contains(&pi) { pi } else { pages[0] };
    let here: Vec<Id> = fs
        .iter()
        .zip(&pages)
        .filter(|(_, p)| **p == target)
        .map(|(f, _)| f.clone())
        .collect();
    let n_pages = {
        let mut v = pages.clone();
        v.sort_unstable();
        v.dedup();
        v.len()
    };
    select_items(s, target, here);
    let mut o = Outcome::default();
    if n_pages > 1 {
        o = o.msg(format!(
            "{} linked item(s) on {n_pages} pages; selected those on one page",
            fs.len()
        ));
    }
    Ok(o)
}

/// Layer names of a set, in board order.
pub fn layer_set(board: &kdraft_board::Board, set: &str) -> Vec<String> {
    board
        .layers
        .iter()
        .filter(|l| {
            let n = l.name.as_str();
            match set {
                "copper" => l.is_copper(),
                "silkscreen" => n.ends_with(".SilkS"),
                "fab" => n.ends_with(".Fab"),
                "mask" => n.ends_with(".Mask"),
                "paste" => n.ends_with(".Paste"),
                "courtyard" => n.ends_with(".CrtYd"),
                "user" => l.kind == "user" && !n.contains('.'),
                _ => false,
            }
        })
        .map(|l| l.name.clone())
        .collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdsArg {
    ids: Option<Vec<String>>,
}

fn link_match_colours(s: &mut Session, a: IdsArg) -> R<Outcome> {
    let (_, ids) = ids_arg(s, a.ids)?;
    let mut d = s.doc().clone();
    let ix = link::index(&d);
    // (reference, follower) pairs.
    let mut pairs: Vec<(Id, Id)> = Vec::new();
    for id in &ids {
        if let Some(it @ Item::View(_)) = link::get(&d, &ix, id) {
            match it.link() {
                Some(l) => pairs.push((l.to.clone(), id.clone())),
                None => pairs.extend(link::followers(&d, id).into_iter().map(|f| (id.clone(), f))),
            }
        }
    }
    let mut changed = Vec::new();
    for (r, f) in pairs {
        let Some(Item::View(rv)) = link::get(&d, &ix, &r).cloned() else {
            continue;
        };
        let Some((pi, zi)) = ix.get(&f).copied() else {
            continue;
        };
        if let Some(Item::View(fv)) = d.pages[pi].items.get_mut(zi) {
            if link::match_colours(&rv, fv) {
                changed.push(f.clone());
            }
        }
    }
    if changed.is_empty() {
        return Ok(Outcome::default().msg("the linked views already draw the same colours"));
    }
    s.set_doc(d);
    let n = changed.len();
    Ok(Outcome::changed(&changed).msg(format!("{n} view(s) now match their reference's colours")))
}
