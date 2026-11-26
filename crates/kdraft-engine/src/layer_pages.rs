//! `view.pages_per_layer`: one page per board layer, each with a view of
//! that layer, all at the same place, scale and rotation.
//!
//! The placement comes from a reference view (given, or the selected one;
//! its layer views link to it) or, with no reference, from a new placement:
//! the first new layer view is centred in the drawing area at the largest
//! standard scale that fits (as `doc new` does) and the others link to it.
//! [`plan`] works out what the command would do without changing anything,
//! so the GUI can show the same one-line summary before OK.

use kdraft_board::geom::Point;
use kdraft_doc::link::{self, Follow, Link};
use kdraft_doc::{ops, DrillMarks, Id, Item, Page, PageSize, View, ViewKind, ViewSide};
use serde::Deserialize;

use crate::commands_link::{layer_set, LAYER_SETS};
use crate::error::{EngineError, R};
use crate::registry::Outcome;
use crate::session::{Selection, Session};

/// `reference` value for a new placement.
pub const NEW: &str = "new";

/// Arguments of `view.pages_per_layer`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerPagesArgs {
    /// Reference view id, or `"new"` for a new placement. Default: the
    /// selected view, else a new placement.
    pub reference: Option<String>,
    /// Older name of `reference` (a view id).
    pub view: Option<String>,
    pub layers: Option<Vec<String>>,
    pub set: Option<String>,
    pub common: Option<Vec<String>>,
    /// Put the pages at the end of the document (default: after the
    /// current page).
    pub at_end: Option<bool>,
    /// Use the current page, if it is empty and untitled, as the first
    /// layer page.
    pub use_current_page: Option<bool>,
}

/// Where the layer views get their placement from.
#[derive(Debug, Clone, PartialEq)]
pub enum Placement {
    /// Linked to this view (the root of its link group).
    Existing {
        root: Id,
        name: String,
        page: usize,
        scale: f64,
    },
    /// The first new view is the reference, at `at` and `scale`.
    New { at: Point, scale: f64 },
}

/// What `view.pages_per_layer` would do.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub placement: Placement,
    /// Layers that get a page, board order as given.
    pub layers: Vec<String>,
    /// Layers left out because they already have a page in this set.
    pub skipped: Vec<String>,
    /// Layers added to every view.
    pub common: Vec<String>,
    /// Index of the first new page in the document.
    pub at: usize,
    /// The current page (index `at`) becomes the first layer page.
    pub use_current: bool,
    size: PageSize,
    sheet: Option<String>,
    /// Template for the new views (side, rotation, render mode...).
    template: View,
}

/// Whether page `pi` is empty and has no title of its own ("Page N").
pub fn page_reusable(s: &Session, pi: usize) -> bool {
    s.doc().pages.get(pi).is_some_and(|p| {
        p.items.is_empty()
            && p.title.as_deref().is_none_or(|t| {
                t.strip_prefix("Page ")
                    .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
            })
    })
}

/// A view's name for messages: its own name or its kind.
pub fn view_name(it: &Item) -> String {
    if let Some(n) = it.name() {
        return n.to_string();
    }
    match it {
        Item::View(v) => match (&v.kind, v.side) {
            (ViewKind::Fabrication, _) => "Fabrication view".into(),
            (ViewKind::Assembly, ViewSide::Front) => "Assembly top".into(),
            (ViewKind::Assembly, ViewSide::Back) => "Assembly bottom".into(),
            (ViewKind::Drill, _) => "Drill drawing".into(),
            (ViewKind::Layer, _) => format!("Layer view {}", v.layers.join(", ")),
            (k, _) => format!("{} view", k.as_str()),
        },
        _ => "item".into(),
    }
}

/// The selected board view, if any (3D views do not count).
pub fn selected_view(s: &Session) -> Option<Id> {
    s.selection
        .items
        .iter()
        .find(|i| {
            s.find_item(i)
                .is_some_and(|(_, it)| matches!(it, Item::View(v) if v.kind != ViewKind::Render3d))
        })
        .cloned()
}

