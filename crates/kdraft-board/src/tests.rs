use std::path::PathBuf;

use crate::geom::{BBox, Point};
use crate::*;

pub(crate) fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
}

fn k9() -> Board {
    Board::load(fixture("kicad9-minimal.kicad_pcb")).unwrap()
}

fn k10() -> Board {
    Board::load(fixture("kicad10-minimal.kicad_pcb")).unwrap()
}

fn near(a: Point, x: f64, y: f64) -> bool {
    a.approx_eq(Point::new(x, y), 1e-9)
}

fn bbox_near(b: Option<BBox>, x0: f64, y0: f64, x1: f64, y1: f64) -> bool {
    b.is_some_and(|b| {
        b.approx_eq(
            &BBox {
                min: Point::new(x0, y0),
                max: Point::new(x1, y1),
            },
            1e-6,
        )
    })
}

#[test]
fn k9_header_layers_title_stackup() {
    let b = k9();
    assert_eq!(b.version, 20241229);
    assert_eq!(b.generator.as_deref(), Some("pcbnew"));
    assert_eq!(b.generator_version.as_deref(), Some("9.0"));
    assert_eq!(b.thickness, Some(1.6));
    assert_eq!(b.paper.as_deref(), Some("A4"));
    assert_eq!(b.layers.len(), 16);
    assert_eq!(b.copper_layer_count(), 2);
    let silk = b.layers.iter().find(|l| l.name == "F.SilkS").unwrap();
    assert_eq!((silk.ordinal, silk.kind.as_str()), (5, "user"));
    assert_eq!(silk.user_name.as_deref(), Some("F.Silkscreen"));
    assert_eq!(b.title_block.title.as_deref(), Some("kdraft test board"));
    assert_eq!(b.title_block.rev.as_deref(), Some("A"));
    assert_eq!(
        b.title_block.comments.get(&4).map(String::as_str),
        Some("KiCad 9 style")
    );
    let s = b.stackup.as_ref().unwrap();
    assert_eq!(s.layers.len(), 7);
    assert_eq!(s.copper_count(), 2);
    assert_eq!(s.copper_finish.as_deref(), Some("ENIG"));
    assert_eq!(s.dielectric_constraints, Some(false));
    let core = &s.layers[3];
    assert_eq!(core.name, "dielectric 1");
    assert_eq!(core.kind, "core");
    assert_eq!(core.thickness, Some(1.51));
    assert_eq!(core.material.as_deref(), Some("FR4"));
    assert_eq!(core.epsilon_r, Some(4.5));
    assert_eq!(core.loss_tangent, Some(0.02));
    assert!((s.total_thickness() - 1.6).abs() < 1e-9);
    assert!(b.warnings.is_empty(), "{:?}", b.warnings);
}

