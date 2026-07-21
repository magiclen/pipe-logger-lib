use std::{error::Error, fmt, io, path::PathBuf};

/// An error returned when building a [`crate::PipeLogger`].
#[derive(Debug)]
pub enum BuildError {
    /// The rotation file size cannot be zero.
    RotateFileSizeZero,
    /// The total number of log files cannot be zero.
    CountZero,
    /// The count option requires a rotation method.
    CountWithoutRotation,
    /// The compression option requires a rotation method.
    CompressionWithoutRotation,
    /// An XZ compression level must be from 0 to 9.
    InvalidXzCompressionLevel(u32),
    /// A log path cannot be a directory.
    LogPathIsDirectory(PathBuf),
    /// Another logger is already using this log path.
    LogPathAlreadyInUse(PathBuf),
    /// An I/O operation failed.
    Io(io::Error),
}

impl fmt::Display for BuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RotateFileSizeZero => {
                formatter.write_str("The rotation file size cannot be zero.")
            },
            Self::CountZero => formatter.write_str("The total number of log files cannot be zero."),
            Self::CountWithoutRotation => {
                formatter.write_str("The count option requires a rotation method.")
            },
            Self::CompressionWithoutRotation => {
                formatter.write_str("The compression option requires a rotation method.")
            },
            Self::InvalidXzCompressionLevel(level) => {
                write!(formatter, "The XZ compression level `{level}` must be from 0 to 9.")
            },
            Self::LogPathIsDirectory(path) => {
                write!(formatter, "The log path `{}` is a directory.", path.display())
            },
            Self::LogPathAlreadyInUse(path) => {
                write!(formatter, "Another logger is already using `{}`.", path.display())
            },
            Self::Io(error) => fmt::Display::fmt(error, formatter),
        }
    }
}

impl Error for BuildError {
    #[inline]
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for BuildError {
    #[inline]
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