/// The reference a dialog offers first: the selected board view, else the
/// only board view on the current page; `None` means a new placement.
pub fn default_reference(s: &Session) -> Option<Id> {
    selected_view(s).or_else(|| {
        let p = s.current_page()?;
        let mut vs = p.views().filter(|v| v.kind != ViewKind::Render3d);
        let first = vs.next()?;
        vs.next().is_none().then(|| first.id.clone())
    })
}

/// A page's board views that can be a reference.
#[derive(Debug, Clone, PartialEq)]
pub struct PageChoices {
    pub page: usize,
    pub title: String,
    /// View id, name, scale.
    pub views: Vec<(Id, String, f64)>,
}

/// Every board view, grouped by page (pages without one left out).
pub fn reference_choices(s: &Session) -> Vec<PageChoices> {
    s.doc()
        .pages
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            let views: Vec<(Id, String, f64)> = p
                .items
                .iter()
                .filter_map(|it| match it {
                    Item::View(v) if v.kind != ViewKind::Render3d => {
                        Some((v.id.clone(), view_name(it), v.scale))
                    }
                    _ => None,
                })
                .collect();
            (!views.is_empty()).then(|| PageChoices {
                page: i,
                title: p.title.clone().unwrap_or_default(),
                views,
            })
        })
        .collect()
}

/// Works out the pages without changing the document.
pub fn plan(s: &mut Session, a: &LayerPagesArgs) -> R<Plan> {
    let board = s
        .board_arc()
        .ok_or_else(|| EngineError::invalid("open a board first: the layer names come from it"))?;
    let layers: Vec<String> = match &a.layers {
        Some(l) => l.clone(),
        None => {
            let set = a.set.as_deref().unwrap_or("copper");
            if !LAYER_SETS.contains(&set) {
                return Err(EngineError::invalid(format!(
                    "unknown layer set {set:?} (one of {})",
                    LAYER_SETS.join(", ")
                )));
            }
            layer_set(&board, set)
        }
    };
    if layers.is_empty() {
        return Err(EngineError::invalid(
            "choose at least one layer (the board has none in this set)",
        ));
    }
    if let Some(bad) = layers
        .iter()
        .find(|l| !board.layers.iter().any(|x| &x.name == *l))
    {
        return Err(EngineError::invalid(format!(
            "the board has no layer {bad}"
        )));
    }
    let common = a
        .common
        .clone()
        .unwrap_or_else(|| vec!["Edge.Cuts".to_string()]);
    let cur = s.current_page_index();
    // The reference.
    let wanted = match a.reference.as_deref().or(a.view.as_deref()) {
        Some(NEW) => None,
        Some(id) => Some(Id(id.to_string())),
        None => selected_view(s),
    };
    let (placement, size, sheet, template, existing, ref_page) = match wanted {
        Some(vid) => {
            let (_, item) = s
                .find_item(&vid)
                .map(|(p, i)| (p, i.clone()))
                .ok_or_else(|| EngineError::invalid(format!("no item {vid}")))?;
            if !matches!(&item, Item::View(v) if v.kind != ViewKind::Render3d) {
                return Err(EngineError::invalid(
                    "the reference must be a board view (not a 3D view or another item); pick one or use a new placement",
                ));
            }
            // A follower view: its group's root is the reference.
            let (root, _) = link::root_of(s.doc(), &vid);
            let (rp, ritem) = match s.find_item(&root).map(|(p, i)| (p, i.clone())) {
                Some((p, i @ Item::View(_))) => (p, i),
                _ => {
                    let (p, _) = s
                        .find_item(&vid)
                        .ok_or_else(|| EngineError::invalid(format!("no item {vid}")))?;
                    (p, item.clone())
                }
            };
            let root = ritem.id().cloned().unwrap_or(vid);
            let Item::View(src) = &ritem else {
                return Err(EngineError::invalid("the reference is not a view"));
            };
            let page = &s.doc().pages[rp];
            let existing: Vec<String> = link::followers(s.doc(), &root)
                .iter()
                .filter_map(|f| match s.find_item(f) {
                    Some((_, Item::View(v))) => v.layers.first().cloned(),
                    _ => None,
                })
                .collect();
            (
                Placement::Existing {
                    root,
                    name: view_name(&ritem),
                    page: rp,
                    scale: src.scale,
                },
                page.size.clone(),
                page.sheet.clone(),
                src.clone(),
                existing,
                Some(rp),
            )
        }
        None => {
            let pi = cur.ok_or_else(|| EngineError::invalid("the document has no pages"))?;
            let page = &s.doc().pages[pi];
            let (size, sheet) = (page.size.clone(), page.sheet.clone());
            let area = s.drawing_area(pi);
            let bb = s.board_box();
            let scale = area
                .and_then(|r| {
                    kdraft_doc::choose_scale(bb.width(), bb.height(), r.width(), r.height())
                })
                .unwrap_or(kdraft_doc::STANDARD_SCALES[0]);
            let at = area.map_or(Point::new(100.0, 100.0), |r| r.center());
            let template = View {
                id: Id::new_v4(),
                kind: ViewKind::Layer,
                side: ViewSide::Front,
                scale,
                rotation: 0.0,
                at,
                layers: Vec::new(),
                crop: None,
                parent: None,
                style: None,
                frame: false,
                drill_marks: Some(DrillMarks::default_for(&ViewKind::Layer)),
                render: None,
                assembly: Default::default(),
                extra: vec![],
            };
            (
                Placement::New { at, scale },
                size,
                sheet,
                template,
                Vec::new(),
                None,
            )
        }
    };
    let (layers, skipped): (Vec<String>, Vec<String>) =
        layers.into_iter().partition(|l| !existing.contains(l));
    let n = s.doc().pages.len();
    let base = cur.or(ref_page);
    let use_current =
        a.use_current_page.unwrap_or(false) && cur.is_some_and(|c| page_reusable(s, c));
    if a.use_current_page == Some(true) && !use_current {
        return Err(EngineError::invalid(
            "the current page is not empty and untitled; leave \"use the current page\" off",
        ));
    }
    let at = if use_current {
        cur.unwrap_or(n)
    } else if a.at_end.unwrap_or(false) {
        n
    } else {
        let mut at = base.map_or(n, |b| b + 1);
        // Keep a set together: after its pages that follow this point.
        if let Placement::Existing { root, .. } = &placement {
            for f in link::followers(s.doc(), root) {
                if let Some((p, _)) = s.find_item(&f) {
                    if base.is_some_and(|b| p >= b) {
                        at = at.max(p + 1);
                    }
                }
            }
        }
        at.min(n)
    };
    Ok(Plan {
        placement,
        layers,
        skipped,
        common,
        at,
        use_current,
        size,
        sheet,
        template,
    })
}

