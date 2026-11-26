//! Placement links: an item (the follower) takes its placement from
//! another item (its reference), possibly on another page.
//!
//! Stored in the follower as
//! `(link (to "<id>") (follow position scale rotation) (offset dx dy))`.
//! The follower's own position stays in the file as the last resolved
//! value, so a reader that ignores links still shows a sensible document.
//! Links never chain in storage: a follower's reference is never itself a
//! follower.

use std::collections::{HashMap, HashSet};

use kdraft_board::geom::Point;
use kdraft_sexpr::Value;

use crate::model::{Document, GraphicShape, Id, Item};

/// Head of the link node.
pub const LINK: &str = "link";

/// A followed property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Follow {
    /// Both coordinates.
    Position,
    X,
    Y,
    /// View scale (views only).
    Scale,
    /// View rotation (views only).
    Rotation,
    /// View side / mirroring (views only).
    Side,
    /// Rectangle size, text wrap width (same kinds only).
    Size,
    /// A view's colours: the reference's palette choice and its own
    /// colours for the layers (and roles) both views draw (views only).
    Colours,
}

impl Follow {
    pub const ALL: [Follow; 8] = [
        Follow::Position,
        Follow::X,
        Follow::Y,
        Follow::Scale,
        Follow::Rotation,
        Follow::Side,
        Follow::Size,
        Follow::Colours,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Follow::Position => "position",
            Follow::X => "x",
            Follow::Y => "y",
            Follow::Scale => "scale",
            Follow::Rotation => "rotation",
            Follow::Side => "side",
            Follow::Size => "size",
            Follow::Colours => "colours",
        }
    }

    pub fn parse(s: &str) -> Option<Follow> {
        Follow::ALL.into_iter().find(|f| f.as_str() == s)
    }
}

/// A placement link.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub to: Id,
    /// Followed properties, sorted, no duplicates.
    pub follow: Vec<Follow>,
    /// Follower position minus reference position, sheet mm.
    pub offset: Point,
    /// Unknown child nodes and unknown follow words, preserved.
    pub extra: Vec<Value>,
}

impl Link {
    /// A link with the default followed properties for the two kinds.
    pub fn new(to: Id, follower: &Item, reference: &Item) -> Link {
        Link {
            to,
            follow: default_follow(follower, reference),
            offset: Point::ORIGIN,
            extra: Vec::new(),
        }
    }

    pub fn follows(&self, f: Follow) -> bool {
        self.follow.contains(&f)
    }

    /// Follows x (directly or through `position`).
    pub fn follows_x(&self) -> bool {
        self.follows(Follow::Position) || self.follows(Follow::X)
    }

    pub fn follows_y(&self) -> bool {
        self.follows(Follow::Position) || self.follows(Follow::Y)
    }

    pub fn normalize(&mut self) {
        self.follow.sort();
        self.follow.dedup();
    }

    pub fn from_value(v: &Value) -> Option<Link> {
        let mut to = None;
        let mut follow = Vec::new();
        let mut offset = Point::ORIGIN;
        let mut extra = Vec::new();
        for c in v.args() {
            match c.head() {
                Some("to") if to.is_none() => {
                    to = c
                        .args()
                        .first()
                        .and_then(Value::atom_text)
                        .map(|s| Id(s.to_string()))
                }
                Some("follow") => {
                    let mut unknown = Vec::new();
                    for a in c.args() {
                        match a.atom_text().and_then(Follow::parse) {
                            Some(f) => follow.push(f),
                            None => unknown.push(a.clone()),
                        }
                    }
                    if !unknown.is_empty() {
                        extra.push(Value::node("follow_unknown", unknown));
                    }
                }
                Some("offset") => match (c.f64_at(1), c.f64_at(2)) {
                    (Ok(x), Ok(y)) if x.is_finite() && y.is_finite() => offset = Point::new(x, y),
                    _ => extra.push(c.clone()),
                },
                _ => extra.push(c.clone()),
            }
        }
        let mut l = Link {
            to: to?,
            follow,
            offset,
            extra,
        };
        l.normalize();
        Some(l)
    }

