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
    pub catalog_request: Option<u64>,
    pub generation: u64,
    pub proxy: ProxyConfig,
    pub command: Command,
}

pub enum Reply {
    Catalog {
        request: u64,
        source: Source,
        rows: Rows,
        cached: bool,
    },
    CatalogError {
        request: u64,
        error: String,
    },
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

#[derive(Default)]
pub struct Catalog {
    pub library: Option<MusicLibrary>,
    pub items: Vec<Item>,
    pub busy: bool,
    pub error: Option<String>,
    pub filter: String,
    pub sort: Option<crate::settings::LibrarySort>,
}

pub struct Browser {
    pub enabled: bool,
    pub provider: Provider,
    pub server_input: String,
    pub source: Option<Source>,
    pub code: Option<String>,
    pub view: View,
    pub rows: Option<Rows>,
    pub libraries: Vec<MusicLibrary>,
    pub album: Option<Item>,
    pub play_album: Option<String>,
    pub catalog: Catalog,
    pub catalog_generation: Arc<AtomicU64>,
    pub data_revision: u64,
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
        let catalog_generation = Arc::new(AtomicU64::new(0));
        let tx = enabled.then(|| {
            crate::backend::jellyfin::spawn(
                dirs,
                waker,
                events,
                generation.clone(),
                catalog_generation.clone(),
            )
        });
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
            libraries: Vec::new(),
            album: None,
            play_album: None,
            catalog: Catalog::default(),
            catalog_generation,
            data_revision: 0,
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
            if !matches!(
                &event.reply,
                Reply::Catalog { .. } | Reply::CatalogError { .. }
            ) && event.generation != self.generation.load(Ordering::SeqCst)
            {
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
            Reply::Catalog {
                request,
                source,
                rows:
                    Rows::Items {
                        items,
                        start,
                        total,
                    },
                cached,
            } if request == self.catalog_generation.load(Ordering::SeqCst)
                && self.source.as_ref() == Some(&source) =>
            {
                if start == 0 {
                    self.catalog.items.clear();
                }
                if start as usize > self.catalog.items.len() || (items.is_empty() && start < total)
                {
                    self.catalog.error = Some(
                        "Jellyfin returned an incomplete library page. Retry the library refresh."
                            .into(),
                    );
                    self.catalog.busy = false;
                    return;
                }
                self.catalog.items.truncate(start as usize);
                let next = start + items.len() as u32;
                self.catalog.items.extend(items);
                if !cached {
                    if next < total && next > start {
                        self.request_catalog_page(next, proxy);
                    } else {
                        self.catalog.busy = false;
                    }
                }
            }
            Reply::CatalogError { request, error }
                if request == self.catalog_generation.load(Ordering::SeqCst) =>
            {
                self.catalog.error = Some(error);
                self.catalog.busy = false;
            }
            Reply::Catalog { .. } | Reply::CatalogError { .. } => {}
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
                if !cached {
                    self.data_revision = self.data_revision.wrapping_add(1);
                }
                if let Rows::Libraries(libraries) = &rows {
                    self.libraries = libraries.clone();
                    if self.catalog.library.is_none()
                        && let Some(library) = libraries.first()
                    {
                        self.select_catalog(library.clone(), proxy);
                    }
                }
                self.rows = Some(rows);
                self.cached = cached;
                self.busy = cached;
            }
            Reply::SignedOut => {
                self.source = None;
                self.busy = false;
            }
            Reply::Error(error) => {
                self.play_album = None;
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
        self.catalog_generation.fetch_add(1, Ordering::SeqCst);
        self.catalog = Catalog::default();
        self.libraries.clear();
        self.album = None;
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
        self.catalog_generation.fetch_add(1, Ordering::SeqCst);
        self.catalog = Catalog::default();
        self.libraries.clear();
        self.album = None;
        self.advance();
        self.rows = None;
        self.view = View::Libraries;
        self.busy = true;
        self.send(proxy, Command::SignOut);
    }

    pub fn select_catalog(&mut self, library: MusicLibrary, proxy: &ProxyConfig) {
        self.catalog_generation.fetch_add(1, Ordering::SeqCst);
        self.catalog = Catalog {
            library: Some(library),
            busy: true,
            ..Catalog::default()
        };
        self.request_catalog_page(0, proxy);
    }

    fn request_catalog_page(&self, start: u32, proxy: &ProxyConfig) {
        if let (Some(tx), Some(library)) = (&self.tx, &self.catalog.library) {
            let _ = tx.send(Job {
                catalog_request: Some(self.catalog_generation.load(Ordering::SeqCst)),
                generation: self.generation.load(Ordering::SeqCst),
                proxy: proxy.clone(),
                command: Command::Browse {
                    view: View::Albums {
                        library_id: library.id.clone(),
                        name: library.name.clone(),
                    },
                    start,
                },
            });
        }
    }