impl Plan {
    /// One line for the GUI before OK and for the command's message.
    pub fn summary(&self) -> String {
        let n = self.layers.len();
        if n == 0 {
            return "every chosen layer already has its page in this set".to_string();
        }
        let names = self.layers.join(", ");
        let first = self.at + 1;
        let pages = if self.use_current && n == 1 {
            format!("page {first} becomes the {names} page")
        } else if self.use_current {
            format!(
                "page {first} becomes the {} page, {} more page(s) after it ({})",
                self.layers[0],
                n - 1,
                self.layers[1..].join(", ")
            )
        } else if n == 1 {
            format!("1 page ({names}) as page {first}")
        } else {
            format!("{n} pages ({names}) as pages {first}-{}", first + n - 1)
        };
        let how = match &self.placement {
            Placement::Existing {
                name, page, scale, ..
            } => format!(
                "views linked to {name} on page {} ({})",
                page + 1,
                kdraft_render::view::ratio(*scale)
            ),
            Placement::New { scale, .. } if n == 1 => format!(
                "new placement, centred at {}",
                kdraft_render::view::ratio(*scale)
            ),
            Placement::New { scale, .. } => format!(
                "new placement: the {} view is centred at {}, the others link to it",
                self.layers[0],
                kdraft_render::view::ratio(*scale)
            ),
        };
        let mut s = format!("{pages}; {how}");
        if let Some(c) = s.get(..1) {
            s = c.to_uppercase() + &s[1..];
        }
        if !self.skipped.is_empty() {
            s.push_str(&format!(
                " ({} already ha{} a page)",
                self.skipped.join(", "),
                if self.skipped.len() == 1 { "s" } else { "ve" }
            ));
        }
        s
    }
}

