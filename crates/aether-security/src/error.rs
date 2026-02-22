use thiserror::Error;

#[derive(Debug, Error)]
pub enum SecurityError {
    #[error("keychain error: {0}")]
    Keychain(#[from] keyring::Error),
    #[error("oauth error: {0}")]
    OAuth(String),
    #[error("request error: {0}")]
    Request(#[from] reqwest::Error),
    #[error("url parse error: {0}")]
    Url(#[from] url::ParseError),
}
