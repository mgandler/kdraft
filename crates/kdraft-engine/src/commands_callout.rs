//! Callout commands: placing with an automatic label, editing, bumping and
//! renumbering labels, switching what the balloon shows, the balloon shape
//! and the document's numbering options.

use kdraft_doc::callout::{self as numbering, ReadingOrder};
use kdraft_doc::{Callout, CalloutKind, Id, Item, Note, REFDES_TARGET};
use serde::Deserialize;

use crate::commands::{
    add_item, anchor_arg, default_note_list, ids_arg, item_mut, notes_add_list, p, page_arg,
    select_items, spec, target_view, with_dependents, AddNotesArgs, P2,
};
use crate::error::{EngineError, R};
use crate::registry::{opt, req, ArgType as T, CommandKind, Outcome, Registry};
use crate::session::Session;

const KINDS: &[&str] = &["label", "note", "bom", "refdes"];
const ORDERS: &[&str] = &["rows", "columns", "selection"];

pub(crate) fn register(r: &mut Registry) {
    use CommandKind::Edit;
    let ids = || {
        opt(
            "ids",
            T::IdList,
            "callout ids (default: the selected callouts)",
        )
    };
    r.add(
        spec(
            "item.add_callout",
            "Add callout",
            Edit,
            "Adds a callout balloon at `at` with a leader to an anchor in a view. By default it shows a label: the given one, else the next in the page's numbering (1, 2, 3...; A1, A2...). kind note/bom/refdes shows a note number, the part's BOM item number or its designator instead.",
            vec![
                opt("view", T::Id, "view (default: selected or first board view on the page)"),
                req("anchor", T::String, "anchor (S-expression)"),
                req("at", T::Point, "balloon centre, sheet mm"),
                opt("label", T::String, "label text (default: the next label)"),
                opt("kind", T::Enum { values: KINDS }, "what the balloon shows (default label, or from target)"),
                opt("target", T::String, "note:ID, bom:KEY or refdes (default for kind note/bom: the part's BOM line, else the first note on the page, else a new note list)"),
            ],
        ),
        callout_add,
    );
    r.add(
        spec(
            "callout.set_label",
            "Set callout label",
            Edit,
            "Sets the label of callouts (they then show it instead of a note, BOM item or designator).",
            vec![ids(), req("label", T::String, "label text, e.g. 7, A3, 3a")],
        ),
        set_label,
    );
    r.add(
        spec(
            "callout.bump",
            "Change callout number",
            Edit,
            "Adds `delta` to the number at the end of each callout's label (A9 + 1 = A10); callouts showing a derived number take it as their label first.",
            vec![ids(), opt("delta", T::Integer, "amount (default 1)")],
        ),
        bump,
    );
    r.add(
        spec(
            "callout.set_kind",
            "Set callout content",
            Edit,
            "Sets what callouts show: label (keeps the number shown now), note (target note id, or \"new\" to append a note), bom (the anchored part's BOM item) or refdes (the anchored part's designator).",
            vec![
                ids(),
                req("kind", T::Enum { values: KINDS }, "content kind"),
                opt("note", T::String, "note id (N1) or \"new\" (kind note; default: the first note on the page, else new)"),
                opt("text", T::String, "text of a new note"),
            ],
        ),
        set_kind,
    );
    r.add(
        spec(
            "callout.renumber",
            "Renumber callouts",
            CommandKind::Edit,
            "Numbers label callouts start, start+step, ... in reading order (rows: top to bottom, then left to right; columns: left to right first; selection: the given order). Works on the given or selected callouts, else on all label callouts of the page.",
            vec![
                ids(),
                opt("order", T::Enum { values: ORDERS }, "order (default rows)"),
                opt("start", T::Integer, "first number (default 1)"),
                opt("step", T::Integer, "step (default 1)"),
                opt("prefix", T::String, "text before the number (default none)"),
            ],
        ),
        renumber,
    );
    r.add(
        spec(
            "callout.set_shape",
            "Set balloon shape",
            Edit,
            "Sets the balloon outline of callouts: circle, square, hexagon or none (text only).",
            vec![
                ids(),
                req(
                    "shape",
                    T::Enum {
                        values: &["circle", "square", "hexagon", "none"],
                    },
                    "outline",
                ),
            ],
        ),
        set_shape,
    );
    r.add(
        spec(
            "callout.duplicate_next",
            "Duplicate with next number",
            Edit,
            "Duplicates the selected items; label callouts among the copies get the next labels.",
            vec![ids(), opt("offset", T::Point, "offset, mm (default 5, 5)")],
        ),
        duplicate_next,
    );
    r.add(
        spec(
            "document.set_callout_numbering",
            "Callout numbering",
            Edit,
            "Numbering scope of new callout labels (page or document) and whether checks report gaps in label sequences.",
            vec![
                opt("scope", T::Enum { values: &["page", "document"] }, "numbering scope"),
                opt("strict", T::Bool, "report gaps (info)"),
            ],
        ),
        set_numbering,
    );
}

