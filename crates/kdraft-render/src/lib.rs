//! Rendering: a back-end independent display list, the native board
//! renderer, import of plotted SVGs, view placement, tables, drawing sheets,
//! and SVG and PDF back ends.
//!
//! Page coordinates are millimetres, origin top-left, Y down.
#![forbid(unsafe_code)]

pub mod assembly;
pub mod dims;
pub mod dl;
mod error;
pub mod font;
pub mod graphic;
pub mod import;
pub mod native;
pub mod notes;
pub mod page;
pub mod pdf;
pub mod raster;
pub mod render3d;
pub mod sheet;
pub mod svg_out;
pub mod symbols;
pub mod table;
pub mod view;

pub use error::RenderError;
pub use font::Fonts;
pub use page::{render_document, PageSheet, PlotContent, SheetContext, NATIVE_NOTE};