#[test]
fn k9_footprints_pads_and_transforms() {
    let b = k9();
    assert_eq!(b.footprints.len(), 5);

    let r1 = b.footprint_by_reference("R1").unwrap();
    assert_eq!(r1.lib_id, "Resistor_SMD:R_0603_1608Metric");
    assert_eq!(r1.value.as_deref(), Some("10k"));
    assert_eq!(r1.side, Side::Front);
    assert_eq!(r1.attributes.mount, MountType::Smd);
    assert_eq!(
        r1.properties.get("MPN").map(String::as_str),
        Some("RC0603FR-0710KL")
    );
    assert_eq!(r1.properties.get("Datasheet").map(String::as_str), Some(""));
    assert_eq!(
        r1.uuid.as_deref(),
        Some("11111111-0000-0000-0000-000000000001")
    );
    assert_eq!(r1.pads.len(), 2);
    assert!(near(r1.pads[0].position, 19.175, 10.0));
    assert_eq!(r1.pads[0].net.as_deref(), Some("VCC"));
    assert_eq!(r1.pads[0].kind, PadType::Smd);
    assert_eq!(r1.pads[0].shape, "roundrect");
    assert_eq!(r1.pads[0].layers, ["F.Cu", "F.Mask", "F.Paste"]);
    assert!(bbox_near(r1.courtyard, 18.52, 9.27, 21.48, 10.73));
    assert!(bbox_near(r1.fab, 19.2, 9.5875, 20.8, 10.4125));

    // Back side, rotated 90: local -X maps to board +Y.
    let c1 = b.footprint_by_reference("C1").unwrap();
    assert_eq!(c1.side, Side::Back);
    assert_eq!(c1.layer, "B.Cu");
    assert_eq!(c1.rotation, 90.0);
    assert!(near(c1.pads[0].position, 30.0, 10.48));
    assert!(near(c1.pads[1].position, 30.0, 9.52));
    assert_eq!(c1.pads[0].rotation, 90.0);
    assert_eq!(c1.pads[0].rel_rotation, 0.0);
    assert!(bbox_near(c1.courtyard, 29.54, 9.09, 30.46, 10.91));

    let j1 = b.footprint_by_reference("J1").unwrap();
    assert_eq!(j1.attributes.mount, MountType::ThroughHole);
    assert!(near(j1.pads[1].position, 40.0, 12.46));
    assert!(near(j1.pads[1].rel_position, 0.0, 2.54));
    assert_eq!(
        j1.pads[1].drill.unwrap().shape,
        DrillShape::Oval {
            width: 1.0,
            height: 1.5
        }
    );
    assert_eq!(
        j1.pads[0].drill.unwrap().shape,
        DrillShape::Round { diameter: 1.0 }
    );
    assert_eq!(j1.pads[1].net.as_deref(), Some("Net-(J1-Pin_2)"));

    let h1 = b.footprint_by_reference("H1").unwrap();
    assert!(h1.attributes.exclude_from_bom && h1.attributes.exclude_from_pos_files);
    assert_eq!(h1.attributes.mount, MountType::Unspecified);
    assert_eq!(h1.pads[0].kind, PadType::NpThruHole);
    assert_eq!(h1.pads[0].number, "");
    assert_eq!(h1.pads[0].net, None);

    // Legacy fp_text reference/value, no uuid, no properties.
    let u9 = b.footprint_by_reference("U9").unwrap();
    assert_eq!(u9.value.as_deref(), Some("LM358"));
    assert_eq!(u9.uuid, None);
    assert!(u9.attributes.dnp);
    assert!(u9.properties.is_empty());
    assert_eq!(u9.pads[0].rel_rotation, 0.0);
}

#[test]
fn k9_vias_outline_holes() {
    let b = k9();
    assert_eq!(b.vias.len(), 2);
    assert_eq!(b.vias[0].net.as_deref(), Some("VCC"));
    assert_eq!(b.vias[1].net.as_deref(), Some("GND"));
    assert_eq!(b.vias[0].kind, ViaType::Through);
    assert_eq!(b.vias[0].layers, ["F.Cu", "B.Cu"]);

    let o = &b.outline;
    assert_eq!(o.primitives.len(), 6);
    assert!(o.is_closed(), "{:?}", o.open_chains);
    assert_eq!(o.contours.len(), 2);
    assert!(bbox_near(o.bbox, 0.0, 0.0, 50.0, 30.0));
    let outer = o.outer().unwrap();
    assert_eq!(outer.vertices.len(), 5);
    assert!(near(outer.vertices[0], 0.0, 0.0));
    assert!(near(outer.vertices[1], 50.0, 0.0));
    assert!(near(outer.vertices[2], 50.0, 25.0));
    assert!(near(outer.vertices[3], 45.0, 30.0));
    assert!(near(outer.vertices[4], 0.0, 30.0));

    let holes = b.holes();
    assert_eq!(holes.len(), 5);
    assert_eq!(
        holes.iter().map(|h| h.index).collect::<Vec<_>>(),
        [0, 1, 2, 3, 4]
    );
    assert_eq!(holes.iter().filter(|h| !h.plated).count(), 1);
    let table = b.drill_table();
    let rows: Vec<_> = table
        .iter()
        .map(|r| (r.shape, r.plated, r.kind, r.count))
        .collect();
    assert_eq!(
        rows,
        vec![
            (DrillShape::Round { diameter: 0.3 }, true, HoleKind::Via, 2),
            (DrillShape::Round { diameter: 1.0 }, true, HoleKind::Pad, 1),
            (
                DrillShape::Oval {
                    width: 1.0,
                    height: 1.5
                },
                true,
                HoleKind::Pad,
                1
            ),
            (DrillShape::Round { diameter: 3.2 }, false, HoleKind::Pad, 1),
        ]
    );
}

