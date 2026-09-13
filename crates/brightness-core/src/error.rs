use std::fmt;

#[derive(Debug)]
pub enum Error {
    NotFound(String),
    InvalidBrightness(u8),
    PlatformUnsupported,
    Ddc(String),
    Overlay(String),
    Internal(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "monitor not found: {id}"),
            Self::InvalidBrightness(v) => write!(f, "brightness must be 0-100, got {v}"),
            Self::PlatformUnsupported => write!(f, "brightness control is not supported on this platform"),
            Self::Ddc(msg) => write!(f, "DDC/CI error: {msg}"),
            Self::Overlay(msg) => write!(f, "overlay error: {msg}"),
            Self::Internal(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