    pub fn to_value(&self) -> Value {
        let num = |v: f64| Value::num(v).unwrap_or_else(|_| Value::int(0));
        let mut o = vec![Value::node("to", [Value::str(self.to.as_str())])];
        o.push(Value::node(
            "follow",
            self.follow.iter().map(|f| Value::sym(f.as_str())),
        ));
        if self.offset != Point::ORIGIN {
            o.push(Value::node(
                "offset",
                [num(self.offset.x), num(self.offset.y)],
            ));
        }
        o.extend(self.extra.iter().cloned());
        Value::node(LINK, o)
    }
}

/// Default followed properties: position, scale and rotation from view to
/// view; position otherwise.
pub fn default_follow(follower: &Item, reference: &Item) -> Vec<Follow> {
    match (follower, reference) {
        (Item::View(_), Item::View(_)) => vec![Follow::Position, Follow::Scale, Follow::Rotation],
        _ => vec![Follow::Position],
    }
}

/// Properties that can be followed between the two kinds.
pub fn allowed_follow(follower: &Item, reference: &Item) -> Vec<Follow> {
    let mut v = vec![Follow::Position, Follow::X, Follow::Y];
    match (follower, reference) {
        (Item::View(_), Item::View(_)) => {
            v.extend([
                Follow::Scale,
                Follow::Rotation,
                Follow::Side,
                Follow::Colours,
            ]);
        }
        (Item::Graphic(a), Item::Graphic(b)) => match (&a.shape, &b.shape) {
            (GraphicShape::Rect { .. }, GraphicShape::Rect { .. })
            | (GraphicShape::Text { .. }, GraphicShape::Text { .. }) => v.push(Follow::Size),
            _ => {}
        },
        _ => {}
    }
    v
}

impl Item {
    pub fn link(&self) -> Option<Link> {
        self.extra_values()?
            .iter()
            .find(|v| v.is(LINK))
            .and_then(Link::from_value)
    }

    /// Sets or removes the link; false for unknown items.
    pub fn set_link(&mut self, l: Option<&Link>) -> bool {
        let Some(e) = self.extra_values_mut() else {
            return false;
        };
        let at = e.iter().position(|v| v.is(LINK));
        e.retain(|v| !v.is(LINK));
        if let Some(l) = l {
            let mut l = l.clone();
            l.normalize();
            let i = at.unwrap_or(e.len()).min(e.len());
            e.insert(i, l.to_value());
        }
        true
    }

    /// Whether the item has a sheet position that a link can drive.
    pub fn linkable(&self) -> bool {
        self.position().is_some()
    }
}

/// Every item of the document by id: (page index, z index).
pub fn index(doc: &Document) -> HashMap<Id, (usize, usize)> {
    let mut m = HashMap::new();
    for (pi, p) in doc.pages.iter().enumerate() {
        for (zi, it) in p.items.iter().enumerate() {
            if let Some(id) = it.id() {
                m.insert(id.clone(), (pi, zi));
            }
        }
    }
    m
}

pub fn get<'a>(doc: &'a Document, ix: &HashMap<Id, (usize, usize)>, id: &Id) -> Option<&'a Item> {
    let (p, z) = ix.get(id)?;
    doc.pages.get(*p)?.items.get(*z)
}

fn get_mut<'a>(
    doc: &'a mut Document,
    ix: &HashMap<Id, (usize, usize)>,
    id: &Id,
) -> Option<&'a mut Item> {
    let (p, z) = ix.get(id)?;
    doc.pages.get_mut(*p)?.items.get_mut(*z)
}

/// Followers of `reference`, in page order then z-order.
pub fn followers(doc: &Document, reference: &Id) -> Vec<Id> {
    doc.pages
        .iter()
        .flat_map(|p| p.items.iter())
        .filter(|it| it.link().is_some_and(|l| &l.to == reference))
        .filter_map(|it| it.id().cloned())
        .collect()
}

/// The item a new link to `target` must point at (its root reference)
/// and the offset of `target` from that root.
pub fn root_of(doc: &Document, target: &Id) -> (Id, Point) {
    let ix = index(doc);
    match get(doc, &ix, target).and_then(Item::link) {
        Some(l) if ix.contains_key(&l.to) => (l.to.clone(), l.offset),
        _ => (target.clone(), Point::ORIGIN),
    }
}

