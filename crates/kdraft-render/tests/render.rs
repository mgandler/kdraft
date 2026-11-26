//! Golden SVG and PDF smoke tests on the fixture boards.
//!
//! Regenerate goldens with `KDRAFT_UPDATE_GOLDEN=1 cargo test -p kdraft-render`.

use std::path::{Path, PathBuf};

use kdraft_board::Board;
use kdraft_doc::Document;
use kdraft_render::dl::DocList;
use kdraft_render::{render_document, Fonts, PageSheet, SheetContext};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn render(board_file: &str) -> (DocList, Vec<String>) {
    let path = root().join("fixtures").join(board_file);
    let board = Board::load(&path).unwrap();
    let sheet = kdraft_sheet::default_sheet().unwrap();
    let doc = Document::new_for_board(&board, board_file, &sheet);
    let fonts = Fonts::load().unwrap();
    let ctx = SheetContext {
        file_name: Some(board_file.to_string()),
        kicad_version: Some("kdraft".to_string()),
        ..Default::default()
    };
    render_document(
        &doc,
        &board,
        &ctx,
        &|_, _| PageSheet {
            wks: &sheet,
            svg: None,
        },
        &|_| None,
        &fonts,
    )
}

fn golden(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(name);
    if std::env::var_os("KDRAFT_UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "missing golden {}; run with KDRAFT_UPDATE_GOLDEN=1",
            path.display()
        )
    });
    if expected != actual {
        let line = expected
            .lines()
            .zip(actual.lines())
            .position(|(a, b)| a != b)
            .map_or(expected.lines().count().min(actual.lines().count()), |i| i);
        panic!(
            "{name} differs from golden at line {}:\n  golden: {:?}\n  actual: {:?}\n(KDRAFT_UPDATE_GOLDEN=1 to accept)",
            line + 1,
            expected.lines().nth(line),
            actual.lines().nth(line)
        );
    }
}

#[test]
fn golden_svg_pages() {
    let fonts = Fonts::load().unwrap();
    for (board, stem) in [
        ("kicad9-minimal.kicad_pcb", "kicad9-minimal"),
        ("kicad10-minimal.kicad_pcb", "kicad10-minimal"),
    ] {
        let (doc, warnings) = render(board);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(doc.pages.len() >= 3);
        for (i, p) in doc.pages.iter().enumerate() {
            let svg = kdraft_render::svg_out::page_svg(p, &fonts);
            // Byte-identical on repeat.
            assert_eq!(
                svg,
                kdraft_render::svg_out::page_svg(&render(board).0.pages[i], &fonts)
            );
            golden(&format!("{stem}-p{}.svg", i + 1), &svg);
        }
    }
}

#[test]
fn pages_contain_sheet_views_and_tables() {
    let (doc, _) = render("kicad9-minimal.kicad_pcb");
    let di = doc
        .pages
        .iter()
        .position(|p| p.title == "Drill and stackup")
        .unwrap();
    let id = format!("Id: {}/{}", di + 1, doc.pages.len());
    let texts: Vec<String> = doc.pages[di]
        .items
        .iter()
        .flat_map(|i| kdraft_render::dl::texts(std::slice::from_ref(i)))
        .map(|t| t.text.clone())
        .collect();
    for needle in [
        "DRILL TABLE",
        "LAYER STACK",
        "Title: kdraft test board",
        id.as_str(),
        "File: kicad9-minimal.kicad_pcb",
        "DRILL DRAWING   SCALE 5:1",
    ] {
        assert!(
            texts.iter().any(|t| t == needle),
            "missing {needle:?} in {texts:?}"
        );
    }
    let p1: Vec<String> = kdraft_render::dl::texts(&doc.pages[1].items)
        .iter()
        .map(|t| t.text.clone())
        .collect();
    // The assembly view is native by choice: no fallback note.
    assert!(!p1.iter().any(|t| t == kdraft_render::NATIVE_NOTE));
    assert!(p1.iter().any(|t| t == "R1"));
    // The fabrication view wanted a plot and fell back.
    let p0: Vec<String> = kdraft_render::dl::texts(&doc.pages[0].items)
        .iter()
        .map(|t| t.text.clone())
        .collect();
    assert!(p0.iter().any(|t| t == kdraft_render::NATIVE_NOTE));
    assert!(p1.iter().any(|t| t == "ASSEMBLY TOP   SCALE 5:1"));
}

