//! Spike S1: run `kicad-cli` exports on a board and measure how plotted SVG
//! coordinates relate to board coordinates.
//!
//! Nothing here assumes a particular SVG layout: every number in the report
//! is derived from the exported files and the parsed board. The run is
//! driven through [`CommandRunner`], so it is testable without KiCad.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use kdraft_board::geom::{BBox, Point};
use kdraft_board::Board;
use kdraft_source::jobs::{
    Arg, DrillExport, Job, PageSizeMode, PosExport, StatsExport, SvgExport, DRILL_REPORT,
};
use kdraft_source::stub::{write_stub, SheetStub};
use kdraft_source::{CommandRunner, KicadCli};
use serde::Serialize;

use crate::error::PlotError;
use crate::fit::{
    box_deviation, fit, fit_board, pad_refs, residuals, shape_centres, BoxDeviation, Fit,
    PlotTransform, PointIndex, Residuals,
};
use crate::svg::{self, SvgDoc};

type R<T> = Result<T, PlotError>;

pub const MODES: [u8; 3] = [0, 1, 2];
/// Page size mode of the single mirrored export.
pub const MIRROR_MODE: u8 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Status {
    #[serde(rename = "PASS")]
    Pass,
    #[serde(rename = "FAIL")]
    Fail,
    #[serde(rename = "N/A")]
    NotApplicable,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Pass => "PASS",
            Status::Fail => "FAIL",
            Status::NotApplicable => "N/A",
        }
    }
}

