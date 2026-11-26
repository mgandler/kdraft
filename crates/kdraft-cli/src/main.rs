#![forbid(unsafe_code)]

mod export;
mod report;
mod settings_cmd;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use kdraft_board::Board;

/// Fabrication and assembly drawings for KiCad boards.
#[derive(Parser)]
#[command(name = "kdraft", version, about)]
struct Cli {
    /// Settings file (default: KDRAFT_SETTINGS, else the user's config
    /// folder: %APPDATA%\kdraft\settings.json on Windows).
    #[arg(long, global = true, value_name = "FILE")]
    settings: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Summarise a board: size, layers, stackup, parts, holes, title block.
    Inspect {
        board: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Print the drill table aggregated from pads and vias.
    DrillTable {
        board: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Semantic change report between two boards. Exit code 0 if they are
    /// equivalent, 1 if there are changes.
    Diff {
        old: PathBuf,
        new: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Check that an S-expression file parses and writes back byte for byte.
    SexprCheck { file: PathBuf },
    /// .kdraft document commands.
    #[command(subcommand)]
    Doc(DocCommand),
    /// Render a .kdraft document to PDF (or one SVG per page).
    Export {
        file: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long, value_enum, default_value = "pdf")]
        format: ExportFormat,
        /// Resolution for --format png.
        #[arg(long, default_value_t = 150)]
        dpi: u32,
        /// kicad-cli executable (default: discovery as for `spike s1`).
        #[arg(long)]
        kicad_cli: Option<PathBuf>,
        /// Do not use kicad-cli; draw views natively from the board model.
        #[arg(long, conflicts_with = "kicad_cli")]
        no_kicad_cli: bool,
        /// Draw drawing sheets with kicad-cli from a stub board (page
        /// numbers then come from KiCad and read 1/1).
        #[arg(long)]
        sheet_from_kicad_cli: bool,
        /// Answer kicad-cli calls from recorded outputs in DIR (tests and
        /// sample output on machines without KiCad).
        #[arg(long, value_name = "DIR", hide = true)]
        fake_kicad: Option<PathBuf>,
    },
    /// Drawing checks. Exit code 0 clean, 1 warnings, 2 errors.
    Check {
        file: PathBuf,
        #[arg(long)]
        json: bool,
        /// Check as for an export without kicad-cli (3D views then warn).
        #[arg(long)]
        no_kicad_cli: bool,
    },
    /// Engine commands: the same table the GUI uses.
    #[command(subcommand)]
    Cmd(CmdCommand),
    /// Technical spikes that need a real kicad-cli.
    #[command(subcommand)]
    Spike(SpikeCommand),
    /// User settings shared with the editor.
    #[command(subcommand)]
    Settings(settings_cmd::SettingsCommand),
    /// kicad-cli installs.
    #[command(subcommand)]
    Kicad(settings_cmd::KicadCommand),
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum ExportFormat {
    Pdf,
    Svg,
    Png,
}

#[derive(Subcommand)]
enum CmdCommand {
    /// List all commands with their arguments.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Run commands on a document: `--do NAME [--args JSON]`, repeatable;
    /// each `--args` belongs to the `--do` before it. Saves to `-o`, or
    /// back to the file if anything changed.
    Run {
        file: PathBuf,
        /// Command name (see `kdraft cmd list`).
        #[arg(long = "do", value_name = "NAME", action = clap::ArgAction::Append, required = true)]
        commands: Vec<String>,
        /// JSON arguments for the preceding --do.
        #[arg(long = "args", value_name = "JSON", action = clap::ArgAction::Append)]
        args: Vec<String>,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum SpikeCommand {
    /// S1: export layer SVGs with kicad-cli and derive the board-to-SVG
    /// transform. Writes report.md, report.json, SVGs and overlays into
    /// OUT/<board>-kicad-<version>/.
    S1 {
        board: PathBuf,
        /// Output folder.
        #[arg(long, default_value = "s1-out")]
        out: PathBuf,
        /// kicad-cli executable (default: KDRAFT_KICAD_CLI, PATH, standard
        /// install folders).
        #[arg(long)]
        kicad_cli: Option<PathBuf>,
        /// .kicad_wks for the second stub-sheet export with
        /// --drawing-sheet (default: kdraft's copy of KiCad's default sheet).
        #[arg(long)]
        drawing_sheet: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum DocCommand {
    /// Create a default drawing document for a board.
    New {
        board: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        /// Overwrite an existing output file.
        #[arg(long)]
        force: bool,
        /// kicad-cli to fetch the schematic BOM with (default: found on
        /// PATH or in the KiCad install folders).
        #[arg(long)]
        kicad_cli: Option<PathBuf>,
        /// Do not run kicad-cli: BOM pages from the PCB.
        #[arg(long)]
        no_kicad_cli: bool,
        /// Add one page per layer of a set (copper, silkscreen, fab, mask,
        /// paste, courtyard, user), each with a view linked to the first
        /// page's view (same position, scale and rotation).
        #[arg(long, value_name = "SET")]
        layer_pages: Option<String>,
    },
    /// Read a .kdraft file and report problems.
    Validate { file: PathBuf },
}

/// The sheet a new document for `board` will render with: the project's
/// board sheet if set, else the built-in default.
fn project_sheet(
    board: &Path,
) -> Result<(
    kdraft_sheet::SheetChoice,
    kdraft_sheet::SheetOrigin,
    kdraft_sheet::WksFile,
)> {
    let project = match kdraft_sheet::Project::for_board(board) {
        Some(p) => Some((
            kdraft_sheet::Project::load(&p)?,
            p.parent().map(Path::to_path_buf).unwrap_or_default(),
        )),
        None => None,
    };
    let (choice, origin) = kdraft_sheet::select_sheet(
        None,
        None,
        Path::new("."),
        project.as_ref().map(|(p, d)| (p, d.as_path())),
    );
    let sheet = kdraft_sheet::load(&choice)
        .with_context(|| format!("loading sheet {}", sheet_name(&choice)))?;
    Ok((choice, origin, sheet))
}

fn sheet_name(c: &kdraft_sheet::SheetChoice) -> String {
    match c {
        kdraft_sheet::SheetChoice::Default => "KiCad default".to_string(),
        kdraft_sheet::SheetChoice::File(p) => p.display().to_string(),
    }
}

fn origin_name(o: kdraft_sheet::SheetOrigin) -> &'static str {
    match o {
        kdraft_sheet::SheetOrigin::Page => "page",
        kdraft_sheet::SheetOrigin::Document => "document",
        kdraft_sheet::SheetOrigin::Project => "project",
        kdraft_sheet::SheetOrigin::Default => "default",
    }
}

/// Locates kicad-cli: `--kicad-cli`, then KDRAFT_KICAD_CLI, then the
/// `kicad_cli` setting, then the newest install; adds the settings' extra
/// variables.
fn find_kicad_cli(explicit: Option<&Path>) -> Result<kdraft_source::KicadCli> {
    let runner = kdraft_source::SystemRunner;
    let k = settings_cmd::get().kicad();
    let r = kdraft_source::resolve(explicit, &k.cli, &kdraft_source::SystemEnvironment, &runner)
        .map(|(mut c, _)| {
            c.vars = k.vars.clone();
            c
        });
    r.map_err(|e| {
        anyhow::anyhow!(
            "{e}\n\nkicad-cli is part of KiCad 9/10. Install KiCad, or point kdraft at it:\n  \
             --kicad-cli \"C:\\Program Files\\KiCad\\9.0\\bin\\kicad-cli.exe\"\n  \
             or set the environment variable {} to that path,\n  \
             or `kdraft settings set kicad_cli <path>`.",
            kdraft_source::ENV_VAR
        )
    })
}

fn spike_s1(
    board: &Path,
    out: &Path,
    kicad_cli: Option<&Path>,
    drawing_sheet: Option<PathBuf>,
) -> Result<ExitCode> {
    let cli = find_kicad_cli(kicad_cli)?;
    println!(
        "kicad-cli: {} (version {}, reported \"{}\")",
        cli.path.display(),
        cli.version,
        cli.version_string
    );
    let stem = board
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "board".to_string());
    let dir = out.join(format!("{stem}-kicad-{}", cli.version));
    // Without an explicit sheet, the built-in default is passed with
    // --drawing-sheet, so the run also tests that flag.
    let drawing_sheet = match drawing_sheet {
        Some(p) => p,
        None => {
            std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
            let p = dir.join("kdraft-default.kicad_wks");
            std::fs::write(&p, kdraft_sheet::DEFAULT_SHEET_TEXT)
                .with_context(|| format!("writing {}", p.display()))?;
            p
        }
    };
    let drawing_sheet = Some(drawing_sheet);
    let opts = kdraft_plot::s1::S1Options {
        board: board.to_path_buf(),
        out_dir: dir.clone(),
        drawing_sheet,
    };
    let report = kdraft_plot::s1::run(&opts, &cli, &kdraft_source::SystemRunner, &mut |m| {
        println!("  {m}")
    })
    .with_context(|| format!("spike S1 on {}", board.display()))?;
    println!("wrote {}", dir.join("report.md").display());
    println!("VERDICT: {}", report.verdict);
    Ok(if report.pass {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

fn load(path: &Path) -> Result<Board> {
    Board::load(path).with_context(|| format!("loading {}", path.display()))
}

fn print_json<T: serde::Serialize>(v: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

fn run(cli: Cli) -> Result<ExitCode> {
    settings_cmd::init(cli.settings.as_deref());
    match cli.command {
        Command::Settings(c) => settings_cmd::run(c),
        Command::Kicad(c) => settings_cmd::kicad(c),
        Command::Inspect { board, json } => {
            let b = load(&board)?;
            let r = report::inspect(&board, &b);
            if json {
                print_json(&r)?;
            } else {
                print!("{}", report::inspect_text(&r));
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::DrillTable { board, json } => {
            let b = load(&board)?;
            let rows = b.drill_table();
            if json {
                print_json(&rows)?;
            } else {
                print!("{}", report::drill_table_text(&rows));
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Diff { old, new, json } => {
            let a = load(&old)?;
            let b = load(&new)?;
            let d = kdraft_board::diff(&a, &b);
            if json {
                print_json(&report::DiffReport {
                    identical_bytes: a.hash == b.hash,
                    changed: !d.is_empty(),
                    diff: &d,
                })?;
            } else {
                print!("{}", report::diff_text(&d, a.hash == b.hash));
            }
            Ok(if d.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
        Command::Check {
            file,
            json,
            no_kicad_cli,
        } => {
            let mut s = kdraft_engine::Session::open(&file)
                .with_context(|| format!("opening {}", file.display()))?;
            // An export would find kicad-cli the same way, so 3D views only
            // warn when it is really missing.
            s.kicad_available = !no_kicad_cli && find_kicad_cli(None).is_ok();
            let r = kdraft_engine::check::check(&mut s);
            if json {
                print_json(&r)?;
            } else {
                print!("{}", r.text());
            }
            Ok(ExitCode::from(r.exit_code() as u8))
        }
        Command::SexprCheck { file } => {
            let text = std::fs::read_to_string(&file)
                .with_context(|| format!("reading {}", file.display()))?;
            let tree = kdraft_sexpr::Tree::parse(text.as_str())
                .with_context(|| format!("parsing {}", file.display()))?;
            let out = tree.to_text();
            if out == text {
                println!(
                    "ok: {} bytes, {} nodes, byte-exact round trip",
                    text.len(),
                    tree.node_count()
                );
                Ok(ExitCode::SUCCESS)
            } else {
                let at = out
                    .bytes()
                    .zip(text.bytes())
                    .position(|(a, b)| a != b)
                    .unwrap_or(out.len().min(text.len()));
                let (line, col) = kdraft_sexpr::line_col(&text, at);
                println!("MISMATCH: first difference at {line}:{col} (byte {at})");
                Ok(ExitCode::from(1))
            }
        }
        Command::Doc(DocCommand::New {
            board,
            output,
            force,
            kicad_cli,
            no_kicad_cli,
            layer_pages,
        }) => {
            if output.exists() && !force {
                anyhow::bail!("{} exists; use --force to overwrite", output.display());
            }
            let (choice, origin, _) = project_sheet(&board)?;
            let reg = kdraft_engine::Registry::new();
            // The schematic BOM, so `doc new` and `export` number the BOM
            // from the same source.
            let bom = if no_kicad_cli {
                None
            } else {
                match find_kicad_cli(kicad_cli.as_deref()) {
                    Ok(cli) => {
                        let k = kdraft_engine::pipeline::Kicad {
                            cli,
                            runner: std::sync::Arc::new(kdraft_source::SystemRunner),
                        };
                        let cache = kdraft_engine::pipeline::cache_dir(Some(&output));
                        let mut notes = Vec::new();
                        let b = kdraft_engine::pipeline::fetch_bom(&board, &k, &cache, &mut notes);
                        for n in notes {
                            println!("{n}");
                        }
                        b
                    }
                    Err(e) if kicad_cli.is_some() => return Err(e),
                    Err(_) => {
                        println!("kicad-cli not found: BOM pages from the PCB");
                        None
                    }
                }
            };
            // The user's document defaults (paper, sheet, palette, ...).
            let user = settings_cmd::get();
            let mut s = kdraft_engine::Session::new_for_board_with(
                &board,
                Some(&output),
                bom,
                &user.documents(),
                user.default_palette(),
            )
            .with_context(|| format!("creating a document for {}", board.display()))?;
            if let Some(set) = &layer_pages {
                let view = s
                    .doc()
                    .pages
                    .first()
                    .and_then(|p| p.views().next())
                    .map(|v| v.id.0.clone())
                    .context("--layer-pages: the first page has no view")?;
                let o = reg
                    .execute(
                        &mut s,
                        "view.pages_per_layer",
                        serde_json::json!({ "view": view, "set": set }),
                    )
                    .context("--layer-pages")?;
                for m in o.messages {
                    println!("{m}");
                }
            }
            reg.execute(&mut s, "document.save", serde_json::json!({}))
                .with_context(|| format!("writing {}", output.display()))?;
            let doc = s.doc();
            let scale = doc
                .pages
                .first()
                .and_then(|p| p.views().next())
                .map(|v| v.scale);
            println!(
                "wrote {} ({} pages, view scale {}; sheet {} from {})",
                output.display(),
                doc.pages.len(),
                scale.map_or("-".to_string(), report::fmt_scale),
                sheet_name(&choice),
                origin_name(origin)
            );
            Ok(ExitCode::SUCCESS)
        }
        Command::Export {
            file,
            output,
            format,
            kicad_cli,
            no_kicad_cli,
            sheet_from_kicad_cli,
            dpi,
            fake_kicad,
        } => {
            if !(10..=1200).contains(&dpi) {
                anyhow::bail!("--dpi must be between 10 and 1200");
            }
            let kicad = if let Some(dir) = fake_kicad {
                println!(
                    "kicad-cli: recorded outputs from {} (fixture runner)",
                    dir.display()
                );
                Some((
                    kdraft_source::KicadCli {
                        path: "/fake/kicad-cli".into(),
                        version: kdraft_source::Version::new(10, 0, 8),
                        version_string: kdraft_source::fixture::VERSION.into(),
                        vars: Vec::new(),
                    },
                    std::sync::Arc::new(kdraft_source::fixture::fixture_runner(
                        dir,
                        Box::new(kdraft_render::render3d::fake_board_render),
                    )) as std::sync::Arc<dyn kdraft_source::CommandRunner>,
                ))
            } else if no_kicad_cli {
                None
            } else {
                match find_kicad_cli(kicad_cli.as_deref()) {
                    Ok(cli) => {
                        println!(
                            "kicad-cli: {} (version {})",
                            cli.path.display(),
                            cli.version
                        );
                        Some((
                            cli,
                            std::sync::Arc::new(kdraft_source::SystemRunner)
                                as std::sync::Arc<dyn kdraft_source::CommandRunner>,
                        ))
                    }
                    Err(e) if kicad_cli.is_some() => return Err(e),
                    Err(_) => {
                        println!("kicad-cli not found: views are drawn natively (pass --no-kicad-cli to silence)");
                        None
                    }
                }
            };
            let cache = kdraft_engine::pipeline::cache_dir(Some(&file));
            let r = export::run(&export::Options {
                doc: file,
                out: output,
                format: match format {
                    ExportFormat::Pdf => export::Format::Pdf,
                    ExportFormat::Svg => export::Format::Svg,
                    ExportFormat::Png => export::Format::Png { dpi },
                },
                kicad,
                sheet_from_kicad: sheet_from_kicad_cli,
                cache,
            })?;
            for n in &r.notes {
                println!("note: {n}");
            }
            for f in &r.files {
                println!("wrote {}", f.display());
            }
            println!("{} pages", r.pages);
            Ok(ExitCode::SUCCESS)
        }
        Command::Cmd(CmdCommand::List { json }) => {
            let reg = kdraft_engine::Registry::new();
            if json {
                println!("{}", serde_json::to_string_pretty(&reg.to_json())?);
            } else {
                for c in reg.specs() {
                    let args: Vec<String> = c
                        .args
                        .iter()
                        .map(|a| {
                            if a.required {
                                a.name.to_string()
                            } else {
                                format!("[{}]", a.name)
                            }
                        })
                        .collect();
                    println!("{:<26} {:<20} {}", c.name, c.title, args.join(" "));
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Cmd(CmdCommand::Run {
            file,
            commands,
            args: _,
            output,
        }) => cmd_run(&file, &commands, output.as_deref()),
        Command::Spike(SpikeCommand::S1 {
            board,
            out,
            kicad_cli,
            drawing_sheet,
        }) => spike_s1(&board, &out, kicad_cli.as_deref(), drawing_sheet),
        Command::Doc(DocCommand::Validate { file }) => {
            let text = std::fs::read_to_string(&file)
                .with_context(|| format!("reading {}", file.display()))?;
            let doc = kdraft_doc::Document::parse(&text)
                .with_context(|| format!("parsing {}", file.display()))?;
            let problems = doc.validate();
            for p in &problems {
                println!("{p}");
            }
            if problems.is_empty() {
                let items: usize = doc.pages.iter().map(|p| p.items.len()).sum();
                println!(
                    "ok: version {}, {} pages, {} items",
                    doc.version,
                    doc.pages.len(),
                    items
                );
                Ok(ExitCode::SUCCESS)
            } else {
                Ok(ExitCode::from(1))
            }
        }
    }
}

/// `--do`/`--args` pairs in command-line order (clap's derive API does not
/// keep the interleaving).
fn do_pairs(raw: &[String]) -> Vec<(String, Option<String>)> {
    let mut out: Vec<(String, Option<String>)> = Vec::new();
    let mut it = raw.iter();
    while let Some(a) = it.next() {
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f == "--do" || f == "--args" => (f, Some(v.to_string())),
            _ => (a.as_str(), None),
        };
        let value =
            |it: &mut std::slice::Iter<'_, String>| inline.clone().or_else(|| it.next().cloned());
        match flag {
            "--do" => {
                if let Some(v) = value(&mut it) {
                    out.push((v, None));
                }
            }
            "--args" => {
                if let (Some(v), Some(last)) = (value(&mut it), out.last_mut()) {
                    last.1 = Some(v);
                }
            }
            _ => {}
        }
    }
    out
}

fn cmd_run(file: &Path, names: &[String], output: Option<&Path>) -> Result<ExitCode> {
    let raw: Vec<String> = std::env::args().collect();
    let pairs = do_pairs(&raw);
    // The raw scan must agree with clap's view of the command line.
    let pairs = if pairs.iter().map(|p| &p.0).eq(names.iter()) {
        pairs
    } else {
        names.iter().map(|n| (n.clone(), None)).collect()
    };
    let reg = kdraft_engine::Registry::new();
    let mut s = kdraft_engine::Session::open(file)
        .with_context(|| format!("opening {}", file.display()))?;
    if let Some(e) = &s.board_state().error {
        eprintln!("warning: board not loaded ({e}); views use a placeholder size");
    }
    let start = s.revision();
    for (name, args) in &pairs {
        let v: serde_json::Value = match args {
            Some(a) => serde_json::from_str(a)
                .with_context(|| format!("--args for {name} is not valid JSON"))?,
            None => serde_json::Value::Null,
        };
        let out = reg
            .execute(&mut s, name, v)
            .with_context(|| format!("command {name}"))?;
        println!(
            "{}",
            serde_json::json!({ "command": name, "outcome": out, "selection": s.selection.to_json() })
        );
    }
    let changed = s.revision() != start;
    match output {
        Some(o) => {
            s.save(Some(o))?;
            println!("wrote {}", o.display());
        }
        None if changed => {
            let p = s.save(None)?;
            println!("wrote {}", p.display());
        }
        None => println!("no changes"),
    }
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}
