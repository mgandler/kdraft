use std::path::{Path, PathBuf};

use kdraft_board::geom::Point;
use kdraft_board::TitleBlock;

use super::*;

fn a3() -> PageSpec {
    PageSpec {
        width: 420.0,
        height: 297.0,
        page_number: 1,
    }
}

fn vars() -> TextVars {
    let tb = TitleBlock {
        title: Some("Board".into()),
        rev: Some("B".into()),
        ..Default::default()
    };
    TextVars::from_title_block(
        &tb,
        &PageInfo {
            page_number: 1,
            page_count: 3,
            paper: "A3".into(),
            ..Default::default()
        },
    )
}

#[test]
fn default_sheet_resolves_on_a3() {
    let w = default_sheet().unwrap();
    assert_eq!(w.version, 20210606);
    assert!(w.warnings.is_empty(), "{:?}", w.warnings);
    assert_eq!(w.setup.left_margin, 10.0);
    let r = w.resolve(&a3(), &vars());
    // Two frame rectangles (margin box and 2 mm inset) plus the title block.
    assert_eq!(r.rects.len(), 3);
    let f = r.frame();
    assert!(f.min.approx_eq(Point::new(12.0, 12.0), 1e-9), "{f:?}");
    assert!(f.max.approx_eq(Point::new(408.0, 285.0), 1e-9), "{f:?}");
    let tb = r.title_block().unwrap();
    assert!(tb.min.approx_eq(Point::new(300.0, 253.0), 1e-9), "{tb:?}");
    assert!(tb.max.approx_eq(Point::new(408.0, 285.0), 1e-9), "{tb:?}");
    let area = r.drawing_area(0.0);
    assert!(
        area.max.approx_eq(Point::new(408.0, 253.0), 1e-9),
        "{area:?}"
    );
    // Column labels 1..8 top and bottom, row labels A..F left and right.
    let labels: Vec<&str> = r.texts.iter().map(|t| t.text.as_str()).collect();
    assert_eq!(labels.iter().filter(|t| **t == "8").count(), 2);
    assert_eq!(labels.iter().filter(|t| **t == "9").count(), 0);
    assert_eq!(labels.iter().filter(|t| **t == "F").count(), 2);
    assert_eq!(labels.iter().filter(|t| **t == "G").count(), 0);
    assert!(labels.contains(&"Title: Board"));
    assert!(labels.contains(&"Rev: B"));
    assert!(labels.contains(&"Id: 1/3"));
    assert!(labels.contains(&"Size: A3"));
    assert!(labels.contains(&"${KICAD_VERSION}"));
    let title = r.texts.iter().find(|t| t.text == "Title: Board").unwrap();
    assert!(title.bold && title.italic && title.size == (2.0, 2.0));
    // pos 109 10.7 from the bottom-right margin corner (410, 287).
    assert!(title.at.approx_eq(Point::new(301.0, 276.3), 1e-9));
    let a = r.texts.iter().find(|t| t.text == "A").unwrap();
    assert_eq!(a.h_justify, HJustify::Center);
}

#[test]
fn corners_options_and_repeat() {
    let w = WksFile::parse(
        r#"(kicad_wks (version 20231118)
  (setup (textsize 2 2) (left_margin 5) (right_margin 5) (top_margin 5) (bottom_margin 5))
  (line (start 1 1 ltcorner) (end 1 1 rtcorner) (linewidth 0.3))
  (tbtext "P1" (pos 0 0 lbcorner) (option page1only))
  (tbtext "Other" (pos 0 0) (option notonpage1) (justify right bottom) (rotate 90) (maxlen 20))
  (tbtext "1" (pos 10 0 ltcorner) (repeat 50) (incrx 30) (incrlabel 2))
  (polygon (pos 10 10 ltcorner) (rotate 90) (pts (xy 0 0) (xy 1 0) (xy 1 1)))
  (unknown_item (foo 1))
  (rect (start x y))
)"#,
    )
    .unwrap();
    assert_eq!(w.warnings.len(), 1, "{:?}", w.warnings);
    let page = |n| PageSpec {
        width: 100.0,
        height: 50.0,
        page_number: n,
    };
    let r1 = w.resolve(&page(1), &TextVars::new());
    let l = &r1.lines[0];
    assert!(l.a.approx_eq(Point::new(6.0, 6.0), 1e-12));
    assert!(l.b.approx_eq(Point::new(94.0, 6.0), 1e-12));
    assert_eq!(l.width, 0.3);
    let texts: Vec<&str> = r1.texts.iter().map(|t| t.text.as_str()).collect();
    assert_eq!(texts, ["P1", "1", "3", "5"]);
    assert!(r1.texts[0].at.approx_eq(Point::new(5.0, 45.0), 1e-12));
    let r2 = w.resolve(&page(2), &TextVars::new());
    let o = r2.texts.iter().find(|t| t.text == "Other").unwrap();
    assert_eq!(
        (o.h_justify, o.v_justify),
        (HJustify::Right, VJustify::Bottom)
    );
    assert_eq!(o.rotation, 90.0);
    assert_eq!(o.max_len, Some(20.0));
    assert_eq!(o.size, (2.0, 2.0));
    // Polygon rotated 90 degrees counter-clockwise on screen: +X goes up.
    let p = &r1.polygons[0].points;
    assert!(p[1].approx_eq(Point::new(15.0, 14.0), 1e-12), "{p:?}");
}

