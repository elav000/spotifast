//! Main-thread presentation model. Secrets stay in the backend worker.
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

use super::{Item, MusicLibrary};
use crate::{backend::Waker, paths::AppDirs, settings::ProxyConfig};
use serde::{Deserialize, Serialize};

pub use crate::provider::Provider;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Source {
    pub id: String,
    pub server_url: String,
    pub server_id: String,
    pub user_id: String,
    pub device_id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum View {
    #[default]
    Libraries,
    Albums {
        library_id: String,
        name: String,
    },
    Tracks {
        library_id: String,
        library_name: String,
        album_id: String,
        name: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Rows {
    Libraries(Vec<MusicLibrary>),
    Items {
        items: Vec<Item>,
        start: u32,
        total: u32,
    },
}

pub enum Command {
    Restore,
    Begin(String),
    Poll,
    Browse { view: View, start: u32 },
    Cancel,
    SignOut,
    Select(Provider),
}

pub struct Job {
    pub generation: u64,
    pub proxy: ProxyConfig,
    pub command: Command,
}

pub enum Reply {
    Restored {
        source: Option<Source>,
        provider: Provider,
    },
    Challenge(String),
    Pending,
    Connected(Source),
    Data {
        rows: Rows,
        cached: bool,
    },
    SignedOut,
    Error(String),
}

pub struct Event {
    pub generation: u64,
    pub reply: Reply,
}

pub struct Browser {
    pub enabled: bool,
    pub provider: Provider,
    pub server_input: String,
    pub source: Option<Source>,
    pub code: Option<String>,
    pub view: View,
    pub rows: Option<Rows>,
    pub busy: bool,
    pub cached: bool,
    pub error: Option<String>,
    pub start: u32,
    pub filter: String,
    generation: Arc<AtomicU64>,
    tx: Option<mpsc::Sender<Job>>,
    rx: mpsc::Receiver<Event>,
    initialized: bool,
    selection_changed: bool,
    poll_at: Option<Instant>,
    expires: Option<Instant>,
}

impl Browser {
    pub fn new(dirs: AppDirs, waker: Waker, enabled: bool) -> Self {
        let (events, rx) = mpsc::channel();
        let generation = Arc::new(AtomicU64::new(0));
        let tx = enabled
            .then(|| crate::backend::jellyfin::spawn(dirs, waker, events, generation.clone()));
        Self {
            enabled,
            provider: if enabled {
                Provider::Jellyfin
            } else {
                Provider::Spotify
            },
            server_input: String::new(),
            source: None,
            code: None,
            view: View::Libraries,
            rows: None,
            busy: false,
            cached: false,
            error: None,
            start: 0,
            filter: String::new(),
            generation,
            tx,
            rx,
            initialized: false,
            selection_changed: false,
            poll_at: None,
            expires: None,
        }
    }

    pub fn poll(&mut self, proxy: &ProxyConfig) {
        if !self.enabled {
            return;
        }
        // Spotify restores protected proxy credentials first; never bypass a
        // temporarily invalid configuration with a direct connection.
        if !self.initialized && !matches!(proxy, ProxyConfig::Invalid(_)) {
            self.initialized = true;
            self.busy = true;
            self.send(proxy, Command::Restore);
        }
        while let Ok(event) = self.rx.try_recv() {
            if event.generation != self.generation.load(Ordering::SeqCst) {
                continue;
            }
            self.apply(event.reply, proxy);
        }
        if self
            .expires
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.cancel(proxy);
            self.error = Some("Quick Connect expired. Connect again for a new code.".into());
        } else if self
            .poll_at
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.poll_at = None;
            self.send(proxy, Command::Poll);
        }
    }

    fn apply(&mut self, reply: Reply, proxy: &ProxyConfig) {
        match reply {
            Reply::Restored { source, provider } => {
                if !self.selection_changed {
                    self.provider = provider;
                }
                self.busy = false;
                if let Some(source) = source {
                    self.server_input = source.server_url.clone();
                    self.source = Some(source);
                    self.browse(View::Libraries, 0, proxy);
                }
            }
            Reply::Challenge(code) => {
                self.code = Some(code);
                self.poll_at = Some(Instant::now() + Duration::from_secs(5));
                self.expires = Some(Instant::now() + Duration::from_secs(300));
            }
            Reply::Pending => self.poll_at = Some(Instant::now() + Duration::from_secs(5)),
            Reply::Connected(source) => {
                self.source = Some(source);
                self.clear_challenge();
                self.browse(View::Libraries, 0, proxy);
            }
            Reply::Data { rows, cached } => {
                self.rows = Some(rows);
                self.cached = cached;
                self.busy = cached;
            }
            Reply::SignedOut => {
                self.source = None;
                self.busy = false;
            }
            Reply::Error(error) => {
                self.error = Some(error);
                self.busy = false;
                self.clear_challenge();
            }
        }
    }

    pub fn select(&mut self, provider: Provider, proxy: &ProxyConfig) {
        self.selection_changed = true;
        self.provider = provider;
        self.send(proxy, Command::Select(provider));
    }

    pub fn connect(&mut self, proxy: &ProxyConfig) {
        self.advance();
        self.rows = None;
        self.source = None;
        self.view = View::Libraries;
        self.busy = true;
        self.send(proxy, Command::Begin(self.server_input.clone()));
    }

    pub fn cancel(&mut self, proxy: &ProxyConfig) {
        self.advance();
        self.busy = false;
        self.send(proxy, Command::Cancel);
    }

    pub fn sign_out(&mut self, proxy: &ProxyConfig) {
        self.advance();
        self.rows = None;
        self.view = View::Libraries;
        self.busy = true;
        self.send(proxy, Command::SignOut);
    }

    pub fn browse(&mut self, view: View, start: u32, proxy: &ProxyConfig) {
        let same = self.view == view && self.start == start;
        self.advance();
        if !same {
            self.rows = None;
            self.filter.clear();
        }
        self.view = view.clone();
        self.start = start;
        self.busy = true;
        self.send(proxy, Command::Browse { view, start });
    }

    fn clear_challenge(&mut self) {
        self.code = None;
        self.poll_at = None;
        self.expires = None;
    }
    fn advance(&mut self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.error = None;
        self.clear_challenge();
    }
    fn send(&self, proxy: &ProxyConfig, command: Command) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Job {
                generation: self.generation.load(Ordering::SeqCst),
                proxy: proxy.clone(),
                command,
            });
        }
    }
}

