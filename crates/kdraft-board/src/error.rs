use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum BoardError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("board file is not valid UTF-8 (byte {offset})")]
    NotUtf8 { offset: usize },
    #[error(transparent)]
    Syntax(#[from] kdraft_sexpr::Error),
    #[error("not a KiCad board: root is {found:?}, expected kicad_pcb")]
    NotABoard { found: Option<String> },
    #[error("editing board text: {0}")]
    Edit(String),
}
