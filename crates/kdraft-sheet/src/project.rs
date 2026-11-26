//! The `.kicad_pro` settings kdraft reads, and drawing sheet selection.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::SheetError;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Project {
    pub text_variables: BTreeMap<String, String>,
    /// `pcbnew.page_layout_descr_file` as written (empty means unset).
    pub board_sheet: Option<String>,
    /// `schematic.page_layout_descr_file`, informational.
    pub schematic_sheet: Option<String>,
}

impl Project {
    pub fn parse(text: &str, path: &Path) -> Result<Project, SheetError> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| SheetError::Project {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;
        let mut p = Project::default();
        if let Some(o) = v.get("text_variables").and_then(|t| t.as_object()) {
            for (k, val) in o {
                if let Some(s) = val.as_str() {
                    p.text_variables.insert(k.clone(), s.to_string());
                }
            }
        }
        let sheet = |section: &str| {
            v.get(section)
                .and_then(|s| s.get("page_layout_descr_file"))
                .and_then(|f| f.as_str())
                .filter(|f| !f.trim().is_empty())
                .map(str::to_string)
        };
        p.board_sheet = sheet("pcbnew");
        p.schematic_sheet = sheet("schematic");
        Ok(p)
    }

    pub fn load(path: &Path) -> Result<Project, SheetError> {
        let text = std::fs::read_to_string(path).map_err(|source| SheetError::Io {
            context: "reading",
            path: path.to_path_buf(),
            source,
        })?;
        Project::parse(&text, path)
    }

    /// The project file next to a board (`x.kicad_pcb` to `x.kicad_pro`),
    /// if it exists.
    pub fn for_board(board: &Path) -> Option<PathBuf> {
        let p = board.with_extension("kicad_pro");
        p.is_file().then_some(p)
    }
}

/// Resolves a sheet path as KiCad stores it: `${KIPRJMOD}` is the project
/// folder, relative paths are relative to it, `\` and `/` both separate.
pub fn resolve_path(raw: &str, project_dir: &Path) -> PathBuf {
    let dir = project_dir.to_string_lossy();
    let s = raw
        .replace("${KIPRJMOD}", &dir)
        .replace("$(KIPRJMOD)", &dir);
    let s = if cfg!(windows) {
        s
    } else {
        s.replace('\\', "/")
    };
    let p = PathBuf::from(&s);
    if p.is_absolute() || is_windows_absolute(&s) {
        p
    } else {
        project_dir.join(p)
    }
}

fn is_windows_absolute(s: &str) -> bool {
    let b = s.as_bytes();
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
        || s.starts_with("\\\\")
}

/// Which sheet to use and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum SheetChoice {
    /// The built-in default (KiCad's default sheet).
    Default,
    File(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SheetOrigin {
    Page,
    Document,
    Project,
    Default,
}

/// Value of a page or document sheet setting that selects the built-in
/// default explicitly.
pub const DEFAULT_SHEET: &str = "default";

/// Selection order: page override, document override, the project's board
/// sheet, the built-in default. Overrides are paths relative to the
/// document folder, or [`DEFAULT_SHEET`].
pub fn select_sheet(
    page: Option<&str>,
    document: Option<&str>,
    doc_dir: &Path,
    project: Option<(&Project, &Path)>,
) -> (SheetChoice, SheetOrigin) {
    let choose = |s: &str| {
        if s == DEFAULT_SHEET {
            SheetChoice::Default
        } else {
            SheetChoice::File(resolve_path(s, doc_dir))
        }
    };
    if let Some(p) = page.filter(|s| !s.is_empty()) {
        return (choose(p), SheetOrigin::Page);
    }
    if let Some(d) = document.filter(|s| !s.is_empty()) {
        return (choose(d), SheetOrigin::Document);
    }
    if let Some((proj, dir)) = project {
        if let Some(s) = &proj.board_sheet {
            return (
                SheetChoice::File(resolve_path(s, dir)),
                SheetOrigin::Project,
            );
        }
    }
    (SheetChoice::Default, SheetOrigin::Default)
}
