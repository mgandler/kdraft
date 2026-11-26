//! "Black means black": every colour a view draws (plotted layers, native
//! pads, pad fills, holes, designators, leaders, values, DNP marks, drill
//! symbols) follows one resolution order, so switching a view or the
//! document to Black leaves nothing coloured, whichever way it is done.

use std::path::PathBuf;
use std::sync::Arc;

use kdraft_doc::palette::{Palette, BLACK, BUILTIN, KICAD_DEFAULT_NAME};
use kdraft_doc::{Item, ViewKind};
use kdraft_render::dl::{Color, Item as Dl};
use kdraft_source::{Cache, FileBoardSource, KicadCli, Version};
use serde_json::json;

use crate::{Registry, Session};

fn board() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/kicad9-minimal.kicad_pcb")
}

/// A document with a native assembly view, a native drill view and a
/// plotted four-layer view on its pages, with document palette `p`.
fn session(p: Option<Palette>) -> (Session, [String; 3]) {
    let r = Registry::new();
    let mut s = Session::new_for_board_with(&board(), None, None, &Default::default(), p).unwrap();
    let find = |s: &Session, k: ViewKind| {
        s.doc()
            .pages
            .iter()
            .flat_map(|p| p.views())
            .find(|v| v.kind == k)
            .unwrap()
            .id
            .0
            .clone()
    };
    let asm = find(&s, ViewKind::Assembly);
    let drill = find(&s, ViewKind::Drill);
    let p0 = s.doc().pages[0].id.0.clone();
    let o = r
        .execute(
            &mut s,
            "item.add_view",
            json!({ "page": p0, "kind": "layer", "layers": ["B.Cu", "F.Cu", "F.CrtYd", "Edge.Cuts"], "at": [300, 200], "scale": 1 }),
        )
        .unwrap();
    (s, [asm, drill, o.created[0].clone()])
}

/// The rendered document (plots from the recorded kicad-cli outputs).
fn render(s: &mut Session, dir: &std::path::Path) -> Vec<Vec<Dl>> {
    let runner = Arc::new(kdraft_source::fixture::fixture_runner(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/kicad-cli-svg"),
        Box::new(|_, _, _| Vec::new()),
    ));
    let cli = KicadCli {
        path: "/fake/kicad-cli".into(),
        version: Version::new(10, 0, 8),
        version_string: "10.0.8".into(),
        vars: Vec::new(),
    };
    let src = FileBoardSource::new(board(), cli, runner, Cache::new(dir.join("c")));
    let mut notes = Vec::new();
    let plots = crate::pipeline::fetch_plots(
        &crate::pipeline::needed_layers(s.doc()),
        &src,
        &s.board_arc().unwrap(),
        &mut notes,
    )
    .unwrap_or_else(|| panic!("{notes:?}"));
    let (mut inputs, _) = crate::pipeline::RenderInputs::from_session(s, None).unwrap();
    inputs.plots = Some(Arc::new(plots));
    (0..s.doc().pages.len())
        .map(|i| inputs.render_page(i).unwrap().0.items)
        .collect()
}

/// The display list of one view (rendered alone).
fn render_view(s: &mut Session, dir: &std::path::Path, id: &str) -> Vec<Dl> {
    let pi = s
        .doc()
        .pages
        .iter()
        .position(|p| p.item(&kdraft_doc::Id(id.into())).is_some())
        .unwrap();
    // Keep only that view on its page.
    let mut d = s.doc().clone();
    d.pages[pi]
        .items
        .retain(|i| i.id().map(|x| x.0.as_str()) == Some(id));
    let mut one = s.fork();
    one.set_doc(d);
    render(&mut one, dir).swap_remove(pi)
}

