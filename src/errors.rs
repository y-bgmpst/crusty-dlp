use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("could not determine the Linux configuration directory")]
    ConfigDirectory,
    #[error("invalid URL: use a complete http:// or https:// URL")]
    InvalidUrl,
    #[error("custom format cannot be empty")]
    EmptyFormat,
}
