use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("{context} {path}: {source}")]
    Io {
        context: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Board(#[from] kdraft_board::BoardError),
    #[error("cannot start {program}: {source}")]
    Spawn {
        program: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("kicad-cli not found; looked in: {}", fmt_paths(searched))]
    CliNotFound { searched: Vec<PathBuf> },
    #[error("KDRAFT_KICAD_CLI points to {0}, which is not a file")]
    CliEnvInvalid(PathBuf),
    #[error("the kicad-cli set in the settings, {0}, is not a file (choose another in Settings > KiCad, or `kdraft settings set kicad_cli auto`)")]
    CliSettingInvalid(PathBuf),
    #[error("--kicad-cli {0} is not a file")]
    CliFlagInvalid(PathBuf),
    #[error("cannot read the kicad-cli version from {output:?}")]
    CliVersion { output: String },
    #[error("{feature} needs KiCad {needs} or newer; kicad-cli is {have}")]
    Unsupported {
        feature: &'static str,
        needs: &'static str,
        have: String,
    },
    #[error("invalid job: {0}")]
    InvalidJob(String),
    #[error("{program} {args} failed ({status}): {stderr}")]
    CommandFailed {
        program: PathBuf,
        args: String,
        status: String,
        stderr: String,
    },
    #[error("command succeeded but did not write {0}")]
    MissingOutput(PathBuf),
}

fn fmt_paths(p: &[PathBuf]) -> String {
    p.iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

impl SourceError {
    pub(crate) fn io(
        context: &'static str,
        path: impl Into<PathBuf>,
        source: std::io::Error,
    ) -> Self {
        SourceError::Io {
            context,
            path: path.into(),
            source,
        }
    }
}