/// The command.
pub(crate) fn run(s: &mut Session, a: LayerPagesArgs) -> R<Outcome> {
    let plan = plan(s, &a)?;
    let summary = plan.summary();
    if plan.layers.is_empty() {
        let own = match &plan.placement {
            Placement::Existing { root, .. } => own_colour_count(s.doc(), root),
            Placement::New { .. } => 0,
        };
        return Ok(Outcome::default().msg(with_own(summary, own)));
    }
    let mut d = s.doc().clone();
    let mut root = match &plan.placement {
        Placement::Existing { root, .. } => Some(root.clone()),
        Placement::New { .. } => None,
    };
    let mut created = Vec::new();
    let mut first_page = None;
    for (at, (k, layer)) in (plan.at..).zip(plan.layers.iter().enumerate()) {
        let mut ls = vec![layer.clone()];
        ls.extend(plan.common.iter().filter(|c| *c != layer).cloned());
        let v = View {
            id: Id::new_v4(),
            kind: ViewKind::Layer,
            layers: ls,
            parent: None,
            extra: Vec::new(),
            ..plan.template.clone()
        };
        let vid = v.id.clone();
        let mut it = Item::View(v);
        match &root {
            Some(r) => {
                it.set_link(Some(&Link {
                    to: r.clone(),
                    // Colours too: a set of layer pages looks uniform.
                    follow: vec![
                        Follow::Position,
                        Follow::Rotation,
                        Follow::Scale,
                        Follow::Colours,
                    ],
                    offset: Point::ORIGIN,
                    extra: Vec::new(),
                }));
            }
            // New placement: the first view is the reference.
            None => root = Some(vid.clone()),
        }
        let pid = if k == 0 && plan.use_current {
            let p = &mut d.pages[at];
            p.title = Some(layer.clone());
            p.size = plan.size.clone();
            p.sheet = plan.sheet.clone();
            p.items.push(it);
            p.id.clone()
        } else {
            let mut page = Page::new(layer, plan.size.clone());
            page.sheet = plan.sheet.clone();
            page.items.push(it);
            let pid = page.id.clone();
            d = ops::add_page(&d, page, Some(at.min(d.pages.len())))?;
            pid
        };
        first_page.get_or_insert(pid.clone());
        created.push(pid);
        created.push(vid);
    }
    link::resolve(&mut d);
    let own = root.as_ref().map_or(0, |r| own_colour_count(&d, r));
    s.set_doc(d);
    if let Some(pid) = first_page {
        s.selection = Selection {
            page: Some(pid),
            items: vec![],
        };
    }
    Ok(Outcome::created(created).msg(with_own(summary, own)))
}

fn with_own(summary: String, own: usize) -> String {
    if own > 0 {
        format!(
            "{summary}; {own} view(s) of the set keep their own colours (Match layer colours to reference makes them uniform)"
        )
    } else {
        summary
    }
}

/// Followers of `root` that do not follow its colours and draw a shared
/// layer differently (linked by hand, or made before colours followed).
fn own_colour_count(d: &kdraft_doc::Document, root: &Id) -> usize {
    let ix = link::index(d);
    let Some(Item::View(rv)) = link::get(d, &ix, root) else {
        return 0;
    };
    link::followers(d, root)
        .iter()
        .filter_map(|f| match link::get(d, &ix, f) {
            Some(it @ Item::View(v)) => Some((it.link(), v)),
            _ => None,
        })
        .filter(|(l, v)| {
            !l.as_ref().is_some_and(|l| l.follows(Follow::Colours))
                && !link::colour_differences(&d.styles, rv, v).is_empty()
        })
        .count()
}
