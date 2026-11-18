use std::{fmt, path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

/// Opaque track identity, shared by local library items and remote items.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TrackId(String);

impl TrackId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TrackId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where the bytes of a track come from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MediaLocation {
    Local { path: PathBuf },
    Remote { url: String },
}

/// Minimal M0 track metadata. Richer tags arrive with the library crate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub location: MediaLocation,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration: Option<Duration>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_round_trips_through_json() {
        let track = Track {
            id: TrackId::new("local:1"),
            location: MediaLocation::Local {
                path: "/music/a.flac".into(),
            },
            title: "A".into(),
            artist: Some("B".into()),
            album: None,
            duration: Some(Duration::from_millis(1234)),
        };

        let json = serde_json::to_string(&track).unwrap();
        assert_eq!(serde_json::from_str::<Track>(&json).unwrap(), track);
        assert!(json.contains("\"duration\":{\"secs\":1"));
    }

    #[test]
    fn remote_location_round_trips() {
        let location = MediaLocation::Remote {
            url: "https://example.com/stream/42".into(),
        };

        let json = serde_json::to_string(&location).unwrap();
        assert_eq!(
            serde_json::from_str::<MediaLocation>(&json).unwrap(),
            location
        );
    }
}