/// The text a callout shows now (its label, or the number of its note or
/// BOM item, or the designator).
pub fn shown_label(s: &Session, c: &Callout) -> Option<String> {
    let bom = s.effective_bom();
    let numbers = kdraft_board::bom::number_lines(
        &bom.lines,
        &kdraft_doc::bom_numbers(s.doc(), bom.source.tag()),
    );
    let lookup = |key: &str| {
        bom.lines
            .iter()
            .position(|l| l.key() == key)
            .and_then(|i| numbers.get(i).copied())
    };
    kdraft_render::notes::callout_label(s.doc(), c, &lookup)
}

/// The label a new callout on page `pi` gets.
pub fn next_label(s: &Session, pi: usize) -> String {
    numbering::next_on_page(s.doc(), pi)
}

/// Page index and the callouts among `ids` (default: the selection).
fn callouts(s: &Session, ids: Option<Vec<String>>) -> R<(usize, Vec<Id>)> {
    let (pi, ids) = ids_arg(s, ids)?;
    let page = &s.doc().pages[pi];
    let cs: Vec<Id> = ids
        .into_iter()
        .filter(|i| matches!(page.item(i), Some(Item::Callout(_))))
        .collect();
    if cs.is_empty() {
        return Err(EngineError::invalid("no callout among the items"));
    }
    Ok((pi, cs))
}

fn callout_mut<'a>(s: &'a mut Session, pi: usize, id: &Id) -> R<&'a mut Callout> {
    match item_mut(s, pi, id)? {
        Item::Callout(c) => Ok(c),
        _ => Err(EngineError::invalid(format!("item {id} is not a callout"))),
    }
}

fn get_callout(s: &Session, pi: usize, id: &Id) -> R<Callout> {
    match s.doc().pages[pi].item(id) {
        Some(Item::Callout(c)) => Ok(c.clone()),
        _ => Err(EngineError::invalid(format!("item {id} is not a callout"))),
    }
}

