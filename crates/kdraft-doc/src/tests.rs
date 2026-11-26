use std::path::PathBuf;

use kdraft_board::geom::Point;
use kdraft_board::Board;

use crate::*;

const PLAN_EXAMPLE: &str = r#"(kdraft (version 1)
  (source (board "../hp-controller.kicad_pcb") (variant "default"))
  (page (id "p1") (sheet "A3-fab.kicad_wks") (title "Assembly top")
    (view (id "v1") (kind assembly) (side front) (scale 2)
      (at 148 105) (layers F.Fab F.Courtyard Edge.Cuts))
    (dimension (id "d1") (kind linear) (view "v1")
      (from (outline-vertex 0)) (to (outline-vertex 1)) (offset 8))))
"#;

#[test]
fn reads_plan_example() {
    let d = Document::parse(PLAN_EXAMPLE).unwrap();
    assert_eq!(d.version, 1);
    assert_eq!(d.source.board, "../hp-controller.kicad_pcb");
    assert_eq!(d.source.variant.as_deref(), Some("default"));
    assert_eq!(d.pages.len(), 1);
    let p = &d.pages[0];
    assert_eq!(p.sheet.as_deref(), Some("A3-fab.kicad_wks"));
    assert_eq!(p.title.as_deref(), Some("Assembly top"));
    let Item::View(v) = &p.items[0] else { panic!() };
    assert_eq!(v.kind, ViewKind::Assembly);
    assert_eq!(v.scale, 2.0);
    assert_eq!(v.at, Point::new(148.0, 105.0));
    assert_eq!(v.layers, ["F.Fab", "F.Courtyard", "Edge.Cuts"]);
    let Item::Dimension(dim) = &p.items[1] else {
        panic!()
    };
    assert_eq!(dim.view.as_ref().map(Id::as_str), Some("v1"));
    assert_eq!(dim.from, Some(Anchor::OutlineVertex(0)));
    assert_eq!(dim.to, Some(Anchor::OutlineVertex(1)));
    assert_eq!(dim.offset, 8.0);
    assert!(d.validate().is_empty());
}

#[test]
fn canonical_writer_is_a_fixed_point() {
    let d = Document::parse(PLAN_EXAMPLE).unwrap();
    let t1 = d.to_text().unwrap();
    let d2 = Document::parse(&t1).unwrap();
    assert_eq!(d2, d);
    assert_eq!(d2.to_text().unwrap(), t1);
}

#[test]
fn canonical_text_layout() {
    let d = Document::parse(PLAN_EXAMPLE).unwrap();
    let expected = r#"(kdraft
  (version 1)
  (source
    (board "../hp-controller.kicad_pcb")
    (variant "default")
  )
  (styles
    (text_height 2.5)
    (line_width 0.25)
    (snap_grid 0.5)
  )
  (page
    (id "p1")
    (title "Assembly top")
    (sheet "A3-fab.kicad_wks")
    (size A3 420 297)
    (view
      (id "v1")
      (kind assembly)
      (side front)
      (scale 2)
      (at 148 105)
      (layers F.Fab F.Courtyard Edge.Cuts)
    )
    (dimension
      (id "d1")
      (kind linear)
      (view "v1")
      (from
        (outline-vertex 0)
      )
      (to
        (outline-vertex 1)
      )
      (offset 8)
    )
  )
)
"#;
    assert_eq!(d.to_text().unwrap(), expected);
}

#[test]
fn unknown_nodes_survive_round_trip() {
    let text = r#"(kdraft (version 1)
  (source (board "b.kicad_pcb") (future_source_field 1))
  (styles (text_height 3) (font "Osifont"))
  (page (id "p1")
    (future_item (id "f1") (x 1))
    (view (id "v1") (kind fancy) (layers F.Cu) (future_view_field "a b") flag)
    (table (id "t1") (kind bom) (at 10 20) (columns "Item" "Qty") (split 30))
    (callout (id "c1") (at 1 2) (view "v1") (anchor (pad "U7" "3")) (target "N1") (leader (xy 3 4)))
    (notes (id "n1") (at 5 5) (note "N1" "All dimensions in mm") (numbering roman))
    (graphic (id "g1") (polygon (pts (xy 0 0) (xy 1 0) (xy 1 1))) (fill yes))
    (dimension (id "d1") (from (via-center 4)) (to (hole 3))))
  (future_top_level (a b)))
