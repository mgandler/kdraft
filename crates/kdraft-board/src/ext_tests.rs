//! Checks against real KiCad boards in `fixtures-ext/` (see
//! `scripts/fetch-fixtures.sh`). Each test skips when the directory is absent.

use std::collections::HashMap;
use std::path::PathBuf;

use kdraft_sexpr::{Node, NodeKind, Tree};

use crate::geom::{normalize_deg, Point};
use crate::*;

fn ext_boards() -> Vec<(String, PathBuf)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures-ext");
    let mut out = Vec::new();
    for major in ["9", "10"] {
        let Ok(rd) = std::fs::read_dir(root.join(major)) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "kicad_pcb") {
                let name = format!("{major}/{}", p.file_name().unwrap().to_string_lossy());
                out.push((name, p));
            }
        }
    }
    out.sort();
    if out.is_empty() {
        eprintln!("fixtures-ext/ not present; skipping (run scripts/fetch-fixtures.sh)");
    }
    out
}

/// Upstream files that are malformed S-expressions. At tag 9.0.9 this demo
/// contains 349 corrupted teardrop blocks (`(curved_edges no)filter_ratio
/// 0.9)`) that close lists early; the 10.0.8 copy is clean.
const KNOWN_MALFORMED: &[&str] = &["9/royalblue54L_feather__RoyalBlue54L-Feather.kicad_pcb"];

#[test]
fn ext_round_trip_and_load() {
    for (name, path) in ext_boards() {
        let text = std::fs::read_to_string(&path).unwrap();
        let tree = match Tree::parse(text.as_str()) {
            Ok(t) => t,
            Err(e) if KNOWN_MALFORMED.contains(&name.as_str()) => {
                eprintln!("{name}: known malformed upstream file, parse error as expected: {e}");
                continue;
            }
            Err(e) => panic!("{name}: {e}"),
        };
        assert!(
            !KNOWN_MALFORMED.contains(&name.as_str()),
            "{name} now parses; update KNOWN_MALFORMED"
        );
        assert!(tree.to_text() == text, "{name}: round trip differs");
        let board = Board::from_tree(&tree, BoardHash::of(text.as_bytes()))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let pads: usize = board.footprints.iter().map(|f| f.pads.len()).sum();
        let bb = board.outline.bbox;
        eprintln!(
            "{name}: v{} fp={} pads={} vias={} holes={} edge_prims={} contours={} open={} stackup={} bbox={}",
            board.version,
            board.footprints.len(),
            pads,
            board.vias.len(),
            board.holes().len(),
            board.outline.primitives.len(),
            board.outline.contours.len(),
            board.outline.open_chains.len(),
            board.stackup.as_ref().map_or(0, |s| s.layers.len()),
            bb.map_or("-".to_string(), |b| format!("{:.2}x{:.2}", b.width(), b.height())),
        );
        assert!(bb.is_some(), "{name}: no outline");
        let b = bb.unwrap();
        assert!(
            b.width() > 1.0 && b.width() < 1000.0,
            "{name}: width {}",
            b.width()
        );
        assert!(
            b.height() > 1.0 && b.height() < 1000.0,
            "{name}: height {}",
            b.height()
        );
        assert!(board.warnings.is_empty(), "{name}: {:?}", board.warnings);
        assert_eq!(board.outline.contours.len(), 1, "{name}");
        assert!(board.outline.open_chains.is_empty(), "{name}");
        assert!(diff(&board, &board).is_empty());
    }
}