/// Light PDF check without a PDF library: header, page objects, media boxes.
fn pdf_pages(bytes: &[u8]) -> Vec<(f64, f64)> {
    let text = String::from_utf8_lossy(bytes);
    let mut sizes = Vec::new();
    for (i, _) in text.match_indices("/MediaBox[") {
        let rest = &text[i + "/MediaBox[".len()..];
        let end = rest.find(']').unwrap();
        let v: Vec<f64> = rest[..end]
            .split_whitespace()
            .map(|n| n.parse().unwrap())
            .collect();
        sizes.push((v[2] - v[0], v[3] - v[1]));
    }
    sizes
}

#[test]
fn pdf_smoke_and_determinism() {
    let fonts = Fonts::load().unwrap();
    let (doc, _) = render("kicad10-minimal.kicad_pcb");
    let a = kdraft_render::pdf::write_pdf(&doc, &fonts).unwrap();
    let b = kdraft_render::pdf::write_pdf(&render("kicad10-minimal.kicad_pcb").0, &fonts).unwrap();
    assert!(a.starts_with(b"%PDF-"));
    assert_eq!(a, b, "PDF output is not deterministic");
    let sizes = pdf_pages(&a);
    let n = doc.pages.len();
    assert_eq!(sizes.len(), n, "{sizes:?}");
    for (w, h) in sizes {
        assert!((w - 420.0 * 72.0 / 25.4).abs() < 0.01, "{w}");
        assert!((h - 297.0 * 72.0 / 25.4).abs() < 0.01, "{h}");
    }
    let s = String::from_utf8_lossy(&a);
    assert!(s.contains("/Outlines"));
    assert!(
        s.contains("FontFile2") || s.contains("FontFile3"),
        "font not embedded"
    );
    assert!(s.contains(&format!("/Type/Pages/Count {n}")));
    assert_eq!(s.matches("/Type/Page/").count(), n);
    assert!(s.contains("/Title(Drill and stackup)"));
}

#[test]
fn plotted_view_clip_includes_content_past_the_outline() {
    use kdraft_render::dl::Item;
    use kdraft_render::PlotContent;
    let path = root().join("fixtures/kicad9-minimal.kicad_pcb");
    let board = Board::load(&path).unwrap();
    let ob = board.outline.bbox.unwrap();
    let sheet = kdraft_sheet::default_sheet().unwrap();
    let mut doc = Document::new_for_board(&board, "b.kicad_pcb", &sheet);
    doc.pages.truncate(1);
    // A fab-text-like stroke 4 mm left of the outline, in board mm.
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"297mm\" height=\"210mm\" viewBox=\"0 0 297 210\">\
         <path d=\"M {} {} L {} {}\" stroke=\"#000\" stroke-width=\"0.1\" fill=\"none\"/></svg>",
        ob.min.x - 4.0,
        ob.center().y,
        ob.min.x + 2.0,
        ob.center().y
    );
    let plot = kdraft_plot::svg::parse(&svg).unwrap();
    let fit = kdraft_plot::fit::PlotTransform {
        sx: 1.0,
        ox: 0.0,
        sy: 1.0,
        oy: 0.0,
    };
    let fonts = Fonts::load().unwrap();
    let (list, _) = render_document(
        &doc,
        &board,
        &SheetContext::default(),
        &|_, _| PageSheet {
            wks: &sheet,
            svg: None,
        },
        &|_| {
            Some(PlotContent {
                layers: vec![plot.clone()],
                fit,
                image: None,
            })
        },
        &fonts,
    );
    let view = doc.pages[0].views().next().unwrap().clone();
    let map = kdraft_render::view::ViewMap::new(&view, ob);
    let left_sheet = map
        .point(kdraft_board::geom::Point::new(ob.min.x - 4.0, 0.0))
        .x;
    // The view's content group is the last group on the page.
    let clip = list.pages[0]
        .items
        .iter()
        .rev()
        .find_map(|i| match i {
            Item::Group(g) => g.clip,
            _ => None,
        })
        .unwrap();
    assert!(
        clip.min.x < left_sheet,
        "clip {clip:?} vs content {left_sheet}"
    );
    assert!(clip.min.x < map.frame.min.x);
    // Without content past the outline the clip is the frame.
    assert!((clip.max.x - map.frame.max.x).abs() < 1e-9);
}

