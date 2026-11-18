//! Parser and matcher for automatic playlist queries.
//!
//! Tokens are separated by whitespace and combined with AND. Supported keys are
//! `artist`, `album`, `title`, `format`, `path`, `rating` and `year`. Text
//! values are case-insensitive substrings; values with spaces can be quoted
//! (`artist:"Foo Bar"`). Unkeyed tokens match title, artist or album.
//!
//! Numeric keys accept `>=`, `<=`, `>`, `<`, `=` or an exact value:
//! `rating:>=3`, `year:2000`.

use std::str::FromStr;

use crate::TrackRecord;

/// A parsed automatic playlist query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoPlaylistQuery {
    conditions: Vec<Condition>,
}

/// A query syntax error with its byte position in the original input.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("query error at byte {position}: {message}")]
pub struct QueryError {
    pub position: usize,
    pub message: String,
}

impl QueryError {
    fn new(position: usize, message: impl Into<String>) -> Self {
        Self {
            position,
            message: message.into(),
        }
    }
}

impl AutoPlaylistQuery {
    /// Parse a query. An empty query matches every track.
    pub fn parse(input: &str) -> Result<Self, QueryError> {
        let mut conditions = Vec::new();
        for token in tokenize(input)? {
            if let Some(key) = token.key {
                match key.to_ascii_lowercase().as_str() {
                    "artist" => {
                        conditions.push(Condition::Text(TextField::Artist, lower(&token.value)))
                    }
                    "album" => {
                        conditions.push(Condition::Text(TextField::Album, lower(&token.value)))
                    }
                    "title" => {
                        conditions.push(Condition::Text(TextField::Title, lower(&token.value)))
                    }
                    "format" => {
                        conditions.push(Condition::Text(TextField::Format, lower(&token.value)))
                    }
                    "path" => {
                        conditions.push(Condition::Text(TextField::Path, lower(&token.value)))
                    }
                    "rating" => conditions.push(Condition::Rating(parse_comparison(
                        &token.value,
                        "rating",
                        token.value_position,
                        0,
                        5,
                    )?)),
                    "year" => conditions.push(Condition::Year(parse_comparison(
                        &token.value,
                        "year",
                        token.value_position,
                        1000,
                        9999,
                    )?)),
                    _ => {
                        return Err(QueryError::new(
                            token.position,
                            format!("unknown key `{key}`"),
                        ));
                    }
                }
            } else {
                conditions.push(Condition::Free(lower(&token.value)));
            }
        }
        Ok(Self { conditions })
    }

    /// All conditions must match.
    pub fn matches(&self, track: &TrackRecord) -> bool {
        self.conditions
            .iter()
            .all(|condition| condition.matches(track))
    }
}

