//! What a unit can fail with.

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The bitstream is malformed, or refers to a picture the decoder does not
    /// have (a reference lost to a dropped unit).
    Invalid(String),
    /// The stream is valid HEVC but not the Mac's shape of it.
    Unsupported(String),
}

impl Error {
    #[cold]
    pub fn invalid(msg: impl Into<String>) -> Self {
        Error::Invalid(msg.into())
    }
    #[cold]
    pub fn unsupported(msg: impl Into<String>) -> Self {
        Error::Unsupported(msg.into())
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Invalid(msg) => write!(f, "invalid stream: {msg}"),
            Error::Unsupported(msg) => write!(f, "unsupported stream: {msg}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<crate::bits::OutOfData> for Error {
    fn from(_: crate::bits::OutOfData) -> Self {
        Error::Invalid("ran out of data".into())
    }
}
