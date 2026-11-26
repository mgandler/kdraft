//! KiCad drawing sheets: `.kicad_wks` reader, resolution to page geometry,
//! text variables, and the `.kicad_pro` settings that select a sheet.
#![forbid(unsafe_code)]

mod error;
mod project;
mod resolve;
pub mod vars;
mod wks;

pub use error::SheetError;
pub use project::{resolve_path, select_sheet, Project, SheetChoice, SheetOrigin, DEFAULT_SHEET};
pub use resolve::{increment_label, PageSpec, ResolvedSheet, SLine, SPolygon, SRect, SText};
pub use vars::{legacy_to_vars, PageInfo, TextVars};
pub use wks::*;

/// KiCad's default drawing sheet (see `assets/sheets/NOTICE`).
pub const DEFAULT_SHEET_TEXT: &str = include_str!("../../../assets/sheets/default.kicad_wks");

/// The parsed built-in default sheet.
pub fn default_sheet() -> Result<WksFile, SheetError> {
    WksFile::parse(DEFAULT_SHEET_TEXT)
}

/// Loads the chosen sheet.
pub fn load(choice: &SheetChoice) -> Result<WksFile, SheetError> {
    match choice {
        SheetChoice::Default => default_sheet(),
        SheetChoice::File(p) => {
            let text = std::fs::read_to_string(p).map_err(|source| SheetError::Io {
                context: "reading",
                path: p.clone(),
                source,
            })?;
            WksFile::parse(&text)
        }
    }
}

/// ISO A and US paper sizes in mm, portrait (width < height).
pub fn paper_size(name: &str) -> Option<(f64, f64)> {
    Some(match name {
        "A5" => (148.0, 210.0),
        "A4" => (210.0, 297.0),
        "A3" => (297.0, 420.0),
        "A2" => (420.0, 594.0),
        "A1" => (594.0, 841.0),
        "A0" => (841.0, 1189.0),
        "A" | "USLetter" => (215.9, 279.4),
        "B" => (279.4, 431.8),
        "C" => (431.8, 558.8),
        "D" => (558.8, 863.6),
        "E" => (863.6, 1117.6),
        "USLegal" => (215.9, 355.6),
        "USLedger" => (279.4, 431.8),
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