#[test]
fn k10_model() {
    let b = k10();
    assert_eq!(b.version, 20260206);
    assert_eq!(b.generator_version.as_deref(), Some("10.0"));
    assert_eq!(b.copper_layer_count(), 4);
    let in1 = b.layers.iter().find(|l| l.name == "In1.Cu").unwrap();
    assert_eq!(
        (in1.kind.as_str(), in1.user_name.as_deref()),
        ("power", Some("GND_plane"))
    );

    let s = b.stackup.as_ref().unwrap();
    let d1: Vec<_> = s
        .layers
        .iter()
        .filter(|l| l.name == "dielectric 1")
        .collect();
    assert_eq!(d1.len(), 2);
    assert_eq!(
        (d1[0].sublayer, d1[0].material.as_deref()),
        (0, Some("2116"))
    );
    assert_eq!(
        (d1[1].sublayer, d1[1].material.as_deref()),
        (1, Some("1080"))
    );
    assert_eq!(d1[1].kind, "prepreg");
    assert_eq!(d1[1].epsilon_r, Some(3.9));
    assert_eq!(s.copper_count(), 4);
    assert_eq!(s.layers.len(), 12);
    assert!((s.total_thickness() - 1.6).abs() < 1e-9);
    assert_eq!(s.dielectric_constraints, Some(true));
    assert_eq!(s.castellated_pads, Some(true));

    let u1 = b.footprint_by_reference("U1").unwrap();
    assert!(near(u1.pads[0].position, 9.25, 11.45));
    assert_eq!(u1.pads[0].net.as_deref(), Some("VIN"));
    assert_eq!(u1.pads[1].net.as_deref(), Some("GND"));
    assert!(bbox_near(u1.fab, 8.5, 8.5, 11.5, 11.5));
    assert_eq!(
        u1.properties.get("ki_fp_filters").map(String::as_str),
        Some("QFN*")
    );

    let mk1 = b.footprint_by_reference("MK1").unwrap();
    assert!(mk1.attributes.dnp && mk1.attributes.exclude_from_bom);
    assert!(near(mk1.pads[0].position, 30.0, 14.0));
    assert_eq!(mk1.pads[0].rel_rotation, 0.0);
    let holes = b.holes();
    assert_eq!(holes.len(), 4);
    assert!(holes[0].position.approx_eq(Point::new(29.8, 14.0), 1e-9));
    assert_eq!(
        holes[0].shape,
        DrillShape::Oval {
            width: 1.2,
            height: 2.2
        }
    );

    let kinds: Vec<_> = b.vias.iter().map(|v| v.kind).collect();
    assert_eq!(
        kinds,
        [ViaType::Through, ViaType::BlindBuried, ViaType::Micro]
    );
    assert_eq!(b.vias[2].net.as_deref(), Some("GND"));

    let o = &b.outline;
    assert_eq!(o.primitives.len(), 2);
    assert!(matches!(
        o.primitives[0].source,
        outline::EdgeSource::Footprint { index: 1 }
    ));
    assert!(matches!(o.primitives[1].source, outline::EdgeSource::Board));
    assert_eq!(o.contours.len(), 2);
    assert!(o.is_closed());
    assert!(bbox_near(o.bbox, 0.0, 0.0, 40.0, 20.0));
    assert!(bbox_near(o.contours[0].bbox, 27.0, 9.0, 33.0, 11.0));
    assert_eq!(o.outer().unwrap().vertices.len(), 4);
    assert!(b.warnings.is_empty(), "{:?}", b.warnings);
}

#[test]
fn hash_covers_bytes() {
    let a = k9();
    let text = std::fs::read_to_string(fixture("kicad9-minimal.kicad_pcb")).unwrap();
    let b = Board::parse_str(&(text.clone() + "\n")).unwrap();
    assert_ne!(a.hash, b.hash);
    assert_eq!(a.hash, Board::parse_str(&text).unwrap().hash);
    assert_eq!(a.hash.to_hex().len(), 64);
}

#[test]
fn unknown_nodes_are_ignored() {
    let text = std::fs::read_to_string(fixture("kicad9-minimal.kicad_pcb")).unwrap();
    let text = text.replacen(
        "(attr smd)",
        "(attr smd future_flag)\n\t\t(future_node (x 1) \"y\")",
        1,
    );
    let text = text.replacen("(embedded_fonts no)\n)", "(something_new 1 2 3)\n)", 1);
    let b = Board::parse_str(&text).unwrap();
    assert_eq!(b.footprints.len(), 5);
}

