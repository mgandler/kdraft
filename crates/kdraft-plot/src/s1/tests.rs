use std::path::PathBuf;

use kdraft_board::Board;
use kdraft_source::{KicadCli, Version};

use super::*;
use crate::fake::{fake_kicad, fake_transform, FakeOptions};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/kicad9-minimal.kicad_pcb")
}

fn cli() -> KicadCli {
    KicadCli {
        path: PathBuf::from("kicad-cli"),
        version: Version::new(9, 0, 6),
        version_string: "9.0.6".into(),
        vars: Vec::new(),
    }
}

fn run_with(opts: FakeOptions) -> (tempfile::TempDir, S1Report) {
    let board = Board::load(fixture()).unwrap();
    let runner = fake_kicad(board, opts);
    let dir = tempfile::tempdir().unwrap();
    let o = S1Options {
        board: fixture(),
        out_dir: dir.path().join("s1"),
        drawing_sheet: None,
    };
    let r = run(&o, &cli(), &runner, &mut |_| {}).unwrap();
    (dir, r)
}

#[test]
fn end_to_end_recovers_each_mode_transform() {
    let (dir, r) = run_with(FakeOptions {
        reject_layers: vec!["F.Courtyard".into()],
        ..Default::default()
    });
    let board = Board::load(fixture()).unwrap();
    assert!(r.pass, "{}", r.verdict);
    for m in &r.modes {
        assert_eq!(m.status, Status::Pass, "mode {}: {:?}", m.mode, m.notes);
        let t = m.fit.as_ref().unwrap().transform;
        let (_, _, truth) = fake_transform(&board, m.mode, false);
        assert!(
            (t.sx - truth.a).abs() < 1e-6 * truth.a.abs(),
            "{t:?} {truth:?}"
        );
        assert!(
            (t.sy - truth.d).abs() < 1e-6 * truth.d.abs(),
            "{t:?} {truth:?}"
        );
        assert!((t.ox - truth.e).abs() < 1e-3 * t.scale(), "{t:?} {truth:?}");
        assert!((t.oy - truth.f).abs() < 1e-3 * t.scale(), "{t:?} {truth:?}");
        // The fake plots at 1:1 on the page.
        assert!((m.page_mm_per_board_mm.unwrap() - 1.0).abs() < 1e-6);
        let cu = m.copper.iter().find(|c| c.layer == "F.Cu").unwrap();
        assert_eq!(cu.residuals.within_0_01, 1.0);
        assert!(m.overlay.is_some());
    }
    // Mode 2 of the fake is Y-up.
    assert!(!r.modes[2].fit.as_ref().unwrap().transform.y_down());
    assert!(r.modes[0].fit.as_ref().unwrap().transform.y_down());
    let mi = r.mirrored.as_ref().unwrap();
    assert_eq!(mi.status, Status::Pass, "{}", mi.note);
    assert_eq!(mi.mirrored_detected, Some(true));
    assert_eq!(mi.same_y_mapping, Some(true));
    // Courtyard fell back to the canonical name.
    assert_eq!(r.layer_name_notes.len(), 3, "{:?}", r.layer_name_notes);
    assert!(r.flags_rejected.is_empty());
    // Cross-checks agree with the synthetic outputs.
    assert_eq!(r.drill.status, Status::Pass);
    assert!(
        r.drill
            .findings
            .iter()
            .any(|f| f.contains("3 tool lines, 5 holes") || f.contains("3 tools")),
        "{:?}",
        r.drill.findings
    );
    assert!(
        r.drill
            .findings
            .iter()
            .any(|f| f.contains("tool list matches")),
        "{:?}",
        r.drill.findings
    );
    assert!(
        !r.drill.findings.iter().any(|f| f.contains("MISMATCH")),
        "{:?}",
        r.drill.findings
    );
    assert!(
        r.pos
            .findings
            .iter()
            .any(|f| f.contains("y' = -1.000000 y")),
        "{:?}",
        r.pos.findings
    );
    assert!(
        r.stats
            .findings
            .iter()
            .any(|f| f.starts_with("board.width = 50") && f.contains("matches")),
        "{:?}",
        r.stats.findings
    );
    assert!(r.help.iter().all(|h| h.flags.values().all(|v| *v)));
    let sh = &r.sheets[0];
    assert_eq!(sh.status, Status::Pass, "{}", sh.note);
    assert_eq!(sh.svg_size_mm, Some([420.0, 297.0]));
    let m = sh.content_margins_mm.unwrap();
    assert!(
        (m[0] - 10.0).abs() < 1e-9 && (m[3] - 10.0).abs() < 1e-9,
        "{m:?}"
    );
    assert!(sh.markers_found["title"]);
    assert!(!sh.markers_found["text_variable"]);
    assert_eq!(sh.unexpanded_variables, 1);
    let out = dir.path().join("s1");
    // The stub references the project variable from a printed field.
    let stub = std::fs::read_to_string(out.join("sheet/stub.kicad_pcb")).unwrap();
    assert!(stub.contains("(comment 2 \"${KDRAFT_S1_VAR}\")"), "{stub}");
    for f in [
        "report.json",
        "report.md",
        "overlay-mode0.svg",
        "svg/F_Cu-mode0-mirror.svg",
    ] {
        assert!(out.join(f).is_file(), "{f}");
    }
    let md = std::fs::read_to_string(out.join("report.md")).unwrap();
    assert!(md.contains("VERDICT: S1 PASS"), "{md}");
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("report.json")).unwrap()).unwrap();
    assert_eq!(json["pass"], true);
    // The overlay is still a parsable SVG with the added group.
    let ov = std::fs::read_to_string(out.join("overlay-mode0.svg")).unwrap();
    assert!(ov.contains("kdraft-s1-overlay"));
    crate::svg::parse(&ov).unwrap();
}

