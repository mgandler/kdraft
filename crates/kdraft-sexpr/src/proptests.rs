use proptest::prelude::*;

use crate::{parse_values, to_canonical, Atom, Tree, Value};

fn atom_text() -> impl Strategy<Value = String> {
    prop_oneof![
        "[A-Za-z_.*+-][A-Za-z0-9_.*+-]{0,8}",
        "-?[0-9]{1,4}(\\.[0-9]{1,4})?",
        any::<String>(),
        "[ \"\\\\\\n\\r\\t()µΩ]{0,6}",
    ]
}

fn atom() -> impl Strategy<Value = Value> {
    prop_oneof![
        atom_text().prop_map(|t| Value::Atom(Atom::symbol(t))),
        atom_text().prop_map(|t| Value::Atom(Atom::string(t))),
        any::<f64>()
            .prop_filter("finite", |v| v.is_finite())
            .prop_map(|v| Value::Atom(Atom::number(v).unwrap_or_else(|_| Atom::int(0)))),
    ]
}

fn value() -> impl Strategy<Value = Value> {
    atom().prop_recursive(6, 64, 8, |inner| {
        prop::collection::vec(inner, 0..8).prop_map(Value::List)
    })
}

fn ws() -> impl Strategy<Value = String> {
    "[ \\t]{0,2}(\\r?\\n)?[ \\t]{0,2}"
}

/// Random KiCad-ish text with arbitrary whitespace, quoting and spelling.
fn raw_text() -> impl Strategy<Value = String> {
    let leaf = prop_oneof![
        "[A-Za-z0-9_.+-]{1,6}",
        "\"([^\"\\\\]|\\\\[\"\\\\nrtq]){0,6}\"",
        "-?[0-9]{1,3}\\.[0-9]{0,3}0*",
    ];
    leaf.prop_recursive(5, 48, 6, |inner| {
        (ws(), prop::collection::vec((inner, ws()), 0..6)).prop_map(|(w, items)| {
            let mut s = String::from("(");
            s.push_str(&w);
            for (i, (item, w)) in items.into_iter().enumerate() {
                if i > 0 && !w.is_empty() {
                    s.push_str(&w);
                } else if i > 0 {
                    s.push(' ');
                }
                s.push_str(&item);
            }
            s.push(')');
            s
        })
    })
    .prop_flat_map(|body| (ws(), Just(body), ws()))
    .prop_map(|(a, b, c)| format!("{a}{b}{c}"))
}

proptest! {
    #[test]
    fn canonical_print_parse_round_trip(vals in prop::collection::vec(value(), 1..4)) {
        let text = to_canonical(&vals);
        let back = parse_values(&text).unwrap();
        prop_assert_eq!(&back, &vals);
        // Canonical output is a fixed point.
        prop_assert_eq!(to_canonical(&back), text);
    }

    #[test]
    fn raw_text_round_trip_is_byte_exact(text in raw_text()) {
        let tree = Tree::parse(text.as_str()).unwrap();
        prop_assert_eq!(tree.to_text(), text);
    }

    #[test]
    fn any_parsable_text_round_trips(text in any::<String>()) {
        if let Ok(tree) = Tree::parse(text.as_str()) {
            prop_assert_eq!(tree.to_text(), text);
        }
    }

    #[test]
    fn lossless_and_canonical_agree_semantically(text in raw_text()) {
        let tree = Tree::parse(text.as_str()).unwrap();
        let vals: Vec<Value> = tree.top().map(|n| n.to_value()).collect();
        let canon = to_canonical(&vals);
        prop_assert_eq!(parse_values(&canon).unwrap(), vals);
    }
}
