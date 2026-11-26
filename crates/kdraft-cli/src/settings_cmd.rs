//! `kdraft settings ...` and `kdraft kicad list`, and the user settings
//! the other commands read (kicad-cli choice and variables, cache folder,
//! document defaults).

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use clap::Subcommand;
use kdraft_source::settings::{self, SettingsFile};

#[derive(Subcommand)]
pub enum SettingsCommand {
    /// Print the settings file location.
    Path,
    /// Print the shared settings (kicad-cli, variables, cache, document
    /// defaults).
    Show,
    /// Set one: kicad_cli <path|auto>, define_vars "A=1 B=2", cache_dir
    /// <dir|default>, documents.<paper|portrait|sheet|layer_palette|
    /// render_dpi|render_quality|callout_numbering|snap_grid> <value>.
    Set { key: String, value: String },
    /// Delete the settings file (everything back to the defaults).
    Reset,
    /// Layer colour palettes.
    #[command(subcommand)]
    Palettes(PalettesCommand),
}

#[derive(Subcommand)]
pub enum PalettesCommand {
    /// List built-in and user palettes, and KiCad colour themes found.
    List,
    /// Import a KiCad colour theme (.json) as a palette.
    ImportKicad {
        file: PathBuf,
        /// Palette name (default: the theme's name).
        #[arg(long)]
        name: Option<String>,
        /// Take the colours as they are (default: darken colours too light
        /// for white paper).
        #[arg(long)]
        no_adapt: bool,
    },
}

#[derive(Subcommand)]
pub enum KicadCommand {
    /// List the kicad-cli installs found, with their versions, and the one
    /// kdraft uses.
    List,
}

static FILE: OnceLock<(Option<PathBuf>, SettingsFile)> = OnceLock::new();

/// Loads the settings once (`--settings`, else `KDRAFT_SETTINGS`, else the
/// default location) and applies the cache folder.
pub fn init(flag: Option<&Path>) {
    let path = settings::resolve_path(flag, &kdraft_source::SystemEnvironment);
    let s = match &path {
        Some(p) => {
            let l = SettingsFile::load(p);
            if let Some(w) = l.warning {
                eprintln!("warning: {w}");
            }
            l.settings
        }
        None => SettingsFile::default(),
    };
    kdraft_engine::pipeline::set_cache_root(s.kicad().cache_dir);
    let _ = FILE.set((path, s));
}

pub fn path() -> Option<PathBuf> {
    FILE.get().and_then(|f| f.0.clone())
}

pub fn get() -> SettingsFile {
    FILE.get().map(|f| f.1.clone()).unwrap_or_default()
}

fn save(s: &SettingsFile) -> Result<PathBuf> {
    let p = path().context("no settings location (set KDRAFT_SETTINGS or pass --settings)")?;
    s.save(&p).map_err(anyhow::Error::msg)?;
    Ok(p)
}

pub fn run(cmd: SettingsCommand) -> Result<ExitCode> {
    match cmd {
        SettingsCommand::Path => {
            match path() {
                Some(p) => println!("{}", p.display()),
                None => println!("(no settings location)"),
            }
            Ok(ExitCode::SUCCESS)
        }
        SettingsCommand::Show => {
            if let Some(p) = path() {
                println!("# {}", p.display());
            }
            for (k, v) in get().summary() {
                println!("{k} = {v}");
            }
            Ok(ExitCode::SUCCESS)
        }
        SettingsCommand::Set { key, value } => {
            let mut s = get();
            if key == "documents.layer_palette" && s.palette(&value).is_none() {
                anyhow::bail!("no palette {value:?} (see `kdraft settings palettes list`)");
            }
            s.set_text(&key, &value).map_err(anyhow::Error::msg)?;
            let p = save(&s)?;
            println!("{key} set ({})", p.display());
            Ok(ExitCode::SUCCESS)
        }
        SettingsCommand::Reset => {
            let p = path().context("no settings location")?;
            match std::fs::remove_file(&p) {
                Ok(()) => println!("removed {}", p.display()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    println!("{} does not exist (already the defaults)", p.display())
                }
                Err(e) => return Err(e).with_context(|| format!("removing {}", p.display())),
            }
            Ok(ExitCode::SUCCESS)
        }
        SettingsCommand::Palettes(PalettesCommand::List) => {
            let s = get();
            let default = s.documents().layer_palette;
            for n in kdraft_doc::palette::BUILTIN {
                let mark = if n == default { " (default)" } else { "" };
                println!("{n}  [built-in]{mark}");
            }
            for p in s.palettes() {
                let mark = if p.name == default { " (default)" } else { "" };
                println!("{}  [{} colours]{mark}", p.name, p.colors.len());
            }
            let themes = kdraft_source::kicad_theme::list_themes(&kdraft_source::SystemEnvironment);
            if !themes.is_empty() {
                println!();
                println!("KiCad colour themes found (import with `kdraft settings palettes import-kicad FILE`):");
                for t in themes {
                    println!("  {} (KiCad {}): {}", t.name, t.version, t.path.display());
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        SettingsCommand::Palettes(PalettesCommand::ImportKicad {
            file,
            name,
            no_adapt,
        }) => {
            let imp = kdraft_source::kicad_theme::import_file(&file, name.as_deref(), !no_adapt)
                .map_err(anyhow::Error::msg)?;
            let mut s = get();
            let existed = s.palettes().iter().any(|p| p.name == imp.name);
            let changed = imp
                .original
                .colors
                .iter()
                .filter(|(k, c)| imp.palette.colors.get(*k) != Some(*c))
                .count();
            s.upsert_palette(imp.palette).map_err(anyhow::Error::msg)?;
            let p = save(&s)?;
            println!(
                "{} palette {:?}: {} colours from the theme, {} darkened for white paper ({})",
                if existed { "updated" } else { "imported" },
                imp.name,
                imp.from_file,
                changed,
                p.display()
            );
            Ok(ExitCode::SUCCESS)
        }
    }
}

pub fn kicad(cmd: KicadCommand) -> Result<ExitCode> {
    match cmd {
        KicadCommand::List => {
            let env = kdraft_source::SystemEnvironment;
            let runner = kdraft_source::SystemRunner;
            let installs = kdraft_source::list_installs(&env, &runner);
            if installs.is_empty() {
                println!("no kicad-cli found on PATH or in the standard install folders");
            }
            for i in &installs {
                match (&i.cli, &i.error) {
                    (Some(c), _) => println!("{}  {}", c.version, i.path.display()),
                    (None, e) => println!(
                        "?  {}  ({})",
                        i.path.display(),
                        e.as_deref().unwrap_or("unknown")
                    ),
                }
            }
            let setting = get().kicad().cli;
            match kdraft_source::resolve(None, &setting, &env, &runner) {
                Ok((c, src)) => println!(
                    "in use: {} (version {}, from {})",
                    c.path.display(),
                    c.version,
                    src.label()
                ),
                Err(e) => println!("in use: none ({e})"),
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}
