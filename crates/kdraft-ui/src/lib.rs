//! kdraft desktop editor: a thin egui layer over `kdraft-engine`. Canvas
//! math, hit testing, the drag state machine, background workers and file
//! watching are plain modules with unit tests; `ui` holds the egui code.
#![forbid(unsafe_code)]

pub mod actions;
pub mod anchors;
pub mod app;
mod appx;
pub mod camera;
pub mod dialogs;
pub mod drag;
pub mod handles;
pub mod hit;
pub mod icons;
pub mod jobs;
pub mod layers;
pub mod measure;
pub mod movectx;
pub mod pages;
mod persist;
pub mod rulers;
pub mod settings;
pub mod shot;
pub mod strings;
pub mod summary;
pub mod theme;
pub mod tools;
pub mod tree;
mod ui;
pub mod ui_callout;
mod ui_canvas;
mod ui_handles;
mod ui_inspector;
mod ui_left;
mod ui_link;
mod ui_menu;
pub mod ui_prefs;
pub mod ui_style;
mod ui_textedit;
pub mod vector;
pub mod watch;
pub mod widgets;
pub mod worker;

pub use app::{App, UiAction};
