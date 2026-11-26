//! Callout tests: automatic labels, editing, bumping, renumbering, content
//! kinds, shapes, duplicates, numbering options, the file format and
//! property tests over random edits.

use std::path::PathBuf;

use kdraft_doc::{Callout, CalloutKind, Document, Item};
use proptest::prelude::*;
use serde_json::{json, Value};

use crate::{Registry, Session};

fn session() -> Session {
    let mut s = Session::new_for_board(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/kicad9-minimal.kicad_pcb"),
        None,
    )
    .unwrap();
    let p0 = s.doc().pages[0].id.0.clone();
    Registry::new()
        .execute(&mut s, "select.set", json!({ "page": p0, "ids": [] }))
        .unwrap();
    s
}

fn reference(s: &Session) -> String {
    s.board()
        .unwrap()
        .footprints
        .iter()
        .find_map(|f| f.reference.clone())
        .unwrap()
}

fn anchor(s: &Session) -> String {
    format!("(fp-origin \"{}\")", reference(s))
}

fn add(r: &Registry, s: &mut Session, at: [f64; 2], extra: Value) -> String {
    let mut args = json!({ "anchor": anchor(s), "at": at });
    if let (Some(a), Some(e)) = (args.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            a.insert(k.clone(), v.clone());
        }
    }
    r.execute(s, "item.add_callout", args).unwrap().created[0].clone()
}

fn callout(s: &Session, id: &str) -> Callout {
    match s.find_item(&kdraft_doc::Id(id.into())).unwrap().1 {
        Item::Callout(c) => c.clone(),
        o => panic!("{o:?}"),
    }
}

fn label(s: &Session, id: &str) -> String {
    callout(s, id).label.unwrap_or_default()
}

fn shown(s: &Session, id: &str) -> String {
    crate::commands_callout::shown_label(s, &callout(s, id)).unwrap_or_default()
}

#[test]
fn placing_numbers_automatically_and_continues_after_edits() {
    let r = Registry::new();
    let mut s = session();
    let a = add(&r, &mut s, [300.0, 50.0], json!({}));
    let b = add(&r, &mut s, [310.0, 50.0], json!({}));
    let c = add(&r, &mut s, [320.0, 50.0], json!({}));
    assert_eq!(
        [label(&s, &a), label(&s, &b), label(&s, &c)],
        ["1", "2", "3"]
    );
    r.execute(
        &mut s,
        "callout.set_label",
        json!({ "ids": [b], "label": "7" }),
    )
    .unwrap();
    let d = add(&r, &mut s, [330.0, 50.0], json!({}));
    assert_eq!(label(&s, &d), "8");
    // A prefixed series continues with its own prefix.
    r.execute(
        &mut s,
        "callout.set_label",
        json!({ "ids": [d], "label": "A1" }),
    )
    .unwrap();
    let e = add(&r, &mut s, [340.0, 50.0], json!({}));
    assert_eq!(label(&s, &e), "A2");
    // An explicit label.
    let f = add(&r, &mut s, [350.0, 50.0], json!({ "label": "3a" }));
    assert_eq!(label(&s, &f), "3a");
    assert!(r
        .execute(
            &mut s,
            "callout.set_label",
            json!({ "ids": [f], "label": " " })
        )
        .is_err());
    // Bump: one, several, below zero stops at 0.
    r.execute(&mut s, "callout.bump", json!({ "ids": [f] }))
        .unwrap();
    assert_eq!(label(&s, &f), "4a");
    r.execute(
        &mut s,
        "callout.bump",
        json!({ "ids": [a, c], "delta": -2 }),
    )
    .unwrap();
    assert_eq!([label(&s, &a), label(&s, &c)], ["0", "1"]);
    // Duplicates keep the label; "with next number" continues.
    r.execute(&mut s, "select.set", json!({ "ids": [e] }))
        .unwrap();
    let o = r.execute(&mut s, "item.duplicate", json!({})).unwrap();
    assert_eq!(label(&s, &o.created[0]), "A2");
    let o = r
        .execute(&mut s, "callout.duplicate_next", json!({ "ids": [e] }))
        .unwrap();
    assert_eq!(label(&s, &o.created[0]), "A3");
}