#[derive(Clone, Debug)]
pub enum BrowserAction {
    Select(Provider),
    ServerInput(String),
    Filter(String),
    Connect,
    Cancel,
    SignOut,
    Browse { view: View, start: u32 },
    Play(String),
    PlayQueue(usize),
    Queue(String),
    PlayManual(usize),
    ClearQueue,
    Resume,
}

impl crate::app::App {
    pub(crate) fn apply_jellyfin_action(&mut self, action: BrowserAction) {
        match action {
            BrowserAction::Select(provider) => self
                .jellyfin
                .select(provider, &self.applied_jellyfin_proxy()),
            BrowserAction::ServerInput(value) => self.jellyfin.server_input = value,
            BrowserAction::Filter(value) => self.jellyfin.filter = value,
            BrowserAction::Connect => {
                self.jellyfin_player.stop();
                self.jellyfin.connect(&self.applied_jellyfin_proxy());
            }
            BrowserAction::Cancel => self.jellyfin.cancel(&self.applied_jellyfin_proxy()),
            BrowserAction::SignOut => {
                self.jellyfin_player.stop();
                self.jellyfin.sign_out(&self.applied_jellyfin_proxy());
            }
            BrowserAction::Play(id) => {
                let (
                    Some(source),
                    View::Tracks { album_id, .. },
                    Some(Rows::Items {
                        items,
                        start,
                        total,
                    }),
                ) = (
                    &self.jellyfin.source,
                    &self.jellyfin.view,
                    &self.jellyfin.rows,
                )
                else {
                    return;
                };
                let Some(item) = items.iter().find(|item| item.id == id).cloned() else {
                    return;
                };
                let source = source.clone();
                let album_id = album_id.clone();
                let tracks = (*start == 0 && items.len() as u32 == *total).then(|| items.clone());
                self.prepare_jellyfin_playback();
                self.jellyfin_player.configure(crate::app::engine_config(
                    &self.dirs,
                    &self.settings,
                    self.applied_jellyfin_proxy(),
                    self.winamp.tap.clone(),
                    self.winamp.eq.clone(),
                    self.local_output.clone(),
                ));
                self.jellyfin_player.play(source, album_id, item, tracks);
            }
            BrowserAction::PlayQueue(index) => {
                self.prepare_jellyfin_playback();
                self.jellyfin_player.play_index(index);
            }
            BrowserAction::Queue(id) => {
                if !matches!(self.jellyfin.view, View::Tracks { .. }) {
                    return;
                }
                if let (Some(source), Some(Rows::Items { items, .. })) =
                    (&self.jellyfin.source, &self.jellyfin.rows)
                    && let Some(item) = items.iter().find(|item| item.id == id)
                {
                    self.jellyfin_player.enqueue(source, item.clone());
                }
            }
            BrowserAction::PlayManual(index) => {
                self.prepare_jellyfin_playback();
                self.jellyfin_player.play_queued(index);
            }
            BrowserAction::ClearQueue => self.jellyfin_player.clear_queue(),
            BrowserAction::Resume => {
                self.prepare_jellyfin_playback();
                self.jellyfin_player.resume();
            }
            BrowserAction::Browse { view, start } => {
                self.jellyfin
                    .browse(view, start, &self.applied_jellyfin_proxy())
            }
        }
    }
}