fn check_label(label: &str) -> R<String> {
    let t = label.trim();
    if t.is_empty() {
        return Err(EngineError::invalid("a callout label cannot be empty"));
    }
    if t.contains('\n') {
        return Err(EngineError::invalid("a callout label is one line"));
    }
    Ok(t.to_string())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddArgs {
    view: Option<String>,
    anchor: String,
    at: P2,
    label: Option<String>,
    kind: Option<String>,
    target: Option<String>,
}

fn parse_kind(k: &str) -> R<CalloutKind> {
    CalloutKind::parse(k)
        .ok_or_else(|| EngineError::invalid(format!("kind must be one of {}", KINDS.join(", "))))
}

fn callout_add(s: &mut Session, a: AddArgs) -> R<Outcome> {
    let (pi, vid) = target_view(s, a.view)?;
    let anchor = anchor_arg(&a.anchor)?;
    if let Some(b) = s.board() {
        kdraft_doc::anchor::resolve(&anchor, b, &b.holes())
            .map_err(|e| EngineError::invalid(format!("anchor does not resolve: {e}")))?;
    }
    let kind = match (&a.kind, &a.target) {
        (Some(k), _) => parse_kind(k)?,
        (None, Some(t)) => target_kind(t)?,
        (None, None) => CalloutKind::Label,
    };
    let mut c = Callout::new(p(a.at)?, Some(vid), Some(anchor), "");
    c.label = None;
    let mut msg = None;
    match kind {
        CalloutKind::Label => {
            let label = match a.label {
                Some(l) => check_label(&l)?,
                None => next_label(s, pi),
            };
            c.label = Some(label);
        }
        CalloutKind::Refdes => {
            refdes_ok(s, &c)?;
            c.target = Some(REFDES_TARGET.to_string());
        }
        CalloutKind::Bom | CalloutKind::Note => {
            let (t, m) = match a.target {
                Some(t) => {
                    target_kind(&t)?;
                    (t, None)
                }
                None => default_target(s, pi, &c, kind)?,
            };
            c.target = Some(t);
            msg = m;
        }
    }
    let o = add_item(s, pi, Item::Callout(c))?;
    Ok(match msg {
        Some(m) => o.msg(m),
        None => o,
    })
}

fn target_kind(t: &str) -> R<CalloutKind> {
    if t == REFDES_TARGET {
        Ok(CalloutKind::Refdes)
    } else if t.starts_with("bom:") {
        Ok(CalloutKind::Bom)
    } else if t.starts_with("note:") {
        Ok(CalloutKind::Note)
    } else {
        Err(EngineError::invalid(
            "target must be note:ID, bom:KEY or refdes",
        ))
    }
}

fn refdes_ok(s: &Session, c: &Callout) -> R<()> {
    let r = c.anchor_reference().ok_or_else(|| {
        EngineError::invalid("a designator callout needs an anchor on a part (pad or origin)")
    })?;
    if let Some(b) = s.board() {
        if !b
            .footprints
            .iter()
            .any(|f| f.reference.as_deref() == Some(r))
        {
            return Err(EngineError::invalid(format!("the board has no part {r}")));
        }
    }
    Ok(())
}

/// The BOM line key of the anchored part.
fn bom_target(s: &Session, c: &Callout) -> Option<String> {
    let r = c.anchor_reference()?;
    let bom = s.effective_bom();
    bom.lines
        .iter()
        .find(|l| l.refs.iter().any(|x| x == r))
        .map(|l| format!("bom:{}", l.key()))
}

/// Appends a note to the page's note list (starting one if there is
/// none); returns its id and a message when a list was started.
fn new_note(s: &mut Session, pi: usize, text: &str) -> R<(String, Option<&'static str>)> {
    match default_note_list(s)? {
        Some((lpi, lid)) => {
            let Item::NoteList(n) = item_mut(s, lpi, &lid)? else {
                return Err(EngineError::invalid("not a note list"));
            };
            let nid = n.next_id();
            n.notes.push(Note {
                id: nid.clone(),
                text: text.to_string(),
            });
            Ok((nid, None))
        }
        None => {
            let _ = pi;
            notes_add_list(
                s,
                AddNotesArgs {
                    at: None,
                    width: None,
                    title: None,
                    notes: Some(vec![text.to_string()]),
                },
            )?;
            Ok((
                "N1".to_string(),
                Some("added a note list with a note for the callout (edit its text)"),
            ))
        }
    }
}

/// Target for a BOM or note callout without one: the part's BOM line,
/// else the first note on the page, else a new note.
fn default_target(
    s: &mut Session,
    pi: usize,
    c: &Callout,
    kind: CalloutKind,
) -> R<(String, Option<&'static str>)> {
    if kind == CalloutKind::Bom {
        if let Some(t) = bom_target(s, c) {
            return Ok((t, None));
        }
    }
    if let Some((lpi, lid)) = default_note_list(s)? {
        if let Some(Item::NoteList(n)) = s.doc().pages[lpi].item(&lid) {
            if let Some(first) = n.notes.first() {
                return Ok((format!("note:{}", first.id), None));
            }
        }
    }
    let text = c
        .anchor_reference()
        .map_or("Note".to_string(), |r| format!("Note for {r}"));
    let (nid, m) = new_note(s, pi, &text)?;
    Ok((
        format!("note:{nid}"),
        m.or(Some(
            "no BOM line or note to point at: added a note for the callout (edit its text)",
        )),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LabelArgs {
    ids: Option<Vec<String>>,
    label: String,
}

fn set_label(s: &mut Session, a: LabelArgs) -> R<Outcome> {
    let label = check_label(&a.label)?;
    let (pi, ids) = callouts(s, a.ids)?;
    for id in &ids {
        callout_mut(s, pi, id)?.set_label(&label);
    }
    Ok(Outcome::changed(ids))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BumpArgs {
    ids: Option<Vec<String>>,
    delta: Option<i64>,
}

fn bump(s: &mut Session, a: BumpArgs) -> R<Outcome> {
    let delta = a.delta.unwrap_or(1);
    let (pi, ids) = callouts(s, a.ids)?;
    for id in &ids {
        let c = get_callout(s, pi, id)?;
        let cur = match c.kind() {
            CalloutKind::Label => c.label.clone(),
            _ => shown_label(s, &c),
        }
        .unwrap_or_else(|| "0".to_string());
        callout_mut(s, pi, id)?.set_label(&numbering::bump(&cur, delta));
    }
    Ok(Outcome::changed(ids))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KindArgs {
    ids: Option<Vec<String>>,
    kind: String,
    note: Option<String>,
    text: Option<String>,
}

fn set_kind(s: &mut Session, a: KindArgs) -> R<Outcome> {
    let kind = parse_kind(&a.kind)?;
    let (pi, ids) = callouts(s, a.ids)?;
    let mut msg = None;
    // A new note is shared by all the callouts.
    let mut made_note: Option<String> = None;
    for id in &ids {
        let c = get_callout(s, pi, id)?;
        match kind {
            CalloutKind::Label => {
                let label = match c.kind() {
                    CalloutKind::Label => c.label.clone(),
                    _ => shown_label(s, &c),
                }
                .unwrap_or_else(|| next_label(s, pi));
                callout_mut(s, pi, id)?.set_label(&label);
            }
            CalloutKind::Refdes => {
                refdes_ok(s, &c)?;
                let m = callout_mut(s, pi, id)?;
                m.target = Some(REFDES_TARGET.to_string());
                m.label = None;
            }
            CalloutKind::Bom => {
                let t = bom_target(s, &c).ok_or_else(|| {
                    EngineError::invalid(match c.anchor_reference() {
                        Some(r) => format!("{r} has no BOM line"),
                        None => "the callout's anchor is not on a part".to_string(),
                    })
                })?;
                let m = callout_mut(s, pi, id)?;
                m.target = Some(t);
                m.label = None;
            }
            CalloutKind::Note => {
                let nid = match a.note.as_deref() {
                    Some("new") => match &made_note {
                        Some(n) => n.clone(),
                        None => {
                            let text = a.text.clone().unwrap_or_else(|| "New note".to_string());
                            let (n, m) = new_note(s, pi, &text)?;
                            msg = m;
                            made_note = Some(n.clone());
                            n
                        }
                    },
                    Some(n) => n.strip_prefix("note:").unwrap_or(n).to_string(),
                    None => {
                        let (t, m) = default_target(s, pi, &c, CalloutKind::Note)?;
                        msg = msg.or(m);
                        t.trim_start_matches("note:").to_string()
                    }
                };
                let m = callout_mut(s, pi, id)?;
                m.target = Some(format!("note:{nid}"));
                m.label = None;
            }
        }
    }
    let o = Outcome::changed(ids);
    Ok(match msg {
        Some(m) => o.msg(m),
        None => o,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenumberArgs {
    ids: Option<Vec<String>>,
    order: Option<String>,
    start: Option<i64>,
    step: Option<i64>,
    prefix: Option<String>,
}

fn renumber(s: &mut Session, a: RenumberArgs) -> R<Outcome> {
    let pi = page_arg(s, None)?;
    let page = &s.doc().pages[pi];
    let is_label =
        |i: &Id| matches!(page.item(i), Some(Item::Callout(c)) if c.kind() == CalloutKind::Label);
    let given: Vec<Id> = match a.ids {
        Some(v) => v.into_iter().map(Id).collect(),
        None => s.selection.items.clone(),
    };
    let mut ids: Vec<Id> = given.into_iter().filter(|i| is_label(i)).collect();
    if ids.is_empty() {
        ids = page
            .items
            .iter()
            .filter_map(|it| match it {
                Item::Callout(c) if c.kind() == CalloutKind::Label => Some(c.id.clone()),
                _ => None,
            })
            .collect();
    }
    if ids.is_empty() {
        return Err(EngineError::invalid(
            "no numbered callouts on this page (callouts showing a note, BOM item or designator are not renumbered)",
        ));
    }
    let order = a.order.as_deref().unwrap_or("rows");
    if !ORDERS.contains(&order) {
        return Err(EngineError::invalid(format!(
            "order must be one of {}",
            ORDERS.join(", ")
        )));
    }
    let start = a.start.unwrap_or(1);
    let step = a.step.unwrap_or(1);
    if start < 0 || step < 1 {
        return Err(EngineError::invalid(
            "start must be 0 or more and step 1 or more",
        ));
    }
    let prefix = a.prefix.unwrap_or_default();
    if prefix.contains('\n') {
        return Err(EngineError::invalid("the prefix is one line"));
    }
    if order != "selection" {
        let pts: Vec<_> = ids
            .iter()
            .filter_map(|i| page.item(i).and_then(Item::position))
            .collect();
        let band = s.doc().styles.text_height.max(1.0);
        let ro = if order == "columns" {
            ReadingOrder::Columns
        } else {
            ReadingOrder::Rows
        };
        let idx = numbering::reading_order(&pts, ro, band);
        ids = idx.into_iter().map(|i| ids[i].clone()).collect();
    }
    for (k, id) in ids.iter().enumerate() {
        let n = start + step * k as i64;
        callout_mut(s, pi, id)?.set_label(&format!("{prefix}{n}"));
    }
    let n = ids.len();
    Ok(Outcome::changed(ids).msg(format!("{n} callout(s) renumbered")))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShapeArgs {
    ids: Option<Vec<String>>,
    shape: String,
}

fn set_shape(s: &mut Session, a: ShapeArgs) -> R<Outcome> {
    let (pi, ids) = callouts(s, a.ids)?;
    for id in &ids {
        let it = item_mut(s, pi, id)?;
        let mut st = it.style_override();
        st.set("shape", Some(&a.shape))
            .map_err(|e| EngineError::invalid(e.to_string()))?;
        it.set_style_override(&st);
    }
    Ok(Outcome::changed(ids))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DupArgs {
    ids: Option<Vec<String>>,
    offset: Option<P2>,
}

fn duplicate_next(s: &mut Session, a: DupArgs) -> R<Outcome> {
    let (pi, ids) = ids_arg(s, a.ids)?;
    let off = p(a
        .offset
        .unwrap_or([crate::DUPLICATE_OFFSET, crate::DUPLICATE_OFFSET]))?;
    let page = s.doc().pages[pi].id.clone();
    let all = with_dependents(s.doc(), &ids);
    let (d, new) = kdraft_doc::ops::copy_items(s.doc(), &all, &page, off, &mut Id::new_v4)?;
    s.set_doc(d);
    // The copies, in order: each label callout takes the next label.
    for id in &new {
        let is_label = matches!(
            s.doc().pages[pi].item(id),
            Some(Item::Callout(c)) if c.kind() == CalloutKind::Label
        );
        if is_label {
            // The scope without this copy, so it continues the sequence.
            let mut d = s.doc().clone();
            d.pages[pi].items.retain(|i| i.id() != Some(id));
            let next = numbering::next_on_page(&d, pi);
            callout_mut(s, pi, id)?.set_label(&next);
        }
    }
    select_items(s, pi, new[..ids.len()].to_vec());
    Ok(Outcome::created(&new))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NumberingArgs {
    scope: Option<String>,
    strict: Option<bool>,
}

fn set_numbering(s: &mut Session, a: NumberingArgs) -> R<Outcome> {
    if a.scope.is_none() && a.strict.is_none() {
        return Err(EngineError::invalid("give scope or strict"));
    }
    let st = &mut s.doc_mut().styles;
    if let Some(sc) = a.scope.as_deref() {
        st.callouts_per_document = match sc {
            "page" => false,
            "document" => true,
            _ => return Err(EngineError::invalid("scope must be page or document")),
        };
    }
    if let Some(v) = a.strict {
        st.strict_callout_numbering = v;
    }
    Ok(Outcome::default())
}