/// End points of tracks and via centres per net name.
fn track_ends(tree: &Tree) -> HashMap<String, Vec<Point>> {
    let root = tree.root().unwrap();
    let mut table: HashMap<i64, String> = HashMap::new();
    for n in root.find_all("net") {
        if n.len() >= 3 {
            table.insert(n.i64_at(1).unwrap(), n.str_at(2).unwrap().into_owned());
        }
    }
    let net = |n: Node<'_>| -> Option<String> {
        let net = n.find("net")?;
        let c = net.child(net.len() - 1)?;
        if net.len() == 2 && c.kind() != NodeKind::String {
            return table.get(&net.i64_at(1).ok()?).cloned();
        }
        Some(c.atom()?.into_owned())
    };
    let pt = |n: Node<'_>, h: &str| -> Option<Point> {
        let p = n.find(h)?;
        Some(Point::new(p.f64_at(1).ok()?, p.f64_at(2).ok()?))
    };
    let mut out: HashMap<String, Vec<Point>> = HashMap::new();
    for item in root.lists() {
        let Some(name) = net(item) else { continue };
        let pts: Vec<Point> = match item.head() {
            Some("segment") | Some("arc") => [pt(item, "start"), pt(item, "end")]
                .into_iter()
                .flatten()
                .collect(),
            Some("via") => pt(item, "at").into_iter().collect(),
            _ => continue,
        };
        out.entry(name).or_default().extend(pts);
    }
    out
}

