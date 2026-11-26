//! Geometry editing commands: each edit followed by its inverse restores
//! the document, and undo restores it too.

use std::path::PathBuf;

use kdraft_doc::{GraphicShape, Id, Item};
use proptest::prelude::*;
use serde_json::json;

use crate::{Registry, Session};

fn session() -> Session {
    Session::new_for_board(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/kicad9-minimal.kicad_pcb"),
        None,
    )
    .unwrap()
}

fn shape(s: &Session, id: &str) -> GraphicShape {
    match s.find_item(&Id(id.into())).map(|(_, i)| i.clone()) {
        Some(Item::Graphic(g)) => g.shape,
        o => panic!("{o:?}"),
    }
}

fn pts_json(v: &[kdraft_board::geom::Point]) -> serde_json::Value {
    json!(v.iter().map(|p| json!([p.x, p.y])).collect::<Vec<_>>())
}

#[test]
fn commands_and_their_errors() {
    let r = Registry::new();
    let mut s = session();
    let o = r
        .execute(
            &mut s,
            "item.add_polyline",
            json!({ "points": [[0, 0], [10, 0], [10, 10]] }),
        )
        .unwrap();
    let id = o.created[0].clone();
    assert!(r
        .execute(&mut s, "item.add_polyline", json!({ "points": [[0, 0]] }))
        .is_err());
    r.execute(
        &mut s,
        "graphic.insert_point",
        json!({ "id": id, "index": 1, "at": [5, 0] }),
    )
    .unwrap();
    assert_eq!(shape(&s, &id).points().len(), 4);
    r.execute(
        &mut s,
        "graphic.remove_point",
        json!({ "id": id, "index": 1 }),
    )
    .unwrap();
    r.execute(
        &mut s,
        "graphic.remove_point",
        json!({ "id": id, "index": 1 }),
    )
    .unwrap();
    // Two points left: a line again.
    assert!(matches!(shape(&s, &id), GraphicShape::Line { .. }));
    assert!(r
        .execute(
            &mut s,
            "graphic.remove_point",
            json!({ "id": id, "index": 0 })
        )
        .is_err());
    let c = r
        .execute(
            &mut s,
            "item.add_circle",
            json!({ "center": [50, 50], "radius": 5 }),
        )
        .unwrap()
        .created[0]
        .clone();
    r.execute(
        &mut s,
        "graphic.set_points",
        json!({ "id": c, "points": [[50, 50], [50, 58]] }),
    )
    .unwrap();
    assert!(
        matches!(shape(&s, &c), GraphicShape::Circle { radius, .. } if (radius - 8.0).abs() < 1e-9)
    );
    assert!(r
        .execute(
            &mut s,
            "graphic.set_points",
            json!({ "id": c, "points": [[50, 50]] })
        )
        .is_err());
    r.execute(&mut s, "item.set_size", json!({ "ids": [c], "width": 4 }))
        .unwrap();
    assert!(
        matches!(shape(&s, &c), GraphicShape::Circle { radius, .. } if (radius - 2.0).abs() < 1e-9)
    );
    let rc = r
        .execute(
            &mut s,
            "item.add_rect",
            json!({ "start": [0, 0], "end": [10, 5] }),
        )
        .unwrap()
        .created[0]
        .clone();
    r.execute(
        &mut s,
        "item.set_geometry",
        json!({ "id": rc, "radius": 1.5 }),
    )
    .unwrap();
    r.execute(
        &mut s,
        "item.set_size",
        json!({ "ids": [rc], "width": 20, "height": 8 }),
    )
    .unwrap();
    match shape(&s, &rc) {
        GraphicShape::Rect { end, radius, .. } => {
            assert_eq!((end.x, end.y, radius), (20.0, 8.0, 1.5));
        }
        o => panic!("{o:?}"),
    }
    // Dimension text shift and re-anchoring.
    let dim = s.doc().pages[0]
        .items
        .iter()
        .find_map(|i| match i {
            Item::Dimension(d) if d.to.is_some() => Some(d.id.0.clone()),
            _ => None,
        })
        .unwrap();
    r.execute(
        &mut s,
        "dimension.set_text_shift",
        json!({ "ids": [dim], "shift": 12 }),
    )
    .unwrap();
    assert!(s.doc().to_text().unwrap().contains("(text_shift 12)"));
    r.execute(
        &mut s,
        "dimension.set_anchor",
        json!({ "id": dim, "end": "from", "anchor": "(hole 0)" }),
    )
    .unwrap();
    assert!(r
        .execute(
            &mut s,
            "dimension.set_anchor",
            json!({ "id": dim, "end": "from", "anchor": "(hole 999)" })
        )
        .is_err());
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

    #[test]
    fn edit_then_inverse_restores(
        pts in prop::collection::vec((0.0..200.0f64, 0.0..150.0f64), 3..7),
        k in 0usize..7,
        q in (0.0..200.0f64, 0.0..150.0f64),
        d in (-20.0..20.0f64, -20.0..20.0f64),
    ) {
        let r = Registry::new();
        let mut s = session();
        let start: Vec<kdraft_board::geom::Point> = pts.iter().map(|(x, y)| kdraft_board::geom::Point::new(*x, *y)).collect();
        let id = r.execute(&mut s, "item.add_polyline", json!({ "points": pts_json(&start) })).unwrap().created[0].clone();
        let base = s.doc().clone();
        // Move a vertex, then put it back.
        let i = k % start.len();
        let mut moved = start.clone();
        moved[i] = kdraft_board::geom::Point::new(q.0, q.1);
        r.execute(&mut s, "graphic.set_points", json!({ "id": id, "points": pts_json(&moved) })).unwrap();
        r.execute(&mut s, "graphic.set_points", json!({ "id": id, "points": pts_json(&start) })).unwrap();
        prop_assert_eq!(s.doc(), &base);
        // Insert, then remove the same vertex.
        r.execute(&mut s, "graphic.insert_point", json!({ "id": id, "index": i, "at": [q.0, q.1] })).unwrap();
        r.execute(&mut s, "graphic.remove_point", json!({ "id": id, "index": i })).unwrap();
        prop_assert_eq!(s.doc(), &base);
        // Move, then move back (within float rounding of the sum).
        r.execute(&mut s, "item.move", json!({ "ids": [id], "delta": [d.0, d.1] })).unwrap();
        r.execute(&mut s, "item.move", json!({ "ids": [id], "delta": [-d.0, -d.1] })).unwrap();
        for (a, b) in shape(&s, &id).points().iter().zip(&start) {
            prop_assert!(a.dist(*b) < 1e-9);
        }
        while s.can_undo() { s.undo(); }
        prop_assert_eq!(s.doc().pages[0].items.iter().filter(|i| i.id().map(|x| x.0.as_str()) == Some(id.as_str())).count(), 0);
    }
}