/// One program invocation.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct RunRecord {
    pub argv: Vec<String>,
    /// Display form, arguments with spaces double-quoted.
    pub command_line: String,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    /// Set if the program could not be started.
    pub spawn_error: Option<String>,
    pub seconds: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HelpProbe {
    pub command: String,
    pub run: RunRecord,
    /// Each flag kdraft uses for this command: found in the help text?
    pub flags: BTreeMap<String, bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FlagProbe {
    pub removed: String,
    pub run: RunRecord,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SvgAnalysis {
    pub width: Option<String>,
    pub height: Option<String>,
    pub view_box: Option<[f64; 4]>,
    pub physical_size: bool,
    pub mm_per_user_unit: f64,
    pub shapes: usize,
    pub filled: usize,
    pub stroked: usize,
    pub background_shapes: usize,
    pub text_elements: usize,
    pub skipped: BTreeMap<String, usize>,
    pub warnings: Vec<String>,
    /// Centre-line box of the content in root user units.
    pub content_bbox: Option<BBox>,
    /// The same box including half stroke widths.
    pub content_ink_bbox: Option<BBox>,
    /// Centre-line box in page millimetres (root size and viewBox applied).
    pub content_bbox_page_mm: Option<BBox>,
    /// Centre-line box mapped back to board mm with this mode's fit.
    pub content_bbox_board_mm: Option<BBox>,
}

fn analyse(doc: &SvgDoc) -> SvgAnalysis {
    let content: Vec<_> = doc.content().collect();
    let bbox = doc.content_bbox();
    SvgAnalysis {
        width: doc.width.as_ref().map(|l| format!("{}{}", l.value, l.unit)),
        height: doc
            .height
            .as_ref()
            .map(|l| format!("{}{}", l.value, l.unit)),
        view_box: doc.view_box.map(|v| [v.x, v.y, v.w, v.h]),
        physical_size: doc.physical_size,
        mm_per_user_unit: doc.mm_per_user_unit(),
        shapes: content.len(),
        filled: content.iter().filter(|s| s.fill.is_some()).count(),
        stroked: content.iter().filter(|s| s.stroke.is_some()).count(),
        background_shapes: doc.shapes.len() - content.len(),
        text_elements: doc.text_elements,
        skipped: doc.skipped.clone(),
        warnings: doc.warnings.iter().take(10).cloned().collect(),
        content_bbox: bbox,
        content_ink_bbox: doc.content_ink_bbox(),
        content_bbox_page_mm: bbox.map(|b| doc.user_to_mm.apply_bbox(&b)),
        content_bbox_board_mm: None,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SvgRun {
    pub layer: String,
    pub mode: u8,
    pub mirror: bool,
    pub run: RunRecord,
    /// Output file relative to the report directory.
    pub file: Option<String>,
    pub analysis: Option<SvgAnalysis>,
    pub parse_error: Option<String>,
    pub status: Status,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CopperCheck {
    pub layer: String,
    pub residuals: Residuals,
    pub status: Status,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EdgeCheck {
    /// Plot box (centre lines) against the mapped outline box, board mm.
    pub deviation: BoxDeviation,
    /// Same, with the ink box (half stroke widths added).
    pub ink_deviation: Option<BoxDeviation>,
    pub status: Status,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModeResult {
    pub mode: u8,
    pub fit: Option<Fit>,
    /// Physical page mm per board mm (1.0 for a 1:1 plot).
    pub page_mm_per_board_mm: Option<f64>,
    pub edge: Option<EdgeCheck>,
    pub copper: Vec<CopperCheck>,
    pub layer_status: BTreeMap<String, Status>,
    pub overlay: Option<String>,
    pub status: Status,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MirrorResult {
    pub mode: u8,
    pub fit: Option<Fit>,
    pub mirrored_detected: Option<bool>,
    /// Compared with the unmirrored fit of the same mode.
    pub same_y_mapping: Option<bool>,
    pub status: Status,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DrillReportRow {
    /// Tool name, e.g. `T3`.
    pub tool: Option<String>,
    pub diameter_mm: f64,
    pub holes: usize,
    pub slots: usize,
    /// From the section heading ("plated" / "unplated through holes");
    /// `None` when the line is outside such a section.
    pub plated: Option<bool>,
    pub line: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExportCheck {
    pub run: RunRecord,
    pub files: Vec<String>,
    pub status: Status,
    pub findings: Vec<String>,
}

/// The drawing sheet rendered by kicad-cli from a stub board.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SheetCheck {
    /// `default` (the stub's own sheet) or the `--drawing-sheet` file.
    pub sheet: String,
    pub run: RunRecord,
    pub file: Option<String>,
    pub analysis: Option<SvgAnalysis>,
    /// Paper size written into the stub, mm.
    pub paper_mm: [f64; 2],
    /// Root width/height in mm, if absolute units.
    pub svg_size_mm: Option<[f64; 2]>,
    /// Content box distance to the paper edges in page mm
    /// (left, top, right, bottom); negative = outside the paper.
    pub content_margins_mm: Option<[f64; 4]>,
    /// Title-block marker strings found anywhere in the SVG text.
    pub markers_found: BTreeMap<String, bool>,
    /// Occurrences of `${` in the SVG text (unexpanded variables).
    pub unexpanded_variables: usize,
    pub status: Status,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BoardInfo {
    pub path: String,
    pub file_version: i64,
    pub generator_version: Option<String>,
    pub outline_bbox: Option<BBox>,
    pub footprints: usize,
    pub pads: usize,
    pub holes: usize,
    pub drill_sizes: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CliInfo {
    pub path: String,
    pub version: String,
    pub version_string: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct S1Report {
    pub kdraft_version: String,
    pub kicad_cli: CliInfo,
    pub board: BoardInfo,
    pub layers: Vec<String>,
    pub help: Vec<HelpProbe>,
    pub flag_probes: Vec<FlagProbe>,
    /// Flags whose removal made a failing export succeed.
    pub flags_rejected: Vec<String>,
    pub layer_name_notes: Vec<String>,
    pub svg_runs: Vec<SvgRun>,
    pub modes: Vec<ModeResult>,
    pub mirrored: Option<MirrorResult>,
    pub drill: ExportCheck,
    pub pos: ExportCheck,
    pub stats: ExportCheck,
    pub sheets: Vec<SheetCheck>,
    pub pass: bool,
    pub verdict: String,
}

#[derive(Debug, Clone)]
pub struct S1Options {
    pub board: PathBuf,
    /// Directory that receives all outputs (created if missing).
    pub out_dir: PathBuf,
    /// Optional `.kicad_wks` for a second stub-sheet export with
    /// `--drawing-sheet`.
    pub drawing_sheet: Option<PathBuf>,
}

fn quote(s: &str) -> String {
    if s.is_empty() || s.contains([' ', '\t', '"']) {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

fn run_program(runner: &dyn CommandRunner, cli: &KicadCli, args: &[OsString]) -> RunRecord {
    let argv: Vec<String> = std::iter::once(cli.path.to_string_lossy().into_owned())
        .chain(args.iter().map(|a| a.to_string_lossy().into_owned()))
        .collect();
    let command_line = argv.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ");
    let t = Instant::now();
    let mut rec = RunRecord {
        argv,
        command_line,
        ..Default::default()
    };
    match runner.run(&cli.path, args) {
        Ok(out) => {
            rec.exit_code = out.status;
            rec.success = out.success();
            rec.stdout = truncate(out.stdout_text(), 20_000);
            rec.stderr = truncate(out.stderr_text(), 20_000);
        }
        Err(e) => rec.spawn_error = Some(e.to_string()),
    }
    rec.seconds = (t.elapsed().as_secs_f64() * 1000.0).round() / 1000.0;
    rec
}

fn truncate(s: String, n: usize) -> String {
    if s.len() <= n {
        return s;
    }
    let mut end = n;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}... [truncated]", &s[..end])
}

/// The job with `flag` (and its value, if it takes one) removed.
fn without(job: &Job, flag: &str) -> Job {
    let mut j = job.clone();
    let takes_value = matches!(flag, "--page-size-mode" | "--layers" | "--theme");
    if let Some(i) = j
        .options
        .iter()
        .position(|a| matches!(a, Arg::Lit(s) if s == flag))
    {
        j.options.remove(i);
        if takes_value && i < j.options.len() {
            j.options.remove(i);
        }
    }
    j
}

fn mode_of(m: u8) -> PageSizeMode {
    match m {
        1 => PageSizeMode::Current,
        2 => PageSizeMode::Board,
        _ => PageSizeMode::Sheet,
    }
}

fn svg_job(layer: &str, mode: u8, mirror: bool, board: &Path, cli: &KicadCli) -> R<Job> {
    let req = SvgExport {
        layers: vec![layer.to_string()],
        mirror,
        black_and_white: true,
        page_size_mode: mode_of(mode),
        exclude_drawing_sheet: true,
        ..Default::default()
    };
    Ok(req.job(board, cli)?)
}

const SVG_OPTIONAL_FLAGS: [&str; 4] = [
    "--mode-single",
    "--page-size-mode",
    "--exclude-drawing-sheet",
    "--black-and-white",
];

fn file_name(layer: &str, mode: u8, mirror: bool) -> String {
    let l = layer.replace(['.', ' '], "_");
    if mirror {
        format!("{l}-mode{mode}-mirror.svg")
    } else {
        format!("{l}-mode{mode}.svg")
    }
}

fn courtyard_names(board: &Board) -> (String, String) {
    let has = |n: &str| {
        board
            .layers
            .iter()
            .any(|l| l.name == n || l.user_name.as_deref() == Some(n))
    };
    if has("F.Courtyard") {
        ("F.Courtyard".to_string(), "F.CrtYd".to_string())
    } else {
        ("F.CrtYd".to_string(), "F.Courtyard".to_string())
    }
}

struct Ctx<'a> {
    cli: &'a KicadCli,
    runner: &'a dyn CommandRunner,
    out: &'a Path,
    board_path: &'a Path,
    rejected: Vec<String>,
    probes: Vec<FlagProbe>,
    probed: bool,
}

impl Ctx<'_> {
    fn rel(&self, p: &Path) -> String {
        p.strip_prefix(self.out)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
    }

    /// Runs one SVG export into `svg/`, diagnosing rejected flags on the
    /// first failure.
    fn export_svg(&mut self, layer: &str, mode: u8, mirror: bool) -> R<(RunRecord, PathBuf)> {
        let mut job = svg_job(layer, mode, mirror, self.board_path, self.cli)?;
        for f in &self.rejected {
            job = without(&job, f);
        }
        let target = self.out.join("svg").join(file_name(layer, mode, mirror));
        let rec = run_program(self.runner, self.cli, &job.args(&target));
        if rec.success || self.probed {
            return Ok((rec, target));
        }
        self.probed = true;
        for flag in SVG_OPTIONAL_FLAGS {
            let j = without(&job, flag);
            if j == job {
                continue;
            }
            let probe_target = self.out.join("probe").join(format!(
                "without{}-{}",
                flag.replace('-', "_"),
                file_name(layer, mode, mirror)
            ));
            let r = run_program(self.runner, self.cli, &j.args(&probe_target));
            if r.success {
                self.rejected.push(flag.to_string());
            }
            self.probes.push(FlagProbe {
                removed: flag.to_string(),
                run: r,
            });
        }
        if self.rejected.is_empty() {
            return Ok((rec, target));
        }
        for f in &self.rejected {
            job = without(&job, f);
        }
        let rec = run_program(self.runner, self.cli, &job.args(&target));
        Ok((rec, target))
    }
}

/// Runs spike S1 and writes `report.json`, `report.md`, the SVGs and
/// overlays into `opts.out_dir`. `log` receives progress lines.
pub fn run(
    opts: &S1Options,
    cli: &KicadCli,
    runner: &dyn CommandRunner,
    log: &mut dyn FnMut(&str),
) -> R<S1Report> {
    let board = Board::load(&opts.board)?;
    let out = opts.out_dir.as_path();
    for d in ["", "svg", "probe", "drill", "help", "sheet"] {
        let p = out.join(d);
        std::fs::create_dir_all(&p).map_err(|e| PlotError::io("creating", &p, e))?;
    }
    // kicad-cli runs on a copy of the board and its project file, so it
    // writes nothing (such as `.kicad_prl`) into the project folder.
    let copy = kdraft_source::private_copy(&opts.board, &out.join("input"), false)?;
    let mut ctx = Ctx {
        cli,
        runner,
        out,
        board_path: &copy.main,
        rejected: Vec::new(),
        probes: Vec::new(),
        probed: false,
    };

    log("probing --help of the export commands");
    let help = help_probes(&ctx)?;

    let (cy_first, cy_second) = courtyard_names(&board);
    let mut layers: Vec<String> = ["Edge.Cuts", "F.Cu", "B.Cu", "F.Fab"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    layers.push(cy_first.clone());
    let mut layer_name_notes = Vec::new();

    let mut svg_runs: Vec<SvgRun> = Vec::new();
    let mut docs: BTreeMap<(String, u8, bool), SvgDoc> = BTreeMap::new();
    let mut jobs: Vec<(String, u8, bool)> = Vec::new();
    for &m in &MODES {
        for l in &layers {
            jobs.push((l.clone(), m, false));
        }
    }
    jobs.push(("F.Cu".to_string(), MIRROR_MODE, true));
    for (layer, mode, mirror) in jobs {
        log(&format!(
            "export svg {layer} mode {mode}{}",
            if mirror { " mirrored" } else { "" }
        ));
        let (mut rec, mut target) = ctx.export_svg(&layer, mode, mirror)?;
        let mut used_layer = layer.clone();
        if !rec.success && layer == cy_first {
            let (r2, t2) = ctx.export_svg(&cy_second, mode, mirror)?;
            layer_name_notes.push(format!(
                "mode {mode}: {cy_first:?} failed ({}), {cy_second:?} {}",
                rec.exit_code
                    .map_or("no exit code".into(), |c| format!("exit {c}")),
                if r2.success {
                    "succeeded"
                } else {
                    "failed too"
                }
            ));
            if r2.success {
                rec = r2;
                target = t2;
                used_layer = cy_second.clone();
            }
        }
        let mut sr = SvgRun {
            layer: layer.clone(),
            mode,
            mirror,
            run: rec.clone(),
            file: None,
            analysis: None,
            parse_error: None,
            status: Status::Fail,
            note: String::new(),
        };
        if used_layer != layer {
            sr.note = format!("exported as {used_layer:?}");
        }
        if !rec.success {
            sr.note = format!("export failed: {}", first_line(&rec));
        } else if !target.is_file() {
            sr.note = "exit 0 but no output file".to_string();
        } else {
            sr.file = Some(ctx.rel(&target));
            let text = std::fs::read_to_string(&target)
                .map_err(|e| PlotError::io("reading", &target, e))?;
            match svg::parse(&text) {
                Ok(doc) => {
                    let a = analyse(&doc);
                    if a.content_bbox.is_some() {
                        sr.status = Status::Pass;
                    } else {
                        sr.note = "parsed, but no visible content".to_string();
                    }
                    sr.analysis = Some(a);
                    docs.insert((layer.clone(), mode, mirror), doc);
                }
                Err(e) => {
                    sr.parse_error = Some(e.to_string());
                    sr.note = format!("SVG parse error: {e}");
                }
            }
        }
        svg_runs.push(sr);
    }

    log("fitting transforms");
    let mut modes = Vec::new();
    for &m in &MODES {
        modes.push(analyse_mode(&board, m, &layers, &docs, &mut svg_runs, out)?);
    }
    let mirrored = analyse_mirror(&board, &docs, &modes, &mut svg_runs);

    log("export drill / pos / stats");
    let drill = drill_check(&ctx, &board)?;
    let pos = pos_check(&ctx, &board)?;
    let stats = stats_check(&ctx, &board)?;
    log("export drawing sheet from a stub board");
    let mut sheets = vec![sheet_check(&ctx, None)?];
    if let Some(ws) = &opts.drawing_sheet {
        sheets.push(sheet_check(&ctx, Some(ws))?);
    }

    let holes = board.holes();
    let mut sizes: Vec<f64> = holes
        .iter()
        .map(|h| round3(h.shape.tool_diameter()))
        .collect();
    sizes.sort_by(f64::total_cmp);
    sizes.dedup();
    let passing: Vec<u8> = modes
        .iter()
        .filter(|m| m.status == Status::Pass)
        .map(|m| m.mode)
        .collect();
    let pass = !passing.is_empty();
    let verdict = verdict_line(&modes, &mirrored, &passing);
    let report = S1Report {
        kdraft_version: env!("CARGO_PKG_VERSION").to_string(),
        kicad_cli: CliInfo {
            path: cli.path.to_string_lossy().into_owned(),
            version: cli.version.to_string(),
            version_string: cli.version_string.clone(),
        },
        board: BoardInfo {
            path: opts.board.to_string_lossy().into_owned(),
            file_version: board.version,
            generator_version: board.generator_version.clone(),
            outline_bbox: board.outline.bbox,
            footprints: board.footprints.len(),
            pads: board.footprints.iter().map(|f| f.pads.len()).sum(),
            holes: holes.len(),
            drill_sizes: sizes,
        },
        layers,
        help,
        flag_probes: ctx.probes,
        flags_rejected: ctx.rejected,
        layer_name_notes,
        svg_runs,
        modes,
        mirrored,
        drill,
        pos,
        stats,
        sheets,
        pass,
        verdict,
    };
    let json = serde_json::to_string_pretty(&report)?;
    write(&out.join("report.json"), &json)?;
    write(&out.join("report.md"), &markdown(&report))?;
    Ok(report)
}

fn write(path: &Path, text: &str) -> R<()> {
    std::fs::write(path, text).map_err(|e| PlotError::io("writing", path, e))
}

fn first_line(r: &RunRecord) -> String {
    if let Some(e) = &r.spawn_error {
        return e.clone();
    }
    let code = r
        .exit_code
        .map_or("signal".to_string(), |c| format!("exit {c}"));
    let msg = r
        .stderr
        .lines()
        .chain(r.stdout.lines())
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    format!("{code}: {msg}")
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn help_probes(ctx: &Ctx) -> R<Vec<HelpProbe>> {
    let cmds: [(&str, &[&str]); 6] = [
        (
            "pcb export svg",
            &[
                "--layers",
                "--mode-single",
                "--page-size-mode",
                "--exclude-drawing-sheet",
                "--black-and-white",
                "--mirror",
                "--drill-shape-opt",
                "--theme",
                "--variant",
            ],
        ),
        (
            "pcb export drill",
            &[
                "--format",
                "--generate-map",
                "--map-format",
                "--generate-report",
                "--report-path",
            ],
        ),
        (
            "pcb export pos",
            &[
                "--format",
                "--units",
                "--side",
                "--exclude-dnp",
                "--variant",
            ],
        ),
        ("pcb export stats", &["--format", "--units"]),
        // The flags kdraft's 3D render job uses (all present on 10.0.8).
        (
            "pcb render",
            &[
                "--width",
                "--height",
                "--side",
                "--background",
                "--quality",
                "--perspective",
                "--zoom",
                "--rotate",
                "--define-var",
                "--variant",
                "--preset",
                "--use-board-stackup-colors",
                "--floor",
            ],
        ),
        // The flags kdraft's BOM job uses (all present on 10.0.8).
        (
            "sch export bom",
            &[
                "--fields",
                "--labels",
                "--group-by",
                "--exclude-dnp",
                "--field-delimiter",
                "--string-delimiter",
                "--ref-delimiter",
                "--ref-range-delimiter",
                "--keep-tabs",
                "--keep-line-breaks",
                "--variant",
            ],
        ),
    ];
    let mut v = Vec::new();
    for (cmd, flags) in cmds {
        let mut args: Vec<OsString> = cmd.split(' ').map(OsString::from).collect();
        args.push("--help".into());
        let run = run_program(ctx.runner, ctx.cli, &args);
        let text = format!("{}\n{}", run.stdout, run.stderr);
        let path = ctx
            .out
            .join("help")
            .join(format!("{}.txt", cmd.replace(' ', "_")));
        write(&path, &text)?;
        let flags = flags
            .iter()
            .map(|f| (f.to_string(), help_mentions(&text, f)))
            .collect();
        v.push(HelpProbe {
            command: cmd.to_string(),
            run,
            flags,
        });
    }
    Ok(v)
}

/// The flag appears as a whole word (not as a prefix of a longer flag).
fn help_mentions(text: &str, flag: &str) -> bool {
    text.match_indices(flag).any(|(i, _)| {
        let after = text[i + flag.len()..].chars().next();
        !after.is_some_and(|c| c.is_alphanumeric() || c == '-' || c == '_')
    })
}

fn analyse_mode(
    board: &Board,
    mode: u8,
    layers: &[String],
    docs: &BTreeMap<(String, u8, bool), SvgDoc>,
    runs: &mut [SvgRun],
    out: &Path,
) -> R<ModeResult> {
    let get = |l: &str| docs.get(&(l.to_string(), mode, false));
    let mut res = ModeResult {
        mode,
        fit: None,
        page_mm_per_board_mm: None,
        edge: None,
        copper: Vec::new(),
        layer_status: BTreeMap::new(),
        overlay: None,
        status: Status::Fail,
        notes: Vec::new(),
    };
    for r in runs.iter().filter(|r| r.mode == mode && !r.mirror) {
        res.layer_status.insert(r.layer.clone(), r.status);
    }
    let (Some(edge), Some(fcu)) = (get("Edge.Cuts"), get("F.Cu")) else {
        res.notes
            .push("Edge.Cuts or F.Cu export missing; no fit".to_string());
        return Ok(res);
    };
    let Some(f) = fit_board(board, edge, Some((fcu, "F.Cu"))) else {
        res.notes
            .push("no fit possible (empty outline or empty Edge.Cuts plot)".to_string());
        return Ok(res);
    };
    let t = f.transform;
    res.page_mm_per_board_mm = Some(t.scale() * edge.mm_per_user_unit());

    if let (Some(ob), Some(sb)) = (board.outline.bbox, edge.content_bbox()) {
        if let Some(dev) = box_deviation(&t, &ob, &sb) {
            let ink = edge
                .content_ink_bbox()
                .and_then(|ib| box_deviation(&t, &ob, &ib));
            let (status, note) = if dev.max_abs() <= 0.05 {
                (
                    Status::Pass,
                    "centre-line box matches the outline box".to_string(),
                )
            } else if dev.spread() <= 0.02 {
                (
                    Status::Pass,
                    format!(
                        "uniformly inflated by {:.4} mm per side (filled stroke outlines?)",
                        (dev.left + dev.right + dev.top + dev.bottom) / 4.0
                    ),
                )
            } else {
                (
                    Status::Fail,
                    format!("max side deviation {:.4} mm", dev.max_abs()),
                )
            };
            res.edge = Some(EdgeCheck {
                deviation: dev,
                ink_deviation: ink,
                status,
                note,
            });
        }
    }

    for layer in ["F.Cu", "B.Cu"] {
        let Some(doc) = get(layer) else { continue };
        let refs = pad_refs(board, layer);
        if refs.is_empty() {
            continue;
        }
        let index = PointIndex::new(shape_centres(doc), t.scale());
        let r = residuals(&t, &refs, &index);
        let status = if r.within_0_05 >= 0.95 {
            Status::Pass
        } else {
            Status::Fail
        };
        res.copper.push(CopperCheck {
            layer: layer.to_string(),
            residuals: r,
            status,
        });
    }

    for r in runs.iter_mut().filter(|r| r.mode == mode && !r.mirror) {
        if let (Some(a), Some(doc)) = (r.analysis.as_mut(), get(&r.layer)) {
            a.content_bbox_board_mm = doc.content_bbox().and_then(|b| t.unmap_bbox(&b));
        }
    }

    let overlay_name = format!("overlay-mode{mode}.svg");
    let fcu_file = runs
        .iter()
        .find(|r| r.mode == mode && !r.mirror && r.layer == "F.Cu")
        .and_then(|r| r.file.clone());
    if let Some(file) = fcu_file {
        let src = out.join(&file);
        let text = std::fs::read_to_string(&src).map_err(|e| PlotError::io("reading", &src, e))?;
        let refs = pad_refs(board, "F.Cu");
        let o = overlay(
            &text,
            &t,
            &refs.iter().map(|r| r.at).collect::<Vec<_>>(),
            board.outline.bbox,
        );
        write(&out.join(&overlay_name), &o)?;
        res.overlay = Some(overlay_name);
    }

    let fcu_ok = res
        .copper
        .iter()
        .find(|c| c.layer == "F.Cu")
        .map(|c| c.status);
    let edge_ok = res.edge.as_ref().map(|e| e.status);
    let layers_ok = layers
        .iter()
        .all(|l| res.layer_status.get(l) == Some(&Status::Pass));
    if fcu_ok.is_none() {
        res.notes
            .push("board has no F.Cu pads: transform rests on outline vertices only".to_string());
    }
    if !layers_ok {
        res.notes
            .push("at least one layer export failed or was empty".to_string());
    }
    res.status = if layers_ok
        && edge_ok == Some(Status::Pass)
        && fcu_ok.unwrap_or(Status::Pass) == Status::Pass
        && f.is_good()
    {
        Status::Pass
    } else {
        Status::Fail
    };
    res.fit = Some(f);
    Ok(res)
}

fn analyse_mirror(
    board: &Board,
    docs: &BTreeMap<(String, u8, bool), SvgDoc>,
    modes: &[ModeResult],
    runs: &mut [SvgRun],
) -> Option<MirrorResult> {
    let doc = docs.get(&("F.Cu".to_string(), MIRROR_MODE, true))?;
    let mut res = MirrorResult {
        mode: MIRROR_MODE,
        fit: None,
        mirrored_detected: None,
        same_y_mapping: None,
        status: Status::Fail,
        note: String::new(),
    };
    let refs = pad_refs(board, "F.Cu");
    let svg_box = doc.content_bbox()?;
    // Board-frame box of the same layer's content, from the unmirrored fit.
    let base = modes.iter().find(|m| m.mode == MIRROR_MODE)?;
    let base_t = base.fit.as_ref().map(|f| f.transform);
    let board_box = base_t
        .and_then(|t| {
            docs.get(&("F.Cu".to_string(), MIRROR_MODE, false))
                .and_then(SvgDoc::content_bbox)
                .and_then(|b| t.unmap_bbox(&b))
        })
        .or(board.outline.bbox)?;
    if refs.len() < 3 {
        res.status = Status::NotApplicable;
        res.note = "fewer than 3 F.Cu pads".to_string();
        return Some(res);
    }
    let guess = base_t.map_or(1.0, |t| t.scale());
    let index = PointIndex::new(shape_centres(doc), guess);
    let f = fit(&board_box, &svg_box, &refs, &index, "pads")?;
    res.mirrored_detected = Some(f.transform.mirrored());
    if let Some(bt) = base_t {
        res.same_y_mapping = Some(
            (f.transform.sy - bt.sy).abs() <= 1e-6 * bt.sy.abs()
                && (f.transform.oy - bt.oy).abs() <= 0.01 * bt.scale(),
        );
    }
    res.status = if f.is_good() && f.transform.mirrored() {
        Status::Pass
    } else {
        Status::Fail
    };
    res.note = if f.transform.mirrored() {
        "plot is mirrored relative to the board".to_string()
    } else {
        "plot is NOT mirrored relative to the board".to_string()
    };
    if let Some(r) = runs
        .iter_mut()
        .find(|r| r.mirror && r.layer == "F.Cu" && r.mode == MIRROR_MODE)
    {
        if let Some(a) = r.analysis.as_mut() {
            a.content_bbox_board_mm = f.transform.unmap_bbox(&svg_box);
        }
    }
    res.fit = Some(f);
    Some(res)
}

/// The SVG with red crosses at `points` and a green rectangle at `bbox`
/// (board mm, mapped through `t`) inserted before the closing tag.
pub fn overlay(svg_text: &str, t: &PlotTransform, points: &[Point], bbox: Option<BBox>) -> String {
    let s = t.scale();
    let arm = 0.3 * s;
    let w = 0.04 * s;
    let mut g = String::new();
    let _ = writeln!(
        g,
        "<g id=\"kdraft-s1-overlay\" fill=\"none\" stroke-linecap=\"round\">"
    );
    if let Some(b) = bbox {
        let m = t.map_bbox(&b);
        let _ = writeln!(
            g,
            "<rect x=\"{:.4}\" y=\"{:.4}\" width=\"{:.4}\" height=\"{:.4}\" stroke=\"#00AA00\" stroke-width=\"{:.4}\"/>",
            m.min.x,
            m.min.y,
            m.width(),
            m.height(),
            w * 2.0
        );
    }
    let _ = write!(g, "<path stroke=\"#FF0000\" stroke-width=\"{w:.4}\" d=\"");
    for p in points {
        let q = t.apply(*p);
        let _ = write!(
            g,
            "M{:.4} {:.4}L{:.4} {:.4}M{:.4} {:.4}L{:.4} {:.4}",
            q.x - arm,
            q.y,
            q.x + arm,
            q.y,
            q.x,
            q.y - arm,
            q.x,
            q.y + arm
        );
    }
    let _ = writeln!(g, "\"/>\n</g>");
    match svg_text.rfind("</svg>") {
        Some(i) => format!("{}{}{}", &svg_text[..i], g, &svg_text[i..]),
        None => format!("{svg_text}{g}"),
    }
}

fn list_files(dir: &Path, base: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| e.path().is_file())
                .map(|e| {
                    e.path()
                        .strip_prefix(base)
                        .unwrap_or(&e.path())
                        .to_string_lossy()
                        .replace('\\', "/")
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn drill_check(ctx: &Ctx, board: &Board) -> R<ExportCheck> {
    let job = DrillExport {
        generate_report: true,
        ..Default::default()
    }
    .job(ctx.board_path, ctx.cli)?;
    let dir = ctx.out.join("drill");
    let run = run_program(ctx.runner, ctx.cli, &job.args(&dir));
    let files = list_files(&dir, ctx.out);
    let mut findings = Vec::new();
    let mut status = if run.success {
        Status::Pass
    } else {
        Status::Fail
    };
    if !run.success {
        findings.push(format!("drill export failed: {}", first_line(&run)));
    }
    let report_path = dir.join(DRILL_REPORT);
    let report = if report_path.is_file() {
        Some(report_path)
    } else {
        let other = files
            .iter()
            .find(|f| f.ends_with(".rpt") || f.ends_with(".txt"));
        if run.success {
            findings.push(format!(
                "report not at the requested --report-path {}; other report-like file: {}",
                DRILL_REPORT,
                other.map_or("none", String::as_str)
            ));
        }
        other.map(|f| ctx.out.join(f))
    };
    if let Some(p) = report {
        let text = std::fs::read_to_string(&p).map_err(|e| PlotError::io("reading", &p, e))?;
        let rows = parse_drill_report(&text);
        let holes = board.holes();
        let theirs: usize = rows.iter().map(|r| r.holes).sum();
        findings.push(format!(
            "drill report (heuristic parse): {} tool lines, {} holes; kdraft: {} holes in {} drill rows",
            rows.len(),
            theirs,
            holes.len(),
            board.drill_table().len()
        ));
        if rows.is_empty() {
            findings.push("no tool lines recognised in the report; see the raw file".to_string());
        } else if theirs != holes.len() {
            findings.push(format!(
                "MISMATCH: hole count report {theirs} vs kdraft {}",
                holes.len()
            ));
        }
        // Compare on KiCad's grouping (tool diameter = smaller oval
        // dimension, plated and non-plated apart; confirmed on 10.0.8 with
        // a real report); kdraft's own table is finer. The check is on
        // (diameter, hole count) pairs, then plating and slots.
        let groups = kdraft_board::kicad_tool_groups(&holes);
        findings.push(format!(
            "kdraft in KiCad tool grouping: {} tools ({})",
            groups.len(),
            groups
                .iter()
                .map(|g| format!(
                    "{} {} x{}{}",
                    g.diameter,
                    if g.plated { "PTH" } else { "NPTH" },
                    g.holes,
                    if g.slots > 0 {
                        format!(" ({} slots)", g.slots)
                    } else {
                        String::new()
                    }
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        let mut ours: Vec<(i64, usize)> = groups
            .iter()
            .map(|g| ((g.diameter * 1000.0).round() as i64, g.holes))
            .collect();
        ours.sort_unstable();
        let mut rep: Vec<(i64, usize)> = rows
            .iter()
            .map(|r| ((round3(r.diameter_mm) * 1000.0).round() as i64, r.holes))
            .collect();
        rep.sort_unstable();
        if !rows.is_empty() {
            if ours == rep {
                findings.push(
                    "tool list matches kdraft's holes in KiCad grouping (diameter, count)"
                        .to_string(),
                );
                let key =
                    |p: bool, d: f64, h: usize, sl: usize| (p, (d * 1000.0).round() as i64, h, sl);
                let mut full_ours: Vec<_> = groups
                    .iter()
                    .map(|g| key(g.plated, g.diameter, g.holes, g.slots))
                    .collect();
                full_ours.sort_unstable();
                let mut full_rep: Vec<_> = rows
                    .iter()
                    .filter_map(|r| {
                        r.plated
                            .map(|p| key(p, round3(r.diameter_mm), r.holes, r.slots))
                    })
                    .collect();
                full_rep.sort_unstable();
                if full_rep.len() == rows.len() {
                    findings.push(if full_ours == full_rep {
                        "plating and slot counts match too".to_string()
                    } else {
                        format!("MISMATCH in plating/slots: report {full_rep:?} vs kdraft {full_ours:?}")
                    });
                }
            } else {
                findings.push(format!(
                    "MISMATCH: tools (diameter um, holes) report {rep:?} vs kdraft in KiCad grouping {ours:?}"
                ));
            }
        }
        let rows_json = serde_json::to_string_pretty(&rows)?;
        write(&ctx.out.join("drill-report-parsed.json"), &rows_json)?;
    } else if run.success {
        status = Status::Fail;
    }
    Ok(ExportCheck {
        run,
        files,
        status,
        findings,
    })
}

/// Marker values written into the stub's title block.
pub const SHEET_MARKERS: [(&str, &str); 5] = [
    ("title", "KDRAFT-S1-TITLE"),
    ("rev", "S1REV"),
    ("company", "KDRAFT-S1-COMPANY"),
    ("comment1", "KDRAFT-S1-COMMENT1"),
    ("text_variable", "KDRAFT-S1-VARVALUE"),
];

/// Project text variable the stub defines and references from COMMENT2.
pub const SHEET_VARIABLE: &str = "KDRAFT_S1_VAR";

fn sheet_check(ctx: &Ctx, sheet: Option<&Path>) -> R<SheetCheck> {
    let (pw, ph) = (420.0, 297.0);
    let mut stub = SheetStub {
        paper: "A3".to_string(),
        width: pw,
        height: ph,
        title: Some(SHEET_MARKERS[0].1.to_string()),
        rev: Some(SHEET_MARKERS[1].1.to_string()),
        company: Some(SHEET_MARKERS[2].1.to_string()),
        date: Some("2026-11-24".to_string()),
        ..Default::default()
    };
    stub.comments.insert(1, SHEET_MARKERS[3].1.to_string());
    // The default sheet prints only title-block fields, never arbitrary
    // project variables, so the variable is referenced from a printed field
    // (COMMENT2). Up to the 10.0.8 run the probe defined the variable but
    // no field used it, so its marker could not appear.
    stub.comments.insert(2, format!("${{{}}}", SHEET_VARIABLE));
    stub.text_variables
        .insert(SHEET_VARIABLE.to_string(), SHEET_MARKERS[4].1.to_string());
    let dir = ctx.out.join("sheet");
    let pcb = write_stub(&dir, "stub", &stub)?;
    let req = SvgExport {
        layers: vec!["Edge.Cuts".to_string()],
        black_and_white: true,
        page_size_mode: PageSizeMode::Sheet,
        exclude_drawing_sheet: false,
        drawing_sheet: sheet.map(Path::to_path_buf),
        ..Default::default()
    };
    let mut job = req.job(&pcb, ctx.cli)?;
    for f in &ctx.rejected {
        job = without(&job, f);
    }
    let name = if sheet.is_some() {
        "sheet-custom.svg"
    } else {
        "sheet-default.svg"
    };
    let target = dir.join(name);
    let run = run_program(ctx.runner, ctx.cli, &job.args(&target));
    let mut c = SheetCheck {
        sheet: sheet.map_or("default".to_string(), |p| p.to_string_lossy().into_owned()),
        run: run.clone(),
        file: None,
        analysis: None,
        paper_mm: [pw, ph],
        svg_size_mm: None,
        content_margins_mm: None,
        markers_found: BTreeMap::new(),
        unexpanded_variables: 0,
        status: Status::Fail,
        note: String::new(),
    };
    if !run.success {
        c.note = format!("export failed: {}", first_line(&run));
        return Ok(c);
    }
    if !target.is_file() {
        c.note = "exit 0 but no output file".to_string();
        return Ok(c);
    }
    c.file = Some(ctx.rel(&target));
    let text =
        std::fs::read_to_string(&target).map_err(|e| PlotError::io("reading", &target, e))?;
    for (k, v) in SHEET_MARKERS {
        c.markers_found.insert(k.to_string(), text.contains(v));
    }
    c.unexpanded_variables = text.matches("${").count();
    match svg::parse(&text) {
        Ok(doc) => {
            let a = analyse(&doc);
            c.svg_size_mm = match (
                doc.width.as_ref().and_then(|l| l.to_mm()),
                doc.height.as_ref().and_then(|l| l.to_mm()),
            ) {
                (Some(w), Some(h)) if doc.physical_size => Some([w, h]),
                _ => None,
            };
            c.content_margins_mm = a
                .content_bbox_page_mm
                .map(|b| [b.min.x, b.min.y, pw - b.max.x, ph - b.max.y]);
            let inside = c
                .content_margins_mm
                .is_some_and(|m| m.iter().all(|v| *v >= -0.01));
            c.status = if a.content_bbox.is_some() && inside {
                Status::Pass
            } else {
                Status::Fail
            };
            c.note = if a.content_bbox.is_none() {
                "no visible content".to_string()
            } else if inside {
                "sheet content lies on the paper".to_string()
            } else {
                "sheet content extends beyond the paper size".to_string()
            };
            c.analysis = Some(a);
        }
        Err(e) => c.note = format!("SVG parse error: {e}"),
    }
    Ok(c)
}

/// Tolerant parse of a drill report: every line with a millimetre size and
/// a `(N hole...)` count. Not tied to one report layout. Layout seen on
/// kicad-cli 10.0.8 (fixture `fixtures/kicad-cli/10.0.8/`):
///
/// ```text
///     plated through holes:
///     T3  0.500mm  0.0197"  (4 holes)  (with 4 slots)
///     T1  0.300mm  0.0118"  (523 holes))
///     unplated through holes:
///     T11  1.600mm  0.0630"  (2 holes))
/// ```
///
/// (the doubled `)` is KiCad's). A slot is a hole of its tool; equal-sided
/// ovals appear as plain round tools.
pub fn parse_drill_report(text: &str) -> Vec<DrillReportRow> {
    let mut rows = Vec::new();
    let mut plated = None;
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        let t = lower.trim();
        if t.ends_with("through holes:") || t.ends_with("holes:") {
            if t.contains("unplated") || t.contains("not plated") || t.contains("non-plated") {
                plated = Some(false);
            } else if t.contains("plated") {
                plated = Some(true);
            }
            continue;
        }
        let Some(holes) = count_before(&lower, "hole") else {
            continue;
        };
        let Some(d) = mm_value(&lower) else { continue };
        let tool = t
            .split_whitespace()
            .next()
            .filter(|w| {
                w.starts_with('t') && w[1..].chars().all(|c| c.is_ascii_digit()) && w.len() > 1
            })
            .map(|w| w.to_ascii_uppercase());
        rows.push(DrillReportRow {
            tool,
            diameter_mm: d,
            holes,
            slots: count_before(&lower, "slot")
                .or_else(|| with_count(&lower, "slot"))
                .unwrap_or(0),
            plated,
            line: line.trim().to_string(),
        });
    }
    rows
}

/// `N` in "(with N word...)".
fn with_count(s: &str, word: &str) -> Option<usize> {
    let i = s.find("(with ")?;
    let rest = &s[i + 6..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let after = rest[digits.len()..].trim_start();
    if !digits.is_empty() && after.starts_with(word) {
        digits.parse().ok()
    } else {
        None
    }
}

/// `N` in "(N word...)".
fn count_before(s: &str, word: &str) -> Option<usize> {
    let mut search = 0;
    while let Some(i) = s[search..].find(word) {
        let at = search + i;
        let before = s[..at].trim_end();
        let digits: String = before
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let rest = before[..before.len() - digits.len()].trim_end();
        if !digits.is_empty() && rest.ends_with('(') {
            return digits.parse().ok();
        }
        search = at + word.len();
    }
    None
}

/// First number directly followed by "mm".
fn mm_value(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    let mut i = 0;
    while let Some(j) = s[i..].find("mm") {
        let end = i + j;
        let mut k = end;
        while k > 0 && (b[k - 1].is_ascii_digit() || b[k - 1] == b'.') {
            k -= 1;
        }
        if k < end {
            if let Ok(v) = s[k..end].parse() {
                return Some(v);
            }
        }
        i = end + 2;
    }
    None
}

fn pos_check(ctx: &Ctx, board: &Board) -> R<ExportCheck> {
    let job = PosExport::default().job(ctx.board_path, ctx.cli)?;
    let target = ctx.out.join("pos.csv");
    let run = run_program(ctx.runner, ctx.cli, &job.args(&target));
    let mut findings = Vec::new();
    let mut files = Vec::new();
    let mut status = if run.success {
        Status::Pass
    } else {
        Status::Fail
    };
    if !run.success {
        findings.push(format!("pos export failed: {}", first_line(&run)));
    } else if !target.is_file() {
        findings.push("exit 0 but no pos.csv".to_string());
        status = Status::Fail;
    } else {
        files.push("pos.csv".to_string());
        let text =
            std::fs::read_to_string(&target).map_err(|e| PlotError::io("reading", &target, e))?;
        findings.extend(compare_pos(&text, board));
    }
    Ok(ExportCheck {
        run,
        files,
        status,
        findings,
    })
}

fn csv_fields(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut q = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if q && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => q = !q,
            ',' if !q => out.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    out.push(cur);
    out.into_iter().map(|s| s.trim().to_string()).collect()
}

/// Compares a position CSV with the board: row count and a per-axis linear
/// fit of the CSV coordinates against board footprint positions.
pub fn compare_pos(text: &str, board: &Board) -> Vec<String> {
    let mut f = Vec::new();
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let Some(header) = lines.next() else {
        f.push("pos.csv is empty".to_string());
        return f;
    };
    let h: Vec<String> = csv_fields(header)
        .into_iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();
    let col = |names: &[&str]| h.iter().position(|c| names.contains(&c.as_str()));
    let (Some(ri), Some(xi), Some(yi)) = (
        col(&["ref", "reference", "designator"]),
        col(&["posx", "pos x", "x", "mid x"]),
        col(&["posy", "pos y", "y", "mid y"]),
    ) else {
        f.push(format!("pos.csv header not recognised: {header:?}"));
        return f;
    };
    let rows: Vec<Vec<String>> = lines.map(csv_fields).collect();
    let expected = board
        .footprints
        .iter()
        .filter(|fp| !fp.attributes.exclude_from_pos_files)
        .count();
    f.push(format!(
        "pos rows {}; kdraft footprints without exclude_from_pos_files {expected} (total {})",
        rows.len(),
        board.footprints.len()
    ));
    if rows.len() != expected {
        f.push(format!(
            "MISMATCH (informational): pos rows {} vs {expected}",
            rows.len()
        ));
    }
    let mut pairs = Vec::new();
    for r in &rows {
        let (Some(rf), Some(x), Some(y)) = (r.get(ri), r.get(xi), r.get(yi)) else {
            continue;
        };
        let (Ok(x), Ok(y)) = (x.parse::<f64>(), y.parse::<f64>()) else {
            continue;
        };
        if let Some(fp) = board.footprint_by_reference(rf) {
            pairs.push((fp.position, Point::new(x, y)));
        }
    }
    let fitx = line_fit_pub(pairs.iter().map(|(p, q)| (p.x, q.x)));
    let fity = line_fit_pub(pairs.iter().map(|(p, q)| (p.y, q.y)));
    match (fitx, fity) {
        (Some((ax, bx)), Some((ay, by))) => {
            let max = pairs
                .iter()
                .map(|(p, q)| {
                    ((ax * p.x + bx - q.x).powi(2) + (ay * p.y + by - q.y).powi(2)).sqrt()
                })
                .fold(0.0f64, f64::max);
            f.push(format!(
                "pos coordinates vs board: x' = {ax:.6} x + {bx:.4}, y' = {ay:.6} y + {by:.4} ({} refs matched, max residual {max:.4} mm)",
                pairs.len()
            ));
        }
        _ => f.push(format!(
            "pos coordinates: too few matched refs for a fit ({})",
            pairs.len()
        )),
    }
    f
}

fn line_fit_pub(pairs: impl Iterator<Item = (f64, f64)>) -> Option<(f64, f64)> {
    let v: Vec<(f64, f64)> = pairs.collect();
    if v.len() < 2 {
        return None;
    }
    let n = v.len() as f64;
    let su: f64 = v.iter().map(|p| p.0).sum();
    let sv: f64 = v.iter().map(|p| p.1).sum();
    let suu: f64 = v.iter().map(|p| p.0 * p.0).sum();
    let suv: f64 = v.iter().map(|p| p.0 * p.1).sum();
    let var = suu - su * su / n;
    if var.abs() < 1e-9 {
        return None;
    }
    let a = (suv - su * sv / n) / var;
    Some((a, (sv - a * su) / n))
}

fn stats_check(ctx: &Ctx, board: &Board) -> R<ExportCheck> {
    let job = StatsExport::default().job(ctx.board_path, ctx.cli)?;
    let target = ctx.out.join("stats.json");
    let run = run_program(ctx.runner, ctx.cli, &job.args(&target));
    let mut findings = Vec::new();
    let mut files = Vec::new();
    let status = if !run.success {
        findings.push(format!(
            "stats export failed (unsupported on this version?): {}",
            first_line(&run)
        ));
        Status::Fail
    } else if !target.is_file() {
        findings.push("exit 0 but no stats.json".to_string());
        Status::Fail
    } else {
        files.push("stats.json".to_string());
        let text =
            std::fs::read_to_string(&target).map_err(|e| PlotError::io("reading", &target, e))?;
        match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(v) => {
                findings.extend(compare_stats(&v, board));
                Status::Pass
            }
            Err(e) => {
                findings.push(format!("stats.json is not JSON: {e}"));
                Status::Fail
            }
        }
    };
    Ok(ExportCheck {
        run,
        files,
        status,
        findings,
    })
}

/// Lists numeric fields whose key mentions width or height and compares
/// them with the outline size. Key names are not assumed.
pub fn compare_stats(v: &serde_json::Value, board: &Board) -> Vec<String> {
    let mut leaves = Vec::new();
    collect_numbers(v, String::new(), &mut leaves);
    let mut f = vec![format!(
        "stats.json: {} numeric fields; top-level keys {:?}",
        leaves.len(),
        v.as_object()
            .map(|o| o.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default()
    )];
    let Some(b) = board.outline.bbox else {
        return f;
    };
    for (key, val) in &leaves {
        let k = key.to_ascii_lowercase();
        let want = if k.contains("width") {
            Some(b.width())
        } else if k.contains("height") {
            Some(b.height())
        } else {
            None
        };
        if let Some(w) = want {
            let tag = if (val - w).abs() <= 0.01 {
                "matches"
            } else {
                "differs from"
            };
            f.push(format!("{key} = {val} {tag} kdraft outline {w:.4} mm"));
        }
    }
    f
}

fn collect_numbers(v: &serde_json::Value, path: String, out: &mut Vec<(String, f64)>) {
    match v {
        serde_json::Value::Number(n) => {
            if let Some(x) = n.as_f64() {
                out.push((path, x));
            }
        }
        serde_json::Value::Object(o) => {
            for (k, x) in o {
                let p = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                collect_numbers(x, p, out);
            }
        }
        serde_json::Value::Array(a) => {
            for (i, x) in a.iter().enumerate().take(200) {
                collect_numbers(x, format!("{path}[{i}]"), out);
            }
        }
        _ => {}
    }
}

fn verdict_line(modes: &[ModeResult], mirrored: &Option<MirrorResult>, passing: &[u8]) -> String {
    if passing.is_empty() {
        let why: Vec<String> = modes
            .iter()
            .map(|m| {
                let r = m.copper.iter().find(|c| c.layer == "F.Cu").map_or(
                    "no F.Cu check".to_string(),
                    |c| {
                        format!(
                            "F.Cu {:.1}% within 0.05 mm",
                            c.residuals.within_0_05 * 100.0
                        )
                    },
                );
                format!("mode {}: {}", m.mode, r)
            })
            .collect();
        return format!("S1 FAIL: no page size mode passed ({})", why.join("; "));
    }
    let mut s = format!(
        "S1 PASS for page size mode(s) {:?}: board-to-SVG transform derived and validated on pads",
        passing
    );
    if let Some(m) = mirrored {
        let _ = write!(s, "; mirrored export {}", m.status.as_str());
    }
    s
}

fn fmt_opt(v: Option<f64>, digits: usize) -> String {
    v.map_or("-".to_string(), |x| format!("{x:.digits$}"))
}

/// Short human summary.
pub fn markdown(r: &S1Report) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "# kdraft spike S1 report\n");
    let _ = writeln!(
        s,
        "- kicad-cli: `{}` version {} (`{}`)",
        r.kicad_cli.path, r.kicad_cli.version, r.kicad_cli.version_string
    );
    let _ = writeln!(
        s,
        "- board: `{}` (file version {}, generator {})",
        r.board.path,
        r.board.file_version,
        r.board.generator_version.as_deref().unwrap_or("-")
    );
    if let Some(b) = r.board.outline_bbox {
        let _ = writeln!(
            s,
            "- outline: {:.3} x {:.3} mm at ({:.3}, {:.3})..({:.3}, {:.3}); {} footprints, {} pads, {} holes",
            b.width(),
            b.height(),
            b.min.x,
            b.min.y,
            b.max.x,
            b.max.y,
            r.board.footprints,
            r.board.pads,
            r.board.holes
        );
    }
    let _ = writeln!(s, "- kdraft {}\n", r.kdraft_version);

    let _ = writeln!(s, "## SVG exports (PASS = exported, parsed, non-empty)\n");
    let _ = writeln!(s, "| layer | mode 0 | mode 1 | mode 2 |");
    let _ = writeln!(s, "| --- | --- | --- | --- |");
    for l in &r.layers {
        let cell = |m: u8| {
            r.svg_runs
                .iter()
                .find(|x| &x.layer == l && x.mode == m && !x.mirror)
                .map_or("-".to_string(), |x| {
                    if x.note.is_empty() {
                        x.status.as_str().to_string()
                    } else {
                        format!("{} ({})", x.status.as_str(), x.note.replace('|', "/"))
                    }
                })
        };
        let _ = writeln!(s, "| {l} | {} | {} | {} |", cell(0), cell(1), cell(2));
    }

    let _ = writeln!(s, "\n## Transform per mode (board mm -> SVG user units)\n");
    let _ = writeln!(
        s,
        "u = sx * x + ox, v = sy * y + oy. `page mm/mm` is the physical plot scale (1 = 1:1).\n"
    );
    let _ = writeln!(
        s,
        "| mode | status | method | sx | ox | sy | oy | page mm/mm | Y down | mirrored | F.Cu pads <=0.05 mm | max | rms | B.Cu pads <=0.05 mm | Edge.Cuts |"
    );
    let _ = writeln!(s, "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |");
    for m in &r.modes {
        let t = m.fit.as_ref().map(|f| f.transform);
        let cu = |l: &str| m.copper.iter().find(|c| c.layer == l);
        let pct = |c: Option<&CopperCheck>| {
            c.map_or("-".to_string(), |c| {
                format!(
                    "{:.1}% of {} ({})",
                    c.residuals.within_0_05 * 100.0,
                    c.residuals.count,
                    c.status.as_str()
                )
            })
        };
        let f = cu("F.Cu");
        let _ = writeln!(
            s,
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            m.mode,
            m.status.as_str(),
            m.fit.as_ref().map_or("-", |f| f.method.as_str()),
            fmt_opt(t.map(|t| t.sx), 6),
            fmt_opt(t.map(|t| t.ox), 4),
            fmt_opt(t.map(|t| t.sy), 6),
            fmt_opt(t.map(|t| t.oy), 4),
            fmt_opt(m.page_mm_per_board_mm, 6),
            t.map_or("-".to_string(), |t| t.y_down().to_string()),
            t.map_or("-".to_string(), |t| t.mirrored().to_string()),
            pct(f),
            fmt_opt(f.map(|c| c.residuals.max_mm), 4),
            fmt_opt(f.map(|c| c.residuals.rms_mm), 4),
            pct(cu("B.Cu")),
            m.edge.as_ref().map_or("-".to_string(), |e| format!(
                "{} ({})",
                e.status.as_str(),
                e.note
            )),
        );
        for n in &m.notes {
            let _ = writeln!(s, "\nmode {}: {n}", m.mode);
        }
    }
    if let Some(mi) = &r.mirrored {
        let _ = writeln!(s, "\n## Mirrored export (F.Cu, mode {})\n", mi.mode);
        let t = mi.fit.as_ref().map(|f| f.transform);
        let _ = writeln!(
            s,
            "{}: {}; sx {}, ox {}, sy {}, oy {}; pads within 0.05 mm {}; same Y mapping as unmirrored: {}",
            mi.status.as_str(),
            mi.note,
            fmt_opt(t.map(|t| t.sx), 6),
            fmt_opt(t.map(|t| t.ox), 4),
            fmt_opt(t.map(|t| t.sy), 6),
            fmt_opt(t.map(|t| t.oy), 4),
            mi.fit.as_ref().map_or("-".to_string(), |f| format!(
                "{:.1}%",
                f.residuals.within_0_05 * 100.0
            )),
            mi.same_y_mapping.map_or("-".to_string(), |b| b.to_string())
        );
    }
    let _ = writeln!(s, "\n## Flags\n");
    for h in &r.help {
        let missing: Vec<&String> = h
            .flags
            .iter()
            .filter(|(_, v)| !**v)
            .map(|(k, _)| k)
            .collect();
        let _ = writeln!(
            s,
            "- `{} --help`: exit {}; not mentioned: {}",
            h.command,
            h.run.exit_code.map_or("-".to_string(), |c| c.to_string()),
            if missing.is_empty() {
                "none".to_string()
            } else {
                missing
                    .iter()
                    .map(|m| format!("`{m}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        );
    }
    let _ = writeln!(
        s,
        "- rejected by export (removal made it succeed): {}",
        if r.flags_rejected.is_empty() {
            "none".to_string()
        } else {
            r.flags_rejected.join(", ")
        }
    );
    for n in &r.layer_name_notes {
        let _ = writeln!(s, "- layer name: {n}");
    }
    let _ = writeln!(s, "\n## Cross-checks (informational)\n");
    for (name, c) in [("drill", &r.drill), ("pos", &r.pos), ("stats", &r.stats)] {
        let _ = writeln!(s, "- {name}: {}", c.status.as_str());
        for f in &c.findings {
            let _ = writeln!(s, "  - {f}");
        }
    }
    let _ = writeln!(
        s,
        "\n## Drawing sheet via stub board (paper A3 420 x 297 mm)\n"
    );
    for c in &r.sheets {
        let found: Vec<String> = c
            .markers_found
            .iter()
            .map(|(k, v)| format!("{k} {}", if *v { "yes" } else { "no" }))
            .collect();
        let _ = writeln!(
            s,
            "- sheet `{}`: {} ({}); SVG size mm {}; content margins l/t/r/b mm {}; markers in SVG text: {}; `${{` occurrences: {}",
            c.sheet,
            c.status.as_str(),
            c.note,
            c.svg_size_mm.map_or("-".to_string(), |v| format!("{:.3} x {:.3}", v[0], v[1])),
            c.content_margins_mm.map_or("-".to_string(), |m| format!(
                "{:.3}/{:.3}/{:.3}/{:.3}",
                m[0], m[1], m[2], m[3]
            )),
            found.join(", "),
            c.unexpanded_variables
        );
    }
    let _ = writeln!(
        s,
        "\n(Markers can be missing if text is plotted as strokes only; check the SVG visually.)"
    );
    let failed: Vec<&SvgRun> = r.svg_runs.iter().filter(|x| !x.run.success).collect();
    if !failed.is_empty() {
        let _ = writeln!(s, "\n## Failed commands\n");
        for x in failed.iter().take(5) {
            let _ = writeln!(s, "- `{}`: {}", x.run.command_line, first_line(&x.run));
        }
    }
    let _ = writeln!(s, "\n## Verdict\n\nVERDICT: {}", r.verdict);
    s
}

#[cfg(test)]
mod tests;
