//! Lossless S-expression reader and canonical writer for KiCad's dialect.
//!
//! Two representations:
//!
//! - [`Tree`]: read-only, lossless view over the original text, used for
//!   KiCad files. `Tree::parse(x)?.to_text() == x` for any input that parses.
//! - [`Value`]: owned tree for building and editing documents, written in a
//!   canonical layout by [`Value::to_canonical`] / [`to_canonical`].
#![forbid(unsafe_code)]

pub mod edit;
mod error;
mod escape;
mod tree;
mod value;

#[cfg(test)]
mod proptests;

pub use error::{line_col, Error, ErrorKind};
pub use escape::{escape_into, needs_quotes, unescape};
pub use tree::{parse_f64, Children, Node, NodeKind, ParseOptions, Tree};
pub use value::{format_number, parse_values, to_canonical, Atom, Value};
