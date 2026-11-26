//! Plain-English, three-part errors: *what happened · why · exact fix*.

use std::fmt;

/// Every error stagetui reports to a human or an agent.
#[derive(Debug)]
pub enum Error {
    /// A fully explained error.
    Friendly {
        what: String,
        why: String,
        fix: String,
    },
    /// An I/O failure (terminal, pipe, file).
    Io(std::io::Error),
    /// A malformed app description crossing the Python/Rust boundary.
    Json(serde_json::Error),
}

/// Shorthand result type.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// Build a three-part error.
    pub fn friendly(
        what: impl Into<String>,
        why: impl Into<String>,
        fix: impl Into<String>,
    ) -> Self {
        Self::Friendly {
            what: what.into(),
            why: why.into(),
            fix: fix.into(),
        }
    }

    /// The `(what, why, fix)` triple, for every variant.
    pub fn parts(&self) -> (String, String, String) {
        match self {
            Self::Friendly { what, why, fix } => (what.clone(), why.clone(), fix.clone()),
            Self::Io(e) => (
                format!("A terminal or file operation failed: {e}"),
                "the terminal, pipe or file was closed, missing, or not readable".into(),
                "run inside an interactive terminal, or use headless mode (--headless or STAGETUI_HEADLESS=1)"
                    .into(),
            ),
            Self::Json(e) => (
                "The app description could not be read".into(),
                e.to_string(),
                "if you built the app with the Python API this is a stagetui bug; please report it with `stagetui inspect` output"
                    .into(),
            ),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (what, why, fix) = self.parts();
        write!(f, "{what}\n  why: {why}\n  fix: {fix}")
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Json(e) => Some(e),
            Self::Friendly { .. } => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_has_three_parts() {
        let e = Error::friendly(
            "Unknown theme \"neon\"",
            "it is not one of the 5 themes",
            "use \"aurora-dark\"",
        );
        let s = e.to_string();
        assert!(s.starts_with("Unknown theme"));
        assert!(s.contains("\n  why: "));
        assert!(s.contains("\n  fix: "));
    }

    #[test]
    fn io_errors_are_explained() {
        let e: Error = std::io::Error::other("boom").into();
        let (what, why, fix) = e.parts();
        assert!(what.contains("boom"));
        assert!(!why.is_empty() && !fix.is_empty());
    }
}
