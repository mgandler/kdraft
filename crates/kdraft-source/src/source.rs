//! Board sources: where the board model and plotted geometry come from.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use kdraft_board::{hide_fab_texts, Board, BoardHash, FabTextShow};

use crate::cache::{Cache, CacheKey};
use crate::error::SourceError;
use crate::jobs::{Job, SvgExport};
use crate::kicad_cli::KicadCli;
use crate::private::{private_copy, private_copy_edited};
use crate::runner::CommandRunner;

pub trait BoardSource {
    /// Hash of the board's current content.
    fn board_hash(&self) -> Result<BoardHash, SourceError>;
    /// The board model for the current content.
    fn board(&self) -> Result<Arc<Board>, SourceError>;
    /// Path to an SVG of the requested layers for the current content,
    /// generated on demand and cached.
    fn layer_svg(&self, request: &SvgExport) -> Result<PathBuf, SourceError>;
    /// As `layer_svg`, plotted from a copy with the fab-layer footprint
    /// texts that `show` turns off hidden.
    fn layer_svg_fab(&self, request: &SvgExport, show: FabTextShow)
        -> Result<PathBuf, SourceError>;
}

/// Runs `job` unless its output is cached; returns the output path (file
/// for file jobs, entry directory for directory jobs).
pub fn run_cached(
    job: &Job,
    input_hash: &[u8; 32],
    cli: &KicadCli,
    runner: &dyn CommandRunner,
    cache: &Cache,
) -> Result<PathBuf, SourceError> {
    let args = job.cache_args();
    let key = CacheKey::new(input_hash, &cli.version_string, &args);
    let out_path = |dir: &Path| match job.output_file_name() {
        Some(name) => dir.join(name),
        None => dir.to_path_buf(),
    };
    if let Some(dir) = cache.get(&key) {
        return Ok(out_path(&dir));
    }
    let description = format!("kicad-cli {}\n{}\n", cli.version_string, args.join(" "));
    let dir = cache.put(&key, &description, |tmp| {
        let target = out_path(tmp);
        let argv = job.args(&target);
        let out = runner.run(&cli.path, &argv)?;
        if !out.success() {
            return Err(SourceError::CommandFailed {
                program: cli.path.clone(),
                args: argv
                    .iter()
                    .map(|a| a.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join(" "),
                status: out.status_text(),
                stderr: out.stderr_text().trim().to_string(),
            });
        }
        if job.output_file_name().is_some() && !target.is_file() {
            return Err(SourceError::MissingOutput(target));
        }
        Ok(())
    })?;
    Ok(out_path(&dir))
}

/// Runs `sch export bom` on a private copy of `schematic` and its sheet
/// files (and project file) in the cache; returns the CSV path and the
/// sheet files that could not be copied.
pub fn run_bom(
    schematic: &Path,
    export: &crate::jobs::BomExport,
    cli: &KicadCli,
    runner: &dyn CommandRunner,
    cache: &Cache,
) -> Result<(PathBuf, Vec<String>), SourceError> {
    let copy = private_copy(schematic, cache.root(), true)?;
    let job = export.job(&copy.main, cli)?;
    let out = run_cached(&job, &copy.hash, cli, runner, cache)?;
    Ok((out, copy.skipped))
}

/// The schematic next to a board: same stem, `.kicad_sch`.
pub fn schematic_for(board: &Path) -> Option<PathBuf> {
    let s = board.with_extension("kicad_sch");
    s.is_file().then_some(s)
}

/// A board file on disk, plotted with `kicad-cli`.
///
/// Every call re-reads the file, so changes on disk are picked up; the
/// parsed model is reused while the hash is unchanged.
pub struct FileBoardSource {
    path: PathBuf,
    cli: KicadCli,
    runner: Arc<dyn CommandRunner>,
    cache: Cache,
    model: Mutex<Option<Arc<Board>>>,
}

impl FileBoardSource {
    pub fn new(
        path: impl Into<PathBuf>,
        cli: KicadCli,
        runner: Arc<dyn CommandRunner>,
        cache: Cache,
    ) -> FileBoardSource {
        FileBoardSource {
            path: path.into(),
            cli,
            runner,
            cache,
            model: Mutex::new(None),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn cli(&self) -> &KicadCli {
        &self.cli
    }

    fn read(&self) -> Result<Vec<u8>, SourceError> {
        std::fs::read(&self.path).map_err(|e| SourceError::io("reading", &self.path, e))
    }

    /// Runs any job against this board. The input is replaced by a private
    /// copy of the board and its project file inside the cache
    /// ([`private_copy`]), so kicad-cli never writes into the project
    /// folder; the cache key uses the copy's content hash.
    pub fn run_job(&self, job: &Job) -> Result<PathBuf, SourceError> {
        self.run_job_fab(job, FabTextShow::ALL)
    }

    /// As [`FileBoardSource::run_job`], with fab-layer footprint texts
    /// hidden in the private copy as `show` says (kicad-cli 10.0.8 has no
    /// option for this). The cache key covers the edited copy.
    pub fn run_job_fab(&self, job: &Job, show: FabTextShow) -> Result<PathBuf, SourceError> {
        let edit = move |bytes: Vec<u8>| -> Result<Vec<u8>, SourceError> {
            let text = String::from_utf8(bytes).map_err(|e| {
                SourceError::Board(kdraft_board::BoardError::NotUtf8 {
                    offset: e.utf8_error().valid_up_to(),
                })
            })?;
            Ok(hide_fab_texts(&text, show)?.0.into_bytes())
        };
        let copy = if show.hides_anything() {
            private_copy_edited(&self.path, self.cache.root(), false, Some(&edit))?
        } else {
            private_copy(&self.path, self.cache.root(), false)?
        };
        let mut job = job.clone();
        job.input = copy.main.clone();
        run_cached(
            &job,
            &copy.hash,
            &self.cli,
            self.runner.as_ref(),
            &self.cache,
        )
    }

    pub fn cache(&self) -> &Cache {
        &self.cache
    }

    pub fn runner(&self) -> &Arc<dyn CommandRunner> {
        &self.runner
    }
}

impl BoardSource for FileBoardSource {
    fn board_hash(&self) -> Result<BoardHash, SourceError> {
        Ok(BoardHash::of(&self.read()?))
    }

    fn board(&self) -> Result<Arc<Board>, SourceError> {
        let bytes = self.read()?;
        let hash = BoardHash::of(&bytes);
        // A poisoned lock only means another thread panicked mid-update of
        // this cache slot; the hash check below makes the value safe to use.
        let mut guard = self.model.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(b) = guard.as_ref().filter(|b| b.hash == hash) {
            return Ok(Arc::clone(b));
        }
        let b = Arc::new(Board::from_bytes(bytes)?);
        *guard = Some(Arc::clone(&b));
        Ok(b)
    }

    fn layer_svg(&self, request: &SvgExport) -> Result<PathBuf, SourceError> {
        let job = request.job(&self.path, &self.cli)?;
        self.run_job(&job)
    }

    fn layer_svg_fab(
        &self,
        request: &SvgExport,
        show: FabTextShow,
    ) -> Result<PathBuf, SourceError> {
        let job = request.job(&self.path, &self.cli)?;
        self.run_job_fab(&job, show)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{DrillExport, DRILL_REPORT};
    use crate::kicad_cli::Version;
    use crate::runner::{arg_value, CommandOutput, FakeRunner};

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/kicad9-minimal.kicad_pcb")
    }

    fn cli() -> KicadCli {
        KicadCli {
            path: "/fake/kicad-cli".into(),
            version: Version::new(9, 0, 6),
            version_string: "9.0.6".into(),
            vars: Vec::new(),
        }
    }

    /// Writes "<svg>LAYERS</svg>" to the -o path.
    fn svg_writer() -> Arc<FakeRunner> {
        Arc::new(FakeRunner::new(|_, args| {
            let out = arg_value(args, "-o").unwrap_or_default();
            let layers = arg_value(args, "--layers").unwrap_or_default();
            match std::fs::write(out, format!("<svg>{layers}</svg>")) {
                Ok(()) => CommandOutput::ok(""),
                Err(e) => CommandOutput::failed(1, e.to_string()),
            }
        }))
    }

    fn setup(runner: Arc<FakeRunner>) -> (tempfile::TempDir, FileBoardSource) {
        let dir = tempfile::tempdir().unwrap();
        let board = dir.path().join("b.kicad_pcb");
        std::fs::copy(fixture(), &board).unwrap();
        let src = FileBoardSource::new(&board, cli(), runner, Cache::new(dir.path().join("cache")));
        (dir, src)
    }

    #[test]
    fn layer_svg_runs_once_then_hits_cache() {
        let runner = svg_writer();
        let (_dir, src) = setup(Arc::clone(&runner));
        let a = src.layer_svg(&SvgExport::layer("F.Fab")).unwrap();
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "<svg>F.Fab</svg>");
        let b = src.layer_svg(&SvgExport::layer("F.Fab")).unwrap();
        assert_eq!(a, b);
        assert_eq!(runner.calls().len(), 1);
        let call = &runner.calls()[0];
        assert_eq!(call.program, PathBuf::from("/fake/kicad-cli"));
        assert_eq!(&call.args[..3], ["pcb", "export", "svg"]);
        // The job runs on a private copy inside the cache, not on the board.
        let input = call.args.last().map(PathBuf::from).unwrap();
        assert!(input.starts_with(src.cache().root()), "{input:?}");
        assert_eq!(input.file_name(), src.path().file_name());
        assert_eq!(
            std::fs::read(&input).unwrap(),
            std::fs::read(src.path()).unwrap()
        );

        // Different layer: new job.
        src.layer_svg(&SvgExport::layer("B.Fab")).unwrap();
        assert_eq!(runner.calls().len(), 2);
    }

    #[test]
    fn board_edit_invalidates() {
        let runner = svg_writer();
        let (_dir, src) = setup(Arc::clone(&runner));
        let m1 = src.board().unwrap();
        let m1b = src.board().unwrap();
        assert!(Arc::ptr_eq(&m1, &m1b));
        let a = src.layer_svg(&SvgExport::layer("F.Fab")).unwrap();

        let text = std::fs::read_to_string(src.path()).unwrap();
        std::fs::write(src.path(), text.replacen("(at 20 10)", "(at 21 10)", 1)).unwrap();
        let m2 = src.board().unwrap();
        assert!(!Arc::ptr_eq(&m1, &m2));
        assert_ne!(m1.hash, m2.hash);
        assert_eq!(m2.footprint_by_reference("R1").unwrap().position.x, 21.0);
        let b = src.layer_svg(&SvgExport::layer("F.Fab")).unwrap();
        assert_ne!(a, b);
        assert_eq!(runner.calls().len(), 2);
    }

    #[test]
    fn cli_version_is_part_of_the_key() {
        let runner = svg_writer();
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let job = SvgExport::layer("F.Cu").job(&fixture(), &cli()).unwrap();
        let mut other = cli();
        other.version_string = "9.0.7".into();
        let a = run_cached(&job, &[0; 32], &cli(), runner.as_ref(), &cache).unwrap();
        let b = run_cached(&job, &[0; 32], &other, runner.as_ref(), &cache).unwrap();
        assert_ne!(a, b);
        assert_eq!(runner.calls().len(), 2);
    }

    #[test]
    fn failures_are_reported_and_not_cached() {
        let runner = Arc::new(FakeRunner::new(|_, _| {
            CommandOutput::failed(3, "Failed to load board\n")
        }));
        let (_dir, src) = setup(Arc::clone(&runner));
        match src.layer_svg(&SvgExport::layer("F.Fab")) {
            Err(SourceError::CommandFailed { status, stderr, .. }) => {
                assert_eq!(status, "exit code 3");
                assert_eq!(stderr, "Failed to load board");
            }
            other => panic!("{other:?}"),
        }
        assert!(src.layer_svg(&SvgExport::layer("F.Fab")).is_err());
        assert_eq!(runner.calls().len(), 2);

        let silent = Arc::new(FakeRunner::new(|_, _| CommandOutput::ok("")));
        let (_dir, src) = setup(silent);
        assert!(matches!(
            src.layer_svg(&SvgExport::layer("F.Fab")),
            Err(SourceError::MissingOutput(_))
        ));
    }

    #[test]
    fn directory_jobs() {
        let runner = Arc::new(FakeRunner::new(|_, args| {
            let report = arg_value(args, "--report-path").unwrap_or_default();
            match std::fs::write(report, "Drill report") {
                Ok(()) => CommandOutput::ok(""),
                Err(e) => CommandOutput::failed(1, e.to_string()),
            }
        }));
        let (_dir, src) = setup(Arc::clone(&runner));
        let job = DrillExport {
            generate_report: true,
            ..Default::default()
        }
        .job(Path::new("ignored"), src.cli())
        .unwrap();
        let out = src.run_job(&job).unwrap();
        assert!(out.is_dir());
        assert_eq!(
            std::fs::read_to_string(out.join(DRILL_REPORT)).unwrap(),
            "Drill report"
        );
        assert_eq!(src.run_job(&job).unwrap(), out);
        assert_eq!(runner.calls().len(), 1);
    }

    #[test]
    fn jobs_never_write_into_the_project_folder() {
        // Like kicad-cli 10.0.8 seemed to do: write `<board>.kicad_prl`
        // next to the input, then the output.
        let runner = Arc::new(FakeRunner::new(|_, args| {
            let input = PathBuf::from(args.last().cloned().unwrap_or_default());
            let _ = std::fs::write(input.with_extension("kicad_prl"), "{}");
            let out = arg_value(args, "-o").unwrap_or_default();
            match std::fs::write(out, "<svg/>") {
                Ok(()) => CommandOutput::ok(""),
                Err(e) => CommandOutput::failed(1, e.to_string()),
            }
        }));
        let proj = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let board = proj.path().join("b.kicad_pcb");
        std::fs::copy(fixture(), &board).unwrap();
        std::fs::write(proj.path().join("b.kicad_pro"), "{}").unwrap();
        let listing = || {
            let mut v: Vec<_> = std::fs::read_dir(proj.path())
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            v.sort();
            v
        };
        let before = listing();
        let src = FileBoardSource::new(
            &board,
            cli(),
            Arc::clone(&runner) as Arc<dyn CommandRunner>,
            Cache::new(cache.path()),
        );
        src.layer_svg(&SvgExport::layer("F.Fab")).unwrap();
        src.layer_svg(&SvgExport::layer("B.Fab")).unwrap();
        assert_eq!(listing(), before);
        for call in runner.calls() {
            let input = PathBuf::from(call.args.last().unwrap());
            assert!(input.starts_with(cache.path()), "{input:?}");
            assert!(input.with_extension("kicad_pro").is_file());
        }
    }

    #[test]
    fn hidden_fab_texts_are_in_the_copy_and_the_key() {
        let runner = svg_writer();
        let (_dir, src) = setup(Arc::clone(&runner));
        let original = std::fs::read(src.path()).unwrap();
        let show = FabTextShow {
            values: false,
            references: true,
            other: true,
        };
        let a = src.layer_svg(&SvgExport::layer("F.Fab")).unwrap();
        let b = src.layer_svg_fab(&SvgExport::layer("F.Fab"), show).unwrap();
        assert_ne!(a, b, "toggling the switch must not reuse the plot");
        assert_eq!(
            src.layer_svg_fab(&SvgExport::layer("F.Fab"), show).unwrap(),
            b
        );
        assert_eq!(runner.calls().len(), 2);
        let input = PathBuf::from(runner.calls()[1].args.last().unwrap());
        let copy = Board::load(&input).unwrap();
        let r1 = copy.footprint_by_reference("R1").unwrap();
        assert!(r1
            .texts
            .iter()
            .any(|t| t.kind == kdraft_board::FpTextKind::Value && t.hidden));
        assert_eq!(std::fs::read(src.path()).unwrap(), original);
    }

    #[test]
    fn bom_runs_on_a_private_schematic_copy() {
        let runner = Arc::new(FakeRunner::new(|_, args| {
            let input = PathBuf::from(args.last().cloned().unwrap_or_default());
            let _ = std::fs::write(input.with_extension("kicad_prl"), "{}");
            let sub = input.parent().unwrap().join("sub.kicad_sch").is_file();
            let out = arg_value(args, "-o").unwrap_or_default();
            let csv = format!("\"Refs\",\"Value\"\n\"R1\",\"{sub}\"\n");
            match std::fs::write(out, csv) {
                Ok(()) => CommandOutput::ok(""),
                Err(e) => CommandOutput::failed(1, e.to_string()),
            }
        }));
        let proj = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let sch = proj.path().join("b.kicad_sch");
        std::fs::write(
            &sch,
            "(kicad_sch (sheet (property \"Sheetfile\" \"sub.kicad_sch\")))",
        )
        .unwrap();
        std::fs::write(proj.path().join("sub.kicad_sch"), "(kicad_sch)").unwrap();
        let before: Vec<_> = std::fs::read_dir(proj.path())
            .unwrap()
            .count()
            .to_string()
            .into();
        let (csv, skipped) = run_bom(
            &sch,
            &crate::jobs::BomExport::kdraft(&[]),
            &cli10(),
            runner.as_ref(),
            &Cache::new(cache.path()),
        )
        .unwrap();
        assert!(skipped.is_empty());
        assert!(csv.starts_with(cache.path()));
        // The sheet file travelled with the copy.
        assert!(std::fs::read_to_string(&csv).unwrap().contains("true"));
        let after: Vec<_> = std::fs::read_dir(proj.path())
            .unwrap()
            .count()
            .to_string()
            .into();
        assert_eq!(before, after);
        let input = PathBuf::from(runner.calls()[0].args.last().unwrap());
        assert!(input.starts_with(cache.path()));
    }

    fn cli10() -> KicadCli {
        KicadCli {
            path: "/fake/kicad-cli".into(),
            version: Version::new(10, 0, 8),
            version_string: "10.0.8".into(),
            vars: Vec::new(),
        }
    }

    #[test]
    fn missing_board_file() {
        let runner = svg_writer();
        let src = FileBoardSource::new(
            "/nonexistent/b.kicad_pcb",
            cli(),
            runner,
            Cache::new("/tmp/unused"),
        );
        assert!(matches!(src.board(), Err(SourceError::Io { .. })));
        assert!(matches!(
            src.layer_svg(&SvgExport::layer("F.Fab")),
            Err(SourceError::Io { .. })
        ));
    }

    /// Needs a real kicad-cli: `KDRAFT_KICAD_CLI=/path/to/kicad-cli cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn real_kicad_cli_exports_svg() {
        let runner: Arc<dyn CommandRunner> = Arc::new(crate::runner::SystemRunner);
        let cli = crate::kicad_cli::discover(&crate::kicad_cli::SystemEnvironment, runner.as_ref())
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let src = FileBoardSource::new(fixture(), cli, runner, Cache::new(dir.path()));
        let svg = src.layer_svg(&SvgExport::layer("Edge.Cuts")).unwrap();
        let text = std::fs::read_to_string(svg).unwrap();
        assert!(text.contains("<svg"));
    }
}