"#;
    let d = Document::parse(text).unwrap();
    assert_eq!(d.source.extra.len(), 1);
    assert_eq!(d.styles.text_height, 3.0);
    assert_eq!(d.styles.extra.len(), 1);
    assert_eq!(d.extra.len(), 1);
    let p = &d.pages[0];
    assert!(matches!(&p.items[0], Item::Unknown(v) if v.is("future_item")));
    let Item::View(v) = &p.items[1] else { panic!() };
    assert_eq!(v.kind, ViewKind::Other("fancy".into()));
    assert_eq!(v.extra.len(), 2);
    let Item::Table(t) = &p.items[2] else {
        panic!()
    };
    assert_eq!(t.kind, TableKind::Bom);
    assert_eq!(t.columns, ["Item", "Qty"]);
    let Item::Callout(c) = &p.items[3] else {
        panic!()
    };
    assert_eq!(
        c.anchor,
        Some(Anchor::Pad {
            reference: "U7".into(),
            number: "3".into()
        })
    );
    assert_eq!(c.leader, [Point::new(3.0, 4.0)]);
    let Item::NoteList(n) = &p.items[4] else {
        panic!()
    };
    assert_eq!(n.notes[0].text, "All dimensions in mm");
    let Item::Graphic(g) = &p.items[5] else {
        panic!()
    };
    assert!(matches!(&g.shape, GraphicShape::Polygon { points } if points.len() == 3));
    let Item::Dimension(dim) = &p.items[6] else {
        panic!()
    };
    assert!(matches!(&dim.from, Some(Anchor::Unknown(v)) if v.is("via-center")));
    assert_eq!(dim.to, Some(Anchor::Hole(3)));

    let out = d.to_text().unwrap();
    for needle in [
        "(future_source_field 1)",
        "(font \"Osifont\")",
        "(future_item",
        "(kind fancy)",
        "(future_view_field \"a b\")",
        "flag",
        "(split 30)",
        "(numbering roman)",
        "(fill yes)",
        "(via-center 4)",
        "(future_top_level",
    ] {
        assert!(out.contains(needle), "{needle} missing in\n{out}");
    }
    let again = Document::parse(&out).unwrap();
    assert_eq!(again, d);
    assert_eq!(again.to_text().unwrap(), out);
}

#[test]
fn all_graphic_shapes_round_trip() {
    let shapes = vec![
        GraphicShape::Line {
            start: Point::new(0.0, 0.0),
            end: Point::new(1.5, 2.0),
        },
        GraphicShape::Rect {
            start: Point::new(0.0, 0.0),
            end: Point::new(3.0, 2.0),
            radius: 0.0,
        },
        GraphicShape::Circle {
            center: Point::new(1.0, 1.0),
            radius: 0.5,
        },
        GraphicShape::Arc {
            start: Point::new(1.0, 0.0),
            mid: Point::new(0.0, 1.0),
            end: Point::new(-1.0, 0.0),
        },
        GraphicShape::Text {
            at: Point::new(4.0, 5.0),
            text: "Note \"1\"\nline 2".into(),
            height: Some(3.5),
            rotation: 0.0,
            wrap: None,
        },
        GraphicShape::Text {
            at: Point::new(4.0, 5.0),
            text: "x".into(),
            height: None,
            rotation: 0.0,
            wrap: None,
        },
        GraphicShape::Rect {
            start: Point::new(0.0, 0.0),
            end: Point::new(3.0, 2.0),
            radius: 0.75,
        },
        GraphicShape::Polyline {
            points: vec![
                Point::new(0.0, 0.0),
                Point::new(1.0, 2.0),
                Point::new(3.0, 1.0),
            ],
        },
        GraphicShape::Text {
            at: Point::new(4.0, 5.0),
            text: "Über \"µ\" 10° ±0.1 Ø3 \\ back\\slash\r\n\nend ${TITLE}".into(),
            height: Some(2.0),
            rotation: 90.0,
            wrap: Some(40.0),
        },
        GraphicShape::Image {
            at: Point::new(1.0, 2.0),
            path: "logo.png".into(),
            width: 10.0,
            height: 5.0,
        },
    ];
    let mut d = Document::new("b.kicad_pcb");
    let mut p = Page::new("g", PageSize::a4_landscape());
    for s in shapes {
        p.items.push(Item::Graphic(Graphic {
            id: Id::new_v4(),
            shape: s,
            style: Some("thin".into()),
            extra: vec![],
        }));
    }
    d.pages.push(p);
    let back = Document::parse(&d.to_text().unwrap()).unwrap();
    assert_eq!(back, d);
}

