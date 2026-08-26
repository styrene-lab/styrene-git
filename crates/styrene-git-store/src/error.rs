use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("repository not found: {0}")]
    RepositoryNotFound(styrene_git_core::RepositoryId),
    #[error("repository metadata does not match {0}")]
    RepositoryMismatch(styrene_git_core::RepositoryId),
    #[error("unsupported Git object format: {0}")]
    UnsupportedObjectFormat(String),
    #[error("invalid Git object kind: {0}")]
    InvalidObjectKind(String),
    #[error("Git object identifier does not match the imported object")]
    ObjectIdMismatch,
    #[error("Git pack exceeds the configured {limit}-byte limit")]
    PackTooLarge { limit: u64 },
    #[error("publisher state does not match its repository or namespace")]
    InvalidPublisherState,
    #[error("Git operation `{operation}` failed: {stderr}")]
    Git { operation: String, stderr: String },
    #[error("invalid UTF-8 returned by Git during {0}")]
    InvalidGitOutput(String),
    #[error("invalid object identifier returned by Git: {0}")]
    InvalidObjectId(String),
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Core(#[from] styrene_git_core::CoreError),
}

pub(crate) fn io_error(path: impl Into<PathBuf>, source: std::io::Error) -> StoreError {
    StoreError::Io {
        path: path.into(),
        source,
    }
}
