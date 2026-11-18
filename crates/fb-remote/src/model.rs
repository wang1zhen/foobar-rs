//! Wire types for the Subsonic JSON responses we consume. Unknown fields are
//! ignored, so partial/incomplete servers still parse.

use serde::Deserialize;

/// Payload of `ping` (also the shared header fields of every response).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default, rename = "type")]
    pub server_type: Option<String>,
    #[serde(default)]
    pub server_version: Option<String>,
    #[serde(default)]
    pub open_subsonic: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Extensions {
    #[serde(default, rename = "openSubsonicExtensions")]
    pub extensions: Vec<OpenSubsonicExtension>,
}

impl Extensions {
    pub fn supports(&self, name: &str) -> bool {
        self.extensions.iter().any(|ext| ext.name == name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.extensions.iter().map(|ext| ext.name.as_str())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenSubsonicExtension {
    pub name: String,
    #[serde(default)]
    pub versions: Vec<u32>,
}

/// Search parameters. Counts and offsets are independent per entity type, as
/// required by `search3`; `None` leaves the parameter out entirely.
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    pub query: String,
    pub artist_count: Option<u32>,
    pub artist_offset: Option<u32>,
    pub album_count: Option<u32>,
    pub album_offset: Option<u32>,
    pub song_count: Option<u32>,
    pub song_offset: Option<u32>,
}

impl SearchQuery {
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            ..Self::default()
        }
    }

    pub(crate) fn params(&self) -> Vec<(&'static str, String)> {
        let mut params = vec![("query", self.query.clone())];
        let mut push = |key: &'static str, value: Option<u32>| {
            if let Some(value) = value {
                params.push((key, value.to_string()));
            }
        };
        push("artistCount", self.artist_count);
        push("artistOffset", self.artist_offset);
        push("albumCount", self.album_count);
        push("albumOffset", self.album_offset);
        push("songCount", self.song_count);
        push("songOffset", self.song_offset);
        params
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult3 {
    #[serde(default, rename = "artist")]
    pub artists: Vec<Artist>,
    #[serde(default, rename = "album")]
    pub albums: Vec<Album>,
    #[serde(default, rename = "song")]
    pub songs: Vec<Song>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artist {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub album_count: Option<u32>,
    #[serde(default)]
    pub cover_art: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub artist_id: Option<String>,
    #[serde(default)]
    pub cover_art: Option<String>,
    #[serde(default)]
    pub song_count: Option<u32>,
    #[serde(default)]
    pub year: Option<u32>,
}

/// Payload of `getAlbumList2`; the array key is the singular `album`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AlbumList2 {
    #[serde(default, rename = "album")]
    pub album: Vec<Album>,
}

/// Payload of `getAlbum`. `album` is `None` when the server omits it.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AlbumDetail {
    #[serde(default)]
    pub album: Option<AlbumWithSongs>,
}

/// An album plus its track list. Album fields are flattened at this level.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AlbumWithSongs {
    #[serde(flatten)]
    pub album: Album,
    #[serde(default, rename = "song")]
    pub songs: Vec<Song>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Song {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub album_id: Option<String>,
    #[serde(default)]
    pub duration: Option<u32>,
    #[serde(default)]
    pub track: Option<u32>,
    #[serde(default)]
    pub suffix: Option<String>,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Search3Response {
    #[serde(default)]
    pub search_result3: Option<SearchResult3>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AlbumList2Response {
    #[serde(default)]
    pub album_list2: Option<AlbumList2>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct EmptyResponse {}
