//! Layer palettes: views follow the document palette on every insertion
//! path and when layers are added; changing the palette recolours views
//! without overrides only; palettes round trip in the document.

use std::path::PathBuf;

use kdraft_doc::palette::{Palette, KICAD_DEFAULT_NAME, LAYER};
use kdraft_doc::style::Rgba;
use kdraft_doc::{Document, Id, Item, View};
use serde_json::json;

use crate::{Registry, Session};

fn board() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/kicad9-minimal.kicad_pcb")
}

fn shop() -> Palette {
    let mut p = Palette::new("Shop");
    p.colors
        .insert("copper.front".into(), Rgba::rgb(0xAA, 0x10, 0x10));
    p.colors
        .insert("copper.back".into(), Rgba::rgb(0x10, 0x10, 0xAA));
    p.colors
        .insert("Edge.Cuts".into(), Rgba::rgb(0x40, 0x40, 0x00));
    p.colors.insert(
        "F.Fab".into(),
        Rgba {
            a: 128,
            ..Rgba::rgb(0x30, 0x60, 0x30)
        },
    );
    p
}

fn session_with(p: Option<Palette>) -> Session {
    let mut s = Session::new_for_board_with(&board(), None, None, &Default::default(), p).unwrap();
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

/// Views of every insertion path take the document palette.
#[test]
fn every_insertion_path_uses_the_document_palette() {
    let r = Registry::new();
    let mut s = session_with(Some(shop()));
    let st = |s: &Session| s.doc().styles.clone();
    let want = |layer: &str| shop().layer(layer).unwrap();
    let mut paths: Vec<(&str, String)> = Vec::new();
    // doc new: the assembly views it creates.
    let asm = s
        .doc()
        .pages
        .iter()
        .flat_map(|p| p.views())
        .find(|v| v.kind == kdraft_doc::ViewKind::Assembly)
        .unwrap()
        .id
        .0
        .clone();
    paths.push(("doc new", asm));
    // item.add_view (the tools and the Insert menu run it).
    let o = r
        .execute(
            &mut s,
            "item.add_view",
            json!({ "kind": "layer", "layers": ["F.Cu", "Edge.Cuts"], "at": [100, 100] }),
        )
        .unwrap();
    let added = o.created[0].clone();
    paths.push(("item.add_view", added.clone()));
    // Layer pages.
    let o = r
        .execute(
            &mut s,
            "view.pages_per_layer",
            json!({ "reference": "new", "layers": ["B.Cu"] }),
        )
        .unwrap();
    paths.push(("layer pages", o.created[1].clone()));
    // Paste (copy to another page).
    let p0 = s.doc().pages[0].id.0.clone();
    let o = r.execute(
        &mut s,
        "item.copy_to_page",
        json!({ "ids": [added], "page": p0 }),
    );
    if let Ok(o) = o {
        if let Some(id) = o.created.first() {
            paths.push(("paste", id.clone()));
        }
    }
    assert_eq!(paths.len(), 4, "{paths:?}");
    for (path, id) in &paths {
        let v = view(&s, id);
        for layer in &v.layers {
            let Some(w) = shop().layer(layer) else {
                continue;
            };
            assert_eq!(v.layer_color(&st(&s), layer), w, "{path}: {layer}");
        }
        assert!(
            v.layer_colors().is_empty(),
            "{path}: nothing stored on the view"
        );
    }
    // Adding a layer to a view: it gets its palette colour, not black.
    let id = paths[1].1.clone();
    r.execute(
        &mut s,
        "item.set_layers",
        json!({ "ids": [id], "layers": ["F.Cu", "Edge.Cuts", "F.Fab"] }),
    )
    .unwrap();
    let v = view(&s, &id);
    assert_eq!(v.layer_color(&st(&s), "F.Fab"), want("F.Fab"));
    assert!(!v.plain_black(&st(&s)));
    // Fabrication views stay black line drawings.
    let fab = s.doc().pages[0].views().next().unwrap().clone();
    assert!(fab.plain_black(&st(&s)));
}

/// Changing the document palette recolours views without overrides only;
/// "Reset to document palette" works per view.
#[test]
fn palette_change_recolours_non_overridden_layers() {
    let r = Registry::new();
    let mut s = session_with(Some(shop()));
    let o = r
        .execute(
            &mut s,
            "item.add_view",
            json!({ "kind": "layer", "layers": ["F.Cu", "B.Cu"], "at": [100, 100] }),
        )
        .unwrap();
    let id = o.created[0].clone();
    r.execute(
        &mut s,
        "view.set_layer_style",
        json!({ "ids": [id], "layer": "B.Cu", "color": "#123456" }),
    )
    .unwrap();
    r.execute(
        &mut s,
        "document.set_layer_palette",
        json!({ "name": KICAD_DEFAULT_NAME }),
    )
    .unwrap();
    let st = s.doc().styles.clone();
    let v = view(&s, &id);
    assert_eq!(v.layer_color(&st, "F.Cu").0, Rgba::rgb(200, 52, 52));
    assert_eq!(v.layer_color(&st, "B.Cu").0, Rgba::rgb(0x12, 0x34, 0x56));
    // A specific palette for this view, then back to the document's.
    r.execute(
        &mut s,
        "view.apply_palette",
        json!({ "ids": [id], "name": LAYER }),
    )
    .unwrap();
    let v = view(&s, &id);
    let layer = Palette::builtin(LAYER).unwrap();
    assert_eq!(v.layer_color(&st, "B.Cu"), layer.layer("B.Cu").unwrap());
    r.execute(
        &mut s,
        "view.apply_palette",
        json!({ "ids": [id], "name": "document" }),
    )
    .unwrap();
    let v = view(&s, &id);
    assert!(v.layer_colors().is_empty());
    assert_eq!(v.layer_color(&st, "B.Cu").0, Rgba::rgb(77, 127, 196));
    // Black clears the document palette.
    r.execute(
        &mut s,
        "document.set_layer_palette",
        json!({ "name": "Black" }),
    )
    .unwrap();
    assert!(s.doc().styles.layer_palette.is_none());
    // A user palette passed whole.
    let pj = kdraft_source::settings::palette_to_json(&shop());
    r.execute(
        &mut s,
        "document.set_layer_palette",
        json!({ "palette": pj }),
    )
    .unwrap();
    assert_eq!(s.doc().styles.layer_palette.as_ref(), Some(&shop()));
}

#[test]
fn palette_round_trips_in_the_document_and_old_files_render_as_before() {
    let s = session_with(Some(shop()));
    let text = s.doc().to_text().unwrap();
    assert!(text.contains("(layer_palette"), "{text}");
    assert_eq!(Document::parse(&text).unwrap(), *s.doc());
    // Without a palette nothing is written and views keep the old rules.
    let s = session_with(None);
    let text = s.doc().to_text().unwrap();
    assert!(!text.contains("layer_palette"));
    for v in s.doc().pages.iter().flat_map(|p| p.views()) {
        assert!(v.plain_black(&s.doc().styles), "{:?}", v.kind);
    }
}
