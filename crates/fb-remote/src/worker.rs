//! Background worker for the synchronous UI thread.
//!
//! A dedicated thread owns a current-thread tokio runtime and at most one
//! connected [`NavidromeClient`]. Commands are sent from the UI, events are
//! polled with [`RemoteHandle::try_recv`]. Failures are reported as
//! [`RemoteEvent::Error`]; the worker never panics on server responses and
//! never prints credentials.
//!
//! Downloads are cache-then-play: the whole track lands in a local file before
//! playback. Streaming while downloading, cancellation and resumable/partial
//! downloads are not implemented yet. Every event echoes the `session` of the
//! command that caused it; downloads and scrobbles also echo a caller `request`
//! generation, so replies for the same track can be told apart. A download
//! aborts after 30 seconds without data but has no overall time limit.

use std::fmt;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use tokio::sync::mpsc::{self as tokio_mpsc, UnboundedReceiver, UnboundedSender};

use crate::auth::Auth;
use crate::client::{NavidromeClient, StreamOptions};
use crate::error::{Error, Result};
use crate::model::{Album, SearchQuery, ServerInfo, Song};
use crate::NavidromeConfig;

const NOT_CONNECTED: &str = "未连接";

#[derive(Debug)]
pub enum RemoteCommand {
    Connect {
        session: u64,
        url: String,
        auth: Auth,
    },
    ListAlbums {
        session: u64,
        offset: u32,
        limit: u32,
    },
    Search {
        session: u64,
        query: String,
        limit: u32,
    },
    AlbumTracks {
        session: u64,
        album_id: String,
    },
    Download {
        session: u64,
        request: u64,
        id: String,
        dest: PathBuf,
        format: Option<String>,
    },
    Scrobble {
        session: u64,
        request: u64,
        id: String,
        submission: bool,
        /// Unix milliseconds (OpenSubsonic); passed through unchanged.
        time: Option<u64>,
    },
    Disconnect,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteOperation {
    Connect,
    ListAlbums,
    Search,
    AlbumTracks,
    Download {
        request: u64,
        id: String,
    },
    Scrobble {
        request: u64,
        id: String,
        submission: bool,
    },
}

#[derive(Debug)]
pub enum RemoteEvent {
    Connected {
        session: u64,
        server: ServerInfo,
        extensions: Vec<String>,
    },
    Albums {
        session: u64,
        albums: Vec<Album>,
        offset: u32,
        has_more: bool,
    },
    Search {
        session: u64,
        albums: Vec<Album>,
        songs: Vec<Song>,
    },
    AlbumTracks {
        session: u64,
        album_id: String,
        songs: Vec<Song>,
    },
    Downloaded {
        session: u64,
        request: u64,
        id: String,
        path: PathBuf,
        bytes: u64,
    },
    Scrobbled {
        session: u64,
        request: u64,
        id: String,
        submission: bool,
    },
    Error {
        session: u64,
        operation: RemoteOperation,
        message: String,
    },
}

/// Cheap, non-blocking handle to the worker thread.
pub struct RemoteHandle {
    commands: UnboundedSender<RemoteCommand>,
    events: Receiver<RemoteEvent>,
    thread: Option<JoinHandle<()>>,
}

impl RemoteHandle {
    pub fn spawn() -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| Error::Worker(error.to_string()))?;
        let (command_tx, command_rx) = tokio_mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("fb-remote".into())
            .spawn(move || runtime.block_on(worker_loop(command_rx, event_tx)))
            .map_err(|error| Error::Worker(error.to_string()))?;
        Ok(Self {
            commands: command_tx,
            events: event_rx,
            thread: Some(thread),
        })
    }

    /// Drops the command if the worker has already stopped.
    pub fn send(&self, command: RemoteCommand) {
        let _ = self.commands.send(command);
    }

    pub fn try_recv(&self) -> Option<RemoteEvent> {
        self.events.try_recv().ok()
    }

    /// Sends `Shutdown` and waits for the worker. A JSON request is bounded by
    /// the HTTP timeout and a download aborts after 30 seconds without data, but
    /// a download has no overall limit and cannot be cancelled, so shutdown can
    /// still wait for a long transfer to finish.
    pub fn shutdown(&mut self) {
        let _ = self.commands.send(RemoteCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for RemoteHandle {
    fn drop(&mut self) {
        // Do not join here: dropping the handle must not block the UI.
        let _ = self.commands.send(RemoteCommand::Shutdown);
    }
}

async fn worker_loop(mut commands: UnboundedReceiver<RemoteCommand>, events: Sender<RemoteEvent>) {
    let mut client: Option<NavidromeClient> = None;

    while let Some(command) = commands.recv().await {
        let event = match command {
            RemoteCommand::Connect { session, url, auth } => {
                let operation = RemoteOperation::Connect;
                match connect(&url, auth).await {
                    Ok((connected, server, extensions)) => {
                        client = Some(connected);
                        RemoteEvent::Connected {
                            session,
                            server,
                            extensions,
                        }
                    }
                    Err(error) => failure(session, operation, &error),
                }
            }
            RemoteCommand::ListAlbums {
                session,
                offset,
                limit,
            } => {
                let operation = RemoteOperation::ListAlbums;
                match &client {
                    Some(client) => match client.album_list2(offset, limit).await {
                        Ok(albums) => RemoteEvent::Albums {
                            session,
                            has_more: albums.len() == limit as usize,
                            albums,
                            offset,
                        },
                        Err(error) => failure(session, operation, &error),
                    },
                    None => not_connected(session, operation),
                }
            }
            RemoteCommand::Search {
                session,
                query,
                limit,
            } => {
                let operation = RemoteOperation::Search;
                match &client {
                    Some(client) => {
                        let mut search = SearchQuery::new(query);
                        search.album_count = Some(limit);
                        search.song_count = Some(limit);
                        match client.search3(&search).await {
                            Ok(results) => RemoteEvent::Search {
                                session,
                                albums: results.albums,
                                songs: results.songs,
                            },
                            Err(error) => failure(session, operation, &error),
                        }
                    }
                    None => not_connected(session, operation),
                }
            }
            RemoteCommand::AlbumTracks { session, album_id } => {
                let operation = RemoteOperation::AlbumTracks;
                match &client {
                    Some(client) => match client.album(&album_id).await {
                        Ok(album) => RemoteEvent::AlbumTracks {
                            session,
                            album_id,
                            songs: album.songs,
                        },
                        Err(error) => failure(session, operation, &error),
                    },
                    None => not_connected(session, operation),
                }
            }
            RemoteCommand::Download {
                session,
                request,
                id,
                dest,
                format,
            } => {
                let operation = RemoteOperation::Download {
                    request,
                    id: id.clone(),
                };
                match &client {
                    Some(client) => {
                        let options = StreamOptions {
                            format,
                            ..StreamOptions::default()
                        };
                        match client.download(&id, &options, &dest).await {
                            Ok(bytes) => RemoteEvent::Downloaded {
                                session,
                                request,
                                id,
                                path: dest,
                                bytes,
                            },
                            Err(error) => failure(session, operation, &error),
                        }
                    }
                    None => not_connected(session, operation),
                }
            }
            RemoteCommand::Scrobble {
                session,
                request,
                id,
                submission,
                time,
            } => {
                let operation = RemoteOperation::Scrobble {
                    request,
                    id: id.clone(),
                    submission,
                };
                match &client {
                    Some(client) => match client.scrobble(&id, submission, time).await {
                        Ok(()) => RemoteEvent::Scrobbled {
                            session,
                            request,
                            id,
                            submission,
                        },
                        Err(error) => failure(session, operation, &error),
                    },
                    None => not_connected(session, operation),
                }
            }
            RemoteCommand::Disconnect => {
                client = None;
                continue;
            }
            RemoteCommand::Shutdown => break,
        };

        if events.send(event).is_err() {
            break;
        }
    }
}

async fn connect(url: &str, auth: Auth) -> Result<(NavidromeClient, ServerInfo, Vec<String>)> {
    let config = NavidromeConfig::new(url, auth)?;
    let client = NavidromeClient::new(config)?;
    let server = client.ping().await?;
    let extensions = client.extensions().await?;
    Ok((
        client,
        server,
        extensions.names().map(str::to_string).collect(),
    ))
}

fn failure(session: u64, operation: RemoteOperation, error: impl fmt::Display) -> RemoteEvent {
    RemoteEvent::Error {
        session,
        operation,
        message: error.to_string(),
    }
}

fn not_connected(session: u64, operation: RemoteOperation) -> RemoteEvent {
    failure(session, operation, NOT_CONNECTED)
}
