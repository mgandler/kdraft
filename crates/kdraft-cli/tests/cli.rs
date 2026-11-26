use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
}

/// Runs kdraft with no user settings (unless the test passes
/// `--settings`) and without a kicad-cli from the environment.
fn kdraft(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kdraft"))
        .args(args)
        .env(
            "KDRAFT_SETTINGS",
            "/nonexistent-kdraft-settings/settings.json",
        )
        .output()
        .expect("run kdraft")
}

fn run<S: AsRef<std::ffi::OsStr>>(args: &[S]) -> (i32, String, String) {
    let args: Vec<&std::ffi::OsStr> = args.iter().map(|a| a.as_ref()).collect();
    let o = kdraft(&args);
    (
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

fn k9() -> PathBuf {
    fixture("kicad9-minimal.kicad_pcb")
}

#[test]
fn inspect_text() {
    let (code, out, _) = run(&[Path::new("inspect").as_os_str(), k9().as_os_str()]);
    assert_eq!(code, 0);
    for needle in [
        "format:      version 20241229 (pcbnew 9.0)",
        "size:        50 x 30 mm (x 0..50, y 0..30)",
        "layers:      2 copper, 16 total; board thickness 1.6 mm",
        "stackup:     7 layers (2 copper), 1.6 mm total, finish ENIG",
        "footprints:  5 (front 4, back 1; smd 3, tht 1, other 1); 8 pads",
        "dnp 1, excluded from BOM 1",
        "vias:        2 (through 2, blind/buried 0, micro 0)",
        "holes:       5 (plated 4, non-plated 1, slots 1); 4 drill sizes",
        "title:       kdraft test board",
    ] {
        assert!(out.contains(needle), "missing {needle:?} in\n{out}");
    }
}

#[test]
fn inspect_json() {
    let (code, out, _) = run(&[
        "inspect".as_ref(),
        "--json".as_ref(),
        fixture("kicad10-minimal.kicad_pcb").as_os_str(),
    ]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["version"], 20260206);
    assert_eq!(v["copper_layers"], 4);
    assert_eq!(v["outline"]["size"]["width"], 40.0);
    assert_eq!(v["stackup"]["layers"], 12);
    assert_eq!(v["footprints"]["dnp"], 1);
    assert_eq!(v["footprints"]["back"], 1);
    assert_eq!(v["vias"]["micro"], 1);
    assert_eq!(v["vias"]["blind_buried"], 1);
    assert_eq!(v["holes"]["total"], 4);
}

#[test]
fn drill_table_text_and_json() {
    let (code, out, _) = run(&["drill-table".as_ref(), k9().as_os_str()]);
    assert_eq!(code, 0);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 6, "{out}");
    assert!(lines[1].starts_with("0.3 ") && lines[1].contains("PTH") && lines[1].ends_with(" 2"));
    assert!(lines[3].starts_with("1 x 1.5") && lines[3].contains("slot"));
    assert!(lines[4].contains("NPTH"));
    assert!(lines[5].starts_with("total") && lines[5].ends_with(" 5"));

    let (code, out, _) = run(&["drill-table".as_ref(), "--json".as_ref(), k9().as_os_str()]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let rows = v.as_array().unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0]["shape"]["shape"], "round");
    assert_eq!(rows[0]["shape"]["diameter"], 0.3);
    assert_eq!(rows[0]["kind"], "via");
    assert_eq!(rows[0]["count"], 2);
    assert_eq!(rows[3]["plated"], false);
}

