use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("unknown command {0:?}")]
    UnknownCommand(String),
    #[error("invalid arguments for {command}: {message}")]
    BadArgs { command: String, message: String },
    #[error(transparent)]
    Op(#[from] kdraft_doc::OpError),
    #[error(transparent)]
    Doc(#[from] kdraft_doc::DocError),
    #[error(transparent)]
    Board(#[from] kdraft_board::BoardError),
    #[error(transparent)]
    Sheet(#[from] kdraft_sheet::SheetError),
    #[error(transparent)]
    Render(#[from] kdraft_render::RenderError),
    #[error(transparent)]
    Source(#[from] kdraft_source::SourceError),
    #[error("{context} {}: {source}", path.display())]
    Io {
        context: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{0}")]
    Invalid(String),
    #[error("no interactive gesture in progress")]
    NoGesture,
}

impl EngineError {
    pub fn io(context: &'static str, path: &std::path::Path, source: std::io::Error) -> Self {
        EngineError::Io {
            context,
            path: path.to_path_buf(),
            source,
        }
    }

    pub fn invalid(m: impl Into<String>) -> Self {
        EngineError::Invalid(m.into())
    }
}

pub type R<T> = Result<T, EngineError>;
