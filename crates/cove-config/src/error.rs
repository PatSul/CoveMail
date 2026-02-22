use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to resolve app directories")]
    MissingDirectories,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("toml parse error: {0}")]
    TomlDe(#[from] toml::de::Error),
    #[error("toml serialize error: {0}")]
    TomlSer(#[from] toml::ser::Error),
}