/// Verifies the footprint rotation convention: on rotated footprints, pad
/// centres computed with [`Point::rotate_kicad`] must coincide with track
/// end points of the same net far more often than with the opposite sense.
#[test]
fn ext_pad_positions_match_track_ends() {
    let mut totals = [0usize; 2]; // [ours, opposite]
    let mut sided = [[0usize; 2]; 2]; // per side: [ours, opposite]
    let mut checked = 0usize;
    let (mut angle_abs, mut angle_rel) = (0usize, 0usize);
    for (name, path) in ext_boards() {
        if KNOWN_MALFORMED.contains(&name.as_str()) {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let tree = Tree::parse(text.as_str()).unwrap();
        let board = Board::from_tree(&tree, BoardHash::of(text.as_bytes())).unwrap();
        let ends = track_ends(&tree);
        let mut hits = [0usize; 2];
        for fp in &board.footprints {
            let r = fp.rotation.rem_euclid(180.0);
            if r == 0.0 {
                continue; // 0/180 cannot distinguish the rotation sense
            }
            for pad in &fp.pads {
                // Pad angles in the file include the footprint angle: most
                // library pads are unrotated, so file angle == footprint angle.
                if normalize_deg(pad.rotation - fp.rotation) == 0.0 {
                    angle_abs += 1;
                } else if normalize_deg(pad.rotation) == 0.0 {
                    angle_rel += 1;
                }
                let Some(net) = pad.net.as_ref() else {
                    continue;
                };
                let Some(pts) = ends.get(net) else { continue };
                if pad.rel_position.dist(Point::ORIGIN) < 0.05 {
                    continue;
                }
                checked += 1;
                let ours = pad.position;
                let opposite = fp.position + pad.rel_position.rotate_kicad(-fp.rotation);
                let s = usize::from(fp.side == Side::Back);
                let a = pts.iter().any(|p| p.approx_eq(ours, 1e-4));
                let b = pts.iter().any(|p| p.approx_eq(opposite, 1e-4));
                // Symmetric footprints can match both ways; count exclusive hits.
                if a && !b {
                    hits[0] += 1;
                    sided[s][0] += 1;
                }
                if b && !a {
                    hits[1] += 1;
                    sided[s][1] += 1;
                }
            }
        }
        if hits != [0, 0] {
            eprintln!(
                "{name}: rotated-footprint pads only on track ends with ours={} opposite={}",
                hits[0], hits[1]
            );
        }
        totals[0] += hits[0];
        totals[1] += hits[1];
    }
    if checked == 0 {
        return;
    }
    eprintln!(
        "total: checked={checked} ours={} opposite={} front={:?} back={:?}",
        totals[0], totals[1], sided[0], sided[1]
    );
    eprintln!("pad angle == footprint angle: {angle_abs}, pad angle == 0: {angle_rel}");
    assert!(
        angle_abs > 10 * angle_rel.max(1),
        "{angle_abs} vs {angle_rel}"
    );
    assert!(totals[0] > 100, "too few matches to verify: {totals:?}");
    assert!(totals[0] > 20 * totals[1].max(1), "{totals:?}");
    // Both sides must be covered and agree.
    assert!(
        sided[0][0] > 20 * sided[0][1].max(1),
        "front {:?}",
        sided[0]
    );
    assert!(sided[1][0] > 20 * sided[1][1].max(1), "back {:?}", sided[1]);
}

/// Footprint graphics and texts on all demo boards: counts are plausible
/// and everything lies on or near the board.
#[test]
fn ext_footprint_graphics_and_texts() {
    for (name, path) in ext_boards() {
        if KNOWN_MALFORMED.contains(&name.as_str()) {
            continue;
        }
        let b = Board::load(&path).unwrap();
        let Some(bb) = b.outline.bbox else { continue };
        let margin = 30.0f64.max(0.25 * bb.width().max(bb.height()));
        let near = |p: Point| {
            p.x >= bb.min.x - margin
                && p.x <= bb.max.x + margin
                && p.y >= bb.min.y - margin
                && p.y <= bb.max.y + margin
        };
        let mut counts: HashMap<(FpLayer, Side), usize> = HashMap::new();
        let mut far = 0usize;
        let mut total = 0usize;
        let (mut texts, mut refs, mut hidden) = (0usize, 0usize, 0usize);
        for fp in &b.footprints {
            for g in &fp.graphics {
                *counts.entry((g.kind, g.side)).or_default() += 1;
                total += 1;
                if let Some(gb) = g.shape.bbox() {
                    // Parts may sit off the board (the unrouted demo);
                    // their graphics must still stay near the part.
                    if !near(gb.center()) && gb.center().dist(fp.position) > 60.0 {
                        far += 1;
                    }
                }
                assert!(g.width >= 0.0 && g.width < 5.0, "{name}: width {}", g.width);
                // Side of the graphic follows the footprint for normal
                // footprints (graphics flip with the part).
            }
            for t in &fp.texts {
                texts += 1;
                hidden += usize::from(t.hidden);
                if t.kind == FpTextKind::Reference {
                    refs += 1;
                    if let Some(r) = &fp.reference {
                        assert_eq!(&t.text, r, "{name}");
                    }
                }
                assert!(t.size.h > 0.0 && t.size.h < 20.0, "{name}: {:?}", t.size);
                // Hidden fields may carry odd positions (StickHub has hidden
                // Description fields 150 mm from their part); only visible
                // texts are checked.
                if !t.hidden && !near(t.position) && t.position.dist(fp.position) > 60.0 {
                    far += 1;
                }
            }
        }
        let fab: usize = counts
            .iter()
            .filter(|((k, _), _)| *k == FpLayer::Fab)
            .map(|(_, n)| n)
            .sum();
        eprintln!(
            "{name}: {} fps, graphics {total} (fab {fab}), {:?}, texts {texts} (refs {refs}, hidden {hidden}), far {far}",
            b.footprints.len(),
            {
                let mut v: Vec<_> = counts.iter().map(|((k, s), n)| format!("{k:?}/{s:?}={n}")).collect();
                v.sort();
                v
            }
        );
        assert!(refs <= b.footprints.len(), "{name}");
        assert!(
            far * 100 <= (total + texts).max(1),
            "{name}: {far} of {} items far from the board",
            total + texts
        );
        // Footprints with a fab box have fab graphics and vice versa.
        for fp in &b.footprints {
            assert_eq!(
                fp.fab.is_some(),
                fp.graphics.iter().any(|g| g.kind == FpLayer::Fab),
                "{name}: {:?}",
                fp.reference
            );
        }
    }
}

/// Local and board coordinates agree through the footprint placement.
#[test]
fn ext_local_to_board_graphics() {
    for (name, path) in ext_boards()
        .into_iter()
        .filter(|(n, _)| n.contains("video") || n.contains("pic_programmer"))
    {
        let b = Board::load(&path).unwrap();
        for fp in &b.footprints {
            let place = crate::geom::Placement {
                origin: fp.position,
                rotation: fp.rotation,
            };
            for t in &fp.texts {
                assert!(place.apply(t.local).approx_eq(t.position, 1e-9), "{name}");
            }
            for g in &fp.graphics {
                if let (Some(a), Some(l)) = (g.shape.bbox(), g.local.bbox()) {
                    if fp.rotation == 0.0 {
                        let c = place.apply(l.center());
                        assert!(c.approx_eq(a.center(), 1e-6), "{name}");
                    }
                }
            }
        }
    }
}