#[test]
fn version_and_structure_errors() {
    assert!(matches!(
        Document::parse("(kdraft (version 2) (source (board \"b\")))"),
        Err(DocError::UnsupportedVersion {
            found: 2,
            supported: 1
        })
    ));
    assert!(matches!(
        Document::parse("(kdraft (version 0) (source (board \"b\")))"),
        Err(DocError::UnsupportedVersion { found: 0, .. })
    ));
    assert!(matches!(
        Document::parse("(kdraft (source (board \"b\")))"),
        Err(DocError::Missing { .. })
    ));
    assert!(matches!(
        Document::parse("(kdraft (version 1))"),
        Err(DocError::Missing { .. })
    ));
    assert!(matches!(
        Document::parse("(kicad_pcb)"),
        Err(DocError::NotADocument { .. })
    ));
    let e =
        Document::parse("(kdraft (version 1) (source (board \"b\"))\n  (page (title \"no id\")))")
            .unwrap_err();
    assert!(matches!(e, DocError::Missing { line: 2, .. }), "{e}");
    let e = Document::parse("(kdraft (version 1) (source (board \"b\"))\n (page (id \"p\") (view (id \"v\") (scale x))))").unwrap_err();
    assert!(e.to_string().contains("2:"), "{e}");
    assert!(matches!(
        Document::parse("(kdraft (version 1)"),
        Err(DocError::Syntax(_))
    ));
}

#[test]
fn validate_finds_problems() {
    let text = r#"(kdraft (version 1) (source (board "b"))
  (page (id "p1")
    (view (id "v1") (scale 0) (layers))
    (view (id "v1") (scale 1) (layers F.Fab) (parent "nope"))
    (dimension (id "d1") (view "missing"))))"#;
    let d = Document::parse(text).unwrap();
    let msgs: Vec<String> = d.validate().iter().map(|p| p.to_string()).collect();
    assert_eq!(msgs.len(), 5, "{msgs:?}");
    assert!(msgs.iter().any(|m| m.contains("scale 0")));
    assert!(msgs.iter().any(|m| m.contains("no layers")));
    assert!(msgs.iter().any(|m| m.contains("duplicate id")));
    assert!(msgs
        .iter()
        .any(|m| m.contains("parent refers to missing view nope")));
    assert!(msgs.iter().any(|m| m.contains("missing view missing")));
}

#[test]
fn scale_choice() {
    // KiCad's default sheet on A3: frame 12..408 x 12..285, title block
    // from y 253, 5 mm clearance: 386 x 231.
    let sheet = kdraft_sheet::default_sheet().unwrap();
    let a = drawing_area(&sheet, &PageSize::a3_landscape());
    let (aw, ah) = (a.width(), a.height());
    assert!(
        (aw - 386.0).abs() < 1e-9 && (ah - 231.0).abs() < 1e-9,
        "{a:?}"
    );
    assert_eq!(a.center(), Point::new(210.0, 132.5));
    assert_eq!(choose_scale(30.0, 15.0, aw, ah), Some(10.0));
    assert_eq!(choose_scale(50.0, 30.0, aw, ah), Some(5.0));
    assert_eq!(choose_scale(80.0, 40.0, aw, ah), Some(4.0));
    assert_eq!(choose_scale(100.0, 80.0, aw, ah), Some(2.0));
    assert_eq!(choose_scale(160.0, 99.0, aw, ah), Some(2.0));
    assert_eq!(choose_scale(200.0, 120.0, aw, ah), Some(1.0));
    assert_eq!(choose_scale(312.0, 106.0, aw, ah), Some(1.0));
    assert_eq!(choose_scale(500.0, 200.0, aw, ah), Some(0.5));
    assert_eq!(choose_scale(800.0, 200.0, aw, ah), None);
}