#[test]
fn legacy_page_layout_uses_percent_codes() {
    let w = WksFile::parse(
        "(page_layout (setup (textsize 1.5 1.5)) (tbtext \"Id: %S/%N\" (pos 1 1)) (tbtext %K (pos 2 2)))",
    )
    .unwrap();
    assert!(w.legacy_root);
    let texts: Vec<String> = w
        .items
        .iter()
        .filter_map(|i| match &i.kind {
            ItemKind::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["Id: ${#}/${##}", "${KICAD_VERSION}"]);
    assert!(matches!(
        WksFile::parse("(kicad_pcb)"),
        Err(SheetError::NotASheet { .. })
    ));
}

#[test]
fn labels_increment() {
    assert_eq!(increment_label("1", 3), "4");
    assert_eq!(increment_label("A", 2), "C");
    assert_eq!(increment_label("z", 1), "a");
    assert_eq!(increment_label("Rev", 1), "Rev");
}

#[test]
fn project_sheet_selection_order() {
    let dir = tempfile::tempdir().unwrap();
    let pro = dir.path().join("b.kicad_pro");
    std::fs::write(
        &pro,
        r#"{"pcbnew": {"page_layout_descr_file": "${KIPRJMOD}/sheets/co.kicad_wks"},
            "schematic": {"page_layout_descr_file": "sch.kicad_wks"},
            "text_variables": {"PART": "X1", "N": 3}}"#,
    )
    .unwrap();
    let p = Project::load(&pro).unwrap();
    assert_eq!(p.text_variables.len(), 1);
    assert_eq!(p.schematic_sheet.as_deref(), Some("sch.kicad_wks"));
    let doc_dir = Path::new("docs");
    let proj = Some((&p, dir.path()));
    let (c, o) = select_sheet(Some("page.kicad_wks"), Some("doc.kicad_wks"), doc_dir, proj);
    assert_eq!(
        (c, o),
        (
            SheetChoice::File(doc_dir.join("page.kicad_wks")),
            SheetOrigin::Page
        )
    );
    let (c, o) = select_sheet(None, Some("default"), doc_dir, proj);
    assert_eq!((c, o), (SheetChoice::Default, SheetOrigin::Document));
    let (c, o) = select_sheet(None, None, doc_dir, proj);
    assert_eq!(o, SheetOrigin::Project);
    assert_eq!(
        c,
        SheetChoice::File(dir.path().join("sheets").join("co.kicad_wks"))
    );
    assert_eq!(
        select_sheet(None, None, doc_dir, None).1,
        SheetOrigin::Default
    );
    // Relative project paths with backslashes.
    let r = resolve_path("sub\\x.kicad_wks", Path::new("proj"));
    assert_eq!(r, Path::new("proj").join("sub").join("x.kicad_wks"));
    assert_eq!(
        Project::for_board(&dir.path().join("b.kicad_pcb")),
        Some(pro)
    );
    assert!(Project::parse("not json", Path::new("x")).is_err());
}

fn ext_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures-ext")
}

/// Every `.kicad_wks` and `.kicad_pro` fetched by scripts/fetch-fixtures.sh
/// loads; skipped when fixtures-ext/ is absent.
#[test]
fn ext_sheets_and_projects_load() {
    let mut wks = 0;
    let mut pro = 0;
    for major in ["9", "10"] {
        let Ok(rd) = std::fs::read_dir(ext_dir().join(major)) else {
            continue;
        };
        let mut files: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        files.sort();
        for f in files {
            match f.extension().and_then(|e| e.to_str()) {
                Some("kicad_wks") => {
                    let w = load(&SheetChoice::File(f.clone())).unwrap();
                    assert!(w.warnings.is_empty(), "{}: {:?}", f.display(), w.warnings);
                    let r = w.resolve(&a3(), &vars());
                    assert!(
                        !r.lines.is_empty() || !r.rects.is_empty(),
                        "{}",
                        f.display()
                    );
                    let area = r.drawing_area(5.0);
                    assert!(
                        area.width() > 200.0 && area.height() > 150.0,
                        "{}: {area:?}",
                        f.display()
                    );
                    eprintln!(
                        "{}: version {}, {} items, {} texts, {} polygons, {} bitmaps, area {:.1} x {:.1}",
                        f.display(),
                        w.version,
                        w.items.len(),
                        r.texts.len(),
                        r.polygons.len(),
                        r.bitmaps.len(),
                        area.width(),
                        area.height()
                    );
                    wks += 1;
                }
                Some("kicad_pro") => {
                    Project::load(&f).unwrap();
                    pro += 1;
                }
                _ => {}
            }
        }
    }
    eprintln!("{wks} sheets, {pro} projects");
}
