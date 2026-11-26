//! Board sources, `kicad-cli` discovery and jobs, and the output cache.
//!
//! Everything that runs a program goes through [`CommandRunner`]; tests use
//! [`FakeRunner`]. Tests needing a real `kicad-cli` are `#[ignore]` and find
//! it via `KDRAFT_KICAD_CLI`.
#![forbid(unsafe_code)]

mod cache;
mod error;
pub mod fixture;
pub mod jobs;
mod kicad_cli;
pub mod kicad_theme;
pub mod private;
mod runner;
pub mod settings;
mod source;
pub mod stub;

pub use cache::{Cache, CacheKey};
pub use error::SourceError;
pub use kicad_cli::{
    candidates, discover, list_installs, resolve, Capability, CliSource, Environment, Install,
    KicadCli, Os, SystemEnvironment, Version, ENV_VAR,
};
pub use private::{private_copy, private_copy_edited, PrivateCopy};
pub use runner::{arg_value, Call, CommandOutput, CommandRunner, FakeRunner, SystemRunner};
pub use source::{run_bom, run_cached, schematic_for, BoardSource, FileBoardSource};