#[test]
fn diff_exit_codes_and_report() {
    let (code, out, _) = run(&["diff".as_ref(), k9().as_os_str(), k9().as_os_str()]);
    assert_eq!(code, 0);
    assert!(
        out.contains("No semantic changes (files are identical)"),
        "{out}"
    );

    let dir = tempfile::tempdir().unwrap();
    let text = std::fs::read_to_string(k9()).unwrap();
    let moved = dir.path().join("moved.kicad_pcb");
    std::fs::write(
        &moved,
        text.replacen("(at 20 10)", "(at 22.5 10)", 1)
            .replacen("\"10k\"", "\"22k\"", 1),
    )
    .unwrap();
    let (code, out, _) = run(&["diff".as_ref(), k9().as_os_str(), moved.as_os_str()]);
    assert_eq!(code, 1);
    assert!(out.contains("Footprints changed (1):"), "{out}");
    assert!(
        out.contains("~ R1: moved (20, 10) -> (22.5, 10); value \"10k\" -> \"22k\""),
        "{out}"
    );

    let ws = dir.path().join("ws.kicad_pcb");
    std::fs::write(&ws, text.replace('\t', "  ")).unwrap();
    let (code, out, _) = run(&["diff".as_ref(), k9().as_os_str(), ws.as_os_str()]);
    assert_eq!(code, 0);
    assert!(out.contains("bytes only"), "{out}");

    let (code, out, _) = run(&[
        "diff".as_ref(),
        "--json".as_ref(),
        k9().as_os_str(),
        moved.as_os_str(),
    ]);
    assert_eq!(code, 1);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["changed"], true);
    assert_eq!(v["identical_bytes"], false);
    assert_eq!(
        v["diff"]["footprints_changed"][0]["changes"][0]["change"],
        "moved"
    );
}

#[test]
fn sexpr_check() {
    let (code, out, _) = run(&["sexpr-check".as_ref(), k9().as_os_str()]);
    assert_eq!(code, 0);
    assert!(out.starts_with("ok: "), "{out}");
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.kicad_pcb");
    std::fs::write(&bad, "(kicad_pcb\n  (version 1)\n  (x \"open)\n").unwrap();
    let (code, _, err) = run(&["sexpr-check".as_ref(), bad.as_os_str()]);
    assert_eq!(code, 2);
    assert!(err.contains("unterminated string at 3:6"), "{err}");
}

#[test]
fn errors_exit_2() {
    let (code, _, err) = run(&["inspect", "/nonexistent/board.kicad_pcb"]);
    assert_eq!(code, 2);
    assert!(
        err.starts_with("error: loading /nonexistent/board.kicad_pcb"),
        "{err}"
    );
    let (code, _, _) = run(&["no-such-command"]);
    assert_eq!(code, 2);
}

#[test]
fn doc_new_and_validate() {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("boards").join("b.kicad_pcb");
    std::fs::create_dir_all(board.parent().unwrap()).unwrap();
    std::fs::copy(k9(), &board).unwrap();
    let docs = dir.path().join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    let out = docs.join("b.kdraft");
    let (code, stdout, err) = run(&[
        "doc".as_ref(),
        "new".as_ref(),
        board.as_os_str(),
        "-o".as_ref(),
        out.as_os_str(),
    ]);
    assert_eq!(code, 0, "{err}");
    assert!(
        stdout.contains(" pages, view scale 5:1; sheet KiCad default from default"),
        "{stdout}"
    );
    let text = std::fs::read_to_string(&out).unwrap();
    assert!(text.contains("(board \"../boards/b.kicad_pcb\")"), "{text}");
    assert!(text.contains("(scale 5)"));

    let (code, _, err) = run(&[
        "doc".as_ref(),
        "new".as_ref(),
        board.as_os_str(),
        "-o".as_ref(),
        out.as_os_str(),
    ]);
    assert_eq!(code, 2);
    assert!(err.contains("--force"), "{err}");
    let (code, _, _) = run(&[
        "doc".as_ref(),
        "new".as_ref(),
        board.as_os_str(),
        "-o".as_ref(),
        out.as_os_str(),
        "--force".as_ref(),
    ]);
    assert_eq!(code, 0);

    let (code, stdout, _) = run(&["doc".as_ref(), "validate".as_ref(), out.as_os_str()]);
    assert_eq!(code, 0);
    assert!(
        stdout.starts_with(&format!("ok: version 1, {} pages, ", doc_pages(&out))),
        "{stdout}"
    );

    let broken = docs.join("broken.kdraft");
    std::fs::write(&broken, text.replacen("(scale 5)", "(scale 0)", 1)).unwrap();
    let (code, stdout, _) = run(&["doc".as_ref(), "validate".as_ref(), broken.as_os_str()]);
    assert_eq!(code, 1);
    assert!(stdout.contains("scale 0 must be positive"), "{stdout}");

    std::fs::write(&broken, "(kdraft (version 9) (source (board \"x\")))").unwrap();
    let (code, _, err) = run(&["doc".as_ref(), "validate".as_ref(), broken.as_os_str()]);
    assert_eq!(code, 2);
    assert!(err.contains("version 9 is not supported"), "{err}");
}

