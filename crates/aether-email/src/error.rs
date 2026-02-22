use thiserror::Error;

#[derive(Debug, Error)]
pub enum EmailError {
    #[error("storage error: {0}")]
    Storage(#[from] aether_storage::StorageError),
    #[error("smtp transport error: {0}")]
    Smtp(String),
    #[error("message build error: {0}")]
    Build(String),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("mail parse error: {0}")]
    Parse(#[from] mailparse::MailParseError),
    #[error("invalid data: {0}")]
    Data(String),
    #[error("unimplemented: {0}")]
    Unimplemented(String),
}