#[cfg(any(test, feature = "demo"))]
impl Browser {
    pub fn demo(&mut self, page: &str) {
        self.enabled = true;
        self.initialized = true;
        self.provider = Provider::Jellyfin;
        self.busy = false;
        self.error = None;
        self.code = None;
        self.server_input = "https://music.example/jellyfin/".into();
        self.source = Some(Source {
            id: "0123456789abcdef0123456789abcdef".into(),
            server_url: self.server_input.clone(),
            server_id: "server1".into(),
            user_id: "user1".into(),
            device_id: "device1".into(),
        });
        self.view = View::Libraries;
        self.rows = Some(Rows::Libraries(vec![
            MusicLibrary {
                id: "music1".into(),
                name: "My music".into(),
                collection_type: Some("music".into()),
            },
            MusicLibrary {
                id: "music2".into(),
                name: "DJ collection".into(),
                collection_type: Some("music".into()),
            },
        ]));
        if page == "jellyfin-connect" || page == "jellyfin-approval" {
            self.source = None;
            self.rows = None;
            if page == "jellyfin-approval" {
                self.code = Some("123456".into());
                self.busy = true;
            }
        } else if page == "jellyfin-albums" || page == "jellyfin-tracks" || page == "jellyfin-error"
        {
            self.view = View::Albums {
                library_id: "music1".into(),
                name: "My music".into(),
            };
            let names = [
                "Fragments",
                "Mordechai",
                "All Melody",
                "Promises",
                "Immunity",
                "There Is Love in You",
            ];
            let items = names
                .iter()
                .enumerate()
                .map(|(i, name)| super::Item {
                    id: format!("album{i}"),
                    name: Some((*name).into()),
                    album: None,
                    album_id: None,
                    artists: vec!["Your collection".into()],
                    run_time_ticks: None,
                    index_number: None,
                    parent_index_number: None,
                    container: None,
                    media_streams: None,
                    media_sources: None,
                })
                .collect();
            self.rows = Some(Rows::Items {
                items,
                start: 0,
                total: 6,
            });
            if page == "jellyfin-error" {
                self.cached = true;
                self.error = Some("Could not reach the Jellyfin server.".into());
            }
            if page == "jellyfin-tracks" {
                self.view = View::Tracks {
                    library_id: "music1".into(),
                    library_name: "My music".into(),
                    album_id: "album0".into(),
                    name: "Fragments".into(),
                };
                let items = [
                    "Rosewood", "Otomo", "Shadows", "Tides", "Closer", "From You",
                ]
                .iter()
                .enumerate()
                .map(|(i, name)| super::Item {
                    id: format!("track{i}"),
                    name: Some((*name).into()),
                    album: Some("Fragments".into()),
                    album_id: Some("album0".into()),
                    artists: vec!["Bonobo".into()],
                    run_time_ticks: Some(2_400_000_000),
                    index_number: Some(i as u32 + 1),
                    parent_index_number: Some(1),
                    container: None,
                    media_streams: None,
                    media_sources: None,
                })
                .collect();
                self.rows = Some(Rows::Items {
                    items,
                    start: 0,
                    total: 6,
                });
            }
        }
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        // Prevent an outstanding approval from publishing a saved account
        // after the browser has been destroyed.
        self.generation.fetch_add(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> Browser {
        let dirs = AppDirs {
            config: PathBuf::new(),
            state: PathBuf::new(),
            cache: PathBuf::new(),
        };
        let mut model = Browser::new(dirs, Waker::default(), false);
        model.demo("jellyfin-albums");
        model
    }
    use std::path::PathBuf;

    #[test]
    fn switching_provider_preserves_jellyfin_navigation_and_catalog() {
        let mut model = model();
        let source = model.source.clone();
        let view = model.view.clone();
        model.select(Provider::Spotify, &ProxyConfig::Off);
        model.select(Provider::Jellyfin, &ProxyConfig::Off);
        assert_eq!(model.source, source);
        assert_eq!(model.view, view);
        assert!(matches!(model.rows, Some(Rows::Items { total: 6, .. })));
    }

    #[test]
    fn old_account_and_page_replies_cannot_replace_newer_navigation() {
        let mut model = model();
        let (events, rx) = mpsc::channel();
        model.rx = rx;
        let old = model.generation.load(Ordering::SeqCst);
        model.browse(View::Libraries, 0, &ProxyConfig::Off);
        events
            .send(Event {
                generation: old,
                reply: Reply::Connected(model.source.clone().unwrap()),
            })
            .unwrap();
        events
            .send(Event {
                generation: old,
                reply: Reply::Data {
                    rows: Rows::Items {
                        items: vec![],
                        start: 0,
                        total: 9000,
                    },
                    cached: false,
                },
            })
            .unwrap();
        model.poll(&ProxyConfig::Off);
        assert_eq!(model.view, View::Libraries);
        assert!(model.rows.is_none());
    }

    #[test]
    fn failed_refresh_keeps_saved_rows_visible_and_stops_loading() {
        let mut model = model();
        model.browse(model.view.clone(), 0, &ProxyConfig::Off);
        model.apply(Reply::Error("Server unavailable".into()), &ProxyConfig::Off);
        assert!(!model.busy);
        assert!(matches!(model.rows, Some(Rows::Items { total: 6, .. })));
    }

    #[test]
    fn approval_polling_expires_and_late_approval_is_ignored_after_cancel() {
        let mut model = model();
        model.demo("jellyfin-approval");
        model.expires = Some(Instant::now() - Duration::from_secs(1));
        let old = model.generation.load(Ordering::SeqCst);
        model.poll(&ProxyConfig::Off);
        assert!(model.code.is_none());
        assert!(!model.busy);
        assert!(model.error.as_deref().unwrap().contains("expired"));
        assert_ne!(old, model.generation.load(Ordering::SeqCst));
    }

    #[test]
    fn restored_selection_does_not_undo_a_choice_already_made() {
        let mut model = model();
        model.select(Provider::Spotify, &ProxyConfig::Off);
        model.apply(
            Reply::Restored {
                source: None,
                provider: Provider::Jellyfin,
            },
            &ProxyConfig::Off,
        );
        assert_eq!(model.provider, Provider::Spotify);
    }

    #[test]
    fn failed_sign_out_can_be_retried_without_losing_source_identity() {
        let mut model = model();
        let source = model.source.clone();
        model.sign_out(&ProxyConfig::Off);
        model.apply(
            Reply::Error("Could not revoke saved sign-in".into()),
            &ProxyConfig::Off,
        );
        assert_eq!(model.source, source);
        model.apply(Reply::SignedOut, &ProxyConfig::Off);
        assert!(model.source.is_none());
        assert!(model.rows.is_none());
    }
}
