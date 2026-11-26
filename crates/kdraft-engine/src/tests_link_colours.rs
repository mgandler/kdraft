//! Colours across placement links: layer pages look uniform, the
//! `colours` follow property, matching to the reference, the check, and
//! the user's case (a reference with raw KiCad colours, followers on the
//! document palette).

use std::path::PathBuf;

use kdraft_doc::link::Follow;
use kdraft_doc::palette::{Palette, KICAD_DEFAULT_NAME, KICAD_DEFAULT_PAPER, LAYER};
use kdraft_doc::style::Rgba;
use kdraft_doc::{Document, Id, Item, View};
use serde_json::json;

use crate::{Registry, Session};

fn session(p: Option<Palette>) -> Session {
    let mut s = Session::new_for_board_with(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/kicad9-minimal.kicad_pcb"),
        None,
        None,
        &Default::default(),
        p,
    )
    .unwrap();
    let p0 = s.doc().pages[0].id.0.clone();
    Registry::new()
        .execute(&mut s, "select.set", json!({ "page": p0, "ids": [] }))
        .unwrap();
    s
}

fn view(s: &Session, id: &str) -> View {
    match s.find_item(&Id(id.into())).unwrap().1 {
        Item::View(v) => v.clone(),
        o => panic!("{o:?}"),
    }
}

fn colour(s: &Session, id: &str, layer: &str) -> (Rgba, f64) {
    view(s, id).layer_color(&s.doc().styles, layer)
}

/// A layer set from a new placement: reference view and followers.
fn layer_set(r: &Registry, s: &mut Session) -> (String, Vec<String>) {
    let o = r
        .execute(
            s,
            "view.pages_per_layer",
            json!({ "reference": "new", "layers": ["F.Cu", "B.Cu", "F.SilkS"] }),
        )
        .unwrap();
    let views: Vec<String> = o.created.iter().skip(1).step_by(2).cloned().collect();
    (views[0].clone(), views[1..].to_vec())
}

const RAW_EDGE: &str = "#D0D2CD";

#[test]
fn layer_pages_from_a_reference_with_own_colours_are_uniform() {
    let r = Registry::new();
    let mut s = session(Palette::builtin(KICAD_DEFAULT_PAPER));
    // A reference with its own palette and an Edge.Cuts colour.
    let p0 = s.doc().pages[0].id.0.clone();
    let o = r
        .execute(
            &mut s,
            "item.add_view",
            json!({ "page": p0, "kind": "layer", "layers": ["F.Cu", "Edge.Cuts"], "at": [200, 150] }),
        )
        .unwrap();
    let refv = o.created[0].clone();
    r.execute(
        &mut s,
        "view.apply_palette",
        json!({ "ids": [refv], "name": LAYER }),
    )
    .unwrap();
    r.execute(
        &mut s,
        "view.set_layer_style",
        json!({ "ids": [refv], "layer": "Edge.Cuts", "color": "#112233" }),
    )
    .unwrap();
    let o = r
        .execute(
            &mut s,
            "view.pages_per_layer",
            json!({ "reference": refv, "layers": ["B.Cu", "F.SilkS"] }),
        )
        .unwrap();
    let followers: Vec<String> = o.created.iter().skip(1).step_by(2).cloned().collect();
    let layer = Palette::builtin(LAYER).unwrap();
    for f in &followers {
        assert_eq!(
            colour(&s, f, "Edge.Cuts"),
            colour(&s, &refv, "Edge.Cuts"),
            "{f}"
        );
        let v = view(&s, f);
        // The page's own layer: the palette in effect (the reference's).
        let own = &v.layers[0];
        assert_eq!(colour(&s, f, own), layer.layer(own).unwrap(), "{own}");
        let it = Item::View(v);
        assert!(it.link().unwrap().follows(Follow::Colours));
    }
}

#[test]
fn colours_follow_live_preview_unlink_promotion_and_files() {
    let r = Registry::new();
    let mut s = session(Palette::builtin(KICAD_DEFAULT_PAPER));
    let (refv, fs) = layer_set(&r, &mut s);
    // Live: the reference's Edge.Cuts colour reaches every page.
    r.execute(
        &mut s,
        "view.set_layer_style",
        json!({ "ids": [refv], "layer": "Edge.Cuts", "color": "#00AA00" }),
    )
    .unwrap();
    for f in &fs {
        assert_eq!(colour(&s, f, "Edge.Cuts").0, Rgba::rgb(0, 0xAA, 0));
    }
    // Preview (a gesture) too.
    s.begin_gesture("colour");
    r.preview(
        &mut s,
        "view.set_layer_style",
        json!({ "ids": [refv], "layer": "Edge.Cuts", "color": "#0000AA" }),
    )
    .unwrap();
    assert_eq!(colour(&s, &fs[0], "Edge.Cuts").0, Rgba::rgb(0, 0, 0xAA));
    s.cancel_gesture();
    assert_eq!(colour(&s, &fs[0], "Edge.Cuts").0, Rgba::rgb(0, 0xAA, 0));
    // A palette for the reference: followers take the choice.
    r.execute(
        &mut s,
        "view.apply_palette",
        json!({ "ids": [refv], "name": LAYER }),
    )
    .unwrap();
    for f in &fs {
        assert_eq!(view(&s, f).layer_colors().palette.unwrap().name, LAYER);
    }
    r.execute(
        &mut s,
        "view.set_layer_style",
        json!({ "ids": [refv], "layer": "Edge.Cuts", "color": "#00AA00" }),
    )
    .unwrap();
    // File round trip keeps the property.
    let text = s.doc().to_text().unwrap();
    assert!(text.contains("colours"), "{text}");
    assert_eq!(Document::parse(&text).unwrap(), *s.doc());
    // Unlink: the follower keeps the colours it had.
    r.execute(&mut s, "link.clear", json!({ "ids": [fs[0]] }))
        .unwrap();
    assert_eq!(colour(&s, &fs[0], "Edge.Cuts").0, Rgba::rgb(0, 0xAA, 0));
    r.execute(
        &mut s,
        "view.set_layer_style",
        json!({ "ids": [refv], "layer": "Edge.Cuts", "color": "#AA0000" }),
    )
    .unwrap();
    assert_eq!(colour(&s, &fs[0], "Edge.Cuts").0, Rgba::rgb(0, 0xAA, 0));
    assert_eq!(colour(&s, &fs[1], "Edge.Cuts").0, Rgba::rgb(0xAA, 0, 0));
    // Deleting the reference: the next one leads, colours stay.
    r.execute(&mut s, "item.remove", json!({ "ids": [refv] }))
        .unwrap();
    assert_eq!(colour(&s, &fs[1], "Edge.Cuts").0, Rgba::rgb(0xAA, 0, 0));
}

