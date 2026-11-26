#[derive(Debug, thiserror::Error)]
pub enum DocError {
    #[error(transparent)]
    Syntax(#[from] kdraft_sexpr::Error),
    #[error("not a kdraft document: root is {found:?}, expected kdraft")]
    NotADocument { found: Option<String> },
    #[error("document version {found} is not supported (this build reads 1..={supported})")]
    UnsupportedVersion { found: u32, supported: u32 },
    #[error("missing {what} at {line}:{column}")]
    Missing {
        what: String,
        line: usize,
        column: usize,
    },
    #[error("cannot write document: {0}")]
    Write(String),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OpError {
    #[error("no page with id {0}")]
    NoSuchPage(String),
    #[error("no item with id {0}")]
    NoSuchItem(String),
    #[error("page index {index} out of range (0..{len})")]
    PageIndex { index: usize, len: usize },
    #[error("item {0} has no sheet position and cannot be moved")]
    NotMovable(String),
    #[error("item {0} has no known bounding box")]
    NoBounds(String),
}