#[test]
fn new_for_board_default_document() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/kicad9-minimal.kicad_pcb");
    let b = Board::load(path).unwrap();
    let sheet = kdraft_sheet::default_sheet().unwrap();
    let d = Document::new_for_board(&b, "kicad9-minimal.kicad_pcb", &sheet);
    // The fixture has a part (C1) on the back: an assembly bottom page.
    assert_eq!(d.pages.len(), 5);
    assert_eq!(d.pages[4].title.as_deref(), Some("Bill of materials"));
    assert_eq!(d.pages[0].title.as_deref(), Some("Fabrication"));
    assert_eq!(d.pages[1].title.as_deref(), Some("Assembly top"));
    assert_eq!(d.pages[2].title.as_deref(), Some("Assembly bottom"));
    assert_eq!(d.pages[3].title.as_deref(), Some("Drill and stackup"));
    assert!(d.sheet.is_none() && d.pages.iter().all(|p| p.sheet.is_none()));
    assert_eq!(d.styles.snap_grid, 0.5);
    let views: Vec<&View> = d.pages.iter().flat_map(|p| p.views()).collect();
    assert_eq!(views.len(), 4);
    // 50 x 30 board on A3: 5:1 fits (250 x 150), 10:1 does not.
    assert!(views[..3].iter().all(|v| v.scale == 5.0));
    assert_eq!(views[0].kind, ViewKind::Fabrication);
    assert_eq!(views[1].kind, ViewKind::Assembly);
    assert_eq!(views[2].kind, ViewKind::Assembly);
    assert_eq!(views[2].side, ViewSide::Back);
    assert_eq!(views[3].kind, ViewKind::Drill);
    let tables: Vec<&Table> = d.pages[3]
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Table(t) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(tables.len(), 2);
    assert_eq!(tables[0].kind, TableKind::Drill);
    assert_eq!(tables[1].kind, TableKind::Stackup);
    assert!(tables[1].at.y > tables[0].at.y || tables[1].at.x > tables[0].at.x);
    assert!(d.pages.iter().all(|p| p.id.is_uuid()));
    assert!(views.iter().all(|v| v.id.is_uuid()));
    assert!(d.validate().is_empty());
    let text = d.to_text().unwrap();
    assert!(text.contains("(snap_grid 0.5)"));
    assert_eq!(Document::parse(&text).unwrap(), d);
    // Views fit inside the drawing area; the drill view stays left of the
    // tables.
    let area = drawing_area(&sheet, &d.pages[0].size);
    for v in &views {
        let bb = ops::view_bounds(v, b.outline.bbox.unwrap());
        assert!(
            bb.min.x >= area.min.x - 1e-9 && bb.max.x <= area.max.x + 1e-9,
            "{bb:?}"
        );
        assert!(
            bb.min.y >= area.min.y - 1e-9 && bb.max.y <= area.max.y + 1e-9,
            "{bb:?}"
        );
    }
    let dv = ops::view_bounds(views[3], b.outline.bbox.unwrap());
    assert!(tables
        .iter()
        .all(|t| dv.max.x < t.at.x || dv.max.y < t.at.y));
}

#[test]
fn document_sheet_override_round_trips() {
    let mut d = Document::new("b.kicad_pcb");
    d.sheet = Some("sheets/company.kicad_wks".into());
    let mut p = Page::new("x", PageSize::a4_landscape());
    p.sheet = Some("default".into());
    d.pages.push(p);
    let text = d.to_text().unwrap();
    assert!(
        text.contains("(sheet \"sheets/company.kicad_wks\")"),
        "{text}"
    );
    assert_eq!(Document::parse(&text).unwrap(), d);
}

#[test]
fn drill_marks_round_trip_and_defaults() {
    let text = "(kdraft (version 1) (source (board \"b.kicad_pcb\"))\n  (page (id \"p\") (size A4 297 210)\n    (view (id \"v1\") (kind assembly) (side front) (scale 1) (at 0 0) (layers F.Fab))\n    (view (id \"v2\") (kind fabrication) (side front) (scale 1) (at 0 0) (layers Edge.Cuts) (drill_marks small))))\n";
    let d = Document::parse(text).unwrap();
    let v: Vec<_> = d.pages[0].views().collect();
    assert_eq!(v[0].drill_marks, None);
    assert_eq!(v[0].drill_marks(), DrillMarks::None);
    assert_eq!(v[1].drill_marks(), DrillMarks::Small);
    let out = d.to_text().unwrap();
    assert!(out.contains("(drill_marks small)"), "{out}");
    assert_eq!(Document::parse(&out).unwrap(), d);
}

