//! Server URL normalization and Subsonic authentication parameters.

use std::fmt;
use std::time::Duration;

use md5::{Digest, Md5};
use url::Url;

use crate::error::Error;

pub(crate) const API_VERSION: &str = "1.16.1";
pub(crate) const CLIENT_NAME: &str = "foobar-rs";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);

/// Authentication mode. Both variants are supported by Navidrome.
#[derive(Clone)]
pub enum Auth {
    /// `apiKey` query parameter (OpenSubsonic extension `apiKeyAuthentication`).
    ApiKey(String),
    /// Legacy `u` + `t = md5(password + salt)` + per-request `s`.
    Legacy { user: String, password: String },
}

impl Auth {
    pub fn api_key(key: impl Into<String>) -> Self {
        Self::ApiKey(key.into())
    }

    pub fn legacy(user: impl Into<String>, password: impl Into<String>) -> Self {
        Self::Legacy {
            user: user.into(),
            password: password.into(),
        }
    }

    /// Short label safe to log.
    pub fn mode(&self) -> &'static str {
        match self {
            Self::ApiKey(_) => "API key",
            Self::Legacy { .. } => "legacy token",
        }
    }
}

// Deliberately manual: never derive Debug/Display over secrets.
impl fmt::Debug for Auth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for Auth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ApiKey(_) => f.write_str("API key (redacted)"),
            Self::Legacy { user, .. } => {
                write!(f, "legacy token (user {user:?}, password redacted)")
            }
        }
    }
}

/// Normalized server configuration.
#[derive(Clone)]
pub struct NavidromeConfig {
    server: Url,
    auth: Auth,
    timeout: Duration,
}

impl NavidromeConfig {
    /// Parses and normalizes a server URL. A trailing slash, a trailing
    /// `/rest` and query/fragment are all accepted and stripped.
    pub fn new(server: &str, auth: Auth) -> Result<Self, Error> {
        Ok(Self {
            server: normalize_server(server)?,
            auth,
            timeout: DEFAULT_TIMEOUT,
        })
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn server(&self) -> &Url {
        &self.server
    }

    pub fn auth(&self) -> &Auth {
        &self.auth
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Plain HTTP sends credentials in the clear; callers should warn.
    pub fn is_plaintext(&self) -> bool {
        self.server.scheme() == "http"
    }

    pub(crate) fn endpoint_url(&self, endpoint: &str) -> Url {
        let mut url = self.server.clone();
        let base = self.server.path().trim_end_matches('/');
        url.set_path(&format!("{base}/rest/{endpoint}"));
        url
    }
}

impl fmt::Debug for NavidromeConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for NavidromeConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", redacted_url(&self.server), self.auth)
    }
}

fn normalize_server(input: &str) -> Result<Url, Error> {
    let input = input.trim();
    if input.is_empty() {
        return Err(Error::InvalidUrl("server URL is empty".into()));
    }
    let mut url = Url::parse(input).map_err(|e| Error::InvalidUrl(e.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Error::InvalidUrl(format!(
            "unsupported scheme {:?}, expected http or https",
            url.scheme()
        )));
    }
    if url.host_str().is_none() {
        return Err(Error::InvalidUrl("server URL has no host".into()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Error::InvalidUrl(
            "server URL must not contain credentials; pass them separately".into(),
        ));
    }
    url.set_query(None);
    url.set_fragment(None);
    let path = url.path().trim_end_matches('/').to_string();
    let path = path.strip_suffix("/rest").unwrap_or(&path).to_string();
    url.set_path(if path.is_empty() { "/" } else { &path });
    Ok(url)
}

/// Query parameters carrying credentials. `salt` is only used by the legacy
/// mode; a fresh random salt must be passed for every request.
pub(crate) fn auth_pairs(auth: &Auth, salt: &str) -> Vec<(&'static str, String)> {
    match auth {
        Auth::ApiKey(key) => vec![("apiKey", key.clone())],
        Auth::Legacy { user, password } => {
            let mut hasher = Md5::new();
            hasher.update(format!("{password}{salt}").as_bytes());
            vec![
                ("u", user.clone()),
                ("t", format!("{:x}", hasher.finalize())),
                ("s", salt.to_string()),
            ]
        }
    }
}

/// Renders a URL with credential-bearing query values masked. Used by all
/// `Display`/`Debug` impls in this crate.
pub(crate) fn redacted_url(url: &Url) -> String {
    let mut redacted = url.clone();
    redacted.set_query(None);
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(key, value)| {
            let value = if is_secret_param(&key) {
                "***".to_string()
            } else {
                value.into_owned()
            };
            (key.into_owned(), value)
        })
        .collect();
    redacted.query_pairs_mut().extend_pairs(pairs);
    redacted.to_string()
}

fn is_secret_param(key: &str) -> bool {
    matches!(key, "apiKey" | "t" | "s" | "p" | "password" | "token")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_trailing_slash_and_rest_suffix() {
        let config =
            NavidromeConfig::new(" https://music.example.com/ ", Auth::api_key("k")).unwrap();
        assert_eq!(config.server().as_str(), "https://music.example.com/");
        assert_eq!(
            config.endpoint_url("ping").as_str(),
            "https://music.example.com/rest/ping"
        );

        let config =
            NavidromeConfig::new("https://music.example.com/rest/", Auth::api_key("k")).unwrap();
        assert_eq!(
            config.endpoint_url("search3").as_str(),
            "https://music.example.com/rest/search3"
        );

        let config = NavidromeConfig::new("http://host:4533/sub", Auth::api_key("k")).unwrap();
        assert_eq!(
            config.endpoint_url("ping").as_str(),
            "http://host:4533/sub/rest/ping"
        );
    }

    #[test]
    fn rejects_bad_urls() {
        assert!(NavidromeConfig::new("", Auth::api_key("k")).is_err());
        assert!(NavidromeConfig::new("ftp://host", Auth::api_key("k")).is_err());
        assert!(NavidromeConfig::new("https://user:pass@host", Auth::api_key("k")).is_err());
    }

    #[test]
    fn api_key_does_not_mix_legacy_params() {
        let pairs = auth_pairs(&Auth::api_key("secret-key"), "ignored-salt");
        assert_eq!(pairs, vec![("apiKey", "secret-key".to_string())]);
    }

    #[test]
    fn legacy_token_is_md5_of_password_and_salt() {
        let mut hasher = Md5::new();
        hasher.update(b"hunter2fixed-salt");
        let expected = format!("{:x}", hasher.finalize());
        let pairs = auth_pairs(&Auth::legacy("alice", "hunter2"), "fixed-salt");
        assert_eq!(
            pairs,
            vec![
                ("u", "alice".to_string()),
                ("t", expected),
                ("s", "fixed-salt".to_string()),
            ]
        );
    }

    #[test]
    fn redacts_credentials_in_urls() {
        let url =
            Url::parse("https://host/rest/stream?id=1&apiKey=topsecret&t=abc&s=salt").unwrap();
        let shown = redacted_url(&url);
        assert!(shown.contains("id=1"));
        assert!(!shown.contains("topsecret"));
        assert!(!shown.contains("abc"));
        assert!(!shown.contains("salt"));
        assert_eq!(shown.matches("***").count(), 3);
    }

    #[test]
    fn auth_display_hides_secrets() {
        let shown = Auth::legacy("alice", "hunter2").to_string();
        assert!(!shown.contains("hunter2"));
        let shown = Auth::api_key("topsecret").to_string();
        assert!(!shown.contains("topsecret"));
    }
}
