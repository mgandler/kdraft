use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum SheetError {
    #[error("{context} {path}: {source}")]
    Io {
        context: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Syntax(#[from] kdraft_sexpr::Error),
    #[error("not a drawing sheet: root is {found:?}, expected kicad_wks or page_layout")]
    NotASheet { found: Option<String> },
    #[error("{path}: not a KiCad project file: {message}")]
    Project { path: PathBuf, message: String },
}
