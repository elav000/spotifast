//! Main-thread presentation model. Secrets stay in the backend worker.
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

use super::{
    Item, MusicLibrary,
    catalog::{Kind, Query},
};
use crate::{backend::Waker, paths::AppDirs, settings::ProxyConfig};
use serde::{Deserialize, Serialize};

pub use crate::provider::Provider;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Source {
    pub id: String,
    #[serde(default)]
    pub nickname: String,
    pub server_url: String,
    pub server_id: String,
    pub user_id: String,
    pub device_id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum View {
    #[default]
    Libraries,
    Search {
        library: String,
        term: String,
        kind: Option<Kind>,
    },
    Catalog {
        query: Query,
        name: String,
    },
    Settings,
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

impl View {
    pub fn context_id(&self) -> Option<&str> {
        match self {
            Self::Tracks { album_id, .. } => Some(album_id),
            Self::Catalog {
                query: Query::PlaylistSongs { playlist },
                ..
            } => Some(playlist),
            Self::Catalog {
                query: Query::ArtistAlbums { artist, .. } | Query::ArtistSongs { artist, .. },
                ..
            } => Some(artist),
            _ => None,
        }
    }
    pub fn track_query(&self) -> Option<&Query> {
        match self {
            Self::Catalog {
                query:
                    query @ (Query::PlaylistSongs { .. }
                    | Query::ArtistSongs { .. }
                    | Query::History { .. }
                    | Query::Search {
                        kind: Kind::Song, ..
                    }),
                ..
            } => Some(query),
            _ => None,
        }
    }
    pub fn is_tracks(&self) -> bool {
        matches!(self, Self::Tracks { .. }) || self.track_query().is_some()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Rows {
    Libraries(Vec<MusicLibrary>),
    Search(Vec<(Kind, super::Page<Item>)>),
    Items {
        items: Vec<Item>,
        start: u32,
        total: u32,
    },
}

#[derive(Clone, Debug)]
pub enum PlaylistEdit {
    Create {
        name: String,
        public: bool,
        items: Vec<String>,
    },
    Rename {
        id: String,
        name: String,
    },
    Delete {
        id: String,
    },
    Add {
        id: String,
        items: Vec<String>,
    },
    Remove {
        id: String,
        entries: Vec<String>,
    },
    Move {
        id: String,
        entry: String,
        index: u32,
    },
}

pub enum Command {
    Report(super::reporting::Report),
    Playlists(Source),
    Edit {
        source: Source,
        request: u64,
        edit: PlaylistEdit,
    },
    Restore,
    Begin(String),
    Poll,
    Browse {
        view: View,
        start: u32,
    },
    Cancel,
    RenameServer(Source),
    ActivateServer(String),
    RemoveServer(String),
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
    ReportError {
        source: Source,
        error: Option<String>,
    },
    Playlists {
        source: Source,
        result: Result<Vec<Item>, String>,
    },
    Edited {
        source: Source,
        request: u64,
        result: Result<Option<(String, String)>, String>,
    },
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
        servers: Vec<Source>,
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
    Renamed(Source),
    ServersChanged {
        servers: Vec<Source>,
        active: Option<Source>,
    },
    Activated(Source),
    Error(String),
}

pub struct Event {
    pub generation: u64,
    pub reply: Reply,
}

#[derive(Default)]
pub struct Catalog {
    pub kind: Kind,
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
    pub servers: Vec<Source>,
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
    pub reporter: super::reporting::Reporter,
    pub report_error: Option<String>,
    pub playlists: Vec<Item>,
    pub playlists_loading: bool,
    pub playlists_error: Option<String>,
    pub edit_request: u64,
    pub edit_busy: bool,
    pub edit_error: Option<String>,
    pub edit_completed: bool,
    edit_previous: Option<View>,
    edit_rows: Option<Rows>,
    edit_generation: u64,
    edit_deleted: Option<String>,
    pub dialog_source: Option<Source>,
    pub search_input: String,
    pub search_kind: Option<Kind>,
    pub search_typed_at: Option<Instant>,
    pub history: Vec<(View, u32)>,
    pub history_index: usize,
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
            servers: Vec::new(),
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
            reporter: super::reporting::Reporter::default(),
            report_error: None,
            playlists: Vec::new(),
            playlists_loading: false,
            playlists_error: None,
            edit_request: 0,
            edit_busy: false,
            edit_error: None,
            edit_completed: false,
            edit_previous: None,
            edit_rows: None,
            edit_generation: 0,
            edit_deleted: None,
            dialog_source: None,
            search_input: String::new(),
            search_kind: None,
            search_typed_at: None,
            history: vec![(View::Libraries, 0)],
            history_index: 0,
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
                Reply::Catalog { .. }
                    | Reply::CatalogError { .. }
                    | Reply::Edited { .. }
                    | Reply::Playlists { .. }
                    | Reply::ReportError { .. }
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
            Reply::ReportError { source, error } => {
                if self.source.as_ref() == Some(&source) {
                    self.report_error = error;
                }
            }
            Reply::Playlists { source, result } => {
                if self.source.as_ref() != Some(&source) {
                    return;
                }
                self.playlists_loading = false;
                match result {
                    Ok(items) => {
                        self.playlists = items;
                        self.playlists_error = None;
                    }
                    Err(error) => self.playlists_error = Some(error),
                }
            }

            Reply::Edited {
                source,
                request,
                result,
            } => {
                if self.source.as_ref() != Some(&source) || self.edit_request != request {
                    return;
                }
                self.edit_busy = false;
                let unchanged = self.generation.load(Ordering::SeqCst) == self.edit_generation;
                match result {
                    Ok(created) => {
                        self.edit_error = None;
                        self.edit_completed = true;
                        self.load_playlists(proxy);
                        if let Some((id, name)) = created.filter(|_| unchanged) {
                            self.browse(
                                View::Catalog {
                                    query: Query::PlaylistSongs { playlist: id },
                                    name,
                                },
                                0,
                                proxy,
                            );
                        } else if self
                            .edit_deleted
                            .as_deref()
                            .is_some_and(|id| self.view.context_id() == Some(id))
                        {
                            self.browse(View::Libraries, 0, proxy);
                        } else if unchanged {
                            self.load_view(self.view.clone(), self.start, proxy);
                        }
                        self.edit_previous = None;
                        self.edit_rows = None;
                        self.edit_deleted = None;
                    }
                    Err(error) => {
                        self.edit_error = Some(error);
                        let view = self
                            .edit_previous
                            .take()
                            .unwrap_or_else(|| self.view.clone());
                        if unchanged {
                            self.rows = self.edit_rows.take();
                            self.load_view(view, self.start, proxy);
                        } else {
                            self.edit_rows = None;
                        }
                        self.edit_deleted = None;
                    }
                }
                if let Some(library) = self.catalog.library.clone() {
                    self.select_catalog(library, proxy);
                }
            }

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
            Reply::Restored {
                source,
                servers,
                provider,
            } => {
                self.servers = servers;
                if !self.selection_changed {
                    self.provider = provider;
                }
                self.busy = false;
                let changed = self.source.as_ref().map(|source| &source.id)
                    != source.as_ref().map(|source| &source.id);
                if changed {
                    self.rows = None;
                    self.catalog = Catalog::default();
                    self.libraries.clear();
                    self.playlists.clear();
                }
                self.source = source.clone();
                if let Some(source) = source {
                    self.server_input = source.server_url.clone();
                    self.browse(View::Libraries, 0, proxy);
                    self.load_playlists(proxy);
                }
            }
            Reply::Renamed(source) => {
                if self
                    .source
                    .as_ref()
                    .is_some_and(|active| active.id == source.id)
                {
                    self.source = Some(source.clone());
                }
                if let Some(saved) = self.servers.iter_mut().find(|saved| saved.id == source.id) {
                    *saved = source;
                }
            }
            Reply::ServersChanged { servers, active } => {
                self.servers = servers;
                let changed = self.source.as_ref().map(|source| &source.id)
                    != active.as_ref().map(|source| &source.id);
                if changed {
                    self.source = active.clone();
                    if let Some(source) = active {
                        self.server_input = source.server_url;
                        self.reset_for_server_switch();
                        self.browse(View::Libraries, 0, proxy);
                        self.load_playlists(proxy);
                    } else {
                        self.reset_for_server_switch();
                        self.busy = false;
                    }
                }
            }
            Reply::Activated(source) => {
                self.source = Some(source.clone());
                self.server_input = source.server_url.clone();
                self.reset_for_server_switch();
                self.browse(View::Libraries, 0, proxy);
                self.load_playlists(proxy);
            }
            Reply::Challenge(code) => {
                self.code = Some(code);
                self.poll_at = Some(Instant::now() + Duration::from_secs(5));
                self.expires = Some(Instant::now() + Duration::from_secs(300));
            }
            Reply::Pending => self.poll_at = Some(Instant::now() + Duration::from_secs(5)),
            Reply::Connected(source) => {
                self.source = Some(source);
                if let Some(source) = self.source.clone() {
                    self.servers.retain(|saved| saved.id != source.id);
                    self.servers.push(source);
                }
                self.clear_challenge();
                self.browse(View::Libraries, 0, proxy);
                self.load_playlists(proxy);
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

    pub fn report_playback(
        &mut self,
        snapshot: &super::transport::Snapshot,
        controls_output: bool,
        proxy: &ProxyConfig,
    ) {
        let current = self
            .source
            .as_ref()
            .filter(|source| {
                controls_output
                    && !snapshot.finished
                    && snapshot.source_id.as_deref() == Some(&source.id)
            })
            .and_then(|source| {
                snapshot
                    .item
                    .as_ref()
                    .map(|item| super::reporting::Playback {
                        source: source.clone(),
                        item: item.id.clone(),
                        position_ms: snapshot.position_ms,
                        playing: snapshot.playing,
                        loading: snapshot.loading,
                    })
            });
        for report in self.reporter.update(current, Instant::now()) {
            self.send(proxy, Command::Report(report));
        }
    }

    pub fn load_playlists(&mut self, proxy: &ProxyConfig) {
        if let Some(source) = self.source.clone() {
            self.playlists_loading = true;
            self.send(proxy, Command::Playlists(source));
        }
    }

    pub fn edit(&mut self, source: Source, edit: PlaylistEdit, proxy: &ProxyConfig) {
        if self.source.as_ref() != Some(&source) || self.edit_busy {
            return;
        }
        self.edit_previous = Some(self.view.clone());
        self.edit_rows = self.rows.clone();
        self.edit_deleted = None;
        match &edit {
            PlaylistEdit::Rename { id, name } => {
                if self.view.context_id() == Some(id)
                    && let View::Catalog { name: shown, .. } = &mut self.view
                {
                    *shown = name.clone();
                }
            }
            PlaylistEdit::Delete { id } => self.edit_deleted = Some(id.clone()),
            PlaylistEdit::Remove { id, entries } if self.view.context_id() == Some(id) => {
                if let Some(Rows::Items { items, total, .. }) = &mut self.rows {
                    let before = items.len();
                    items.retain(|item| {
                        item.details
                            .playlist_item_id
                            .as_ref()
                            .is_none_or(|entry| !entries.contains(entry))
                    });
                    *total = total.saturating_sub((before - items.len()) as u32);
                }
            }
            PlaylistEdit::Move { id, entry, index } if self.view.context_id() == Some(id) => {
                if let Some(Rows::Items { items, start, .. }) = &mut self.rows
                    && let Some(old) = items
                        .iter()
                        .position(|item| item.details.playlist_item_id.as_ref() == Some(entry))
                    && let Some(next) = index
                        .checked_sub(*start)
                        .map(|index| index as usize)
                        .filter(|index| *index < items.len())
                {
                    let item = items.remove(old);
                    items.insert(next, item);
                }
            }
            _ => {}
        }
        self.edit_request = self.edit_request.wrapping_add(1);
        self.edit_busy = true;
        self.edit_error = None;
        self.advance();
        self.edit_generation = self.generation.load(Ordering::SeqCst);
        self.send(
            proxy,
            Command::Edit {
                source,
                request: self.edit_request,
                edit,
            },
        );
    }

    pub fn select(&mut self, provider: Provider, proxy: &ProxyConfig) {
        self.selection_changed = true;
        self.provider = provider;
        self.send(proxy, Command::Select(provider));
    }

    pub fn connect(&mut self, proxy: &ProxyConfig) {
        self.advance();
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
        self.playlists.clear();
        self.report_error = None;
        self.playlists_loading = false;
        self.playlists_error = None;
        self.edit_busy = false;
        self.edit_error = None;
        self.album = None;
        self.advance();
        self.rows = None;
        self.view = View::Libraries;
        self.history = vec![(View::Libraries, 0)];
        self.history_index = 0;
        self.search_input.clear();
        self.search_typed_at = None;
        self.busy = true;
        self.send(proxy, Command::SignOut);
    }

    pub fn rename_server(&mut self, id: String, nickname: String, proxy: &ProxyConfig) {
        let nickname = nickname.trim();
        if !nickname.is_empty()
            && let Some(mut source) = self.servers.iter().find(|source| source.id == id).cloned()
        {
            source.nickname = nickname.to_owned();
            if self.source.as_ref().is_some_and(|active| active.id == id) {
                self.source = Some(source.clone());
            }
            if let Some(saved) = self.servers.iter_mut().find(|saved| saved.id == id) {
                *saved = source.clone();
            }
            self.send(proxy, Command::RenameServer(source));
        }
    }

    fn reset_for_server_switch(&mut self) {
        self.rows = None;
        self.catalog_generation.fetch_add(1, Ordering::SeqCst);
        self.catalog = Catalog::default();
        self.libraries.clear();
        self.playlists.clear();
        self.playlists_loading = false;
        self.album = None;
        self.view = View::Libraries;
        self.history = vec![(View::Libraries, 0)];
        self.history_index = 0;
        self.busy = true;
    }

    pub fn activate_server(&mut self, id: String, proxy: &ProxyConfig) {
        if self.servers.iter().any(|server| server.id == id) {
            self.advance();
            self.reset_for_server_switch();
            self.send(proxy, Command::ActivateServer(id));
        }
    }

    pub fn remove_server(&mut self, id: String, proxy: &ProxyConfig) {
        if self.source.as_ref().is_some_and(|source| source.id == id) {
            self.advance();
            self.reset_for_server_switch();
        }
        self.send(proxy, Command::RemoveServer(id));
    }

    pub fn select_catalog(&mut self, library: MusicLibrary, proxy: &ProxyConfig) {
        self.catalog_generation.fetch_add(1, Ordering::SeqCst);
        let kind = self.catalog.kind;
        self.catalog = Catalog {
            kind,
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
                    view: if self.catalog.kind == Kind::Album {
                        View::Albums {
                            library_id: library.id.clone(),
                            name: library.name.clone(),
                        }
                    } else {
                        View::Catalog {
                            query: Query::Shelf {
                                library: library.id.clone(),
                                kind: self.catalog.kind,
                            },
                            name: format!("{:?}s", self.catalog.kind),
                        }
                    },
                    start,
                },
            });
        }
    }

    pub fn set_shelf(&mut self, kind: Kind, proxy: &ProxyConfig) {
        self.catalog.kind = kind;
        if let Some(library) = self.catalog.library.clone() {
            self.select_catalog(library, proxy);
        }
    }

    pub fn navigate_history(&mut self, back: bool, proxy: &ProxyConfig) {
        let next = if back {
            self.history_index.checked_sub(1)
        } else {
            self.history_index
                .checked_add(1)
                .filter(|next| *next < self.history.len())
        };
        if let Some(next) = next {
            self.history_index = next;
            let (view, start) = self.history[next].clone();
            self.load_view(view, start, proxy);
        }
    }

    pub fn browse(&mut self, view: View, start: u32, proxy: &ProxyConfig) {
        if self.view != view || self.start != start {
            self.history.truncate(self.history_index + 1);
            self.history.push((view.clone(), start));
            self.history_index = self.history.len() - 1;
        }
        self.load_view(view, start, proxy);
    }

    fn load_view(&mut self, view: View, start: u32, proxy: &ProxyConfig) {
        self.play_album = None;
        if let View::Search { kind, .. } = &view {
            self.search_kind = *kind;
        }
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
        if let Some(album_id) = view.context_id() {
            if self.view.context_id() != Some(album_id) {
                self.album = self
                    .catalog
                    .items
                    .iter()
                    .find(|item| item.id == album_id)
                    .cloned()
                    .or_else(|| {
                        self.rows.as_ref().and_then(|rows| match rows {
                            Rows::Items { items, .. } => {
                                items.iter().find(|item| item.id == album_id).cloned()
                            }
                            Rows::Search(sections) => sections
                                .iter()
                                .flat_map(|(_, page)| &page.items)
                                .find(|item| item.id == album_id)
                                .cloned(),
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
        self.busy = !matches!(&view, View::Settings)
            && !matches!(&view, View::Search { term, .. } if term.is_empty());
        if self.busy {
            self.send(proxy, Command::Browse { view, start });
        }
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
    Edit {
        source: Source,
        edit: PlaylistEdit,
    },
    Select(Provider),
    ServerInput(String),
    Filter(String),
    Connect,
    RenameServer {
        id: String,
        nickname: String,
    },
    ActivateServer(String),
    RemoveServer(String),
    Cancel,
    SignOut,
    Browse {
        view: View,
        start: u32,
    },
    Play(String),
    PlayItem {
        source: Source,
        item: Box<Item>,
        context: String,
        query: Option<Query>,
        index: Option<usize>,
    },
    PlayAlbum(View),
    PlayQueue(usize),
    Queue(String),
    EnqueueItem {
        source: Source,
        item: Box<Item>,
    },
    InsertManual {
        source: Source,
        expected: std::sync::Arc<Vec<Item>>,
        item: Box<Item>,
        to: usize,
    },
    EditManual {
        source: Source,
        expected: std::sync::Arc<Vec<Item>>,
        index: usize,
        to: Option<usize>,
    },
    RemoveManual {
        source: Source,
        expected: std::sync::Arc<Vec<Item>>,
        indices: Vec<usize>,
    },
    PlayQueueContext {
        source: Source,
        view: View,
    },
    OpenQueueContext {
        source: Source,
        view: View,
    },
    PlayManual(usize),
    ClearQueue,
    Resume,
    SelectLibrary(String),
    OpenSettings,
    SelectCollection {
        source: Source,
        id: String,
    },
}

impl crate::app::App {
    pub(crate) fn apply_jellyfin_action(&mut self, action: BrowserAction) {
        match action {
            BrowserAction::Edit { source, edit } => {
                self.jellyfin
                    .edit(source, edit, &self.applied_jellyfin_proxy())
            }
            BrowserAction::PlayAlbum(view) => {
                if view.is_tracks() {
                    let album_id = view.context_id().unwrap_or("tracks").to_owned();
                    self.jellyfin
                        .browse(view, 0, &self.applied_jellyfin_proxy());
                    self.jellyfin.play_album = Some(album_id);
                }
            }
            BrowserAction::SelectCollection { source, id } => {
                if self.jellyfin.source.as_ref() == Some(&source) {
                    self.jellyfin
                        .select(Provider::Jellyfin, &self.applied_jellyfin_proxy());
                    self.apply_jellyfin_action(BrowserAction::SelectLibrary(id));
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
            BrowserAction::OpenSettings => {
                self.jellyfin
                    .select(Provider::Jellyfin, &self.applied_jellyfin_proxy());
                self.jellyfin
                    .browse(View::Settings, 0, &self.applied_jellyfin_proxy());
            }
            BrowserAction::ServerInput(value) => self.jellyfin.server_input = value,
            BrowserAction::RenameServer { id, nickname } => {
                self.jellyfin
                    .rename_server(id, nickname, &self.applied_jellyfin_proxy())
            }
            BrowserAction::ActivateServer(id) => self
                .jellyfin
                .activate_server(id, &self.applied_jellyfin_proxy()),
            BrowserAction::RemoveServer(id) => {
                if self
                    .jellyfin
                    .source
                    .as_ref()
                    .is_some_and(|source| source.id == id)
                {
                    self.jellyfin.report_playback(
                        &self.jellyfin_player.snapshot(),
                        false,
                        &self.applied_jellyfin_proxy(),
                    );
                    self.jellyfin_player.stop();
                }
                self.jellyfin
                    .remove_server(id, &self.applied_jellyfin_proxy());
            }
            BrowserAction::Filter(value) => self.jellyfin.filter = value,
            BrowserAction::Connect => {
                self.jellyfin.connect(&self.applied_jellyfin_proxy());
            }
            BrowserAction::Cancel => self.jellyfin.cancel(&self.applied_jellyfin_proxy()),
            BrowserAction::SignOut => {
                self.jellyfin.report_playback(
                    &self.jellyfin_player.snapshot(),
                    false,
                    &self.applied_jellyfin_proxy(),
                );
                self.jellyfin_player.stop();
                self.jellyfin.sign_out(&self.applied_jellyfin_proxy());
            }
            BrowserAction::PlayItem {
                source,
                item,
                context,
                query,
                index,
            } => {
                if self.jellyfin.source.as_ref() != Some(&source) {
                    return;
                }
                self.prepare_jellyfin_playback();
                self.jellyfin_player.configure(crate::app::engine_config(
                    &self.dirs,
                    &self.settings,
                    self.applied_jellyfin_proxy(),
                    self.winamp.tap.clone(),
                    self.winamp.eq.clone(),
                    self.local_output.clone(),
                ));
                self.jellyfin_player
                    .play_catalog(source, context, *item, query, index);
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
            BrowserAction::EditManual {
                source,
                expected,
                index,
                to,
            } => self
                .jellyfin_player
                .edit_manual(&source, &expected, index, to),
            BrowserAction::RemoveManual {
                source,
                expected,
                indices,
            } => {
                self.jellyfin_player
                    .remove_manual(&source, &expected, &indices);
            }
            BrowserAction::PlayQueueContext { source, view } => {
                if self.jellyfin.source.as_ref() == Some(&source) {
                    self.apply_jellyfin_action(BrowserAction::PlayAlbum(view));
                }
            }
            BrowserAction::OpenQueueContext { source, view } => {
                if self.jellyfin.source.as_ref() == Some(&source) {
                    self.jellyfin
                        .select(Provider::Jellyfin, &self.applied_jellyfin_proxy());
                    self.jellyfin
                        .browse(view, 0, &self.applied_jellyfin_proxy());
                }
            }
            BrowserAction::EnqueueItem { source, item } => {
                self.jellyfin_player.enqueue(&source, *item)
            }
            BrowserAction::InsertManual {
                source,
                expected,
                item,
                to,
            } => self
                .jellyfin_player
                .insert_manual(&source, &expected, *item, to),
            BrowserAction::Queue(id) => {
                if let Some(source) = &self.jellyfin.source {
                    let item = match &self.jellyfin.rows {
                        Some(Rows::Items { items, .. }) if self.jellyfin.view.is_tracks() => {
                            items.iter().find(|item| item.id == id)
                        }
                        Some(Rows::Search(sections)) => sections
                            .iter()
                            .filter(|(kind, _)| *kind == Kind::Song)
                            .flat_map(|(_, page)| &page.items)
                            .find(|item| item.id == id),
                        _ => None,
                    };
                    if let Some(item) = item {
                        self.jellyfin_player.enqueue(source, item.clone());
                    }
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
            nickname: "Music server".into(),
            server_url: self.server_input.clone(),
            server_id: "server1".into(),
            user_id: "user1".into(),
            device_id: "device1".into(),
        });
        self.servers = self.source.clone().into_iter().collect();
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
        } else if matches!(
            page,
            "jellyfin-albums"
                | "jellyfin-tracks"
                | "jellyfin-error"
                | "jellyfin-search"
                | "jellyfin-artists"
                | "jellyfin-playlists"
                | "jellyfin-playlist"
                | "jellyfin-artist"
                | "jellyfin-history"
        ) {
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
            if matches!(
                page,
                "jellyfin-tracks" | "jellyfin-search" | "jellyfin-playlist" | "jellyfin-history"
            ) {
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
        if page == "jellyfin-search" {
            let songs = match self.rows.take() {
                Some(Rows::Items { items, .. }) => items,
                _ => Vec::new(),
            };
            let albums = self.catalog.items.clone();
            self.search_input = "Bonobo".into();
            self.view = View::Search {
                library: "music1".into(),
                term: "Bonobo".into(),
                kind: None,
            };
            self.rows = Some(Rows::Search(vec![
                (
                    Kind::Song,
                    super::Page {
                        total_record_count: songs.len() as u32,
                        start_index: 0,
                        items: songs,
                    },
                ),
                (
                    Kind::Album,
                    super::Page {
                        total_record_count: albums.len() as u32,
                        start_index: 0,
                        items: albums,
                    },
                ),
            ]));
        }
        if matches!(page, "jellyfin-artists" | "jellyfin-playlists") {
            let kind = if page == "jellyfin-artists" {
                Kind::Artist
            } else {
                Kind::Playlist
            };
            self.catalog.kind = kind;
            for (index, item) in self.catalog.items.iter_mut().enumerate() {
                item.details.kind = Some(kind.item_type().into());
                if kind == Kind::Artist {
                    item.name = Some(
                        [
                            "Bonobo",
                            "Khruangbin",
                            "Nils Frahm",
                            "Floating Points",
                            "Jon Hopkins",
                            "Four Tet",
                        ][index % 6]
                            .into(),
                    );
                }
            }
            self.view = View::Catalog {
                query: Query::Shelf {
                    library: "music1".into(),
                    kind,
                },
                name: if kind == Kind::Artist {
                    "Artists".into()
                } else {
                    "Playlists".into()
                },
            };
            self.rows = Some(Rows::Items {
                items: self.catalog.items.clone(),
                start: 0,
                total: self.catalog.items.len() as u32,
            });
        }
        if page == "jellyfin-playlist" {
            self.view = View::Catalog {
                query: Query::PlaylistSongs {
                    playlist: "playlist1".into(),
                },
                name: "Late night focus".into(),
            };
            if let Some(Rows::Items { items, .. }) = &mut self.rows {
                for (index, item) in items.iter_mut().enumerate() {
                    item.details.playlist_item_id = Some(format!("entry{index}"));
                }
                items[1].id = items[0].id.clone();
            }
        }
        if page == "jellyfin-history" {
            self.view = View::Catalog {
                query: Query::History {
                    library: "music1".into(),
                },
                name: "Recently played".into(),
            };
        }
        if page == "jellyfin-artist" {
            let mut artist = self.catalog.items[0].clone();
            artist.name = Some("Bonobo".into());
            artist.artists.clear();
            self.album = Some(artist);
            self.view = View::Catalog {
                query: Query::ArtistAlbums {
                    library: "music1".into(),
                    artist: "artist1".into(),
                },
                name: "Bonobo".into(),
            };
        }
        if page.starts_with("jellyfin") {
            self.history = vec![(View::Libraries, 0), (self.view.clone(), self.start)];
            self.history_index = 1;
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
    fn old_saved_account_without_nickname_remains_readable() {
        let source: Source = serde_json::from_value(serde_json::json!({
            "id": "id",
            "server_url": "https://music.example/jellyfin/",
            "server_id": "server",
            "user_id": "user",
            "device_id": "device"
        }))
        .unwrap();
        assert!(source.nickname.is_empty());
    }

    #[test]
    fn server_management_keeps_names_per_saved_account_and_activates_by_identity() {
        let mut model = model();
        let mut other = model.source.clone().unwrap();
        other.id = "fedcba9876543210fedcba9876543210".into();
        other.nickname = "Bedroom".into();
        model.servers.push(other.clone());
        let (tx, jobs) = mpsc::channel();
        model.tx = Some(tx);
        model.rename_server(other.id.clone(), "Upstairs".into(), &ProxyConfig::Off);
        assert_eq!(model.servers[1].nickname, "Upstairs");
        assert!(matches!(
            jobs.try_recv().unwrap().command,
            Command::RenameServer(_)
        ));
        model.activate_server(other.id.clone(), &ProxyConfig::Off);
        assert!(
            matches!(jobs.try_recv().unwrap().command, Command::ActivateServer(id) if id == other.id)
        );
    }

    #[test]
    fn adding_a_server_keeps_the_current_account_and_settings_page_while_approving() {
        let mut model = model();
        model.view = View::Settings;
        let source = model.source.clone();
        let rows = model.rows.clone();
        let (tx, jobs) = mpsc::channel();
        model.tx = Some(tx);
        model.server_input = "https://another.example/jellyfin".into();
        model.connect(&ProxyConfig::Off);
        assert_eq!(model.source, source);
        assert_eq!(
            model.rows.as_ref().map(|_| &model.view),
            Some(&View::Settings)
        );
        assert!(matches!(
            jobs.try_recv().unwrap().command,
            Command::Begin(_)
        ));
        assert!(rows.is_some());
    }

    #[test]
    fn provider_navigation_keeps_history_and_shelf_queries_independent() {
        let mut model = model();
        let (tx, jobs) = mpsc::channel();
        model.tx = Some(tx);
        model.set_shelf(Kind::Artist, &ProxyConfig::Off);
        assert!(matches!(
            jobs.try_recv().unwrap().command,
            Command::Browse {
                view: View::Catalog {
                    query: Query::Shelf {
                        kind: Kind::Artist,
                        ..
                    },
                    ..
                },
                start: 0
            }
        ));
        let search = View::Search {
            library: "music1".into(),
            term: "BT".into(),
            kind: Some(Kind::Song),
        };
        model.browse(search.clone(), 100, &ProxyConfig::Off);
        let library = model.catalog.library.clone();
        model.browse(View::Settings, 0, &ProxyConfig::Off);
        assert!(!model.busy);
        model.navigate_history(true, &ProxyConfig::Off);
        assert_eq!(model.view, search);
        assert_eq!(model.start, 100);
        assert_eq!(model.catalog.library, library);
        assert_eq!(model.catalog.kind, Kind::Artist);
        model.browse(View::Libraries, 0, &ProxyConfig::Off);
        assert_eq!(model.history_index + 1, model.history.len());
    }

    #[test]
    fn playlist_edits_preserve_occurrences_restore_failed_changes_and_reject_stale_accounts() {
        let mut model = model();
        model.demo("jellyfin-playlist");
        let source = model.source.clone().unwrap();
        let original = match &model.rows {
            Some(Rows::Items { items, .. }) => items.clone(),
            _ => panic!("fixture tracks"),
        };
        model.edit(
            source.clone(),
            PlaylistEdit::Remove {
                id: "playlist1".into(),
                entries: vec!["entry1".into()],
            },
            &ProxyConfig::Off,
        );
        let request = model.edit_request;
        let Some(Rows::Items { items, .. }) = &model.rows else {
            panic!("tracks")
        };
        assert_eq!(items.len(), original.len() - 1);
        assert!(
            items
                .iter()
                .any(|item| item.details.playlist_item_id.as_deref() == Some("entry0"))
        );
        assert!(
            !items
                .iter()
                .any(|item| item.details.playlist_item_id.as_deref() == Some("entry1"))
        );
        let mut other = source.clone();
        other.user_id = "other".into();
        model.apply(
            Reply::Edited {
                source: other,
                request,
                result: Ok(None),
            },
            &ProxyConfig::Off,
        );
        assert!(model.edit_busy);
        model.apply(
            Reply::Edited {
                source: source.clone(),
                request,
                result: Err("Denied".into()),
            },
            &ProxyConfig::Off,
        );
        assert!(!model.edit_busy);
        assert_eq!(model.edit_error.as_deref(), Some("Denied"));
        let Some(Rows::Items { items, .. }) = &model.rows else {
            panic!("restored")
        };
        assert_eq!(*items, original);
        model.edit(
            source.clone(),
            PlaylistEdit::Rename {
                id: "playlist1".into(),
                name: "New name".into(),
            },
            &ProxyConfig::Off,
        );
        model.browse(View::Settings, 0, &ProxyConfig::Off);
        model.apply(
            Reply::Edited {
                source,
                request: model.edit_request,
                result: Err("Denied".into()),
            },
            &ProxyConfig::Off,
        );
        assert_eq!(model.view, View::Settings);
    }

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
                servers: Vec::new(),
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
