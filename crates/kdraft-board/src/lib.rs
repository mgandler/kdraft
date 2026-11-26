//! Semantic board model read from KiCad 9 and 10 `.kicad_pcb` files.
//!
//! Coordinates are KiCad's: millimetres, Y pointing down, angles in degrees
//! counter-clockwise as seen on screen. See [`geom::Point::rotate_kicad`].
#![forbid(unsafe_code)]

pub mod bom;
mod diff;
mod drill;
pub mod edit;
mod error;
pub mod geom;
mod model;
pub mod outline;
mod parse;

use std::path::Path;

use kdraft_sexpr::Tree;

pub use diff::{
    diff, BoardDiff, ChangedFootprint, FieldChange, FootprintChange, FootprintId, HoleChange,
    OutlineChange,
};
pub use drill::{
    drill_row_index, drill_table, holes, kicad_tool_groups, DrillRow, Hole, HoleKind, HoleSource,
    ToolGroup,
};
pub use edit::{hide_fab_texts, FabTextShow};
pub use error::BoardError;
pub use model::*;

impl Board {
    /// Reads and parses a board file.
    pub fn load(path: impl AsRef<Path>) -> Result<Board, BoardError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|source| BoardError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Board::from_bytes(bytes)
    }

    /// Parses board file bytes; the hash covers exactly these bytes.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Board, BoardError> {
        let hash = BoardHash::of(&bytes);
        let text = String::from_utf8(bytes).map_err(|e| BoardError::NotUtf8 {
            offset: e.utf8_error().valid_up_to(),
        })?;
        let tree = Tree::parse(text)?;
        Board::from_tree(&tree, hash)
    }

    pub fn parse_str(text: &str) -> Result<Board, BoardError> {
        Board::from_bytes(text.as_bytes().to_vec())
    }

    /// Reads the model from an already parsed tree.
    pub fn from_tree(tree: &Tree, hash: BoardHash) -> Result<Board, BoardError> {
        parse::read_board(tree, hash)
    }

    pub fn copper_layer_count(&self) -> usize {
        self.layers.iter().filter(|l| l.is_copper()).count()
    }

    pub fn footprint_by_reference(&self, reference: &str) -> Option<&Footprint> {
        self.footprints
            .iter()
            .find(|f| f.reference.as_deref() == Some(reference))
    }

    pub fn holes(&self) -> Vec<Hole> {
        holes(self)
    }

    pub fn drill_table(&self) -> Vec<DrillRow> {
        drill_table(&self.holes())
    }
}

#[cfg(test)]
mod ext_tests;
#[cfg(test)]
mod tests;