/// Applies the reference's placement to one follower. Returns whether it
/// changed.
pub fn apply_one(reference: &Item, follower: &mut Item, l: &Link) -> bool {
    let before = follower.clone();
    if let (Some(rp), Some(fp)) = (reference.position(), follower.position()) {
        let want = Point::new(rp.x + l.offset.x, rp.y + l.offset.y);
        let dx = if l.follows_x() { want.x - fp.x } else { 0.0 };
        let dy = if l.follows_y() { want.y - fp.y } else { 0.0 };
        if dx != 0.0 || dy != 0.0 {
            follower.translate(dx, dy);
        }
    }
    match (reference, &mut *follower) {
        (Item::View(r), Item::View(f)) => {
            if l.follows(Follow::Scale) {
                f.scale = r.scale;
            }
            if l.follows(Follow::Rotation) {
                f.rotation = r.rotation;
            }
            if l.follows(Follow::Side) {
                f.side = r.side;
            }
            if l.follows(Follow::Colours) {
                match_colours(r, f);
            }
        }
        (Item::Graphic(r), Item::Graphic(f)) if l.follows(Follow::Size) => {
            match (&r.shape, &mut f.shape) {
                (
                    GraphicShape::Rect {
                        start: rs, end: re, ..
                    },
                    GraphicShape::Rect { start, end, .. },
                ) => {
                    *end = Point::new(start.x + (re.x - rs.x), start.y + (re.y - rs.y));
                }
                (GraphicShape::Text { wrap: rw, .. }, GraphicShape::Text { wrap, .. }) => {
                    *wrap = *rw;
                }
                _ => {}
            }
        }
        _ => {}
    }
    *follower != before
}

/// Gives `f` the palette choice of `r` and `r`'s own colours for the
/// layers both draw (and the roles); `f`'s own colours for layers only it
/// draws stay. Returns whether anything changed.
pub fn match_colours(r: &crate::View, f: &mut crate::View) -> bool {
    let rc = r.layer_colors();
    let mut fc = f.layer_colors();
    let before = fc.clone();
    fc.palette = rc.palette.clone();
    let shared = |name: &str| {
        name.starts_with("role.")
            || (r.layers.iter().any(|l| l == name) && f.layers.iter().any(|l| l == name))
    };
    fc.layers.retain(|o| !shared(&o.layer));
    for o in rc.layers.iter().filter(|o| shared(&o.layer)) {
        fc.set(&o.layer, Some(o.color), Some(o.opacity));
    }
    if fc == before {
        return false;
    }
    f.set_layer_colors(&fc);
    true
}

/// Layers both views draw whose colours differ (as drawn).
pub fn colour_differences(st: &crate::Styles, a: &crate::View, b: &crate::View) -> Vec<String> {
    a.layers
        .iter()
        .filter(|l| b.layers.contains(l))
        .filter(|l| a.layer_color(st, l) != b.layer_color(st, l))
        .cloned()
        .collect()
}

/// Moves every follower to its reference's placement. Links to missing
/// items are left alone. Returns the ids of changed followers.
pub fn resolve(doc: &mut Document) -> Vec<Id> {
    let ix = index(doc);
    let mut todo = Vec::new();
    for p in &doc.pages {
        for it in &p.items {
            if let (Some(id), Some(l)) = (it.id(), it.link()) {
                todo.push((id.clone(), l));
            }
        }
    }
    let mut changed = Vec::new();
    for (id, l) in todo {
        let Some(r) = get(doc, &ix, &l.to).cloned() else {
            continue;
        };
        if let Some(f) = get_mut(doc, &ix, &id) {
            if apply_one(&r, f, &l) {
                changed.push(id);
            }
        }
    }
    changed
}

/// Errors of [`link`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    NoSuchItem(String),
    NotPlaceable(String),
    /// The reference is the follower itself or one of its followers.
    Cycle(String),
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkError::NoSuchItem(id) => write!(f, "no item {id}"),
            LinkError::NotPlaceable(id) => {
                write!(f, "item {id} has no position that can be linked")
            }
            LinkError::Cycle(id) => write!(
                f,
                "cannot link {id} to itself or to an item that follows it"
            ),
        }
    }
}