fn colours(items: &[Dl], out: &mut Vec<Color>) {
    for it in items {
        match it {
            Dl::Path(p) => {
                if let Some(s) = &p.stroke {
                    out.push(s.color);
                }
                if let Some(f) = &p.fill {
                    out.push(f.color);
                }
            }
            Dl::Text(t) => out.push(t.color),
            Dl::Group(g) => colours(&g.items, out),
            Dl::Image(_) => {}
        }
    }
}

/// Every drawn colour is neutral (black, white or grey), or invisible.
fn neutral(items: &[Dl]) -> Result<(), String> {
    let mut cs = Vec::new();
    colours(items, &mut cs);
    match cs.iter().find(|c| c.a > 0 && !(c.r == c.g && c.g == c.b)) {
        Some(c) => Err(format!("coloured primitive {c:?}")),
        None => Ok(()),
    }
}

fn coloured(items: &[Dl]) -> bool {
    neutral(items).is_err()
}

#[test]
fn black_through_every_path_on_every_kind_of_view() {
    let dir = tempfile::tempdir().unwrap();
    let r = Registry::new();
    for p in BUILTIN.iter().filter(|n| **n != BLACK) {
        // 1. The view's own palette, then Black on the view.
        let (mut s, views) = session(None);
        for v in &views {
            r.execute(
                &mut s,
                "view.apply_palette",
                json!({ "ids": [v], "name": p }),
            )
            .unwrap();
            let before = render_view(&mut s, dir.path(), v);
            if *p != kdraft_doc::palette::GREY {
                assert!(coloured(&before), "{p}: view {v} should be coloured");
            }
            r.execute(
                &mut s,
                "view.apply_palette",
                json!({ "ids": [v], "name": BLACK }),
            )
            .unwrap();
            let after = render_view(&mut s, dir.path(), v);
            neutral(&after).unwrap_or_else(|e| panic!("view palette {p} -> Black, {v}: {e}"));
            // The older preset command, too.
            r.execute(
                &mut s,
                "view.set_layer_style",
                json!({ "ids": [v], "preset": "layer" }),
            )
            .unwrap();
            r.execute(
                &mut s,
                "view.set_layer_style",
                json!({ "ids": [v], "preset": "black" }),
            )
            .unwrap();
            neutral(&render_view(&mut s, dir.path(), v))
                .unwrap_or_else(|e| panic!("preset black, {v}: {e}"));
        }
        // 2. The document palette, then Black on the document; views follow.
        let (mut s, views) = session(Palette::builtin(p));
        r.execute(
            &mut s,
            "document.set_layer_palette",
            json!({ "name": BLACK }),
        )
        .unwrap();
        for v in &views {
            neutral(&render_view(&mut s, dir.path(), v))
                .unwrap_or_else(|e| panic!("document {p} -> Black, {v}: {e}"));
        }
        for page in render(&mut s, dir.path()) {
            neutral(&page).unwrap_or_else(|e| panic!("document {p} -> Black, page: {e}"));
        }
        // 3. A document palette and a view switched to Black.
        let (mut s, views) = session(Palette::builtin(p));
        let asm = &views[0];
        if *p != kdraft_doc::palette::GREY {
            assert!(
                coloured(&render_view(&mut s, dir.path(), asm)),
                "{p} document palette"
            );
        }
        r.execute(
            &mut s,
            "view.apply_palette",
            json!({ "ids": [asm], "name": BLACK }),
        )
        .unwrap();
        neutral(&render_view(&mut s, dir.path(), asm))
            .unwrap_or_else(|e| panic!("document {p}, view Black: {e}"));
        // 4. The settings default for a new document: Black stores nothing.
        let (mut s, _) = session(Palette::builtin(BLACK));
        assert!(s.doc().styles.layer_palette.is_none());
        for page in render(&mut s, dir.path()) {
            neutral(&page).unwrap();
        }
    }
}

