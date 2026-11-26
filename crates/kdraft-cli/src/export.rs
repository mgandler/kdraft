//! `kdraft export`: document and board to PDF, SVG or PNG pages through
//! the engine's render pipeline.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use kdraft_engine::pipeline::{self, ExportOptions, Kicad};
use kdraft_engine::Session;
use kdraft_source::{CommandRunner, KicadCli};

pub use kdraft_engine::pipeline::Format;

pub struct Options {
    pub doc: PathBuf,
    pub out: PathBuf,
    pub format: Format,
    /// kicad-cli and the runner to use; `None` renders natively.
    pub kicad: Option<(KicadCli, Arc<dyn CommandRunner>)>,
    /// Draw drawing sheets with kicad-cli from a stub board.
    pub sheet_from_kicad: bool,
    /// Cache folder for kicad-cli outputs.
    pub cache: PathBuf,
}

pub struct Outcome {
    pub files: Vec<PathBuf>,
    pub pages: usize,
    pub notes: Vec<String>,
}

pub fn run(opts: &Options) -> Result<Outcome> {
    let mut s =
        Session::open(&opts.doc).with_context(|| format!("opening {}", opts.doc.display()))?;
    if let Some(e) = &s.board_state().error {
        anyhow::bail!("loading board: {e}");
    }
    let r = pipeline::export(
        &mut s,
        &ExportOptions {
            out: opts.out.clone(),
            format: opts.format,
            kicad: opts.kicad.as_ref().map(|(cli, runner)| Kicad {
                cli: cli.clone(),
                runner: Arc::clone(runner),
            }),
            sheet_from_kicad: opts.sheet_from_kicad,
            cache: opts.cache.clone(),
        },
    )?;
    Ok(Outcome {
        files: r.files,
        pages: r.pages,
        notes: r.notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kdraft_board::Board;
    use kdraft_doc::Document;
    use kdraft_plot::fake::{fake_kicad, FakeOptions};
    use kdraft_source::Version;

    fn setup() -> (tempfile::TempDir, PathBuf, Board) {
        let dir = tempfile::tempdir().unwrap();
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/kicad9-minimal.kicad_pcb");
        let board_path = dir.path().join("b.kicad_pcb");
        std::fs::copy(&src, &board_path).unwrap();
        let board = Board::load(&board_path).unwrap();
        let sheet = kdraft_sheet::default_sheet().unwrap();
        let doc = Document::new_for_board(&board, "b.kicad_pcb", &sheet);
        let docp = dir.path().join("b.kdraft");
        std::fs::write(&docp, doc.to_text().unwrap()).unwrap();
        (dir, docp, board)
    }

    #[test]
    fn export_with_fake_kicad_uses_fitted_plots() {
        let (dir, docp, board) = setup();
        let runner = Arc::new(fake_kicad(board, FakeOptions::default()));
        let cli = KicadCli {
            path: "kicad-cli".into(),
            version: Version::new(9, 0, 6),
            version_string: "9.0.6".into(),
            vars: Vec::new(),
        };
        let out = dir.path().join("o.pdf");
        let r = run(&Options {
            doc: docp.clone(),
            out: out.clone(),
            format: Format::Pdf,
            kicad: Some((cli.clone(), runner.clone() as Arc<dyn CommandRunner>)),
            sheet_from_kicad: true,
            cache: dir.path().join("cache"),
        })
        .unwrap();
        let n = r.pages;
        assert!(n >= 3);
        assert!(
            r.notes
                .iter()
                .any(|n| n.contains("plot transform fitted on pads")),
            "{:?}",
            r.notes
        );
        assert!(std::fs::read(&out).unwrap().starts_with(b"%PDF"));
        // The fake's sheet is a bare frame, which does not match the default
        // sheet's geometry: rendering falls back to the native sheet.
        assert!(
            r.notes.iter().any(|n| n.contains("drawn natively")),
            "{:?}",
            r.notes
        );
        let svg_jobs = runner
            .calls()
            .iter()
            .filter(|c| c.args.get(2).map(String::as_str) == Some("svg"))
            .count();
        assert!(svg_jobs >= 5, "{svg_jobs}");
        // Per-view drill marks: actual for the fabrication view; the
        // assembly view is native by default and is not plotted.
        let opts: Vec<String> = runner
            .calls()
            .iter()
            .filter_map(|c| {
                let i = c.args.iter().position(|a| a == "--drill-shape-opt")?;
                c.args.get(i + 1).cloned()
            })
            .collect();
        assert!(
            !opts.iter().any(|o| o == "0") && opts.iter().any(|o| o == "2"),
            "{opts:?}"
        );
        // Plotted views carry no native-preview note.
        let svg_out = dir.path().join("s.svg");
        let r = run(&Options {
            doc: docp,
            out: svg_out,
            format: Format::Svg,
            kicad: Some((cli, runner as Arc<dyn CommandRunner>)),
            sheet_from_kicad: false,
            cache: dir.path().join("cache"),
        })
        .unwrap();
        let p1 = std::fs::read_to_string(&r.files[0]).unwrap();
        assert!(!p1.contains("NATIVE PREVIEW"));
        // Native by choice: no note either.
        let p2 = std::fs::read_to_string(&r.files[1]).unwrap();
        assert!(!p2.contains("NATIVE PREVIEW"));
        let all: Vec<String> = r
            .files
            .iter()
            .map(|f| std::fs::read_to_string(f).unwrap())
            .collect();
        assert!(all.iter().any(|p| p.contains("DRILL TABLE")));
        // No schematic next to the board: the BOM comes from the PCB.
        assert!(all
            .iter()
            .any(|p| p.contains("BILL OF MATERIALS (FROM PCB)")));
    }

    #[test]
    fn export_without_kicad_is_native() {
        let (dir, docp, _) = setup();
        let out = dir.path().join("o.pdf");
        let r = run(&Options {
            doc: docp,
            out,
            format: Format::Svg,
            kicad: None,
            sheet_from_kicad: false,
            cache: dir.path().join("cache"),
        })
        .unwrap();
        assert_eq!(r.files.len(), r.pages);
        let p1 = std::fs::read_to_string(&r.files[0]).unwrap();
        assert!(p1.contains("NATIVE PREVIEW"));
    }
}
