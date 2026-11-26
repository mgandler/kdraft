//! `kicad-cli` job builders. A [`Job`] is an argument vector plus where the
//! output goes; it is executed by a [`crate::CommandRunner`] elsewhere.
//!
//! Flags follow the KiCad 10 CLI reference (docs.kicad.org/10.0/en/cli).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::error::SourceError;
use crate::kicad_cli::{Capability, KicadCli};

type R<T> = Result<T, SourceError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arg {
    Lit(String),
    /// A path inside the job's output directory (only for directory output).
    OutputFile(String),
    /// An extra input file; the cache key uses its content hash, not the path.
    InputFile {
        path: PathBuf,
        content_hash: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputKind {
    /// `-o` names a file with this extension.
    File { extension: &'static str },
    /// `-o` names a directory.
    Dir,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    /// Subcommand words, e.g. `pcb export svg`.
    pub command: Vec<&'static str>,
    pub options: Vec<Arg>,
    pub output: OutputKind,
    pub input: PathBuf,
}

/// Name of the output inside a cache entry for file outputs.
pub const OUTPUT_STEM: &str = "out";

impl Job {
    /// File name of the output inside a cache entry (`out.svg`), or `None`
    /// for directory outputs (the entry directory itself).
    pub fn output_file_name(&self) -> Option<String> {
        match self.output {
            OutputKind::File { extension } => Some(format!("{OUTPUT_STEM}.{extension}")),
            OutputKind::Dir => None,
        }
    }

    /// Full argument vector writing to `output` (a file or a directory as
    /// given by [`Job::output`]).
    pub fn args(&self, output: &Path) -> Vec<OsString> {
        let mut v: Vec<OsString> = self.command.iter().map(OsString::from).collect();
        v.push("-o".into());
        match self.output {
            // UNVERIFIED: kicad-cli 8 needed a trailing separator for
            // directory outputs; it is added to be safe.
            OutputKind::Dir => {
                let mut s = output.as_os_str().to_os_string();
                s.push(std::path::MAIN_SEPARATOR_STR);
                v.push(s);
            }
            OutputKind::File { .. } => v.push(output.as_os_str().to_os_string()),
        }
        for a in &self.options {
            match a {
                Arg::Lit(s) => v.push(s.into()),
                Arg::OutputFile(name) => v.push(output.join(name).into_os_string()),
                Arg::InputFile { path, .. } => v.push(path.as_os_str().to_os_string()),
            }
        }
        v.push(self.input.as_os_str().to_os_string());
        v
    }

    /// Arguments that determine the output, without any paths: the input is
    /// covered by its content hash, the output location is irrelevant.
    pub fn cache_args(&self) -> Vec<String> {
        let mut v: Vec<String> = self.command.iter().map(|s| s.to_string()).collect();
        for a in &self.options {
            match a {
                Arg::Lit(s) => v.push(s.clone()),
                Arg::OutputFile(name) => v.push(format!("<out>/{name}")),
                Arg::InputFile { content_hash, .. } => v.push(format!("<file:{content_hash}>")),
            }
        }
        v
    }
}

fn lit(s: impl Into<String>) -> Arg {
    Arg::Lit(s.into())
}

fn flag(v: &mut Vec<Arg>, on: bool, name: &str) {
    if on {
        v.push(lit(name));
    }
}

fn opt(v: &mut Vec<Arg>, name: &str, value: impl Into<String>) {
    v.push(lit(name));
    v.push(lit(value));
}

fn variant(v: &mut Vec<Arg>, cli: &KicadCli, variant: &Option<String>) -> R<()> {
    if let Some(name) = variant {
        cli.require(Capability::Variant)?;
        opt(v, "--variant", name.clone());
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageSizeMode {
    /// Whole sheet including frame (KiCad default, `0`).
    #[default]
    Sheet,
    /// Current page size (`1`).
    Current,
    /// Board area only (`2`).
    Board,
}

/// How kicad-cli draws DNP footprints on fab layers (`--hdnp`, `--sdnp`,
/// `--cdnp`; in the 10.0.8 help text).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnpOnFab {
    Hide,
    Sketch,
    CrossOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrillShapeOpt {
    None,
    Small,
    Actual,
}

/// `pcb export svg --mode-single`: one SVG file with the given layers.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SvgExport {
    pub layers: Vec<String>,
    pub mirror: bool,
    pub black_and_white: bool,
    pub page_size_mode: PageSizeMode,
    pub exclude_drawing_sheet: bool,
    /// `None` keeps KiCad's default (actual size).
    pub drill_shape: Option<DrillShapeOpt>,
    pub theme: Option<String>,
    pub variant: Option<String>,
    /// `--drawing-sheet`: a `.kicad_wks` replacing the board's own sheet
    /// (only meaningful with the drawing sheet included).
    pub drawing_sheet: Option<PathBuf>,
    /// `--sketch-pads-on-fab-layers`.
    pub sketch_pads_on_fab: bool,
    pub dnp_on_fab: Option<DnpOnFab>,
    /// `--define-var KEY=VALUE`, in order.
    pub define_vars: Vec<(String, String)>,
}

impl SvgExport {
    /// A single layer with kdraft's defaults: no drawing sheet.
    pub fn layer(name: &str) -> SvgExport {
        SvgExport {
            layers: vec![name.to_string()],
            exclude_drawing_sheet: true,
            ..Default::default()
        }
    }

    pub fn job(&self, input: &Path, cli: &KicadCli) -> R<Job> {
        if self.layers.is_empty() {
            return Err(SourceError::InvalidJob(
                "svg export needs at least one layer".into(),
            ));
        }
        if let Some(bad) = self.layers.iter().find(|l| l.is_empty() || l.contains(',')) {
            return Err(SourceError::InvalidJob(format!(
                "invalid layer name {bad:?}"
            )));
        }
        let mut v = Vec::new();
        opt(&mut v, "--layers", self.layers.join(","));
        // Accepted by kicad-cli 10.0.8 (S1). UNVERIFIED: --mode-single on
        // KiCad 9.0.x.
        flag(&mut v, true, "--mode-single");
        let mode = match self.page_size_mode {
            PageSizeMode::Sheet => "0",
            PageSizeMode::Current => "1",
            PageSizeMode::Board => "2",
        };
        opt(&mut v, "--page-size-mode", mode);
        flag(
            &mut v,
            self.exclude_drawing_sheet,
            "--exclude-drawing-sheet",
        );
        if let Some(d) = self.drill_shape {
            let n = match d {
                DrillShapeOpt::None => "0",
                DrillShapeOpt::Small => "1",
                DrillShapeOpt::Actual => "2",
            };
            opt(&mut v, "--drill-shape-opt", n);
        }
        flag(&mut v, self.black_and_white, "--black-and-white");
        flag(&mut v, self.mirror, "--mirror");
        if let Some(t) = &self.theme {
            opt(&mut v, "--theme", t.clone());
        }
        if let Some(ws) = &self.drawing_sheet {
            // Accepted by `pcb export svg` on kicad-cli 10.0.8 (S1).
            // UNVERIFIED on KiCad 9.0.x.
            let bytes = std::fs::read(ws).map_err(|e| SourceError::io("reading", ws, e))?;
            v.push(lit("--drawing-sheet"));
            v.push(Arg::InputFile {
                path: ws.clone(),
                content_hash: blake3::hash(&bytes).to_hex().to_string(),
            });
        }
        // In the kicad-cli 10.0.8 `pcb export svg --help` text (fixture
        // fixtures/kicad-cli/10.0.8). UNVERIFIED on KiCad 9.0.x.
        flag(
            &mut v,
            self.sketch_pads_on_fab,
            "--sketch-pads-on-fab-layers",
        );
        if let Some(d) = self.dnp_on_fab {
            v.push(lit(match d {
                DnpOnFab::Hide => "--hide-DNP-footprints-on-fab-layers",
                DnpOnFab::Sketch => "--sketch-DNP-footprints-on-fab-layers",
                DnpOnFab::CrossOut => "--crossout-DNP-footprints-on-fab-layers",
            }));
        }
        let extra = cli
            .vars
            .iter()
            .filter(|(k, _)| !self.define_vars.iter().any(|(d, _)| d == k));
        for (k, val) in self.define_vars.iter().chain(extra) {
            if k.is_empty() || k.contains('=') {
                return Err(SourceError::InvalidJob(format!(
                    "invalid variable name {k:?}"
                )));
            }
            opt(&mut v, "--define-var", format!("{k}={val}"));
        }
        variant(&mut v, cli, &self.variant)?;
        Ok(Job {
            command: vec!["pcb", "export", "svg"],
            options: v,
            output: OutputKind::File { extension: "svg" },
            input: input.to_path_buf(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DrillFormat {
    #[default]
    Excellon,
    Gerber,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapFormat {
    Pdf,
    GerberX2,
    Ps,
    Dxf,
    Svg,
}

/// `pcb export drill` into a directory, optionally with map and report.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DrillExport {
    pub format: DrillFormat,
    pub generate_map: bool,
    pub map_format: Option<MapFormat>,
    pub generate_report: bool,
}

/// Report file name inside the drill output directory.
pub const DRILL_REPORT: &str = "drill-report.rpt";

impl DrillExport {
    pub fn job(&self, input: &Path, _cli: &KicadCli) -> R<Job> {
        let mut v = Vec::new();
        let f = match self.format {
            DrillFormat::Excellon => "excellon",
            DrillFormat::Gerber => "gerber",
        };
        opt(&mut v, "--format", f);
        if self.generate_map {
            v.push(lit("--generate-map"));
            if let Some(m) = self.map_format {
                let m = match m {
                    MapFormat::Pdf => "pdf",
                    MapFormat::GerberX2 => "gerberx2",
                    MapFormat::Ps => "ps",
                    MapFormat::Dxf => "dxf",
                    MapFormat::Svg => "svg",
                };
                opt(&mut v, "--map-format", m);
            }
        } else if self.map_format.is_some() {
            return Err(SourceError::InvalidJob(
                "map format given without generate_map".into(),
            ));
        }
        if self.generate_report {
            v.push(lit("--generate-report"));
            v.push(lit("--report-path"));
            // An absolute path inside the output directory; kicad-cli 10.0.8
            // wrote the report there (S1). UNVERIFIED on KiCad 9.0.x.
            v.push(Arg::OutputFile(DRILL_REPORT.to_string()));
        }
        Ok(Job {
            command: vec!["pcb", "export", "drill"],
            options: v,
            output: OutputKind::Dir,
            input: input.to_path_buf(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Units {
    #[default]
    Mm,
    In,
}

impl Units {
    fn as_str(self) -> &'static str {
        match self {
            Units::Mm => "mm",
            Units::In => "in",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PosSide {
    Front,
    Back,
    #[default]
    Both,
}

/// `pcb export pos --format csv`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PosExport {
    pub units: Units,
    pub side: PosSide,
    pub exclude_dnp: bool,
    pub variant: Option<String>,
}

impl PosExport {
    pub fn job(&self, input: &Path, cli: &KicadCli) -> R<Job> {
        let mut v = Vec::new();
        opt(&mut v, "--format", "csv");
        opt(&mut v, "--units", self.units.as_str());
        let side = match self.side {
            PosSide::Front => "front",
            PosSide::Back => "back",
            PosSide::Both => "both",
        };
        opt(&mut v, "--side", side);
        flag(&mut v, self.exclude_dnp, "--exclude-dnp");
        variant(&mut v, cli, &self.variant)?;
        Ok(Job {
            command: vec!["pcb", "export", "pos"],
            options: v,
            output: OutputKind::File { extension: "csv" },
            input: input.to_path_buf(),
        })
    }
}

/// `pcb export stats --format json`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StatsExport {
    pub units: Units,
}

impl StatsExport {
    pub fn job(&self, input: &Path, _cli: &KicadCli) -> R<Job> {
        // Runs on kicad-cli 10.0.8 (S1). UNVERIFIED: availability of
        // `pcb export stats` on KiCad 9.0.x.
        let mut v = Vec::new();
        opt(&mut v, "--format", "json");
        opt(&mut v, "--units", self.units.as_str());
        Ok(Job {
            command: vec!["pcb", "export", "stats"],
            options: v,
            output: OutputKind::File { extension: "json" },
            input: input.to_path_buf(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderSide {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
    Front,
    Back,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderQuality {
    #[default]
    Basic,
    High,
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderBackground {
    #[default]
    Default,
    Transparent,
    Opaque,
}

/// `pcb render` to PNG.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderJob {
    pub width: u32,
    pub height: u32,
    pub side: RenderSide,
    pub quality: RenderQuality,
    pub background: RenderBackground,
    pub variant: Option<String>,
    /// `--perspective` (orthographic otherwise).
    pub perspective: bool,
    /// `--zoom` camera zoom (kicad-cli default 1). kdraft asks for
    /// fractional zooms (three decimals); UNVERIFIED that kicad-cli 10.0.8
    /// honours fractions (the help does not give a type).
    pub zoom: Option<f64>,
    /// `--rotate 'X,Y,Z'` degrees around the pivot, e.g. `-45,0,45`
    /// (isometric).
    pub rotate: Option<String>,
    /// `--define-var KEY=VALUE`, in order.
    pub define_vars: Vec<(String, String)>,
    /// `--preset` appearance preset (kicad-cli default
    /// `follow_plot_settings`).
    pub preset: Option<String>,
    /// `--use-board-stackup-colors`.
    pub stackup_colors: bool,
    /// `--floor`: floor, shadows and post-processing.
    pub floor: bool,
}

impl Default for RenderJob {
    fn default() -> Self {
        RenderJob {
            width: 1600,
            height: 900,
            side: RenderSide::Top,
            quality: RenderQuality::Basic,
            background: RenderBackground::Default,
            variant: None,
            perspective: false,
            zoom: None,
            rotate: None,
            define_vars: Vec::new(),
            preset: None,
            stackup_colors: false,
            floor: false,
        }
    }
}

impl RenderJob {
    pub fn job(&self, input: &Path, cli: &KicadCli) -> R<Job> {
        if self.width == 0 || self.height == 0 {
            return Err(SourceError::InvalidJob(
                "render size must be non-zero".into(),
            ));
        }
        let mut v = Vec::new();
        // Long forms: `-h` is the height here, not help.
        opt(&mut v, "--width", self.width.to_string());
        opt(&mut v, "--height", self.height.to_string());
        let side = match self.side {
            RenderSide::Top => "top",
            RenderSide::Bottom => "bottom",
            RenderSide::Left => "left",
            RenderSide::Right => "right",
            RenderSide::Front => "front",
            RenderSide::Back => "back",
        };
        opt(&mut v, "--side", side);
        let q = match self.quality {
            RenderQuality::Basic => "basic",
            RenderQuality::High => "high",
            RenderQuality::User => "user",
        };
        opt(&mut v, "--quality", q);
        let b = match self.background {
            RenderBackground::Default => "default",
            RenderBackground::Transparent => "transparent",
            RenderBackground::Opaque => "opaque",
        };
        opt(&mut v, "--background", b);
        // All flags present in the real kicad-cli 10.0.8 `pcb render
        // --help` (fixtures/kicad-cli/10.0.8/pcb_render.txt, S1 on the
        // user's board: none rejected). The render itself is not yet
        // exercised with real kicad-cli. UNVERIFIED on KiCad 9.0.x.
        if let Some(p) = &self.preset {
            if p.is_empty() {
                return Err(SourceError::InvalidJob("empty render preset".into()));
            }
            opt(&mut v, "--preset", p.clone());
        }
        flag(&mut v, self.stackup_colors, "--use-board-stackup-colors");
        flag(&mut v, self.floor, "--floor");
        flag(&mut v, self.perspective, "--perspective");
        if let Some(z) = self.zoom {
            if !(z.is_finite() && z > 0.0) {
                return Err(SourceError::InvalidJob(format!(
                    "zoom must be positive, got {z}"
                )));
            }
            let t = format!("{:.3}", z);
            let t = t.trim_end_matches('0').trim_end_matches('.').to_string();
            opt(&mut v, "--zoom", t);
        }
        if let Some(r) = &self.rotate {
            let ok = r.split(',').count() == 3
                && r.split(',')
                    .all(|x| x.trim().parse::<f64>().is_ok_and(f64::is_finite));
            if !ok {
                return Err(SourceError::InvalidJob(format!(
                    "rotate must be 'X,Y,Z' degrees, got {r:?}"
                )));
            }
            opt(&mut v, "--rotate", r.clone());
        }
        let extra = cli
            .vars
            .iter()
            .filter(|(k, _)| !self.define_vars.iter().any(|(d, _)| d == k));
        for (k, val) in self.define_vars.iter().chain(extra) {
            if k.is_empty() || k.contains('=') {
                return Err(SourceError::InvalidJob(format!(
                    "invalid variable name {k:?}"
                )));
            }
            opt(&mut v, "--define-var", format!("{k}={val}"));
        }
        variant(&mut v, cli, &self.variant)?;
        Ok(Job {
            command: vec!["pcb", "render"],
            options: v,
            output: OutputKind::File { extension: "png" },
            input: input.to_path_buf(),
        })
    }
}

/// `sch export bom` from a schematic.
///
/// Every flag kdraft emits is in the real kicad-cli 10.0.8 `sch export bom
/// --help` (fixtures/kicad-cli/10.0.8/sch_export_bom.txt; S1 on the
/// user's board: none rejected). The export itself has not yet run with
/// a real kicad-cli. UNVERIFIED on KiCad 9.0.x.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BomExport {
    /// Ordered field list; `None` keeps KiCad's default.
    pub fields: Option<Vec<String>>,
    pub labels: Option<Vec<String>>,
    pub group_by: Option<Vec<String>>,
    pub exclude_dnp: bool,
    pub variant: Option<String>,
    /// `--string-delimiter` (KiCad default: none).
    pub string_delimiter: Option<String>,
    /// `--ref-range-delimiter`; `Some("")` disables ranges.
    pub ref_range_delimiter: Option<String>,
}

/// Field names kdraft asks for, with their labels.
pub const BOM_FIELDS: [(&str, &str); 5] = [
    ("Reference", "Refs"),
    ("Value", "Value"),
    ("Footprint", "Footprint"),
    ("${QUANTITY}", "Qty"),
    ("${DNP}", "DNP"),
];

impl BomExport {
    /// kdraft's request: references, value, footprint, quantity and DNP
    /// (plus `extra` fields), grouped by value and footprint, every field
    /// quoted with `"`, no reference ranges (kdraft compresses them).
    pub fn kdraft(extra: &[String]) -> BomExport {
        let mut fields: Vec<String> = BOM_FIELDS.iter().map(|f| f.0.to_string()).collect();
        let mut labels: Vec<String> = BOM_FIELDS.iter().map(|f| f.1.to_string()).collect();
        for e in extra {
            fields.push(e.clone());
            labels.push(e.clone());
        }
        BomExport {
            fields: Some(fields),
            labels: Some(labels),
            group_by: Some(vec!["Value".into(), "Footprint".into()]),
            exclude_dnp: false,
            variant: None,
            string_delimiter: Some("\"".into()),
            ref_range_delimiter: Some(String::new()),
        }
    }

    pub fn job(&self, schematic: &Path, cli: &KicadCli) -> R<Job> {
        let mut v = Vec::new();
        for (name, list) in [
            ("--fields", &self.fields),
            ("--labels", &self.labels),
            ("--group-by", &self.group_by),
        ] {
            if let Some(l) = list {
                if l.iter().any(|f| f.contains(',')) {
                    return Err(SourceError::InvalidJob(format!(
                        "{name} entries must not contain ','"
                    )));
                }
                opt(&mut v, name, l.join(","));
            }
        }
        flag(&mut v, self.exclude_dnp, "--exclude-dnp");
        if let Some(d) = &self.string_delimiter {
            opt(&mut v, "--string-delimiter", d.clone());
        }
        if let Some(d) = &self.ref_range_delimiter {
            opt(&mut v, "--ref-range-delimiter", d.clone());
        }
        variant(&mut v, cli, &self.variant)?;
        Ok(Job {
            command: vec!["sch", "export", "bom"],
            options: v,
            output: OutputKind::File { extension: "csv" },
            input: schematic.to_path_buf(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kicad_cli::Version;

    fn cli(major: u32) -> KicadCli {
        KicadCli {
            path: "/usr/bin/kicad-cli".into(),
            version: Version::new(major, 0, 1),
            version_string: format!("{major}.0.1"),
            vars: Vec::new(),
        }
    }

    fn strs(v: Vec<OsString>) -> Vec<String> {
        v.into_iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect()
    }

    fn help(name: &str) -> String {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/kicad-cli/10.0.8")
            .join(name);
        std::fs::read_to_string(p).unwrap()
    }

    /// `flag` occurs in `text` as a whole word.
    fn mentions(text: &str, flag: &str) -> bool {
        text.match_indices(flag).any(|(i, _)| {
            let before = text[..i].chars().next_back();
            let after = text[i + flag.len()..].chars().next();
            !before.is_some_and(|c| c.is_alphanumeric() || c == '-')
                && !after.is_some_and(|c| c.is_alphanumeric() || c == '-' || c == '_')
        })
    }

    fn flags(j: &Job) -> Vec<String> {
        j.cache_args()
            .into_iter()
            .filter(|a| a.starts_with('-'))
            .collect()
    }

    /// Every flag the builders can emit for these commands appears in the
    /// real kicad-cli 10.0.8 help texts.
    #[test]
    fn emitted_flags_are_in_the_real_10_0_8_help() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("a.kicad_wks");
        std::fs::write(&ws, "(kicad_wks)").unwrap();
        let mut svg_jobs = Vec::new();
        for dnp in [DnpOnFab::Hide, DnpOnFab::Sketch, DnpOnFab::CrossOut] {
            for d in [
                DrillShapeOpt::None,
                DrillShapeOpt::Small,
                DrillShapeOpt::Actual,
            ] {
                let e = SvgExport {
                    layers: vec!["F.Fab".into()],
                    mirror: true,
                    black_and_white: true,
                    page_size_mode: PageSizeMode::Board,
                    exclude_drawing_sheet: true,
                    drill_shape: Some(d),
                    theme: Some("t".into()),
                    variant: Some("v".into()),
                    drawing_sheet: Some(ws.clone()),
                    sketch_pads_on_fab: true,
                    dnp_on_fab: Some(dnp),
                    define_vars: vec![("A".into(), "1".into())],
                };
                svg_jobs.push(e.job(Path::new("b"), &cli(10)).unwrap());
            }
        }
        let drill = DrillExport {
            format: DrillFormat::Gerber,
            generate_map: true,
            map_format: Some(MapFormat::Svg),
            generate_report: true,
        }
        .job(Path::new("b"), &cli(10))
        .unwrap();
        let pos = PosExport {
            units: Units::Mm,
            side: PosSide::Back,
            exclude_dnp: true,
            variant: Some("v".into()),
        }
        .job(Path::new("b"), &cli(10))
        .unwrap();
        let stats = StatsExport { units: Units::Mm }
            .job(Path::new("b"), &cli(10))
            .unwrap();
        let render = RenderJob {
            side: RenderSide::Bottom,
            quality: RenderQuality::High,
            background: RenderBackground::Transparent,
            variant: Some("v".into()),
            perspective: true,
            zoom: Some(1.75),
            rotate: Some("-45,0,45".into()),
            define_vars: vec![("KIPRJMOD".into(), "/p".into())],
            preset: Some("follow_pcb_editor".into()),
            stackup_colors: true,
            floor: true,
            ..Default::default()
        }
        .job(Path::new("b"), &cli(10))
        .unwrap();
        let mut bom = BomExport::kdraft(&["MPN".into()]);
        bom.exclude_dnp = true;
        bom.variant = Some("v".into());
        let bom = bom.job(Path::new("b.kicad_sch"), &cli(10)).unwrap();
        let mut checked = 0;
        for (file, jobs) in [
            ("pcb_export_svg.txt", svg_jobs),
            ("pcb_export_drill.txt", vec![drill]),
            ("pcb_export_pos.txt", vec![pos]),
            ("pcb_export_stats.txt", vec![stats]),
            ("pcb_render.txt", vec![render]),
            ("sch_export_bom.txt", vec![bom]),
        ] {
            let text = help(file);
            for j in &jobs {
                for f in flags(j) {
                    assert!(mentions(&text, &f), "{f} not in {file}");
                    checked += 1;
                }
                // `-o` is added by Job::args.
                assert!(mentions(&text, "--output"));
            }
        }
        assert!(checked > 55, "{checked}");
        // The isometric preset matches the help's own example.
        assert!(help("pcb_render.txt").contains("'-45,0,45' for isometric"));
        // Confirmed absent on 10.0.8: no option hides reference or value
        // texts on fab layers.
        let svg = help("pcb_export_svg.txt").to_ascii_lowercase();
        let names: Vec<&str> = svg
            .split(|c: char| c.is_whitespace() || c == '[' || c == ']' || c == ',')
            .filter(|w| w.starts_with("--"))
            .collect();
        assert!(names.contains(&"--sketch-pads-on-fab-layers"), "{names:?}");
        assert!(
            !names
                .iter()
                .any(|n| n.contains("ref") || n.contains("value") || n.contains("text")),
            "{names:?}"
        );
    }

    #[test]
    fn svg_single_layer() {
        let j = SvgExport::layer("F.Fab")
            .job(Path::new("/b/x.kicad_pcb"), &cli(9))
            .unwrap();
        assert_eq!(
            strs(j.args(Path::new("/c/out.svg"))),
            [
                "pcb",
                "export",
                "svg",
                "-o",
                "/c/out.svg",
                "--layers",
                "F.Fab",
                "--mode-single",
                "--page-size-mode",
                "0",
                "--exclude-drawing-sheet",
                "/b/x.kicad_pcb"
            ]
        );
        assert_eq!(
            j.cache_args(),
            [
                "pcb",
                "export",
                "svg",
                "--layers",
                "F.Fab",
                "--mode-single",
                "--page-size-mode",
                "0",
                "--exclude-drawing-sheet"
            ]
        );
        assert_eq!(j.output_file_name().as_deref(), Some("out.svg"));
    }

    #[test]
    fn svg_all_options_and_gating() {
        let mut e = SvgExport {
            layers: vec!["B.Cu".into(), "B.Mask".into()],
            mirror: true,
            black_and_white: true,
            page_size_mode: PageSizeMode::Board,
            exclude_drawing_sheet: true,
            drill_shape: Some(DrillShapeOpt::Small),
            theme: Some("KiCad Classic".into()),
            variant: Some("lite".into()),
            drawing_sheet: None,
            ..Default::default()
        };
        assert!(matches!(
            e.job(Path::new("b"), &cli(9)),
            Err(SourceError::Unsupported {
                feature: "--variant",
                ..
            })
        ));
        let j = e.job(Path::new("b"), &cli(10)).unwrap();
        assert_eq!(
            j.cache_args()[3..],
            [
                "--layers",
                "B.Cu,B.Mask",
                "--mode-single",
                "--page-size-mode",
                "2",
                "--exclude-drawing-sheet",
                "--drill-shape-opt",
                "1",
                "--black-and-white",
                "--mirror",
                "--theme",
                "KiCad Classic",
                "--variant",
                "lite"
            ]
        );
        // A drawing sheet is passed by path but keyed by content.
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("a.kicad_wks");
        std::fs::write(&ws, "(kicad_wks)").unwrap();
        let mut with_sheet = e.clone();
        with_sheet.variant = None;
        with_sheet.drawing_sheet = Some(ws.clone());
        let j = with_sheet.job(Path::new("b"), &cli(9)).unwrap();
        let args: Vec<String> = j
            .args(Path::new("o.svg"))
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let i = args.iter().position(|a| a == "--drawing-sheet").unwrap();
        assert_eq!(PathBuf::from(&args[i + 1]), ws);
        let key = j.cache_args();
        assert!(key.iter().any(|a| a.starts_with("<file:")));
        assert!(!key.iter().any(|a| a.contains("a.kicad_wks")));
        with_sheet.drawing_sheet = Some(dir.path().join("missing.kicad_wks"));
        assert!(with_sheet.job(Path::new("b"), &cli(9)).is_err());

        e.layers = vec![];
        assert!(matches!(
            e.job(Path::new("b"), &cli(10)),
            Err(SourceError::InvalidJob(_))
        ));
        e.layers = vec!["F.Cu,B.Cu".into()];
        assert!(matches!(
            e.job(Path::new("b"), &cli(10)),
            Err(SourceError::InvalidJob(_))
        ));
    }

    #[test]
    fn drill_with_map_and_report() {
        let j = DrillExport {
            format: DrillFormat::Excellon,
            generate_map: true,
            map_format: Some(MapFormat::Svg),
            generate_report: true,
        }
        .job(Path::new("/b/x.kicad_pcb"), &cli(9))
        .unwrap();
        assert_eq!(j.output, OutputKind::Dir);
        assert_eq!(j.output_file_name(), None);
        let sep = std::path::MAIN_SEPARATOR_STR;
        let report = Path::new("/c/d")
            .join(DRILL_REPORT)
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            strs(j.args(Path::new("/c/d"))),
            [
                "pcb".to_string(),
                "export".into(),
                "drill".into(),
                "-o".into(),
                format!("/c/d{sep}"),
                "--format".into(),
                "excellon".into(),
                "--generate-map".into(),
                "--map-format".into(),
                "svg".into(),
                "--generate-report".into(),
                "--report-path".into(),
                report,
                "/b/x.kicad_pcb".into()
            ]
        );
        assert!(j
            .cache_args()
            .contains(&"<out>/drill-report.rpt".to_string()));
        let bad = DrillExport {
            map_format: Some(MapFormat::Pdf),
            ..Default::default()
        };
        assert!(bad.job(Path::new("b"), &cli(9)).is_err());
    }

    #[test]
    fn pos_stats_render_bom() {
        let pos = PosExport {
            side: PosSide::Front,
            exclude_dnp: true,
            ..Default::default()
        };
        assert_eq!(
            pos.job(Path::new("b"), &cli(9)).unwrap().cache_args(),
            [
                "pcb",
                "export",
                "pos",
                "--format",
                "csv",
                "--units",
                "mm",
                "--side",
                "front",
                "--exclude-dnp"
            ]
        );
        let pos = PosExport {
            variant: Some("v".into()),
            ..Default::default()
        };
        assert!(pos.job(Path::new("b"), &cli(9)).is_err());
        assert!(pos
            .job(Path::new("b"), &cli(10))
            .unwrap()
            .cache_args()
            .ends_with(&["--variant".into(), "v".into()]));

        let st = StatsExport { units: Units::In }
            .job(Path::new("b"), &cli(10))
            .unwrap();
        assert_eq!(
            st.cache_args(),
            ["pcb", "export", "stats", "--format", "json", "--units", "in"]
        );
        assert_eq!(st.output_file_name().as_deref(), Some("out.json"));

        let r = RenderJob {
            side: RenderSide::Bottom,
            quality: RenderQuality::High,
            background: RenderBackground::Transparent,
            ..Default::default()
        };
        assert_eq!(
            r.job(Path::new("b"), &cli(9)).unwrap().cache_args(),
            [
                "pcb",
                "render",
                "--width",
                "1600",
                "--height",
                "900",
                "--side",
                "bottom",
                "--quality",
                "high",
                "--background",
                "transparent"
            ]
        );
        assert!(RenderJob {
            width: 0,
            ..Default::default()
        }
        .job(Path::new("b"), &cli(9))
        .is_err());

        let bom = BomExport {
            fields: Some(vec![
                "Reference".into(),
                "Value".into(),
                "${QUANTITY}".into(),
            ]),
            labels: Some(vec!["Refs".into(), "Value".into(), "Qty".into()]),
            group_by: Some(vec!["Value".into()]),
            exclude_dnp: true,
            variant: Some("A".into()),
            ..Default::default()
        };
        let j = bom.job(Path::new("/p/x.kicad_sch"), &cli(10)).unwrap();
        assert_eq!(
            j.cache_args(),
            [
                "sch",
                "export",
                "bom",
                "--fields",
                "Reference,Value,${QUANTITY}",
                "--labels",
                "Refs,Value,Qty",
                "--group-by",
                "Value",
                "--exclude-dnp",
                "--variant",
                "A"
            ]
        );
        assert_eq!(j.input, PathBuf::from("/p/x.kicad_sch"));
        assert!(bom.job(Path::new("s"), &cli(9)).is_err());
        // kdraft's own request: quoted fields, no ranges (empty argument).
        let k = BomExport::kdraft(&["MPN".into()])
            .job(Path::new("s"), &cli(10))
            .unwrap();
        let args = k.cache_args();
        let i = args
            .iter()
            .position(|a| a == "--ref-range-delimiter")
            .unwrap();
        assert_eq!(args[i + 1], "");
        assert!(args.contains(&"Reference,Value,Footprint,${QUANTITY},${DNP},MPN".to_string()));
        assert!(args.contains(&"\"".to_string()));
    }
}