/// Layer colours on plotted views, from the real kicad-cli 10.0.8
/// recordings through the fixture runner: colours in the SVG output, the
/// drill knock-outs on top, the plots themselves unchanged (no re-run
/// when colours change).
#[test]
fn layer_colours_on_real_plots() {
    use kdraft_source::{Cache, FileBoardSource, KicadCli, Version};
    let r = Registry::new();
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("b.kicad_pcb");
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/kicad9-minimal.kicad_pcb"),
        &board,
    )
    .unwrap();
    let mut s = Session::new_for_board(&board, None).unwrap();
    let v = s.doc().pages[0].views().next().unwrap().id.0.clone();
    r.execute(
        &mut s,
        "item.set_layers",
        json!({ "ids": [v], "layers": ["B.Cu", "F.Cu", "F.CrtYd", "Edge.Cuts"] }),
    )
    .unwrap();
    let runner = std::sync::Arc::new(kdraft_source::fixture::fixture_runner(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/kicad-cli-svg"),
        Box::new(|_, _, _| Vec::new()),
    ));
    let cli = KicadCli {
        path: "/fake/kicad-cli".into(),
        version: Version::new(10, 0, 8),
        version_string: "10.0.8".into(),
        vars: Vec::new(),
    };
    let src = FileBoardSource::new(
        &board,
        cli,
        runner.clone(),
        Cache::new(dir.path().join("c")),
    );
    let render = |s: &mut Session| {
        let mut notes = Vec::new();
        let plots = crate::pipeline::fetch_plots(
            &crate::pipeline::needed_layers(s.doc()),
            &src,
            &s.board_arc().unwrap(),
            &mut notes,
        )
        .unwrap_or_else(|| panic!("{notes:?}"));
        let (mut inputs, _) = crate::pipeline::RenderInputs::from_session(s, None).unwrap();
        inputs.plots = Some(std::sync::Arc::new(plots));
        let (page, _) = inputs.render_page(0).unwrap();
        kdraft_render::svg_out::page_svg(&page, &inputs.fonts)
    };
    let black = render(&mut s);
    assert!(!black.contains("#C83434"));
    let calls = runner.calls().len();
    r.execute(
        &mut s,
        "view.set_layer_style",
        json!({ "ids": [v], "preset": "layer" }),
    )
    .unwrap();
    r.execute(
        &mut s,
        "view.set_layer_style",
        json!({ "ids": [v], "layer": "B.Cu", "color": "#00AA00", "opacity": 0.5 }),
    )
    .unwrap();
    let svg = render(&mut s);
    // Cached plots: no new kicad-cli runs for a colour change.
    assert_eq!(runner.calls().len(), calls);
    assert!(svg.contains("#C83434"), "F.Cu red");
    assert!(
        svg.contains("#00AA00") && svg.contains("opacity=\"0.50"),
        "B.Cu override"
    );
    assert!(svg.contains("#A08200"), "edge cuts");
    // Knock-outs (white) come after the last coloured copper.
    let last_red = svg.rfind("#C83434").unwrap();
    let last_white = svg.rfind("fill=\"#FFFFFF\"").unwrap();
    assert!(last_white > last_red);
    // Black plots keep the knock-outs too (in plot order).
    assert!(black.matches("#FFFFFF").count() > 1);
    // Round trip of the settings.
    let text = s.doc().to_text().unwrap();
    assert!(text.contains("(layer_colors"));
    assert_eq!(kdraft_doc::Document::parse(&text).unwrap(), *s.doc());
    r.execute(
        &mut s,
        "document.set_layer_preset",
        json!({ "preset": "grey" }),
    )
    .unwrap();
    // Written as the document palette (the older preset form is read only).
    let text = s.doc().to_text().unwrap();
    assert!(
        text.contains("(layer_palette") && text.contains("(name \"Greyscale\")"),
        "{text}"
    );
}

