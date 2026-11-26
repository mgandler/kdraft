//! Layout geometry shared by the renderer, the engine and `doc new`:
//! designator declutter with a spatial index, dimension geometry and
//! helpers for splitting tables. Pure functions on sheet millimetres; no
//! fonts (callers pass measured text widths).
#![forbid(unsafe_code)]

pub mod declutter;
pub mod dimension;
pub mod grid;
pub mod text;