#[test]
fn old_links_without_colours_and_hand_made_links() {
    let r = Registry::new();
    let mut s = session(Palette::builtin(KICAD_DEFAULT_PAPER));
    let (refv, fs) = layer_set(&r, &mut s);
    // An old file: links follow position, rotation and scale only.
    let text = s.doc().to_text().unwrap().replace(" colours)", ")");
    assert!(!text.contains("colours"));
    s.set_doc(Document::parse(&text).unwrap());
    r.execute(
        &mut s,
        "view.set_layer_style",
        json!({ "ids": [refv], "layer": "Edge.Cuts", "color": RAW_EDGE }),
    )
    .unwrap();
    assert_ne!(
        colour(&s, &fs[0], "Edge.Cuts"),
        colour(&s, &refv, "Edge.Cuts")
    );
    // Hand-made links do not follow colours by default.
    let p0 = s.doc().pages[0].id.0.clone();
    let o = r
        .execute(
            &mut s,
            "item.add_view",
            json!({ "page": p0, "kind": "layer", "layers": ["Edge.Cuts"], "at": [50, 50] }),
        )
        .unwrap();
    r.execute(
        &mut s,
        "link.set",
        json!({ "ids": [o.created[0]], "to": refv }),
    )
    .unwrap();
    let l = s
        .find_item(&Id(o.created[0].clone()))
        .unwrap()
        .1
        .link()
        .unwrap();
    assert!(!l.follows(Follow::Colours));
    // The check finds the group.
    let rep = crate::check::check(&mut s);
    let f = rep
        .findings
        .iter()
        .find(|f| f.code == "link-colours")
        .unwrap();
    assert_eq!(f.severity, crate::check::Severity::Info);
    assert!(f.message.contains("Edge.Cuts"), "{}", f.message);
    // Updating the set reports the views that keep their own colours.
    let o = r
        .execute(
            &mut s,
            "view.pages_per_layer",
            json!({ "reference": refv, "layers": ["B.SilkS"] }),
        )
        .unwrap();
    assert!(
        o.messages[0].contains("view(s) of the set keep their own colours"),
        "{:?}",
        o.messages
    );
    // Hand-made follower colours stay.
    assert_ne!(
        colour(&s, &fs[0], "Edge.Cuts"),
        colour(&s, &refv, "Edge.Cuts")
    );
    // Match: a follower takes its reference's colours...
    r.execute(&mut s, "link.match_colours", json!({ "ids": [fs[0]] }))
        .unwrap();
    assert_eq!(
        colour(&s, &fs[0], "Edge.Cuts"),
        colour(&s, &refv, "Edge.Cuts")
    );
    // ... and a reference gives its colours to all its followers.
    r.execute(&mut s, "link.match_colours", json!({ "ids": [refv] }))
        .unwrap();
    let rep = crate::check::check(&mut s);
    assert!(rep.findings.iter().all(|f| f.code != "link-colours"));
}

/// The user's file: the reference carries KiCad Default's raw colours as
/// overrides, the followers follow the adapted document palette.
#[test]
fn users_case_raw_reference_colours() {
    let r = Registry::new();
    let mut imported = Palette::builtin(KICAD_DEFAULT_PAPER).unwrap();
    imported.name = "KiCad Default (imported)".into();
    let mut s = session(Some(imported));
    let (refv, fs) = layer_set(&r, &mut s);
    let text = s.doc().to_text().unwrap().replace(" colours)", ")");
    s.set_doc(Document::parse(&text).unwrap());
    let raw = Palette::builtin(KICAD_DEFAULT_NAME).unwrap();
    for (l, c) in [("Edge.Cuts", RAW_EDGE), ("F.Cu", "#C83434")] {
        r.execute(
            &mut s,
            "view.set_layer_style",
            json!({ "ids": [refv], "layer": l, "color": c }),
        )
        .unwrap();
        assert_eq!(Some(colour(&s, &refv, l).0), raw.layer(l).map(|x| x.0));
    }
    assert_ne!(
        colour(&s, &refv, "Edge.Cuts"),
        colour(&s, &fs[0], "Edge.Cuts")
    );
    // The repair offered: reset the reference to the palette.
    r.execute(
        &mut s,
        "view.set_layer_style",
        json!({ "ids": [refv], "reset": true }),
    )
    .unwrap();
    for f in &fs {
        assert_eq!(colour(&s, &refv, "Edge.Cuts"), colour(&s, f, "Edge.Cuts"));
    }
}
