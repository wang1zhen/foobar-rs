//! Error types. `Display` output is safe to log: request URLs are stripped
//! before wrapping transport errors and server messages are not echoed.

use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid server URL: {0}")]
    InvalidUrl(String),
    #[error("could not build HTTP client: {0}")]
    Client(String),
    #[error("could not start remote worker: {0}")]
    Worker(String),
    #[error("HTTP request failed: {0}")]
    Transport(String),
    #[error("server returned HTTP {0}")]
    HttpStatus(u16),
    #[error(transparent)]
    Subsonic(#[from] SubsonicError),
    #[error("malformed Subsonic response: {0}")]
    Malformed(String),
    #[error("expected audio but received {0}")]
    NotAudio(String),
    #[error("file I/O failed: {0}")]
    Io(String),
    #[error("network timeout: {0}")]
    Timeout(String),
}

impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Self {
        // The URL carries apiKey/t/s; never format it.
        Self::Transport(error.without_url().to_string())
    }
}

/// A `Subsonic error` (HTTP 200 with `status: "failed"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubsonicError {
    pub code: u32,
    /// Human-readable description of the code. Never the server-provided text.
    pub message: String,
    /// Server text kept for debugging; not printed by `Display`.
    pub server_message: Option<String>,
}

impl SubsonicError {
    pub fn new(code: u32, server_message: impl Into<String>) -> Self {
        let server_message = server_message.into();
        Self {
            code,
            message: code_description(code).to_string(),
            server_message: (!server_message.is_empty()).then_some(server_message),
        }
    }
}

impl fmt::Display for SubsonicError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (Subsonic error {})", self.message, self.code)
    }
}

impl std::error::Error for SubsonicError {}

/// Known Subsonic / OpenSubsonic error codes.
pub fn code_description(code: u32) -> &'static str {
    match code {
        0 => "generic error",
        10 => "a required parameter is missing",
        20 => "client must upgrade to a newer protocol version",
        30 => "server must upgrade to a newer protocol version",
        40 => "wrong username or password",
        41 => "token authentication is not supported for LDAP users",
        42 => "the provided authentication mechanism is not supported",
        43 => "multiple conflicting authentication mechanisms were provided",
        44 => "invalid API key",
        50 => "the user is not authorized for this operation",
        60 => "the trial period is over",
        70 => "the requested data was not found",
        _ => "unknown Subsonic error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_known_codes_to_readable_text() {
        assert_eq!(code_description(40), "wrong username or password");
        for code in [41, 42, 43, 44, 50, 60, 70] {
            assert_ne!(code_description(code), "unknown Subsonic error");
        }
    }

    #[test]
    fn display_hides_server_message() {
        let error = SubsonicError::new(40, "alice/bad-password apiKey=leak");
        let shown = error.to_string();
        assert!(shown.contains("wrong username or password"));
        assert!(shown.contains("40"));
        assert!(!shown.contains("leak"));
        assert!(!shown.contains("bad-password"));
    }
}