impl std::error::Error for LinkError {}

/// Links `follower` to `target` (through `target`'s root reference),
/// keeping the follower where it is (the offset is its current distance)
/// unless `keep_offset` is false, in which case the offset is 0 for the
/// position. The follower's own followers move over to the new root.
/// `follow`: `None` for the defaults.
pub fn link(
    doc: &mut Document,
    follower: &Id,
    target: &Id,
    follow: Option<Vec<Follow>>,
    keep_offset: bool,
) -> Result<Link, LinkError> {
    let ix = index(doc);
    let f = get(doc, &ix, follower)
        .ok_or_else(|| LinkError::NoSuchItem(follower.to_string()))?
        .clone();
    if get(doc, &ix, target).is_none() {
        return Err(LinkError::NoSuchItem(target.to_string()));
    }
    let (root, _) = root_of(doc, target);
    if &root == follower {
        return Err(LinkError::Cycle(follower.to_string()));
    }
    let r = get(doc, &ix, &root)
        .ok_or_else(|| LinkError::NoSuchItem(root.to_string()))?
        .clone();
    let (Some(fp), Some(rp)) = (f.position(), r.position()) else {
        return Err(LinkError::NotPlaceable(
            if f.position().is_none() {
                follower
            } else {
                &root
            }
            .to_string(),
        ));
    };
    let allowed = allowed_follow(&f, &r);
    let mut fl: Vec<Follow> = follow
        .unwrap_or_else(|| default_follow(&f, &r))
        .into_iter()
        .filter(|x| allowed.contains(x))
        .collect();
    if fl.is_empty() {
        fl.push(Follow::Position);
    }
    let offset = if keep_offset {
        Point::new(fp.x - rp.x, fp.y - rp.y)
    } else {
        Point::ORIGIN
    };
    let mut l = Link {
        to: root.clone(),
        follow: fl,
        offset,
        extra: Vec::new(),
    };
    l.normalize();
    // The follower's own followers now follow the root.
    let mine = followers(doc, follower);
    for id in &mine {
        if let Some(it) = get_mut(doc, &ix, id) {
            if let Some(mut ol) = it.link() {
                ol.to = root.clone();
                ol.offset = Point::new(ol.offset.x + l.offset.x, ol.offset.y + l.offset.y);
                it.set_link(Some(&ol));
            }
        }
    }
    if let Some(it) = get_mut(doc, &ix, follower) {
        it.set_link(Some(&l));
    }
    resolve(doc);
    Ok(l)
}

/// After items were removed: for every reference that is gone but still
/// has followers, the first follower (page order, then z-order) becomes
/// the reference of the others; everyone stays where they are.
pub fn promote_orphans(doc: &mut Document, existed: &HashSet<Id>) -> Vec<Id> {
    let ix = index(doc);
    let mut by_ref: Vec<(Id, Vec<Id>)> = Vec::new();
    for p in &doc.pages {
        for it in &p.items {
            if let (Some(id), Some(l)) = (it.id(), it.link()) {
                if !ix.contains_key(&l.to) && existed.contains(&l.to) {
                    match by_ref.iter_mut().find(|(r, _)| r == &l.to) {
                        Some((_, v)) => v.push(id.clone()),
                        None => by_ref.push((l.to.clone(), vec![id.clone()])),
                    }
                }
            }
        }
    }
    let mut promoted = Vec::new();
    for (_, fs) in by_ref {
        let Some(first) = fs.first().cloned() else {
            continue;
        };
        let Some(fl) = get(doc, &ix, &first).and_then(Item::link) else {
            continue;
        };
        if let Some(it) = get_mut(doc, &ix, &first) {
            it.set_link(None);
        }
        for id in &fs[1..] {
            if let Some(it) = get_mut(doc, &ix, id) {
                if let Some(mut l) = it.link() {
                    l.to = first.clone();
                    l.offset = Point::new(l.offset.x - fl.offset.x, l.offset.y - fl.offset.y);
                    it.set_link(Some(&l));
                }
            }
        }
        promoted.push(first);
    }
    promoted
}
