use thiserror::Error;

#[derive(Debug, Error)]
pub enum CalendarError {
    #[error("storage error: {0}")]
    Storage(#[from] cove_storage::StorageError),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("invalid data: {0}")]
    Data(String),
    #[error("unimplemented: {0}")]
    Unimplemented(String),
}