#[test]
fn renumber_in_reading_order() {
    let r = Registry::new();
    let mut s = session();
    // Two rows, placed out of order.
    let pts = [[330.0, 80.0], [300.0, 50.5], [320.0, 50.0], [300.0, 81.0]];
    let ids: Vec<String> = pts.iter().map(|p| add(&r, &mut s, *p, json!({}))).collect();
    r.execute(&mut s, "select.set", json!({ "ids": [] }))
        .unwrap();
    r.execute(&mut s, "callout.renumber", json!({})).unwrap();
    let got: Vec<String> = ids.iter().map(|i| label(&s, i)).collect();
    assert_eq!(got, ["4", "1", "2", "3"]);
    r.execute(
        &mut s,
        "callout.renumber",
        json!({ "order": "columns", "start": 10, "step": 5, "prefix": "P" }),
    )
    .unwrap();
    let got: Vec<String> = ids.iter().map(|i| label(&s, i)).collect();
    // Columns: x 300 (y 50.5 then 81), x 320, x 330.
    assert_eq!(got, ["P25", "P10", "P20", "P15"]);
    // Selection order on two of them.
    r.execute(
        &mut s,
        "callout.renumber",
        json!({ "ids": [ids[3], ids[0]], "order": "selection" }),
    )
    .unwrap();
    assert_eq!([label(&s, &ids[3]), label(&s, &ids[0])], ["1", "2"]);
    // One undo step.
    s.undo();
    assert_eq!([label(&s, &ids[3]), label(&s, &ids[0])], ["P15", "P25"]);
}