#[test]
fn errors() {
    let e = Board::parse_str("(kicad_sch (version 1))").unwrap_err();
    assert!(matches!(e, BoardError::NotABoard { found: Some(ref h) } if h == "kicad_sch"));
    let e = Board::parse_str("(kicad_pcb (version 1)\n (via (at 1 nope)))").unwrap_err();
    let msg = e.to_string();
    assert!(msg.contains("2:"), "{msg}");
    assert!(matches!(
        Board::parse_str("(kicad_pcb"),
        Err(BoardError::Syntax(_))
    ));
    assert!(matches!(
        Board::from_bytes(vec![b'(', 0xff, b')']),
        Err(BoardError::NotUtf8 { offset: 1 })
    ));
    assert!(matches!(
        Board::load("/nonexistent/x.kicad_pcb"),
        Err(BoardError::Io { .. })
    ));
}

fn edit(f: impl Fn(String) -> String) -> Board {
    let text = std::fs::read_to_string(fixture("kicad9-minimal.kicad_pcb")).unwrap();
    Board::parse_str(&f(text)).unwrap()
}

#[test]
fn diff_identical_is_empty() {
    let d = diff(&k9(), &k9());
    assert!(d.is_empty(), "{d:?}");
    // Whitespace-only change: different hash, same semantics.
    let b = edit(|t| t.replace("\t", "  "));
    assert!(diff(&k9(), &b).is_empty());
}

#[test]
fn diff_reports_footprint_changes() {
    let new = edit(|t| {
        t.replacen("(at 20 10)", "(at 21 10 90)", 1)
            .replacen("\"10k\"", "\"4k7\"", 1)
            .replacen(
                "(layer \"B.Cu\")\n\t\t(uuid \"11111111-0000-0000-0000-000000000002\")",
                "(layer \"F.Cu\")\n\t\t(uuid \"11111111-0000-0000-0000-000000000002\")",
                1,
            )
            .replacen("(fp_text value \"LM358\"", "(fp_text value \"LM324\"", 1)
    });
    let d = diff(&k9(), &new);
    assert!(d.footprints_added.is_empty() && d.footprints_removed.is_empty());
    let by_ref = |r: &str| {
        d.footprints_changed
            .iter()
            .find(|c| c.id.reference.as_deref() == Some(r))
            .unwrap()
            .changes
            .clone()
    };
    let r1 = by_ref("R1");
    assert!(r1.contains(&FootprintChange::Moved {
        from: Point::new(20.0, 10.0),
        to: Point::new(21.0, 10.0)
    }));
    assert!(r1.contains(&FootprintChange::Rotated {
        from: 0.0,
        to: 90.0
    }));
    assert!(r1.contains(&FootprintChange::Value {
        old: Some("10k".into()),
        new: Some("4k7".into())
    }));
    assert_eq!(
        by_ref("C1"),
        vec![FootprintChange::Flipped {
            from: Side::Back,
            to: Side::Front
        }]
    );
    // U9 has no uuid and is matched by reference.
    assert_eq!(by_ref("U9").len(), 1);
    // R1 moved, so its pads' positions changed but it has no holes; J1 did not move.
    assert!(d.holes.is_none());
    assert!(d.outline.is_none());
}

#[test]
fn diff_reports_added_removed_outline_holes_title_stackup() {
    let new = edit(|t| {
        t.replacen(
            "(uuid \"11111111-0000-0000-0000-000000000004\")",
            "(uuid \"99999999-0000-0000-0000-000000000004\")",
            1,
        )
        .replacen(
            "(property \"Reference\" \"H1\"",
            "(property \"Reference\" \"H2\"",
            1,
        )
        .replacen("(end 12 15)", "(end 13 15)", 1)
        .replacen("(at 26 20)", "(at 27 20)", 1)
        .replacen("(rev \"A\")", "(rev \"B\")", 1)
        .replacen(
            "(copper_finish \"ENIG\")",
            "(copper_finish \"HAL SnPb\")",
            1,
        )
        .replacen("(comment 4 \"KiCad 9 style\")", "", 1)
    });
    let d = diff(&k9(), &new);
    assert_eq!(d.footprints_removed.len(), 1);
    assert_eq!(d.footprints_removed[0].reference.as_deref(), Some("H1"));
    assert_eq!(d.footprints_added.len(), 1);
    assert_eq!(d.footprints_added[0].reference.as_deref(), Some("H2"));
    let oc = d.outline.as_ref().unwrap();
    assert_eq!(oc.old_primitives, 6);
    let hc = d.holes.as_ref().unwrap();
    assert_eq!(
        (hc.old_count, hc.new_count, hc.added, hc.removed),
        (5, 5, 1, 1)
    );
    assert!(d.stackup_changed);
    let fields: Vec<_> = d.title_block.iter().map(|f| f.field.as_str()).collect();
    assert_eq!(fields, ["rev", "comment 4"]);
    assert!(!d.is_empty());
}

