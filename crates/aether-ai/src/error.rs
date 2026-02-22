use thiserror::Error;

#[derive(Debug, Error)]
pub enum AiError {
    #[error("security error: {0}")]
    Security(#[from] aether_security::SecurityError),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("utf8 error: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("invalid config: {0}")]
    Config(String),
    #[error("inference error: {0}")]
    Inference(String),
    #[error("cloud ai feature not allowed: {0}")]
    CloudOptInRequired(String),
}