#[test]
fn content_kinds_and_shapes() {
    let r = Registry::new();
    let mut s = session();
    let a = add(&r, &mut s, [300.0, 50.0], json!({}));
    // BOM item: shows the part's number; back to a label keeps it.
    r.execute(
        &mut s,
        "callout.set_kind",
        json!({ "ids": [a], "kind": "bom" }),
    )
    .unwrap();
    assert_eq!(callout(&s, &a).kind(), CalloutKind::Bom);
    let n = shown(&s, &a);
    assert!(n.parse::<u32>().is_ok(), "{n}");
    r.execute(
        &mut s,
        "callout.set_kind",
        json!({ "ids": [a], "kind": "label" }),
    )
    .unwrap();
    assert_eq!(callout(&s, &a).kind(), CalloutKind::Label);
    assert_eq!(label(&s, &a), n);
    // Designator.
    r.execute(
        &mut s,
        "callout.set_kind",
        json!({ "ids": [a], "kind": "refdes" }),
    )
    .unwrap();
    assert_eq!(shown(&s, &a), reference(&s));
    // A new note, then the first note.
    let o = r
        .execute(
            &mut s,
            "callout.set_kind",
            json!({ "ids": [a], "kind": "note", "note": "new", "text": "Glue" }),
        )
        .unwrap();
    let c = callout(&s, &a);
    assert_eq!(c.kind(), CalloutKind::Note);
    let nid = c.target.unwrap().trim_start_matches("note:").to_string();
    let note_text = s
        .doc()
        .pages
        .iter()
        .flat_map(|p| p.items.iter())
        .find_map(|i| match i {
            Item::NoteList(l) => l.notes.iter().find(|n| n.id == nid).map(|n| n.text.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(note_text, "Glue", "{o:?}");
    // A callout not on a part cannot show a BOM item or designator.
    let b = r
        .execute(
            &mut s,
            "item.add_callout",
            json!({ "anchor": "(outline-vertex 0)", "at": [300, 70] }),
        )
        .unwrap()
        .created[0]
        .clone();
    assert!(r
        .execute(
            &mut s,
            "callout.set_kind",
            json!({ "ids": [b], "kind": "bom" })
        )
        .is_err());
    assert!(r
        .execute(
            &mut s,
            "callout.set_kind",
            json!({ "ids": [b], "kind": "refdes" })
        )
        .is_err());
    // Shapes go through the style model.
    r.execute(
        &mut s,
        "callout.set_shape",
        json!({ "ids": [b], "shape": "hexagon" }),
    )
    .unwrap();
    let st = Item::Callout(callout(&s, &b)).effective_style(&s.doc().styles);
    assert_eq!(st.shape, Some(kdraft_doc::style::BalloonShape::Hexagon));
    assert!(r
        .execute(
            &mut s,
            "callout.set_shape",
            json!({ "ids": [b], "shape": "star" })
        )
        .is_err());
}

#[test]
fn numbering_scope_and_strict_check() {
    let r = Registry::new();
    let mut s = session();
    add(&r, &mut s, [300.0, 50.0], json!({}));
    add(&r, &mut s, [310.0, 50.0], json!({ "label": "4" }));
    // Page scope: a new page starts at 1.
    r.execute(&mut s, "page.add", json!({})).unwrap();
    let pi = s.current_page_index().unwrap();
    assert_eq!(crate::commands_callout::next_label(&s, pi), "1");
    r.execute(
        &mut s,
        "document.set_callout_numbering",
        json!({ "scope": "document" }),
    )
    .unwrap();
    assert_eq!(crate::commands_callout::next_label(&s, pi), "5");
    // Gaps are reported only in strict mode, as info.
    let gap = |s: &mut Session| {
        crate::check::check(s)
            .findings
            .iter()
            .any(|f| f.code == "callout-gap")
    };
    assert!(!gap(&mut s));
    r.execute(
        &mut s,
        "document.set_callout_numbering",
        json!({ "strict": true }),
    )
    .unwrap();
    assert!(gap(&mut s));
    let rep = crate::check::check(&mut s);
    let f = rep
        .findings
        .iter()
        .find(|f| f.code == "callout-gap")
        .unwrap();
    assert_eq!(f.severity, crate::check::Severity::Info);
    assert!(f.message.contains("2, 3"), "{}", f.message);
    // Round trip of the options.
    let text = s.doc().to_text().unwrap();
    assert!(text.contains("(callout_numbering document)"), "{text}");
    assert!(text.contains("(strict_callout_numbering yes)"));
    assert_eq!(Document::parse(&text).unwrap(), *s.doc());
}

#[test]
fn file_format_round_trip_and_old_files() {
    let r = Registry::new();
    let mut s = session();
    let a = add(&r, &mut s, [300.0, 50.0], json!({ "label": "12" }));
    r.execute(
        &mut s,
        "callout.set_shape",
        json!({ "ids": [a], "shape": "square" }),
    )
    .unwrap();
    add(&r, &mut s, [310.0, 50.0], json!({ "kind": "refdes" }));
    let text = s.doc().to_text().unwrap();
    assert!(text.contains("(label \"12\")"), "{text}");
    assert!(text.contains("(target \"refdes\")"));
    assert!(text.contains("(shape square)"));
    assert_eq!(Document::parse(&text).unwrap(), *s.doc());
    // An older file: target only, plus an unknown node, both kept.
    let old = text.replacen(
        "(label \"12\")",
        "(target \"note:N1\") (balloon_future 3)",
        1,
    );
    let d = Document::parse(&old).unwrap();
    let c = d
        .pages
        .iter()
        .flat_map(|p| p.items.iter())
        .find_map(|i| match i {
            Item::Callout(c) if c.target.as_deref() == Some("note:N1") => Some(c.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(c.kind(), CalloutKind::Note);
    assert_eq!(c.label, None);
    let out = d.to_text().unwrap();
    assert!(out.contains("(balloon_future 3)"), "{out}");
    assert_eq!(Document::parse(&out).unwrap(), d);
}

#[derive(Debug, Clone)]
enum Op {
    Add(f64, f64, Option<u8>),
    Set(usize, u8, u8),
    Bump(usize, i8),
    Kind(usize, u8),
    Renumber(u8, u8, u8),
    Shape(usize, u8),
    DupNext(usize),
    Remove(usize),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (
            250.0..400.0f64,
            30.0..250.0f64,
            proptest::option::of(0u8..3)
        )
            .prop_map(|(x, y, p)| Op::Add(x, y, p)),
        (0usize..10, 0u8..3, 0u8..30).prop_map(|(a, p, n)| Op::Set(a, p, n)),
        (0usize..10, -3i8..4).prop_map(|(a, d)| Op::Bump(a, d)),
        (0usize..10, 0u8..4).prop_map(|(a, k)| Op::Kind(a, k)),
        (0u8..3, 0u8..5, 1u8..4).prop_map(|(o, st, sp)| Op::Renumber(o, st, sp)),
        (0usize..10, 0u8..4).prop_map(|(a, k)| Op::Shape(a, k)),
        (0usize..10).prop_map(Op::DupNext),
        (0usize..10).prop_map(Op::Remove),
    ]
}

const PREFIXES: [&str; 3] = ["", "A", "B-"];

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

    /// Random callout edits: a new label is one more than the highest
    /// number with the most recent label's prefix; renumbering gives a
    /// contiguous sequence; undoing everything restores the document.
    #[test]
    fn random_callout_edits(ops in prop::collection::vec(op(), 1..25)) {
        let r = Registry::new();
        let mut s = session();
        let original = s.doc().clone();
        let anchor = anchor(&s);
        for o in &ops {
            let ids: Vec<String> = s.doc().pages[0].items.iter().filter_map(|i| match i {
                Item::Callout(c) => Some(c.id.0.clone()),
                _ => None,
            }).collect();
            let pick = |k: usize| ids.get(k % ids.len().max(1)).cloned();
            let (name, args) = match o {
                Op::Add(x, y, p) => {
                    let mut a = json!({ "anchor": anchor, "at": [x, y] });
                    if let Some(p) = p {
                        a["label"] = json!(format!("{}{}", PREFIXES[*p as usize], (*x as u32) % 9));
                    }
                    ("item.add_callout", a)
                }
                Op::Set(a, p, n) => match pick(*a) {
                    Some(a) => ("callout.set_label", json!({ "ids": [a], "label": format!("{}{n}", PREFIXES[*p as usize]) })),
                    None => continue,
                },
                Op::Bump(a, d) => match pick(*a) { Some(a) => ("callout.bump", json!({ "ids": [a], "delta": d })), None => continue },
                Op::Kind(a, k) => match pick(*a) {
                    Some(a) => ("callout.set_kind", json!({ "ids": [a], "kind": (["label", "note", "bom", "refdes"][*k as usize]) })),
                    None => continue,
                },
                Op::Renumber(o, st, sp) => ("callout.renumber", json!({ "ids": [], "order": (["rows", "columns", "selection"][*o as usize]), "start": st, "step": sp })),
                Op::Shape(a, k) => match pick(*a) {
                    Some(a) => ("callout.set_shape", json!({ "ids": [a], "shape": (["circle", "square", "hexagon", "none"][*k as usize]) })),
                    None => continue,
                },
                Op::DupNext(a) => match pick(*a) { Some(a) => ("callout.duplicate_next", json!({ "ids": [a] })), None => continue },
                Op::Remove(a) => match pick(*a) { Some(a) => ("item.remove", json!({ "ids": [a] })), None => continue },
            };
            let before = s.doc().clone();
            let expected_next = kdraft_doc::callout::next_on_page(&before, 0);
            match r.execute(&mut s, name, args.clone()) {
                Err(_) => prop_assert_eq!(s.doc(), &before),
                Ok(out) => {
                    if let (Op::Add(_, _, None), Some(id)) = (o, out.created.first()) {
                        // One more than the highest number with the most
                        // recent label's prefix and suffix.
                        prop_assert_eq!(label(&s, id), expected_next.clone());
                        let labels = kdraft_doc::callout::scope_labels(&before, 0);
                        if let Some(last) = labels.last() {
                            let lp = kdraft_doc::callout::split(last);
                            let max = labels.iter().map(|l| kdraft_doc::callout::split(l))
                                .filter(|p| p.prefix == lp.prefix && p.suffix == lp.suffix)
                                .filter_map(|p| p.number()).max().unwrap_or(0);
                            prop_assert_eq!(kdraft_doc::callout::split(&expected_next).number(), Some(max + 1));
                        } else {
                            prop_assert_eq!(expected_next.as_str(), "1");
                        }
                    }
                    if let Op::Renumber(_, st, sp) = o {
                        let mut nums: Vec<u64> = s.doc().pages[0].items.iter().filter_map(|i| match i {
                            Item::Callout(c) if c.kind() == CalloutKind::Label => c.label.as_deref().and_then(|l| l.parse().ok()),
                            _ => None,
                        }).collect();
                        nums.sort_unstable();
                        let want: Vec<u64> = (0..nums.len() as u64).map(|k| *st as u64 + *sp as u64 * k).collect();
                        prop_assert_eq!(nums, want);
                    }
                }
            }
        }
        while s.undo().is_some() {}
        prop_assert_eq!(s.doc(), &original);
    }
}

/// Renumbering in rows order numbers the callouts in reading order.
#[test]
fn renumber_rows_is_reading_order() {
    let r = Registry::new();
    let mut s = session();
    let mut placed = Vec::new();
    for (k, y) in [120.0, 60.0, 90.0].iter().enumerate() {
        for x in [360.0, 300.0] {
            placed.push((add(&r, &mut s, [x + k as f64, *y], json!({})), x, *y));
        }
    }
    r.execute(&mut s, "callout.renumber", json!({ "ids": [] }))
        .unwrap();
    let mut by_pos = placed.clone();
    by_pos.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.1.total_cmp(&b.1)));
    let got: Vec<String> = by_pos.iter().map(|(i, _, _)| label(&s, i)).collect();
    assert_eq!(got, ["1", "2", "3", "4", "5", "6"]);
}
