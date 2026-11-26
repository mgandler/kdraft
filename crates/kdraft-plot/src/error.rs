use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum PlotError {
    #[error("XML: {0}")]
    Xml(String),
    #[error("root element is <{0}>, not <svg>")]
    NotSvg(String),
    #[error("{context} {path}: {source}")]
    Io {
        context: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Source(#[from] kdraft_source::SourceError),
    #[error(transparent)]
    Board(#[from] kdraft_board::BoardError),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
}

impl PlotError {
    pub(crate) fn io(
        context: &'static str,
        path: impl Into<PathBuf>,
        source: std::io::Error,
    ) -> Self {
        PlotError::Io {
            context,
            path: path.into(),
            source,
        }
    }
}