#[test]
fn rejected_flag_is_identified_and_dropped() {
    let (_dir, r) = run_with(FakeOptions {
        reject_flag: Some("--mode-single".into()),
        no_stats: true,
        ..Default::default()
    });
    assert_eq!(r.flags_rejected, vec!["--mode-single".to_string()]);
    assert!(r.pass, "{}", r.verdict);
    assert!(r
        .svg_runs
        .iter()
        .all(|s| !s.run.argv.contains(&"--mode-single".to_string())));
    assert_eq!(r.stats.status, Status::Fail);
    assert_eq!(
        r.help
            .iter()
            .find(|h| h.command == "pcb export stats")
            .unwrap()
            .run
            .exit_code,
        Some(1)
    );
}

#[test]
fn drill_report_parse_is_tolerant() {
    let text = "Drill file 'x.drl' contains\n    plated through holes:\n    T1  0,3mm\n    T1  0.30mm  0.012\"  (63 holes)\n    T2  1.00mm  0.039\"  (2 holes)  (2 slots)\n\n    Total plated holes count 65\n";
    let rows = parse_drill_report(text);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].holes, 63);
    assert!((rows[1].diameter_mm - 1.0).abs() < 1e-12);
    assert_eq!(rows[1].slots, 2);
}

#[test]
fn help_flag_matching_is_whole_word() {
    assert!(help_mentions(
        "[--mode-single] [--mode-multi]",
        "--mode-single"
    ));
    assert!(!help_mentions("[--mode-singlefile]", "--mode-single"));
    assert!(!help_mentions("[--mirror-x]", "--mirror"));
}

#[test]
fn quoting_for_display() {
    assert_eq!(quote("a b"), "\"a b\"");
    assert_eq!(quote("plain"), "plain");
}

/// The real drill report from kicad-cli 10.0.8 (user board A, name
/// anonymised): tool grouping by diameter, slots counted inside their
/// tool, NPTH in a separate block.
#[test]
fn real_10_0_8_drill_report() {
    let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/kicad-cli/10.0.8/drill-report-board-a.rpt");
    let text = std::fs::read_to_string(p).unwrap();
    let rows = parse_drill_report(&text);
    assert_eq!(rows.len(), 11, "{rows:#?}");
    let plated: Vec<_> = rows.iter().filter(|r| r.plated == Some(true)).collect();
    let unplated: Vec<_> = rows.iter().filter(|r| r.plated == Some(false)).collect();
    assert_eq!(plated.len(), 10);
    assert_eq!(unplated.len(), 1);
    assert_eq!(plated.iter().map(|r| r.holes).sum::<usize>(), 750);
    assert_eq!(unplated[0].holes, 2);
    assert_eq!(unplated[0].tool.as_deref(), Some("T11"));
    assert!((unplated[0].diameter_mm - 1.6).abs() < 1e-9);
    let t3 = rows
        .iter()
        .find(|r| r.tool.as_deref() == Some("T3"))
        .unwrap();
    assert_eq!((t3.holes, t3.slots), (4, 4));
    assert!((t3.diameter_mm - 0.5).abs() < 1e-9);
    assert_eq!(rows.iter().map(|r| r.slots).sum::<usize>(), 4);
    let t1 = &rows[0];
    assert_eq!((t1.tool.as_deref(), t1.holes), (Some("T1"), 523));
    // Totals lines are not tool lines.
    assert!(!rows.iter().any(|r| r.line.contains("Total")));
}