#[test]
fn spike_s1_without_kicad_cli_explains_how_to_point_at_it() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("no-kicad-cli.exe");
    let o = Command::new(env!("CARGO_BIN_EXE_kdraft"))
        .args(["spike".as_ref(), "s1".as_ref(), k9().as_os_str()])
        .arg("--out")
        .arg(dir.path())
        .env("KDRAFT_KICAD_CLI", &missing)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("KDRAFT_KICAD_CLI"), "{err}");
    assert!(err.contains("--kicad-cli"), "{err}");
}

/// Real spike run; needs KDRAFT_KICAD_CLI pointing at kicad-cli.
#[test]
#[ignore]
fn spike_s1_real_kicad_cli() {
    let dir = tempfile::tempdir().unwrap();
    let (code, out, err) = run(&[
        "spike".as_ref(),
        "s1".as_ref(),
        k9().as_os_str(),
        "--out".as_ref(),
        dir.path().as_os_str(),
    ]);
    assert!(code == 0 || code == 1, "{out}\n{err}");
    assert!(out.contains("VERDICT:"), "{out}");
}

fn export_board(board: &Path, dir: &Path, stem: &str) -> (PathBuf, String) {
    let doc = dir.join(format!("{stem}.kdraft"));
    let (code, _, err) = run(&[
        "doc".as_ref(),
        "new".as_ref(),
        board.as_os_str(),
        "-o".as_ref(),
        doc.as_os_str(),
    ]);
    assert_eq!(code, 0, "{err}");
    let pdf = dir.join(format!("{stem}.pdf"));
    let (code, out, err) = run(&[
        "export".as_ref(),
        doc.as_os_str(),
        "-o".as_ref(),
        pdf.as_os_str(),
        "--no-kicad-cli".as_ref(),
    ]);
    assert_eq!(code, 0, "{out}\n{err}");
    (pdf, out)
}

/// Pages of a .kdraft file.
fn doc_pages(p: &Path) -> usize {
    kdraft_doc::Document::parse(&std::fs::read_to_string(p).unwrap())
        .unwrap()
        .pages
        .len()
}

fn pdf_page_count(bytes: &[u8]) -> usize {
    String::from_utf8_lossy(bytes)
        .matches("/Type/Page/")
        .count()
}

#[test]
fn export_fixtures_to_pdf_and_svg() {
    let dir = tempfile::tempdir().unwrap();
    for (f, stem) in [
        ("kicad9-minimal.kicad_pcb", "k9"),
        ("kicad10-minimal.kicad_pcb", "k10"),
    ] {
        let (pdf, out) = export_board(&fixture(f), dir.path(), stem);
        let n = doc_pages(&dir.path().join(format!("{stem}.kdraft")));
        assert!(n >= 3);
        assert!(out.contains(&format!("{n} pages")), "{out}");
        let bytes = std::fs::read(&pdf).unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
        assert_eq!(pdf_page_count(&bytes), n);
    }
    let doc = dir.path().join("k9.kdraft");
    let svg = dir.path().join("k9.svg");
    let (code, out, err) = run(&[
        "export".as_ref(),
        doc.as_os_str(),
        "-o".as_ref(),
        svg.as_os_str(),
        "--format".as_ref(),
        "svg".as_ref(),
        "--no-kicad-cli".as_ref(),
    ]);
    assert_eq!(code, 0, "{err}");
    for i in 1..=doc_pages(&doc) {
        let p = dir.path().join(format!("k9-p{i}.svg"));
        assert!(p.is_file(), "{out}");
    }
}

