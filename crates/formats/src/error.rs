//! Error type shared by every reader in this crate.

use std::fmt;
use std::path::{Path, PathBuf};

/// Result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// An error while reading game data.
///
/// Carries the file it happened in (when known) and a [`ErrorKind`] describing
/// what went wrong. Parsers that work on in-memory bytes return errors without
/// a path; the file-level helpers attach it.
#[derive(Debug)]
pub struct Error {
    /// File the error refers to, if known.
    pub path: Option<PathBuf>,
    /// What went wrong.
    pub kind: ErrorKind,
}

/// The kind of an [`Error`].
#[derive(Debug)]
#[non_exhaustive]
pub enum ErrorKind {
    /// Reading the file failed.
    Io(std::io::Error),
    /// The bytes do not follow the expected layout.
    Malformed {
        /// Format being read (`"level"`, `"Assets.dat"`, ...).
        format: &'static str,
        /// Byte offset where the problem was found, if meaningful.
        offset: Option<usize>,
        /// Human readable description.
        message: String,
    },
    /// A compressed stream (zlib, LZ4, PNG) failed to decode.
    Decode {
        /// Format being decoded.
        format: &'static str,
        /// Decoder message.
        message: String,
    },
    /// Syntax the data-only Lua reader cannot handle.
    Lua {
        /// 1-based line number.
        line: u32,
        /// Description.
        message: String,
    },
    /// Something required is missing (a file, a table, a key).
    Missing(String),
}

impl Error {
    pub(crate) fn malformed(
        format: &'static str,
        offset: Option<usize>,
        message: impl Into<String>,
    ) -> Self {
        Error {
            path: None,
            kind: ErrorKind::Malformed {
                format,
                offset,
                message: message.into(),
            },
        }
    }

    pub(crate) fn decode(format: &'static str, message: impl Into<String>) -> Self {
        Error {
            path: None,
            kind: ErrorKind::Decode {
                format,
                message: message.into(),
            },
        }
    }

    pub(crate) fn lua(line: u32, message: impl Into<String>) -> Self {
        Error {
            path: None,
            kind: ErrorKind::Lua {
                line,
                message: message.into(),
            },
        }
    }

    pub(crate) fn missing(what: impl Into<String>) -> Self {
        Error {
            path: None,
            kind: ErrorKind::Missing(what.into()),
        }
    }

    pub(crate) fn io(path: &Path, source: std::io::Error) -> Self {
        Error {
            path: Some(path.to_path_buf()),
            kind: ErrorKind::Io(source),
        }
    }

    /// Attaches a file path (keeps an existing one).
    pub fn at(mut self, path: &Path) -> Self {
        if self.path.is_none() {
            self.path = Some(path.to_path_buf());
        }
        self
    }

    /// True if the error is an I/O "not found".
    pub fn is_not_found(&self) -> bool {
        matches!(&self.kind, ErrorKind::Io(e) if e.kind() == std::io::ErrorKind::NotFound)
            || matches!(self.kind, ErrorKind::Missing(_))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(p) = &self.path {
            write!(f, "{}: ", p.display())?;
        }
        match &self.kind {
            ErrorKind::Io(e) => write!(f, "{e}"),
            ErrorKind::Malformed {
                format,
                offset: Some(o),
                message,
            } => write!(f, "malformed {format} at byte {o:#x}: {message}"),
            ErrorKind::Malformed {
                format, message, ..
            } => write!(f, "malformed {format}: {message}"),
            ErrorKind::Decode { format, message } => {
                write!(f, "cannot decode {format}: {message}")
            }
            ErrorKind::Lua { line, message } => write!(f, "lua line {line}: {message}"),
            ErrorKind::Missing(what) => write!(f, "missing {what}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            ErrorKind::Io(e) => Some(e),
            _ => None,
        }
    }
}

/// Reads a whole file, attaching the path to errors.
pub(crate) fn read_file(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| Error::io(path, e))
}

/// Reads a text file (UTF-8, lossy; a leading BOM is removed).
pub(crate) fn read_text(path: &Path) -> Result<String> {
    let bytes = read_file(path)?;
    Ok(decode_text(&bytes))
}

/// Decodes text the way the game's files are stored: UTF-8, optional BOM,
/// invalid sequences replaced.
pub(crate) fn decode_text(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}