impl FromStr for AutoPlaylistQuery {
    type Err = QueryError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Condition {
    Text(TextField, String),
    Free(String),
    Rating(Comparison),
    Year(Comparison),
}

impl Condition {
    fn matches(&self, track: &TrackRecord) -> bool {
        match self {
            Self::Text(field, needle) => match field {
                TextField::Artist => option_contains(track.artist.as_deref(), needle),
                TextField::Album => option_contains(track.album.as_deref(), needle),
                TextField::Title => contains_ci(&track.title, needle),
                TextField::Format => contains_ci(&track.format, needle),
                TextField::Path => contains_ci(&track.path.to_string_lossy(), needle),
            },
            Self::Free(needle) => {
                contains_ci(&track.title, needle)
                    || option_contains(track.artist.as_deref(), needle)
                    || option_contains(track.album.as_deref(), needle)
            }
            Self::Rating(comparison) => comparison.matches(u32::from(track.rating)),
            Self::Year(comparison) => track.year.is_some_and(|year| comparison.matches(year)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextField {
    Artist,
    Album,
    Title,
    Format,
    Path,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Comparison {
    op: CompareOp,
    value: u32,
}

impl Comparison {
    fn matches(self, actual: u32) -> bool {
        match self.op {
            CompareOp::Equal => actual == self.value,
            CompareOp::GreaterOrEqual => actual >= self.value,
            CompareOp::LessOrEqual => actual <= self.value,
            CompareOp::Greater => actual > self.value,
            CompareOp::Less => actual < self.value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompareOp {
    Equal,
    GreaterOrEqual,
    LessOrEqual,
    Greater,
    Less,
}

fn parse_comparison(
    value: &str,
    key: &str,
    position: usize,
    min: u32,
    max: u32,
) -> Result<Comparison, QueryError> {
    let (op, digits) = if let Some(rest) = value.strip_prefix(">=") {
        (CompareOp::GreaterOrEqual, rest)
    } else if let Some(rest) = value.strip_prefix("<=") {
        (CompareOp::LessOrEqual, rest)
    } else if let Some(rest) = value.strip_prefix('>') {
        (CompareOp::Greater, rest)
    } else if let Some(rest) = value.strip_prefix('<') {
        (CompareOp::Less, rest)
    } else if let Some(rest) = value.strip_prefix('=') {
        (CompareOp::Equal, rest)
    } else {
        (CompareOp::Equal, value)
    };

    if digits.is_empty() {
        return Err(QueryError::new(position, format!("missing {key} value")));
    }
    let digits_position = position + (value.len() - digits.len());
    let number = digits
        .parse::<u32>()
        .map_err(|_| QueryError::new(digits_position, format!("invalid {key} value `{digits}`")))?;
    if !(min..=max).contains(&number) {
        return Err(QueryError::new(
            digits_position,
            format!("{key} value {number} is outside {min}..={max}"),
        ));
    }
    Ok(Comparison { op, value: number })
}

fn lower(value: &str) -> String {
    value.to_lowercase()
}

fn contains_ci(haystack: &str, lowered_needle: &str) -> bool {
    haystack.to_lowercase().contains(lowered_needle)
}

fn option_contains(haystack: Option<&str>, lowered_needle: &str) -> bool {
    haystack.is_some_and(|value| contains_ci(value, lowered_needle))
}

#[derive(Debug)]
struct Token {
    position: usize,
    key: Option<String>,
    value: String,
    value_position: usize,
}

fn tokenize(input: &str) -> Result<Vec<Token>, QueryError> {
    let bytes = input.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index].is_ascii_whitespace() {
            index += 1;
            continue;
        }

        let position = index;
        if bytes[index] == b'"' {
            let (value, next) = parse_quoted(input, index)?;
            if value.is_empty() {
                return Err(QueryError::new(position, "empty quoted token"));
            }
            tokens.push(Token {
                position,
                key: None,
                value,
                value_position: position + 1,
            });
            index = next;
            continue;
        }

        let key_start = index;
        while index < bytes.len()
            && !bytes[index].is_ascii_whitespace()
            && bytes[index] != b':'
            && bytes[index] != b'"'
        {
            index += 1;
        }

        if index < bytes.len() && bytes[index] == b':' {
            let key = input[key_start..index].to_owned();
            index += 1;
            let value_position = index;
            let value;
            if index < bytes.len() && bytes[index] == b'"' {
                let (parsed, next) = parse_quoted(input, index)?;
                value = parsed;
                index = next;
            } else {
                let value_start = index;
                while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
                    index += 1;
                }
                value = input[value_start..index].to_owned();
            }
            if value.is_empty() {
                return Err(QueryError::new(
                    value_position,
                    format!("empty value for `{key}`"),
                ));
            }
            tokens.push(Token {
                position,
                key: Some(key),
                value,
                value_position,
            });
        } else if index < bytes.len() && bytes[index] == b'"' {
            return Err(QueryError::new(
                index,
                "unexpected quote; quote the whole token or a value after ':'",
            ));
        } else {
            tokens.push(Token {
                position,
                key: None,
                value: input[key_start..index].to_owned(),
                value_position: position,
            });
        }
    }

    Ok(tokens)
}

fn parse_quoted(input: &str, quote: usize) -> Result<(String, usize), QueryError> {
    let bytes = input.as_bytes();
    let mut index = quote + 1;
    let content_start = index;
    while index < bytes.len() && bytes[index] != b'"' {
        index += 1;
    }
    if index == bytes.len() {
        return Err(QueryError::new(quote, "unterminated quoted string"));
    }

    let value = input[content_start..index].to_owned();
    index += 1;
    if index < bytes.len() && !bytes[index].is_ascii_whitespace() {
        return Err(QueryError::new(
            index,
            "unexpected text after closing quote",
        ));
    }
    Ok((value, index))
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use fb_core::TrackId;

    use super::*;

    fn track() -> TrackRecord {
        TrackRecord {
            id: TrackId::new("local:/music/Foo.flac"),
            path: PathBuf::from("/music/Foo.flac"),
            title: "Foo Song".into(),
            artist: Some("Foo Bar".into()),
            album: Some("Foo Album".into()),
            album_artist: None,
            track_no: Some(1),
            disc_no: None,
            year: Some(2010),
            duration: Some(Duration::from_secs(180)),
            format: "FLAC".into(),
            size: 1000,
            modified: None,
            rating: 4,
            cover_hash: None,
            rg_track_gain: None,
            rg_track_peak: None,
            rg_album_gain: None,
            rg_album_peak: None,
        }
    }

    #[test]
    fn empty_query_matches_everything() {
        let query = AutoPlaylistQuery::parse("   ").unwrap();
        assert!(query.matches(&track()));
    }

    #[test]
    fn text_keys_are_case_insensitive_substrings() {
        let query = AutoPlaylistQuery::parse(
            "artist:\"foo bar\" album:album title:SONG format:flac path:music",
        )
        .unwrap();
        assert!(query.matches(&track()));
    }

    #[test]
    fn quoted_and_free_text_tokens_match() {
        let track = track();
        assert!(
            AutoPlaylistQuery::parse("\"foo bar\"")
                .unwrap()
                .matches(&track)
        );
        assert!(
            AutoPlaylistQuery::parse("album album")
                .unwrap()
                .matches(&track)
        );
        assert!(!AutoPlaylistQuery::parse("missing").unwrap().matches(&track));
    }

    #[test]
    fn numeric_comparisons_work() {
        let track = track();
        for query in [
            "rating:4",
            "rating:=4",
            "rating:>=3",
            "rating:<=5",
            "rating:>3",
            "rating:<5",
            "year:2010",
            "year:>=2000",
            "year:<=2020",
            "year:>2009",
            "year:<2011",
        ] {
            assert!(
                AutoPlaylistQuery::parse(query).unwrap().matches(&track),
                "{query}"
            );
        }
        assert!(
            !AutoPlaylistQuery::parse("rating:>=5")
                .unwrap()
                .matches(&track)
        );
        assert!(
            !AutoPlaylistQuery::parse("year:>2010")
                .unwrap()
                .matches(&track)
        );
    }

    #[test]
    fn multiple_conditions_are_anded() {
        let query =
            AutoPlaylistQuery::parse("artist:foo format:flac rating:>=3 year:>=2000").unwrap();
        assert!(query.matches(&track()));

        let mut quiet = track();
        quiet.rating = 2;
        assert!(!query.matches(&quiet));

        let mut undated = track();
        undated.year = None;
        assert!(
            !AutoPlaylistQuery::parse("year:>=2000")
                .unwrap()
                .matches(&undated)
        );
    }

    #[test]
    fn unknown_keys_and_bad_numbers_report_positions() {
        let input = "artist:foo genre:rock";
        let error = AutoPlaylistQuery::parse(input).unwrap_err();
        assert_eq!(error.position, input.find("genre").unwrap());
        assert!(error.message.contains("unknown key"));

        let input = "rating:>=9";
        let error = AutoPlaylistQuery::parse(input).unwrap_err();
        assert_eq!(error.position, input.find("9").unwrap());
        assert!(error.message.contains("outside 0..=5"));

        let input = "year:abc";
        let error = AutoPlaylistQuery::parse(input).unwrap_err();
        assert_eq!(error.position, input.find("abc").unwrap());
        assert!(error.message.contains("invalid year"));

        let error = AutoPlaylistQuery::parse("artist:\"unterminated").unwrap_err();
        assert!(error.message.contains("unterminated"));
    }

    #[test]
    fn empty_values_are_rejected() {
        for input in ["artist:", "rating:", "artist:\"\""] {
            assert!(AutoPlaylistQuery::parse(input).is_err(), "{input}");
        }
    }
}