/// 3D views: two `pcb render` passes through the fixture runner (a
/// generated board picture with a transparent margin and an overhanging
/// part), trimming, true scale for top views, the picture in all back
/// ends, caching, bounds, the placeholder without kicad-cli.
#[test]
fn render3d_views_in_all_back_ends() {
    use kdraft_render::render3d::State;
    use kdraft_source::{Cache, FileBoardSource, KicadCli, Version};
    let r = Registry::new();
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("b.kicad_pcb");
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/kicad9-minimal.kicad_pcb"),
        &board,
    )
    .unwrap();
    let mut s = Session::new_for_board(&board, None).unwrap();
    let v = r
        .execute(
            &mut s,
            "item.add_view",
            json!({ "kind": "render3d", "at": [300, 80], "scale": 1 }),
        )
        .unwrap()
        .created[0]
        .clone();
    r.execute(
        &mut s,
        "view.set_render3d",
        json!({ "ids": [v], "dpi": 100 }),
    )
    .unwrap();
    assert!(r
        .execute(
            &mut s,
            "view.set_render3d",
            json!({ "ids": [v], "rotate": "1,2" })
        )
        .is_err());
    let runner = std::sync::Arc::new(kdraft_source::fixture::fixture_runner(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/kicad-cli-svg"),
        Box::new(kdraft_render::render3d::fake_board_render),
    ));
    let cli = KicadCli {
        path: "/fake/kicad-cli".into(),
        version: Version::new(10, 0, 8),
        version_string: "10.0.8".into(),
        vars: Vec::new(),
    };
    let src = FileBoardSource::new(
        &board,
        cli.clone(),
        runner.clone(),
        Cache::new(dir.path().join("c")),
    );
    let bb = s.board_box();
    let view = |s: &Session| match s.find_item(&kdraft_doc::Id(v.clone())).map(|x| x.1.clone()) {
        Some(Item::View(x)) => x,
        o => panic!("{o:?}"),
    };
    let mut notes = Vec::new();
    let renders = crate::pipeline::fetch_renders(s.doc(), bb, board.parent(), &src, &mut notes);
    assert_eq!(renders.len(), 1, "{notes:?}");
    let calls = runner.calls();
    assert_eq!(calls.len(), 2, "probe and final");
    let probe = calls[0].args.join(" ");
    let fin = calls[1].args.join(" ");
    assert!(
        probe.contains("--width 480") && !probe.contains("--zoom"),
        "{probe}"
    );
    for f in [
        "--background transparent",
        "--side top",
        "--define-var KIPRJMOD=",
        "--zoom 1.49",
    ] {
        assert!(fin.contains(f), "{f} in {fin}");
    }
    // Ran on the private copy, nothing written next to the board.
    assert!(PathBuf::from(calls[1].args.last().unwrap()).starts_with(dir.path().join("c")));
    let pic = match renders.values().next().unwrap() {
        State::Ready(p) => p.clone(),
        o => panic!("{o:?}"),
    };
    // Trimmed to the content plus padding; the content was about 92 % of
    // the final render, the transparent margin is gone.
    let (rgba, w, h) = kdraft_render::render3d::decode(&pic.png).unwrap();
    let c = kdraft_render::render3d::content_box(&rgba, w, h).unwrap();
    let pad = c[0];
    assert!((2..=6).contains(&pad), "about 1 mm at 100 dpi: {c:?}");
    assert_eq!(c[1], pad);
    assert_eq!(h - c[3], pad);
    // The resolution bucket: the 50 mm board at 1:1 and 100 dpi is about
    // 197 px, so the content's long side gets 256 px (the overhang adds).
    assert!((256..=290).contains(&(c[2] - c[0])), "{c:?}");
    // True scale: the outline box lands on the view frame (50 x 30 mm).
    let frame = kdraft_doc::ops::view_bounds(&view(&s), bb);
    let rect = kdraft_render::render3d::place(&pic, &frame);
    let o = pic.outline_px.unwrap();
    let k = rect.width() / f64::from(pic.width);
    assert!((rect.min.x + o[0] * k - frame.min.x).abs() < 1e-6);
    assert!(((o[2] - o[0]) * k - 50.0).abs() < 1e-6);
    assert!(((o[3] - o[1]) * (rect.height() / f64::from(pic.height)) - 30.0).abs() < 1e-6);
    // The outline box in the picture matches the generated board (55 %
    // of the render times the zoom) within a pixel.
    let rx = (o[2] - o[0]) / (o[3] - o[1]);
    assert!((rx - 50.0 / 30.0).abs() < 0.02, "{rx}");

    // Cached: the same request runs no new job; moving the view keeps the
    // key (no re-render).
    let n = runner.calls().len();
    let key = crate::pipeline::render_key(&view(&s), bb);
    r.execute(&mut s, "item.move", json!({ "ids": [v], "delta": [10, 5] }))
        .unwrap();
    assert_eq!(crate::pipeline::render_key(&view(&s), bb), key);
    crate::pipeline::fetch_renders(s.doc(), bb, board.parent(), &src, &mut notes);
    assert_eq!(runner.calls().len(), n);
    // A different look is a different key.
    let mut other = view(&s);
    let mut ro = other.render3d();
    ro.stackup_colors = true;
    other.set_render3d(&ro);
    assert_ne!(crate::pipeline::render_key(&other, bb), key);

    // Bounds, export and checks use the trimmed picture.
    s.renders = std::sync::Arc::new(renders);
    s.kicad_available = true;
    let b = s.item_bounds(&Item::View(view(&s))).unwrap();
    let frame = kdraft_doc::ops::view_bounds(&view(&s), bb);
    let placed = kdraft_render::render3d::place(&pic, &frame);
    assert_eq!(b, placed);
    assert!(b.width() < 50.0 * 1.2 && b.height() < 30.0 * 1.2, "{b:?}");
    let (inputs, _) = crate::pipeline::RenderInputs::from_session(&mut s, Some(&cli)).unwrap();
    let (page, _) = inputs.render_page(0).unwrap();
    let svg = kdraft_render::svg_out::page_svg(&page, &inputs.fonts);
    assert!(svg.contains("<image ") && svg.contains("data:image/png;base64,iVBOR"));
    assert!(svg.contains("3D VIEW (TOP)") && svg.contains("SCALE 1:1"));
    let pix = kdraft_render::raster::page_pixmap(&page, 2.0, &inputs.fonts).unwrap();
    let c = b.center();
    let px = pix.pixel((c.x * 2.0) as u32, (c.y * 2.0) as u32).unwrap();
    assert_eq!(
        (px.red(), px.green(), px.blue()),
        (28, 28, 32),
        "the part in the middle"
    );
    let (dl, _) = inputs.render();
    let pdf = kdraft_render::pdf::write_pdf(&dl, &inputs.fonts).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    let report = crate::check::check(&mut s);
    let mine: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.item.as_deref() == Some(v.as_str()))
        .collect();
    assert!(mine.iter().all(|f| f.code == "overlap"), "{mine:?}");

    // Isometric: not to scale; the picture keeps its aspect, the width on
    // paper follows the board box at the view's scale.
    r.execute(
        &mut s,
        "view.set_render3d",
        json!({ "ids": [v], "preset": "isometric" }),
    )
    .unwrap();
    let iso = crate::pipeline::fetch_renders(s.doc(), bb, board.parent(), &src, &mut notes);
    let State::Ready(ip) = iso.values().next().unwrap() else {
        panic!()
    };
    assert!(ip.outline_px.is_none());
    let f2 = kdraft_doc::ops::view_bounds(&view(&s), bb);
    let r2 = kdraft_render::render3d::place(ip, &f2);
    assert!((r2.width() - f2.width()).abs() < 1e-9);
    assert!(((r2.width() / r2.height()) - f64::from(ip.width) / f64::from(ip.height)).abs() < 1e-9);

    // Without kicad-cli: the placeholder.
    s.kicad_available = false;
    s.renders = std::sync::Arc::new(Default::default());
    let (inputs, _) = crate::pipeline::RenderInputs::from_session(&mut s, None).unwrap();
    let (page, w) = inputs.render_page(0).unwrap();
    let svg = kdraft_render::svg_out::page_svg(&page, &inputs.fonts);
    assert!(svg.contains(kdraft_render::page::NO_KICAD_3D));
    assert!(svg.contains("NOT TO SCALE"));
    assert!(w.iter().any(|x| x.contains("needs kicad-cli")));
    // With kicad-cli but no picture yet: pending, no warning.
    s.kicad_available = true;
    let (inputs, _) = crate::pipeline::RenderInputs::from_session(&mut s, None).unwrap();
    let (_, w) = inputs.render_page(0).unwrap();
    assert!(!w.iter().any(|x| x.contains("3D")), "{w:?}");
    // Round trip.
    let text = s.doc().to_text().unwrap();
    assert!(text.contains("(kind render3d)") && text.contains("(rotate -45 0 45)"));
    assert_eq!(kdraft_doc::Document::parse(&text).unwrap(), *s.doc());
}