/// Golden PNGs with a tolerance: anti-aliasing may differ by a few levels
/// between platforms, layout changes may not.
fn golden_png(name: &str, actual: &tiny_skia::Pixmap) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(name);
    if std::env::var_os("KDRAFT_UPDATE_GOLDEN").is_some() {
        actual.save_png(&path).unwrap();
        return;
    }
    let expected = tiny_skia::Pixmap::load_png(&path).unwrap_or_else(|_| {
        panic!(
            "missing golden {}; run with KDRAFT_UPDATE_GOLDEN=1",
            path.display()
        )
    });
    assert_eq!(
        (expected.width(), expected.height()),
        (actual.width(), actual.height()),
        "{name}: size"
    );
    let (mut sum, mut big) = (0u64, 0usize);
    for (a, b) in expected.data().iter().zip(actual.data()) {
        let d = a.abs_diff(*b);
        sum += u64::from(d);
        if d > 48 {
            big += 1;
        }
    }
    let n = expected.data().len();
    let mean = sum as f64 / n as f64;
    let frac = big as f64 / n as f64;
    assert!(
        mean < 0.5 && frac < 0.002,
        "{name} differs from golden: mean channel diff {mean:.3}, {:.3}% channels off by > 48 (KDRAFT_UPDATE_GOLDEN=1 to accept)",
        frac * 100.0
    );
}

#[test]
fn golden_png_pages() {
    let fonts = Fonts::load().unwrap();
    let (doc, _) = render("kicad9-minimal.kicad_pcb");
    for (i, p) in doc.pages.iter().enumerate() {
        let pix = kdraft_render::raster::page_pixmap(p, 3.0, &fonts).unwrap();
        assert_eq!((pix.width(), pix.height()), (1260, 891));
        golden_png(&format!("kicad9-minimal-p{}.png", i + 1), &pix);
    }
}

#[test]
fn raster_single_view_into_its_frame() {
    // A view rendered alone into its frame region (how the editor canvas
    // draws view content).
    use kdraft_render::raster::{render, Target};
    let fonts = Fonts::load().unwrap();
    let path = root().join("fixtures/kicad9-minimal.kicad_pcb");
    let board = Board::load(&path).unwrap();
    let sheet = kdraft_sheet::default_sheet().unwrap();
    let doc = Document::new_for_board(&board, "b", &sheet);
    let item = &doc.pages[1].items[0];
    let (items, w) = kdraft_render::page::render_item(
        &doc,
        item,
        &board,
        &SheetContext::default(),
        &|_| None,
        &fonts,
    );
    assert!(w.is_empty());
    let kdraft_doc::Item::View(v) = item else {
        panic!()
    };
    let map = kdraft_render::view::ViewMap::new(v, board.outline.bbox.unwrap());
    let region = map.frame;
    let t = Target {
        region,
        px_per_mm: 4.0,
        background: Some(kdraft_render::dl::Color::WHITE),
    };
    let alone = render(&items, &t, &fonts).unwrap();
    let inked = alone.pixels().iter().filter(|c| c.red() < 128).count();
    assert!(inked > 1000, "{inked}");
}
