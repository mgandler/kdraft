//! Headless tests of view layer colours: preset chips, a layer swatch,
//! drag reordering of the layer list.

use std::path::PathBuf;

use eframe::egui;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use kdraft_doc::layers::PaletteSource;
use kdraft_doc::Item;
use kdraft_engine::Session;
use kdraft_ui::App;
use serde_json::json;

fn harness() -> Harness<'static, App> {
    let ctx = egui::Context::default();
    let s = Session::new_for_board(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/kicad9-minimal.kicad_pcb"),
        None,
    )
    .unwrap();
    Harness::builder()
        .with_size(egui::Vec2::new(1600.0, 1400.0))
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(kdraft_ui::app::install_fonts)
        .build_ui_state(|ui, app: &mut App| app.show(ui), App::new(&ctx, s, false))
}

fn view(app: &App) -> kdraft_doc::View {
    match app.selected_items()[0] {
        Item::View(v) => v.clone(),
        o => panic!("{o:?}"),
    }
}

#[test]
fn preset_chip_swatch_and_reorder() {
    let mut h = harness();
    h.run_steps(3);
    let v = h.state().s.doc().pages[0]
        .views()
        .next()
        .unwrap()
        .id
        .0
        .clone();
    h.state_mut().cmd(
        "item.set_layers",
        json!({ "ids": [v], "layers": ["B.Cu", "F.Cu", "Edge.Cuts"] }),
    );
    h.state_mut().cmd("select.set", json!({ "ids": [v] }));
    h.run_steps(3);
    // The view's palette chooser (a combo box: open it, pick the entry).
    h.get_by(|n| {
        n.role() == egui::accesskit::Role::ComboBox
            && n.value().as_deref() == Some("Document (Black)")
    })
    .click();
    h.run_steps(3);
    h.get_by_label("kdraft layer colours").click();
    h.run_steps(3);
    let st = h.state().s.doc().styles.clone();
    assert_eq!(
        view(h.state()).palette_source(&st),
        PaletteSource::Own("kdraft layer colours".into())
    );
    // Swatch: pick a palette colour for F.Cu.
    h.get_by_label("F.Cu colour").click();
    h.run_steps(3);
    h.get_by_label("#2E8B57").click();
    h.run_steps(3);
    let (c, _) = view(h.state()).layer_color(&st, "F.Cu");
    assert_eq!(c.to_string(), "#2E8B57");
    // Drag Edge.Cuts (top row) onto the B.Cu row: just above B.Cu.
    let from = h.get_by_label("Edge.Cuts").rect().center();
    let to = h.get_by_label("B.Cu").rect().center();
    h.hover_at(from);
    h.step();
    h.drag_at(from);
    h.step();
    for k in 1..=8 {
        h.hover_at(from + (to - from) * (k as f32 / 8.0));
        h.step();
    }
    h.drop_at(to);
    h.run_steps(3);
    assert_eq!(view(h.state()).layers, vec!["B.Cu", "Edge.Cuts", "F.Cu"]);
}

fn neutral(items: &[kdraft_render::dl::Item]) -> bool {
    use kdraft_render::dl::Item as Dl;
    items.iter().all(|it| match it {
        Dl::Path(p) => {
            let ok = |c: kdraft_render::dl::Color| c.a == 0 || (c.r == c.g && c.g == c.b);
            p.stroke.as_ref().is_none_or(|s| ok(s.color))
                && p.fill.as_ref().is_none_or(|f| ok(f.color))
        }
        Dl::Text(t) => t.color.r == t.color.g && t.color.g == t.color.b,
        Dl::Group(g) => neutral(&g.items),
        Dl::Image(_) => true,
    })
}

fn combo(h: &mut Harness<'static, App>, value: &str) {
    h.get_by(|n| {
        n.role() == egui::accesskit::Role::ComboBox && n.value().as_deref() == Some(value)
    })
    .click();
    h.run_steps(3);
}