#[test]
fn dimensions_styles_and_anchors_round_trip() {
    let a = parse_anchor("(outline-edge 3)").unwrap();
    assert_eq!(a, Anchor::OutlineEdge(3));
    assert_eq!(anchor_text(&a).unwrap(), "(outline-edge 3)");
    assert_eq!(
        parse_anchor("(pad \"U7\" \"3\")").unwrap(),
        Anchor::Pad {
            reference: "U7".into(),
            number: "3".into()
        }
    );
    assert!(parse_anchor("(nonsense 1)").is_err());
    let text = r#"(kdraft (version 1) (source (board "b.kicad_pcb") (board_hash "ab"))
  (styles (text_height 2.5) (line_width 0.25) (snap_grid 0.5) (dimension (decimals 3) (trim_zeros yes)))
  (page (id "p") (size A3 420 297)
    (dimension (id "d") (kind ordinate) (view "v") (from (outline-vertex 0)) (to (hole 1)) (offset -5)
      (tolerance "±0.1") (axis y) (angle 30) (last (xy 1 2) (xy 3 4)))))"#;
    let d = Document::parse(text).unwrap();
    assert_eq!(d.source.board_hash.as_deref(), Some("ab"));
    assert_eq!(d.styles.dimension.decimals, 3);
    assert!(d.styles.dimension.trim_zeros);
    let Item::Dimension(dim) = &d.pages[0].items[0] else {
        panic!()
    };
    assert_eq!(dim.kind, DimensionKind::Ordinate);
    assert_eq!(dim.axis.as_deref(), Some("y"));
    assert_eq!(dim.angle, Some(30.0));
    assert_eq!(dim.last, vec![Point::new(1.0, 2.0), Point::new(3.0, 4.0)]);
    let out = d.to_text().unwrap();
    assert_eq!(Document::parse(&out).unwrap(), d);
    assert!(
        out.contains("(decimals 3)") && !out.contains("(gap"),
        "{out}"
    );
}

#[test]
fn board_size_dimensions_of_the_fixture() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/kicad9-minimal.kicad_pcb");
    let b = Board::load(path).unwrap();
    let dims = board_size_dimensions(&b, &Id("v".into()));
    assert_eq!(dims.len(), 2);
    let w = &dims[0];
    assert_eq!(w.kind, DimensionKind::Horizontal);
    assert!((w.last[1].x - w.last[0].x - 50.0).abs() < 1e-9);
    let h = &dims[1];
    assert!((h.last[1].y - h.last[0].y - 30.0).abs() < 1e-9);
}

/// A tall layer stack that does not fit under the drill table goes to a
/// page of its own instead of running over the title block.
#[test]
fn tall_stackup_gets_its_own_page() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/kicad9-minimal.kicad_pcb");
    let mut b = Board::load(path).unwrap();
    let sheet = kdraft_sheet::default_sheet().unwrap();
    if let Some(st) = b.stackup.as_mut() {
        let l = st.layers[3].clone();
        for _ in 0..40 {
            st.layers.insert(3, l.clone());
        }
    }
    let d = Document::new_for_board(&b, "b.kicad_pcb", &sheet);
    let titles: Vec<_> = d.pages.iter().filter_map(|p| p.title.clone()).collect();
    assert!(titles.contains(&"Layer stack".to_string()), "{titles:?}");
}

fn bom_of(source: kdraft_board::bom::BomSource, lines: &[(&str, bool)]) -> kdraft_board::bom::Bom {
    kdraft_board::bom::Bom {
        source,
        lines: lines
            .iter()
            .enumerate()
            .map(|(i, (v, dnp))| kdraft_board::bom::BomLine {
                refs: vec![format!("R{}", i + 1)],
                qty: 1,
                value: v.to_string(),
                footprint: "R_0603".into(),
                dnp: *dnp,
                fields: Default::default(),
            })
            .collect(),
        extra_columns: Vec::new(),
    }
}

/// The reported bug: numbers stored from the PCB fallback at `doc new`
/// must not be mixed with a schematic BOM at export (a group key the PCB
/// did not have got the next free number, out of sequence).
#[test]
fn bom_numbers_follow_their_source() {
    use kdraft_board::bom::{number_lines, BomSource};
    let mut doc = Document::new("b.kicad_pcb");
    let mut page = Page::new("BOM", PageSize::a3_landscape());
    let pcb = bom_of(BomSource::Pcb, &[("10k", false), ("100n", false)]);
    let mut t = Table::new(TableKind::Bom, Point::ORIGIN);
    // As written before the source was recorded: no tag (PCB).
    t.bom_items = pcb
        .lines
        .iter()
        .enumerate()
        .map(|(i, l)| (l.key(), i + 1))
        .collect();
    page.items.push(Item::Table(t));
    doc.pages.push(page);
    let sch = bom_of(
        BomSource::Schematic,
        &[("10k", false), ("CONN", true), ("100n", false)],
    );
    let stored = crate::bom_numbers(&doc, "schematic");
    assert!(
        stored.is_empty(),
        "PCB numbers are not used for the schematic BOM"
    );
    assert_eq!(number_lines(&sch.lines, &stored), vec![1, 2, 3]);
    assert_eq!(crate::bom_numbers(&doc, "pcb").len(), 2);
    // Stored for the schematic source; a later new group gets the next
    // number, the others keep theirs.
    assert!(crate::store_bom_numbers(&mut doc, &sch));
    let text = doc.to_text().unwrap();
    assert!(text.contains("(source schematic)"), "{text}");
    let back = Document::parse(&text).unwrap();
    assert_eq!(back, doc);
    let later = bom_of(
        BomSource::Schematic,
        &[
            ("1u", false),
            ("10k", false),
            ("CONN", true),
            ("100n", false),
        ],
    );
    let n = number_lines(&later.lines, &crate::bom_numbers(&doc, "schematic"));
    assert_eq!(n, vec![4, 1, 2, 3]);
    // Renumbering starts from scratch in table order.
    assert!(crate::renumber_bom(&mut doc, &later));
    let n = number_lines(&later.lines, &crate::bom_numbers(&doc, "schematic"));
    assert_eq!(n, vec![1, 2, 3, 4]);
    assert!(!crate::store_bom_numbers(&mut doc, &later), "stable");
}

