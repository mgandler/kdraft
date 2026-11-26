//! Reading `kicad-cli` plot output and relating it to the board model.
//!
//! - [`svg`]: generic SVG geometry reader (any producer).
//! - [`fit`]: derives the board-mm to SVG-user-unit transform from an
//!   Edge.Cuts plot and pad centres; nothing about KiCad's SVG layout is
//!   assumed.
//! - [`s1`]: spike S1, runs the exports and writes a report.
#![forbid(unsafe_code)]

mod error;
#[cfg(any(test, feature = "test-support"))]
pub mod fake;
pub mod fit;
pub mod path;
pub mod s1;
pub mod svg;

pub use error::PlotError;