#[test]
fn diff_reference_fallback_and_remove() {
    let new = edit(|t| {
        // Drop C1 entirely.
        let start = t.find("\t(footprint \"Capacitor_SMD").unwrap();
        let end = t.find("\t(footprint \"Connector_PinHeader").unwrap();
        format!("{}{}", &t[..start], &t[end..])
    });
    let d = diff(&k9(), &new);
    assert_eq!(d.footprints_removed.len(), 1);
    assert_eq!(d.footprints_removed[0].label(), "C1");
    assert!(d.footprints_added.is_empty());
    assert!(d.footprints_changed.is_empty());
}

#[test]
fn crlf_file_loads_identically() {
    let text = std::fs::read_to_string(fixture("kicad9-minimal.kicad_pcb")).unwrap();
    let crlf = text.replace('\n', "\r\n");
    let tree = kdraft_sexpr::Tree::parse(crlf.as_str()).unwrap();
    assert_eq!(tree.to_text(), crlf);
    let b = Board::parse_str(&crlf).unwrap();
    assert!(diff(&k9(), &b).is_empty());
    assert_eq!(b.footprints.len(), 5);
}

#[test]
fn kicad_tool_grouping_merges_slot_into_round_tool() {
    let b = Board::load(fixture("kicad9-minimal.kicad_pcb")).unwrap();
    let g = crate::kicad_tool_groups(&b.holes());
    let summary: Vec<(f64, bool, usize, usize)> = g
        .iter()
        .map(|t| (t.diameter, t.plated, t.holes, t.slots))
        .collect();
    assert_eq!(
        summary,
        [(0.3, true, 2, 0), (1.0, true, 2, 1), (3.2, false, 1, 0)]
    );
    assert_eq!(b.drill_table().len(), 4);
}

#[test]
fn oval_drill_with_equal_sides_is_round_and_merges() {
    let text = std::fs::read_to_string(fixture("kicad9-minimal.kicad_pcb")).unwrap();
    assert!(text.contains("(drill oval 1 1.5)"));
    let text = text.replace("(drill oval 1 1.5)", "(drill oval 1 1)");
    let b = Board::from_bytes(text.clone().into_bytes()).unwrap();
    let j1 = b.footprint_by_reference("J1").unwrap();
    assert_eq!(
        j1.pads[1].drill.unwrap().shape,
        DrillShape::Round { diameter: 1.0 }
    );
    let t = b.drill_table();
    // 0.3 via, 1.0 pad (now 2 holes), 3.2 NPTH: the slot row is gone.
    assert_eq!(t.len(), 3, "{t:?}");
    assert!(t
        .iter()
        .any(|r| r.shape == DrillShape::Round { diameter: 1.0 } && r.count == 2));
    let single = text.replace("(drill oval 1 1)", "(drill oval 0.8)");
    let b = Board::from_bytes(single.into_bytes()).unwrap();
    assert!(b
        .holes()
        .iter()
        .any(|h| h.shape == DrillShape::Round { diameter: 0.8 }));
}

#[test]
fn footprint_texts_and_graphics() {
    let b = Board::load(fixture("kicad9-minimal.kicad_pcb")).unwrap();
    let c1 = b.footprint_by_reference("C1").unwrap();
    let r = c1
        .texts
        .iter()
        .find(|t| t.kind == FpTextKind::Reference)
        .unwrap();
    assert_eq!(r.text, "C1");
    // Local (0, 1.16) on a footprint at (30, 10) rotated 90 degrees.
    assert!(
        r.position.approx_eq(Point::new(31.16, 10.0), 1e-9),
        "{:?}",
        r.position
    );
    assert_eq!(r.angle, 90.0);
    assert!(r.keep_upright && !r.hidden);
    let r1 = b.footprint_by_reference("R1").unwrap();
    let ds = r1
        .texts
        .iter()
        .find(|t| t.kind == FpTextKind::Property("Datasheet".into()))
        .unwrap();
    assert!(ds.hidden);
    assert!(r1.graphics.iter().any(|g| g.kind == FpLayer::Fab));
    // Legacy fp_text.
    let u9 = b.footprint_by_reference("U9").unwrap();
    assert!(u9
        .texts
        .iter()
        .any(|t| t.kind == FpTextKind::Value && t.text == "LM358"));
}