#[test]
fn styles_round_trip_canonically() {
    use crate::style::{ItemStyle, Rgba, StyleKind};
    let mut d = Document::new("b.kicad_pcb");
    let mut p = Page::new("g", PageSize::a4_landscape());
    let mut it = Item::Graphic(Graphic {
        id: Id("g1".into()),
        shape: GraphicShape::Line {
            start: Point::new(0.0, 0.0),
            end: Point::new(1.0, 0.0),
        },
        style: None,
        extra: vec![],
    });
    let mut st = ItemStyle::default();
    st.set("stroke", Some("#ff000080")).unwrap();
    st.set("width", Some("0.35")).unwrap();
    st.set("pattern", Some("dash-dot")).unwrap();
    st.set("arrows", Some("end")).unwrap();
    st.set("fill", Some("none")).unwrap();
    assert!(it.set_style_override(&st));
    p.items.push(it);
    d.pages.push(p);
    d.styles
        .appearance
        .get_mut(StyleKind::Dimension)
        .set("text_color", Some("#0000FF"))
        .unwrap();
    let text = d.to_text().unwrap();
    assert!(text.contains("(appearance"), "{text}");
    assert!(text.contains("(stroke \"#FF000080\")"), "{text}");
    let back = Document::parse(&text).unwrap();
    assert_eq!(back, d);
    assert_eq!(back.to_text().unwrap(), text);
    let got = back.pages[0].items[0].style_override();
    assert_eq!(
        got.stroke,
        Some(Rgba {
            r: 255,
            g: 0,
            b: 0,
            a: 128
        })
    );
    let eff = back.pages[0].items[0].effective_style(&back.styles);
    assert_eq!(eff.width, Some(0.35));
    assert_eq!(eff.text_height, Some(back.styles.text_height));

    // Tolerant read: out of order, lower-case hex, an unknown attribute
    // and a malformed one are accepted; the writer puts known attributes
    // in canonical order and keeps the rest.
    let messy = text.replace("(stroke \"#FF000080\")", "").replace(
        "(appearance",
        "(appearance (glow 3) (width bogus) (stroke \"#00ff00\")",
    );
    let m = Document::parse(&messy).unwrap();
    let o = m.pages[0].items[0].style_override();
    assert_eq!(o.stroke, Some(Rgba::rgb(0, 255, 0)));
    let out = m.to_text().unwrap();
    assert!(
        out.contains("(glow 3)") && out.contains("(width bogus)"),
        "{out}"
    );
    assert_eq!(Document::parse(&out).unwrap().to_text().unwrap(), out);
}

#[test]
fn rgba_parse_and_print() {
    use crate::style::Rgba;
    assert_eq!(Rgba::parse("#abc"), Some(Rgba::rgb(0xAA, 0xBB, 0xCC)));
    assert_eq!(Rgba::parse("102030"), Some(Rgba::rgb(0x10, 0x20, 0x30)));
    assert_eq!(Rgba::parse("none"), Some(Rgba::NONE));
    assert_eq!(Rgba::parse("#12345"), None);
    assert_eq!(Rgba::parse("#zzzzzz"), None);
    assert_eq!(Rgba::rgb(1, 2, 3).to_string(), "#010203");
    assert_eq!(
        Rgba {
            r: 1,
            g: 2,
            b: 3,
            a: 4
        }
        .to_string(),
        "#01020304"
    );
}