/// Palette A then palette B draws exactly what B alone draws, for every
/// pair of built-ins and every kind of view; a layer added later takes
/// the view palette's colour.
#[test]
fn last_palette_wins_and_added_layers_follow() {
    let dir = tempfile::tempdir().unwrap();
    let r = Registry::new();
    let (base, views) = session(None);
    for v in &views {
        let mut alone = Vec::new();
        for b in BUILTIN {
            let mut s = base.fork();
            r.execute(
                &mut s,
                "view.apply_palette",
                json!({ "ids": [v], "name": b }),
            )
            .unwrap();
            alone.push(render_view(&mut s, dir.path(), v));
        }
        for a in BUILTIN {
            for (k, b) in BUILTIN.iter().enumerate() {
                let mut s = base.fork();
                for n in [a, *b] {
                    r.execute(
                        &mut s,
                        "view.apply_palette",
                        json!({ "ids": [v], "name": n }),
                    )
                    .unwrap();
                }
                assert_eq!(
                    render_view(&mut s, dir.path(), v),
                    alone[k],
                    "{a} then {b} on {v}"
                );
            }
        }
    }
    // A layer added after choosing a view palette.
    let mut s = base.fork();
    let v = &views[2];
    r.execute(
        &mut s,
        "view.apply_palette",
        json!({ "ids": [v], "name": KICAD_DEFAULT_NAME }),
    )
    .unwrap();
    r.execute(
        &mut s,
        "item.set_layers",
        json!({ "ids": [v], "layers": ["B.Cu", "F.Cu", "F.CrtYd", "Edge.Cuts", "F.SilkS"] }),
    )
    .unwrap();
    let Some((_, Item::View(view))) = s.find_item(&kdraft_doc::Id(v.clone())) else {
        panic!()
    };
    let k = Palette::builtin(KICAD_DEFAULT_NAME).unwrap();
    assert_eq!(
        view.layer_color(&s.doc().styles, "F.SilkS"),
        k.layer("F.SilkS").unwrap()
    );
}

/// Older files: `(layer_preset layer)` and a view's `(preset grey)` load
/// as palettes, render as before and go black with Black.
#[test]
fn old_presets_load_render_and_go_black() {
    let dir = tempfile::tempdir().unwrap();
    let r = Registry::new();
    let (mut s, views) = session(None);
    let asm = views[0].clone();
    let text = s.doc().to_text().unwrap();
    // Old form: document preset, and a grey preset on the drill view.
    let old = text.replacen("(styles", "(styles (layer_preset layer)", 1);
    let mut d = kdraft_doc::Document::parse(&old).unwrap();
    assert_eq!(
        d.styles.layer_palette.as_ref().map(|p| p.name.as_str()),
        Some(kdraft_doc::palette::LAYER)
    );
    assert!(
        !d.to_text().unwrap().contains("layer_preset"),
        "written in the new form"
    );
    for p in &mut d.pages {
        for it in &mut p.items {
            if let Item::View(v) = it {
                if v.kind == ViewKind::Drill {
                    v.extra.push(kdraft_sexpr::Value::node(
                        kdraft_doc::layers::LAYER_COLORS,
                        [kdraft_sexpr::Value::node(
                            "preset",
                            [kdraft_sexpr::Value::sym("grey")],
                        )],
                    ));
                }
            }
        }
    }
    s.set_doc(d);
    // As before: the assembly view in the layer palette, the drill view grey.
    let st = s.doc().styles.clone();
    let Some((_, Item::View(av))) = s.find_item(&kdraft_doc::Id(asm.clone())) else {
        panic!()
    };
    assert_eq!(
        av.layer_color(&st, "F.Fab").0,
        kdraft_doc::layers::palette_color("F.Fab")
    );
    assert!(coloured(&render_view(&mut s, dir.path(), &asm)));
    neutral(&render_view(&mut s, dir.path(), &views[1])).unwrap();
    r.execute(
        &mut s,
        "document.set_layer_palette",
        json!({ "name": BLACK }),
    )
    .unwrap();
    for page in render(&mut s, dir.path()) {
        neutral(&page).unwrap();
    }
}