/// Demo boards from fixtures-ext/ (skipped when absent): one small KiCad 9,
/// one KiCad 10 format, one large.
#[test]
fn export_demo_boards_when_present() {
    let ext = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures-ext");
    let boards = [
        "9/ecc83__ecc83-pp.kicad_pcb",
        "10/pic_programmer__pic_programmer.kicad_pcb",
        "10/video__video.kicad_pcb",
    ];
    let dir = tempfile::tempdir().unwrap();
    for (i, b) in boards.iter().enumerate() {
        let p = ext.join(b);
        if !p.is_file() {
            eprintln!("skipping {b}: fixtures-ext not fetched");
            continue;
        }
        let (pdf, _) = export_board(&p, dir.path(), &format!("demo{i}"));
        let n = doc_pages(&dir.path().join(format!("demo{i}.kdraft")));
        assert_eq!(pdf_page_count(&std::fs::read(&pdf).unwrap()), n, "{b}");
    }
}

#[test]
fn cmd_list_text_and_json() {
    let (code, out, _) = run(&["cmd", "list"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("layout.align") && out.contains("page.move"),
        "{out}"
    );
    let (code, out, _) = run(&["cmd", "list", "--json"]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let align = v
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "layout.align")
        .unwrap();
    assert_eq!(align["kind"], "edit");
    assert!(align["args"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["name"] == "mode" && a["type"]["kind"] == "enum"));
}

#[test]
fn cmd_run_edits_and_saves() {
    let dir = tempfile::tempdir().unwrap();
    let doc = dir.path().join("d.kdraft");
    let (code, _, err) = run(&[
        "doc".as_ref(),
        "new".as_ref(),
        k9().as_os_str(),
        "-o".as_ref(),
        doc.as_os_str(),
    ]);
    assert_eq!(code, 0, "{err}");
    let before = std::fs::read_to_string(&doc).unwrap();
    let out_doc = dir.path().join("e.kdraft");
    let (code, out, err) = run(&[
        "cmd".as_ref(),
        "run".as_ref(),
        doc.as_os_str(),
        "--do".as_ref(),
        "page.move".as_ref(),
        "--args".as_ref(),
        r#"{"from": 0, "to": 2}"#.as_ref(),
        "--do".as_ref(),
        "page.add".as_ref(),
        "--do=page.rename".as_ref(),
        r#"--args={"title": "Notes"}"#.as_ref(),
        "--do".as_ref(),
        "item.add_text".as_ref(),
        "--args".as_ref(),
        r#"{"text": "NOTE 1", "at": [20, 30]}"#.as_ref(),
        "--do".as_ref(),
        "layout.align".as_ref(),
        "--args".as_ref(),
        r#"{"mode": "left"}"#.as_ref(),
        "-o".as_ref(),
        out_doc.as_os_str(),
    ]);
    assert_eq!(code, 0, "{out}\n{err}");
    assert_eq!(
        out.lines().filter(|l| l.starts_with('{')).count(),
        5,
        "{out}"
    );
    // The input is untouched; the output has the edits.
    assert_eq!(std::fs::read_to_string(&doc).unwrap(), before);
    let text = std::fs::read_to_string(&out_doc).unwrap();
    let d = kdraft_doc::Document::parse(&text).unwrap();
    let n0 = kdraft_doc::Document::parse(&before).unwrap().pages.len();
    assert_eq!(d.pages.len(), n0 + 1);
    assert_eq!(d.pages[3].title.as_deref(), Some("Notes"));
    assert_eq!(d.pages[2].title.as_deref(), Some("Fabrication"));
    assert!(text.contains("NOTE 1"));
    // Errors exit 2 and name the command.
    let (code, _, err) = run(&[
        "cmd".as_ref(),
        "run".as_ref(),
        doc.as_os_str(),
        "--do".as_ref(),
        "layout.align".as_ref(),
        "--args".as_ref(),
        r#"{"mode": "sideways"}"#.as_ref(),
    ]);
    assert_eq!(code, 2);
    assert!(err.contains("layout.align"), "{err}");
}

#[test]
fn export_png_pages() {
    let dir = tempfile::tempdir().unwrap();
    let doc = dir.path().join("d.kdraft");
    let (code, _, err) = run(&[
        "doc".as_ref(),
        "new".as_ref(),
        k9().as_os_str(),
        "-o".as_ref(),
        doc.as_os_str(),
    ]);
    assert_eq!(code, 0, "{err}");
    let out = dir.path().join("d.png");
    let (code, stdout, err) = run(&[
        "export".as_ref(),
        doc.as_os_str(),
        "-o".as_ref(),
        out.as_os_str(),
        "--format".as_ref(),
        "png".as_ref(),
        "--dpi".as_ref(),
        "50".as_ref(),
        "--no-kicad-cli".as_ref(),
    ]);
    assert_eq!(code, 0, "{stdout}{err}");
    for i in 1..=3 {
        let f = dir.path().join(format!("d-p{i}.png"));
        let bytes = std::fs::read(&f).unwrap();
        assert!(bytes.starts_with(b"\x89PNG"));
        // A3 at 50 dpi: 827 x 585 px (IHDR width/height big-endian).
        let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        assert_eq!((w, h), (827, 585));
    }
}

#[test]
fn check_exit_codes_and_json() {
    let dir = tempfile::tempdir().unwrap();
    let doc = dir.path().join("d.kdraft");
    let (code, _, err) = run(&[
        "doc".as_ref(),
        "new".as_ref(),
        k9().as_os_str(),
        "-o".as_ref(),
        doc.as_os_str(),
    ]);
    assert_eq!(code, 0, "{err}");
    let (code, out, _) = run(&["check".as_ref(), doc.as_os_str()]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("0 error(s), 0 warning(s)"), "{out}");
    let (_, out, _) = run(&["check".as_ref(), doc.as_os_str(), "--json".as_ref()]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v["findings"].is_array());
    // Break a dimension anchor: errors, exit 2.
    let text = std::fs::read_to_string(&doc).unwrap();
    let broken = text.replacen(
        "(to\n        (outline-vertex 1)\n      )",
        "(to (outline-vertex 77))",
        1,
    );
    assert_ne!(broken, text);
    std::fs::write(&doc, broken).unwrap();
    let (code, out, _) = run(&["check".as_ref(), doc.as_os_str()]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("[broken-anchor]"), "{out}");
}

#[test]
fn doc_new_with_layer_pages() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("l.kdraft");
    let (code, stdout, err) = run(&[
        "doc".as_ref(),
        "new".as_ref(),
        k9().as_os_str(),
        "-o".as_ref(),
        out.as_os_str(),
        "--no-kicad-cli".as_ref(),
        "--layer-pages".as_ref(),
        "copper".as_ref(),
    ]);
    assert_eq!(code, 0, "{stdout}{err}");
    assert!(
        stdout.contains(
            "2 pages (F.Cu, B.Cu) as pages 2-3; views linked to Fabrication view on page 1"
        ),
        "{stdout}"
    );
    let text = std::fs::read_to_string(&out).unwrap();
    assert_eq!(text.matches("(link").count(), 2);
    assert!(text.contains("(title \"F.Cu\")") && text.contains("(title \"B.Cu\")"));
}