    pub fn browse(&mut self, view: View, start: u32, proxy: &ProxyConfig) {
        self.play_album = None;
        let library_id = match &view {
            View::Albums { library_id, .. } | View::Tracks { library_id, .. } => Some(library_id),
            _ => None,
        };
        if let Some(id) = library_id
            && self.catalog.library.as_ref().map(|library| &library.id) != Some(id)
            && let Some(library) = self
                .libraries
                .iter()
                .find(|library| &library.id == id)
                .cloned()
        {
            self.select_catalog(library, proxy);
        }
        let same = self.view == view && self.start == start;
        if let View::Tracks { album_id, .. } = &view {
            if !matches!(&self.view, View::Tracks { album_id: current, .. } if current == album_id)
            {
                self.album = self
                    .catalog
                    .items
                    .iter()
                    .find(|item| &item.id == album_id)
                    .cloned()
                    .or_else(|| {
                        self.rows.as_ref().and_then(|rows| match rows {
                            Rows::Items { items, .. } => {
                                items.iter().find(|item| &item.id == album_id).cloned()
                            }
                            _ => None,
                        })
                    });
            }
        } else {
            self.album = None;
        }
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
                catalog_request: None,
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
    PlayAlbum(View),
    PlayQueue(usize),
    Queue(String),
    PlayManual(usize),
    ClearQueue,
    Resume,
    SelectLibrary(String),
}

impl crate::app::App {
    pub(crate) fn apply_jellyfin_action(&mut self, action: BrowserAction) {
        match action {
            BrowserAction::PlayAlbum(view) => {
                if let View::Tracks { album_id, .. } = &view {
                    let album_id = album_id.clone();
                    self.jellyfin
                        .browse(view, 0, &self.applied_jellyfin_proxy());
                    self.jellyfin.play_album = Some(album_id);
                }
            }
            BrowserAction::SelectLibrary(id) => {
                if let Some(library) = self
                    .jellyfin
                    .libraries
                    .iter()
                    .find(|library| library.id == id)
                    .cloned()
                {
                    self.jellyfin.browse(
                        View::Albums {
                            library_id: library.id,
                            name: library.name,
                        },
                        0,
                        &self.applied_jellyfin_proxy(),
                    );
                }
            }
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
        self.album = None;
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
        if let Some(Rows::Libraries(libraries)) = &self.rows {
            self.libraries = libraries.clone();
        }
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
            let items: Vec<_> = names
                .iter()
                .enumerate()
                .map(|(i, name)| super::Item {
                    details: Default::default(),
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
                    image_tags: Some(std::collections::HashMap::from([(
                        "Primary".into(),
                        Some("fixture-v1".into()),
                    )])),
                    album_primary_image_tag: None,
                })
                .collect();
            self.catalog.library = self.libraries.first().cloned();
            self.catalog.items = items.clone();
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
                    details: Default::default(),
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
                    image_tags: Default::default(),
                    album_primary_image_tag: Some("fixture-v1".into()),
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
        self.catalog_generation.fetch_add(1, Ordering::SeqCst);
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
    fn catalog_pages_are_independent_of_center_navigation_and_reject_stale_scope() {
        let mut model = model();
        let source = model.source.clone().unwrap();
        let album = model.catalog.items[0].clone();
        let (jobs, requests) = mpsc::channel();
        model.tx = Some(jobs);
        model.select_catalog(model.libraries[0].clone(), &ProxyConfig::Off);
        let first = requests.try_recv().unwrap();
        let request = first.catalog_request.unwrap();
        model.browse(
            View::Tracks {
                library_id: "music1".into(),
                library_name: "My music".into(),
                album_id: "album0".into(),
                name: "Fragments".into(),
            },
            0,
            &ProxyConfig::Off,
        );
        let center = model.view.clone();
        let old_generation = first.generation;
        let (events, replies) = mpsc::channel();
        model.rx = replies;
        events
            .send(Event {
                generation: old_generation,
                reply: Reply::Catalog {
                    request,
                    source: source.clone(),
                    rows: Rows::Items {
                        items: vec![album.clone()],
                        start: 0,
                        total: 2,
                    },
                    cached: false,
                },
            })
            .unwrap();
        model.poll(&ProxyConfig::Off);
        assert_eq!(model.view, center);
        assert!(model.rows.is_none());
        assert_eq!(model.catalog.items, vec![album.clone()]);
        assert!(model.catalog.busy);
        let queued: Vec<_> = requests.try_iter().collect();
        assert!(queued.iter().any(|job| job.catalog_request == Some(request)
            && matches!(job.command, Command::Browse { start: 1, .. })));
        let mut second = album.clone();
        second.id = "album1".into();
        model.apply(
            Reply::Catalog {
                request,
                source: source.clone(),
                rows: Rows::Items {
                    items: vec![second],
                    start: 1,
                    total: 2,
                },
                cached: true,
            },
            &ProxyConfig::Off,
        );
        assert!(model.catalog.busy);
        model.apply(
            Reply::Catalog {
                request,
                source: source.clone(),
                rows: Rows::Items {
                    items: vec![album.clone()],
                    start: 1,
                    total: 2,
                },
                cached: false,
            },
            &ProxyConfig::Off,
        );
        assert!(!model.catalog.busy);
        assert_eq!(
            model.catalog.items.len(),
            2,
            "fresh page replaces its cached page"
        );
        let mut other_account = source.clone();
        other_account.user_id = "other".into();
        model.apply(
            Reply::Catalog {
                request,
                source: other_account,
                rows: Rows::Items {
                    items: vec![],
                    start: 0,
                    total: 0,
                },
                cached: false,
            },
            &ProxyConfig::Off,
        );
        assert_eq!(model.catalog.items.len(), 2);
        model.select_catalog(model.libraries[1].clone(), &ProxyConfig::Off);
        model.apply(
            Reply::Catalog {
                request,
                source,
                rows: Rows::Items {
                    items: vec![album],
                    start: 0,
                    total: 1,
                },
                cached: false,
            },
            &ProxyConfig::Off,
        );
        model.apply(
            Reply::CatalogError {
                request,
                error: "old account".into(),
            },
            &ProxyConfig::Off,
        );
        assert!(model.catalog.items.is_empty());
        assert!(model.catalog.error.is_none());
        assert!(model.catalog.busy);
    }

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