/// The user's report: a view switched to Black kept red pads and yellow
/// designators from the document palette. Through the inspector.
#[test]
fn black_view_and_black_document_draw_only_black() {
    let mut h = harness();
    h.run_steps(3);
    let asm = h
        .state()
        .s
        .doc()
        .pages
        .iter()
        .flat_map(|p| p.views())
        .find(|v| v.kind == kdraft_doc::ViewKind::Assembly)
        .unwrap()
        .id
        .0
        .clone();
    let pid = h
        .state()
        .s
        .doc()
        .pages
        .iter()
        .find(|p| p.item(&kdraft_doc::Id(asm.clone())).is_some())
        .unwrap()
        .id
        .0
        .clone();
    h.state_mut().cmd("page.select", json!({ "page": pid }));
    h.run_steps(3);
    let drawn = |h: &Harness<'static, App>| {
        let item = h
            .state()
            .s
            .find_item(&kdraft_doc::Id(asm.clone()))
            .unwrap()
            .1
            .clone();
        h.state().inputs.as_ref().unwrap().render_item(&item).0
    };
    // Document palette KiCad Default (page inspector, nothing selected).
    combo(&mut h, "Black");
    h.get_by_label("KiCad Default (screen colours)").click();
    h.run_steps(4);
    assert!(!neutral(&drawn(&h)), "coloured pads and designators");
    // The view to Black (its palette chooser).
    h.state_mut().cmd("select.set", json!({ "ids": [asm] }));
    h.run_steps(3);
    combo(&mut h, "Document (KiCad Default)");
    h.get_by_label("Black").click();
    h.run_steps(4);
    assert!(neutral(&drawn(&h)), "view Black: only black");
    // Back to following the document, then the document to Black.
    combo(&mut h, "Black");
    h.get_by_label("Document (KiCad Default)").click();
    h.run_steps(4);
    assert!(!neutral(&drawn(&h)));
    h.state_mut().cmd("select.set", json!({ "ids": [] }));
    h.run_steps(3);
    combo(&mut h, "KiCad Default (screen colours)");
    h.get_by_label("Black").click();
    h.run_steps(4);
    assert!(neutral(&drawn(&h)), "document Black: only black");
}

/// The user's layer pages: the reference with KiCad's raw Edge.Cuts
/// colour, the other pages on the (paper) document palette. The note shows
/// the difference; Match makes every page draw Edge.Cuts in one colour.
#[test]
fn layer_pages_with_a_raw_reference_colour_are_matched() {
    let mut h = harness();
    h.run_steps(3);
    h.state_mut().cmd(
        "document.set_layer_palette",
        json!({ "name": "KiCad Default (paper)" }),
    );
    let p0 = h.state().s.doc().pages[0].id.0.clone();
    h.state_mut()
        .cmd("select.set", json!({ "page": p0, "ids": [] }));
    h.state_mut().cmd(
        "view.pages_per_layer",
        json!({ "reference": "new", "layers": ["F.Cu", "B.Cu"] }),
    );
    let views: Vec<(String, String)> = h
        .state()
        .s
        .doc()
        .pages
        .iter()
        .filter(|p| matches!(p.title.as_deref(), Some("F.Cu" | "B.Cu")))
        .map(|p| (p.id.0.clone(), p.views().next().unwrap().id.0.clone()))
        .collect();
    let (rp, rv) = views[0].clone();
    let (fp, fv) = views[1].clone();
    // As in the user's file: links from before colours were followed,
    // the reference with KiCad Default's screen colours.
    h.state_mut().cmd(
        "link.set_follow",
        json!({ "id": fv, "follow": ["position", "rotation", "scale"] }),
    );
    h.state_mut().cmd(
        "view.set_layer_style",
        json!({ "ids": [rv], "layer": "Edge.Cuts", "color": "#D0D2CD" }),
    );
    let edge = |h: &Harness<'static, App>, id: &str| {
        let st = h.state().s.doc().styles.clone();
        match h.state().s.find_item(&kdraft_doc::Id(id.into())).unwrap().1 {
            Item::View(v) => v.layer_color(&st, "Edge.Cuts"),
            _ => unreachable!(),
        }
    };
    assert_ne!(edge(&h, &rv), edge(&h, &fv));
    // The follower's page: the note and Match reference.
    h.state_mut()
        .cmd("select.set", json!({ "page": fp, "ids": [fv] }));
    h.run_steps(4);
    h.get_by_label("Colours differ from the reference view: Edge.Cuts");
    h.get_by_label("Match reference").click();
    h.run_steps(3);
    assert_eq!(edge(&h, &rv), edge(&h, &fv));
    // The reference: its own colours are KiCad's screen colours; reset
    // them, then give the linked views the reference's colours.
    h.state_mut()
        .cmd("select.set", json!({ "page": rp, "ids": [rv] }));
    h.run_steps(4);
    h.get_by_label("Its own colours are KiCad Default screen colours (hard to see on paper)");
    h.get_by_label("Reset to palette").click();
    h.run_steps(3);
    h.get_by_label("Linked views draw Edge.Cuts in other colours");
    h.get_by_label("Match linked views to this one").click();
    h.run_steps(3);
    assert_eq!(edge(&h, &rv), edge(&h, &fv));
    let paper = kdraft_doc::palette::Palette::builtin("KiCad Default (paper)").unwrap();
    assert_eq!(edge(&h, &rv), paper.layer("Edge.Cuts").unwrap());
}