#[test]
fn settings_commands_and_document_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let set = dir.path().join("cfg").join("settings.json");
    let with = |args: &[&str]| {
        let mut a: Vec<std::ffi::OsString> = vec!["--settings".into(), set.clone().into()];
        a.extend(args.iter().map(std::ffi::OsString::from));
        run(&a)
    };
    let (_, out, _) = with(&["settings", "path"]);
    assert_eq!(out.trim(), set.display().to_string());
    // Unknown keys in the file survive the CLI's writes.
    std::fs::create_dir_all(set.parent().unwrap()).unwrap();
    std::fs::write(&set, r#"{"ui": {"theme": "dark"}, "future": 1}"#).unwrap();
    for (k, v) in [
        ("documents.paper", "A4"),
        ("documents.portrait", "yes"),
        ("documents.snap_grid", "1"),
        ("documents.callout_numbering", "document"),
        ("kicad_cli", "auto"),
    ] {
        let (code, out, err) = with(&["settings", "set", k, v]);
        assert_eq!(code, 0, "{k}: {out}{err}");
    }
    let (code, _, err) = with(&["settings", "set", "documents.paper", "Q7"]);
    assert_ne!(code, 0);
    assert!(err.contains("not a valid value"), "{err}");
    // A KiCad theme as a palette, then the default for new documents.
    let theme = fixture("kicad-themes/partial.json");
    let (code, out, err) = with(&[
        "settings",
        "palettes",
        "import-kicad",
        theme.to_str().unwrap(),
        "--name",
        "Shop",
    ]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("imported palette \"Shop\""), "{out}");
    let (code, _, _) = with(&["settings", "set", "documents.layer_palette", "Shop"]);
    assert_eq!(code, 0);
    let (code, _, err) = with(&["settings", "set", "documents.layer_palette", "Nope"]);
    assert_ne!(code, 0);
    assert!(err.contains("no palette"), "{err}");
    let (_, out, _) = with(&["settings", "palettes", "list"]);
    assert!(out.contains("Shop  [95 colours] (default)"), "{out}");
    assert!(out.contains("KiCad Default  [built-in]"), "{out}");
    let (_, out, _) = with(&["settings", "show"]);
    assert!(out.contains("documents.paper = A4"), "{out}");
    let text = std::fs::read_to_string(&set).unwrap();
    assert!(
        text.contains("\"future\": 1") && text.contains("\"theme\": \"dark\""),
        "{text}"
    );
    // `doc new` takes the defaults.
    let doc = dir.path().join("d.kdraft");
    let (code, out, err) = with(&[
        "doc",
        "new",
        k9().to_str().unwrap(),
        "-o",
        doc.to_str().unwrap(),
        "--no-kicad-cli",
    ]);
    assert_eq!(code, 0, "{out}{err}");
    let d = std::fs::read_to_string(&doc).unwrap();
    assert!(d.contains("(size A4 210 297)"), "{d}");
    assert!(d.contains("(snap_grid 1)"), "{d}");
    assert!(d.contains("(callout_numbering document)"), "{d}");
    assert!(
        d.contains("(layer_palette") && d.contains("(name \"Shop\")"),
        "{d}"
    );
    // kicad list works without any install.
    let (code, out, _) = with(&["kicad", "list"]);
    assert_eq!(code, 0);
    assert!(out.contains("in use:"), "{out}");
    // Reset removes the file.
    let (code, _, _) = with(&["settings", "reset"]);
    assert_eq!(code, 0);
    assert!(!set.exists());
}

#[test]
fn malformed_settings_are_kept_and_reported() {
    let dir = tempfile::tempdir().unwrap();
    let set = dir.path().join("settings.json");
    std::fs::write(&set, "{ broken").unwrap();
    let (code, out, err) = run(&[
        "--settings".as_ref(),
        set.as_os_str(),
        "settings".as_ref(),
        "show".as_ref(),
    ]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(err.contains("settings.json.bad"), "{err}");
    assert!(out.contains("kicad_cli = auto"), "{out}");
    assert!(dir.path().join("settings.json.bad").exists());
}
