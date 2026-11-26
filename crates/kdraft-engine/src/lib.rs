//! The editing engine shared by the CLI and the GUI: sessions, the command
//! registry, undo/redo, item bounds, snapping and the render pipeline.
//! Nothing here depends on a GUI toolkit.
#![forbid(unsafe_code)]

pub mod bounds;
pub mod check;
mod commands;
pub mod commands_callout;
mod commands_geom;
mod commands_link;
mod commands_style;
pub mod enable;
mod error;
pub mod layer_pages;
pub mod links;
pub mod meta;
pub mod pipeline;
pub mod registry;
pub mod session;
pub mod snap;

pub use commands::{default_layers, paper, DUPLICATE_OFFSET};
pub use commands_link::layer_set;
pub use error::EngineError;
pub use registry::{ArgSpec, ArgType, CommandKind, CommandSpec, Outcome, Registry};
pub use session::{relative_board_path, BoardState, Selection, Session};

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_black;
#[cfg(test)]
mod tests_callout;
#[cfg(test)]
mod tests_geom;
#[cfg(test)]
mod tests_link_colours;
#[cfg(test)]
mod tests_links;
#[cfg(test)]
mod tests_palette;
