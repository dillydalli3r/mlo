//! Error type with *named reasons*.
//!
//! The spec forbids bare "error": every refusal carries a stable reason string
//! ([§10.2.3], [§13.1]). `MloError::reason_code()` yields that string, used by
//! the status line, the CLI and the job journal.

use std::path::PathBuf;

pub type Result<T, E = MloError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum MloError {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("config error: {0}")]
    Config(String),

    #[error("tag error in {path}: {reason}")]
    Tag { path: PathBuf, reason: String },

    #[error("unsupported container {container} for {path}: {reason}")]
    UnsupportedContainer {
        path: PathBuf,
        container: String,
        reason: String,
    },

    #[error("decode error for {path}: {reason}")]
    Decode { path: PathBuf, reason: String },

    #[error("network unavailable: {reason}")]
    Network { reason: String },

    #[error("service {service} unavailable: {reason}")]
    Service { service: String, reason: String },

    #[error("tool {tool} unavailable: {reason}")]
    Tool { tool: String, reason: String },

    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid input: {0}")]
    Invalid(String),

    #[error("{0}")]
    Other(String),
}

impl MloError {
    /// Stable machine-readable reason code (never "error").
    pub fn reason_code(&self) -> String {
        match self {
            MloError::Io { .. } => "IO".into(),
            MloError::Db(_) => "DATABASE".into(),
            MloError::Config(_) => "CONFIG".into(),
            MloError::Tag { .. } => "TAG".into(),
            MloError::UnsupportedContainer { .. } => "UNSUPPORTED_CONTAINER".into(),
            MloError::Decode { .. } => "DECODE".into(),
            MloError::Network { .. } => "NETWORK_UNAVAILABLE".into(),
            MloError::Service { .. } => "SERVICE_UNAVAILABLE".into(),
            MloError::Tool { .. } => "TOOL_UNAVAILABLE".into(),
            MloError::NotFound(_) => "NOT_FOUND".into(),
            MloError::Invalid(_) => "INVALID_INPUT".into(),
            MloError::Other(_) => "ERROR".into(),
        }
    }

    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        MloError::Io { path: path.into(), source }
    }

    pub fn tag(path: impl Into<PathBuf>, reason: impl Into<String>) -> Self {
        MloError::Tag { path: path.into(), reason: reason.into() }
    }

    pub fn service(service: impl Into<String>, reason: impl Into<String>) -> Self {
        MloError::Service { service: service.into(), reason: reason.into() }
    }

    pub fn tool(tool: impl Into<String>, reason: impl Into<String>) -> Self {
        MloError::Tool { tool: tool.into(), reason: reason.into() }
    }
}

/// Convenience for the very common "io at this path" mapping.
pub trait IoResultExt<T> {
    fn at(self, path: impl Into<PathBuf>) -> Result<T>;
}

impl<T, E: Into<std::io::Error>> IoResultExt<T> for std::result::Result<T, E> {
    fn at(self, path: impl Into<PathBuf>) -> Result<T> {
        self.map_err(|e| MloError::io(path, e.into()))
    }
}

impl From<std::io::Error> for MloError {
    fn from(e: std::io::Error) -> Self {
        MloError::Io { path: PathBuf::from("<io>"), source: e }
    }
}