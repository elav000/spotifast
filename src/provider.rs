//! Provider identities and presentation data for shared views.
//! Browsing selection remains independent of playback ownership and catalogs.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Provider {
    Spotify,
    #[default]
    Jellyfin,
}

impl Provider {
    /// Spotify connection and playback state belongs only to Spotify's view.
    pub fn uses_spotify_connection(self) -> bool {
        matches!(self, Self::Spotify)
    }

    /// Soloist is a Spotify playback engine, not a shared player capability.
    pub fn supports_soloist(self) -> bool {
        self.uses_spotify_connection()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaKind {
    #[default]
    Album,
    Artist,
    Playlist,
    Song,
}

/// The device control the shared player bar can render for the active
/// playback provider. Spotify owns Connect routing; Jellyfin uses the host's
/// local audio output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerDeviceControl {
    SpotifyConnect,
    AudioOutput,
}

/// The active provider's central-page route. Shared pages keep one renderer;
/// provider catalogs supply their own presentation through `Collection`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CentralPage {
    Spotify(crate::model::Page),
    Search(Provider),
    Settings,
    Collection,
}

/// Provider-owned shell data consumed by the central page without inspecting
/// service-specific browser state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CentralPagePresentation {
    pub route: CentralPage,
    pub key: String,
    pub overview: bool,
    pub tint_artwork: Option<String>,
}

/// The global search field is shared, while each provider owns its query and
/// page state until combined catalogs exist.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlobalSearchPresentation {
    pub provider: Provider,
    pub query: String,
    pub page_active: bool,
}

/// Provider-owned destinations rendered by the shared now-playing bar.
/// Actions retain the source identity needed to reject stale account links.
pub struct PlaybackContextNavigation {
    pub item: Option<crate::model::Action>,
    pub artists: Vec<Option<crate::model::Action>>,
}

/// Provider-owned state consumed by the shared playlist dialogs.
pub struct PlaylistDialogPresentation {
    pub busy: bool,
    pub error: Option<String>,
    pub delete_body: Option<String>,
}

pub enum DetailLoadMode {
    Scroll,
    Button,
}

pub struct DetailPagination {
    pub mode: DetailLoadMode,
    pub next: Option<crate::model::Action>,
    pub retry: bool,
}

pub fn spotify_artist_pagination(id: &str) -> DetailPagination {
    DetailPagination {
        mode: DetailLoadMode::Button,
        next: Some(crate::model::Action::LoadMoreArtistAlbums(id.to_owned())),
        retry: false,
    }
}

/// Provider identities stay explicit even when the same view draws both catalogs.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ItemReference {
    Spotify {
        uri: String,
    },
    Jellyfin {
        source: String,
        account: String,
        item: String,
    },
}

impl ItemReference {
    pub fn spotify_artist(id: &str) -> Self {
        Self::Spotify {
            uri: format!("spotify:artist:{id}"),
        }
    }

    pub fn provider(&self) -> Provider {
        match self {
            Self::Spotify { .. } => Provider::Spotify,
            Self::Jellyfin { .. } => Provider::Jellyfin,
        }
    }
    pub fn uri(&self) -> String {
        match self {
            Self::Spotify { uri } => uri.clone(),
            Self::Jellyfin { source, item, .. } => format!("jellyfin:{source}:{item}"),
        }
    }
}

pub fn artist_navigation_action(
    reference: ItemReference,
    name: impl Into<String>,
    library: Option<String>,
) -> crate::model::Action {
    crate::model::Action::OpenProviderArtist {
        artist: reference,
        name: name.into(),
        library,
    }
}

pub fn spotify_artist_action(id: &str, name: impl Into<String>) -> crate::model::Action {
    artist_navigation_action(ItemReference::spotify_artist(id), name, None)
}

/// Service-neutral metadata consumed by the existing track-row renderer.
pub struct TrackPresentation {
    pub reference: ItemReference,
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub artwork: Option<String>,
    pub duration_ms: u64,
    pub number: Option<u32>,
}

/// Data and capabilities consumed by the existing Library controls and rows.
/// A future Combined provider can compose these entries while retaining identity.
pub struct LibraryItem {
    pub reference: ItemReference,
    pub name: String,
    pub subtitle: String,
    pub artwork: Option<String>,
    pub open: crate::model::Action,
    pub play: Option<crate::model::Action>,
    pub kind: MediaKind,
    pub menu: Vec<(String, crate::model::Action)>,
    pub round: bool,
    pub selected: bool,
    pub current: bool,
}

pub struct LibraryData {
    pub items: Vec<LibraryItem>,
    pub collections: Vec<(String, String)>,
    pub collection: Option<String>,
    pub filter: String,
    pub sort: crate::settings::LibrarySort,
    pub loading: bool,
    pub error: Option<String>,
    pub home: crate::model::Action,
    pub home_selected: bool,
    pub shelf: crate::settings::LibraryShelf,
    pub shelves: Vec<crate::settings::LibraryShelf>,
    pub sorts: Vec<crate::settings::LibrarySort>,
}

#[derive(Clone, Debug)]
pub enum LibraryAction {
    Filter(String),
    Shelf(crate::settings::LibraryShelf),
    Sort(crate::settings::LibrarySort),
    Collection(String),
    Refresh,
}

impl ItemReference {
    /// Account-scoped context keys also feed the existing recent-context order.
    pub fn context_key(&self) -> String {
        self.context_key_for(MediaKind::Album)
    }
    pub fn context_key_for(&self, kind: MediaKind) -> String {
        let kind = match kind {
            MediaKind::Album => "album",
            MediaKind::Artist => "artist",
            MediaKind::Playlist => "playlist",
            MediaKind::Song => "song",
        };
        match self {
            Self::Spotify { uri } => uri.clone(),
            Self::Jellyfin {
                source,
                account,
                item,
            } => format!("provider:jellyfin:{source}:{account}:{kind}:{item}"),
        }
    }
}

impl crate::app::App {
    pub fn playlist_dialog_presentation(
        &self,
        dialog: &crate::model::Dialog,
    ) -> PlaylistDialogPresentation {
        use crate::{i18n::gettext, model::Dialog};

        let provider_source = self.jellyfin.dialog_source.as_ref();
        let delete_body = match dialog {
            Dialog::ConfirmDeletePlaylist { name, .. } if provider_source.is_some() => Some(
                gettext(
                    self.locale,
                    "Delete “{name}”? This removes the playlist from your Jellyfin server.",
                )
                .replace("{name}", name),
            ),
            Dialog::ConfirmDeletePlaylist {
                name, owned: true, ..
            } => Some(
                gettext(
                    self.locale,
                    "Delete “{name}”? You can recover it from Spotify for 90 days.",
                )
                .replace("{name}", name),
            ),
            Dialog::ConfirmDeletePlaylist {
                name, owned: false, ..
            } => Some(
                gettext(
                    self.locale,
                    "“{name}” will no longer appear in Your Library.",
                )
                .replace("{name}", name),
            ),
            _ => None,
        };
        PlaylistDialogPresentation {
            busy: if provider_source.is_some() {
                self.jellyfin.edit_busy
            } else {
                self.playlist_busy
            },
            error: provider_source.and(self.jellyfin.edit_error.clone()),
            delete_body,
        }
    }

    pub fn playback_context_navigation(
        &self,
        now: &crate::app::NowPlaying,
    ) -> PlaybackContextNavigation {
        use crate::{
            jellyfin::browser::{BrowserAction, View},
            model::{Action, Page},
        };

        match now.provider {
            Provider::Spotify => PlaybackContextNavigation {
                item: now
                    .album_id
                    .as_ref()
                    .map(|id| Action::Open(Page::Album(id.clone())))
                    .or_else(|| {
                        now.show_id
                            .as_ref()
                            .map(|id| Action::Open(Page::Show(id.clone())))
                    }),
                artists: now
                    .artists
                    .iter()
                    .map(|artist| {
                        artist
                            .id
                            .as_deref()
                            .map(|id| spotify_artist_action(id, artist.name.clone()))
                    })
                    .collect(),
            },
            Provider::Jellyfin => {
                let Some((source, _)) = self.jellyfin_player.context() else {
                    return PlaybackContextNavigation {
                        item: None,
                        artists: vec![None; now.artists.len()],
                    };
                };
                let item = now.album_id.as_ref().map(|album_id| {
                    Action::Jellyfin(BrowserAction::OpenQueueContext {
                        source: source.clone(),
                        view: View::Tracks {
                            library_id: String::new(),
                            library_name: String::new(),
                            album_id: album_id.clone(),
                            name: now.album_name.clone(),
                        },
                    })
                });
                let artists = now
                    .artists
                    .iter()
                    .map(|artist| {
                        artist.id.as_ref().map(|id| {
                            artist_navigation_action(
                                ItemReference::Jellyfin {
                                    source: source.id.clone(),
                                    account: source.user_id.clone(),
                                    item: id.clone(),
                                },
                                artist.name.clone(),
                                None,
                            )
                        })
                    })
                    .collect();
                PlaybackContextNavigation { item, artists }
            }
        }
    }

    pub fn central_page_presentation(&self) -> CentralPagePresentation {
        use crate::jellyfin::browser::View;
        use crate::model::Page;

        if self.selected_provider() == Provider::Jellyfin {
            let route = match self.jellyfin.view {
                View::Settings => CentralPage::Settings,
                View::Search { .. } => CentralPage::Search(Provider::Jellyfin),
                _ => CentralPage::Collection,
            };
            let tint_artwork = self.jellyfin.album.as_ref().and_then(|item| {
                self.jellyfin
                    .source
                    .as_ref()
                    .and_then(|source| crate::jellyfin::artwork::Reference::for_item(source, item))
                    .map(|art| art.uri())
            });
            return CentralPagePresentation {
                route,
                key: format!(
                    "jellyfin:{:?}:{:?}",
                    self.jellyfin.source, self.jellyfin.view
                ),
                overview: self.jellyfin.view.context_id().is_none(),
                tint_artwork,
            };
        }

        let page = self.page().clone();
        let route = match page {
            Page::Search => CentralPage::Search(Provider::Spotify),
            Page::Settings => CentralPage::Settings,
            _ => CentralPage::Spotify(page.clone()),
        };
        CentralPagePresentation {
            route,
            key: page.encode(),
            overview: matches!(
                page,
                Page::Home | Page::Search | Page::Settings | Page::Queue
            ),
            tint_artwork: None,
        }
    }

    pub fn global_search_presentation(&self) -> GlobalSearchPresentation {
        match self.selected_provider() {
            Provider::Spotify => GlobalSearchPresentation {
                provider: Provider::Spotify,
                query: self.search.query.clone(),
                page_active: matches!(self.page(), crate::model::Page::Search),
            },
            Provider::Jellyfin => GlobalSearchPresentation {
                provider: Provider::Jellyfin,
                query: self.jellyfin.search_input.clone(),
                page_active: matches!(
                    self.jellyfin.view,
                    crate::jellyfin::browser::View::Search { .. }
                ),
            },
        }
    }

    pub fn set_global_search_query(&mut self, query: String) {
        match self.selected_provider() {
            Provider::Spotify => self.search.query = query,
            Provider::Jellyfin => self.jellyfin.search_input = query,
        }
    }

    pub fn mark_global_search_typed(&mut self) {
        let now = std::time::Instant::now();
        match self.selected_provider() {
            Provider::Spotify => self.search.typed_at = Some(now),
            Provider::Jellyfin => self.jellyfin.search_typed_at = Some(now),
        }
    }

    /// Pagination capability for the selected provider's detail view.
    pub fn provider_detail_pagination(&self) -> Option<DetailPagination> {
        use crate::jellyfin::browser::{BrowserAction, Rows};
        let view = &self.jellyfin.view;
        if !self.jellyfin_selected() || !view.is_detail() {
            return None;
        }
        let mode = if view.is_tracks() {
            DetailLoadMode::Scroll
        } else {
            DetailLoadMode::Button
        };
        let next = match &self.jellyfin.rows {
            Some(Rows::Items {
                items,
                start: 0,
                total,
            }) if !self.jellyfin.busy && !items.is_empty() && (items.len() as u32) < *total => {
                Some(crate::model::Action::Jellyfin(
                    BrowserAction::LoadMoreDetail {
                        view: view.clone(),
                        expected: items.len() as u32,
                    },
                ))
            }
            _ => None,
        };
        Some(DetailPagination {
            mode,
            next,
            retry: self.jellyfin.error.is_some(),
        })
    }

    /// The provider currently selected for browsing.
    pub fn selected_provider(&self) -> Provider {
        if self.jellyfin.enabled {
            self.jellyfin.provider
        } else {
            Provider::Spotify
        }
    }

    pub(crate) fn select_provider(&mut self, provider: Provider) {
        self.jellyfin
            .select(provider, &self.applied_jellyfin_proxy());
        self.wake_soloist_for_selected_provider();
    }

    /// Playback remains visible while another provider's catalog is open.
    /// Starting a song transfers output ownership; browsing never does.
    pub fn playback_now_playing(&self) -> Option<crate::app::NowPlaying> {
        self.now_playing()
            .filter(|now| !now.resuming || now.provider == self.selected_provider())
    }

    /// The provider that owns an active or paused playback surface. A cold
    /// Spotify resume preview is not ownership while another catalog is open.
    pub fn playback_provider(&self) -> Option<Provider> {
        if self.jellyfin_player.controls_output() {
            Some(Provider::Jellyfin)
        } else {
            self.playback_now_playing().map(|now| now.provider)
        }
    }

    /// Describes the device menu for the provider that owns playback. With no
    /// playback yet, the selected catalog supplies the useful empty-state menu.
    pub fn player_device_control(&self) -> PlayerDeviceControl {
        let provider = self
            .playback_provider()
            .unwrap_or_else(|| self.selected_provider());
        match provider {
            Provider::Spotify => PlayerDeviceControl::SpotifyConnect,
            Provider::Jellyfin => PlayerDeviceControl::AudioOutput,
        }
    }

    pub fn provider_library(&self) -> Option<LibraryData> {
        use crate::{
            jellyfin::browser::{BrowserAction, View},
            model::Action,
        };
        if self.jellyfin.provider != Provider::Jellyfin {
            return None;
        }
        self.jellyfin.source.as_ref()?;
        let catalog = &self.jellyfin.catalog;
        let library = catalog.library.as_ref();
        let items = catalog
            .items
            .iter()
            .map(|item| self.jellyfin_library_item(item, catalog.kind))
            .collect();
        Some(LibraryData {
            items,
            collections: self
                .jellyfin
                .libraries
                .iter()
                .map(|lib| (lib.id.clone(), lib.name.clone()))
                .collect(),
            collection: library.map(|lib| lib.id.clone()),
            filter: catalog.filter.clone(),
            sort: catalog
                .sort
                .unwrap_or(crate::settings::LibrarySort::RecentlyPlayed),
            loading: catalog.busy,
            error: catalog.error.clone(),
            home: Action::Jellyfin(BrowserAction::Browse {
                view: View::Libraries,
                start: 0,
            }),
            home_selected: matches!(self.jellyfin.view, View::Libraries),
            shelf: match catalog.kind {
                crate::jellyfin::catalog::Kind::Artist => crate::settings::LibraryShelf::Artists,
                crate::jellyfin::catalog::Kind::Playlist => {
                    crate::settings::LibraryShelf::Playlists
                }
                _ => crate::settings::LibraryShelf::Albums,
            },
            shelves: vec![
                crate::settings::LibraryShelf::Albums,
                crate::settings::LibraryShelf::Artists,
                crate::settings::LibraryShelf::Playlists,
            ],
            sorts: vec![
                crate::settings::LibrarySort::Library,
                crate::settings::LibrarySort::RecentlyPlayed,
                crate::settings::LibrarySort::Name,
            ],
        })
    }

    pub fn jellyfin_selected(&self) -> bool {
        self.jellyfin.enabled && self.jellyfin.provider == Provider::Jellyfin
    }

    pub fn jellyfin_library_item(
        &self,
        item: &crate::jellyfin::Item,
        kind: crate::jellyfin::catalog::Kind,
    ) -> LibraryItem {
        use crate::{
            jellyfin::{
                browser::{BrowserAction, View},
                catalog::{Kind, Query},
            },
            model::Action,
        };
        let source = self
            .jellyfin
            .source
            .as_ref()
            .expect("provider metadata requires a source");
        let library = self.jellyfin.catalog.library.as_ref();
        let library_id = library
            .map(|library| library.id.clone())
            .unwrap_or_default();
        let search_library = match &self.jellyfin.view {
            View::Search { library, .. } => library.clone(),
            _ => library.map(|library| library.id.clone()),
        };
        let name = item.name.clone().unwrap_or_else(|| "Untitled".into());
        let view = match kind {
            Kind::Artist => View::Catalog {
                query: Query::ArtistAlbums {
                    library: search_library.clone(),
                    artist: item.id.clone(),
                },
                name: name.clone(),
            },
            Kind::Playlist => View::Catalog {
                query: Query::PlaylistSongs {
                    playlist: item.id.clone(),
                },
                name: name.clone(),
            },
            Kind::Song => View::Tracks {
                library_id,
                library_name: library.map(|lib| lib.name.clone()).unwrap_or_default(),
                album_id: item.album_id.clone().unwrap_or_else(|| item.id.clone()),
                name: item.album.clone().unwrap_or_else(|| name.clone()),
            },
            Kind::Album => View::Tracks {
                library_id,
                library_name: library.map(|lib| lib.name.clone()).unwrap_or_default(),
                album_id: item.id.clone(),
                name: name.clone(),
            },
        };
        let play = if kind == Kind::Song {
            Action::Jellyfin(BrowserAction::PlayItem {
                source: source.clone(),
                item: Box::new(item.clone()),
                context: item.id.clone(),
                query: None,
                index: None,
            })
        } else {
            let play_view = if kind == Kind::Artist {
                View::Catalog {
                    query: Query::ArtistSongs {
                        library: search_library,
                        artist: item.id.clone(),
                    },
                    name: name.clone(),
                }
            } else {
                view.clone()
            };
            Action::Jellyfin(BrowserAction::PlayAlbum(play_view))
        };
        let context = self.jellyfin_player.context();
        let reference = ItemReference::Jellyfin {
            source: source.id.clone(),
            account: source.user_id.clone(),
            item: item.id.clone(),
        };
        let mut menu = Vec::new();
        if kind == Kind::Playlist {
            menu.push((
                "Rename playlist".into(),
                Action::ShowDialog(crate::model::Dialog::RenamePlaylist {
                    target: reference.clone(),
                    name: name.clone(),
                }),
            ));
            if item.details.can_delete != Some(false) {
                menu.push((
                    "Delete playlist".into(),
                    Action::ShowDialog(crate::model::Dialog::ConfirmDeletePlaylist {
                        id: item.id.clone(),
                        name: name.clone(),
                        owned: true,
                    }),
                ));
            }
        }
        LibraryItem {
            menu,
            reference: ItemReference::Jellyfin {
                source: source.id.clone(),
                account: source.user_id.clone(),
                item: item.id.clone(),
            },
            name,
            subtitle: if kind == Kind::Artist {
                "Artist".into()
            } else if kind == Kind::Playlist {
                "Playlist".into()
            } else {
                item.artists.join(", ")
            },
            artwork: crate::jellyfin::artwork::Reference::for_item(source, item)
                .map(|art| art.uri()),
            open: Action::Jellyfin(BrowserAction::Browse { view, start: 0 }),
            play: Some(play),
            kind,
            round: kind == Kind::Artist,
            selected: self.jellyfin.view.context_id() == Some(item.id.as_str()),
            current: self.jellyfin_player.controls_output()
                && context.as_ref().is_some_and(|(current, context)| {
                    current.same_identity(source) && context == &item.id
                }),
        }
    }

    pub(crate) fn submit_provider_search(&mut self, query: String) {
        let library = match &self.jellyfin.view {
            crate::jellyfin::browser::View::Search { library, .. } => library.clone(),
            _ => None,
        };
        let view = crate::jellyfin::browser::View::Search {
            library,
            term: query.trim().to_owned(),
            kind: self.jellyfin.search_kind,
        };
        self.jellyfin
            .browse(view, 0, &self.applied_jellyfin_proxy());
    }

    pub(crate) fn apply_provider_navigation(&mut self, action: &crate::model::Action) -> bool {
        use crate::{
            jellyfin::{browser::View, catalog::Kind},
            model::{Action, Page, SearchFilter},
        };
        if let Action::OpenProviderArtist {
            artist,
            name,
            library,
        } = action
        {
            match artist {
                ItemReference::Spotify { uri } => {
                    if let Some(Page::Artist(id)) = Page::from_uri(uri) {
                        if self.jellyfin.provider != Provider::Spotify {
                            self.select_provider(Provider::Spotify);
                        }
                        self.open(Page::Artist(id));
                    }
                }
                ItemReference::Jellyfin {
                    source,
                    account,
                    item,
                } => {
                    if !self
                        .jellyfin
                        .source
                        .as_ref()
                        .is_some_and(|current| current.id == *source && current.user_id == *account)
                    {
                        self.toast_error("Jellyfin account changed. Open the song again.");
                        return true;
                    }
                    if self.jellyfin.provider != Provider::Jellyfin {
                        self.select_provider(Provider::Jellyfin);
                    }
                    self.jellyfin.browse(
                        View::Catalog {
                            query: crate::jellyfin::catalog::Query::ArtistAlbums {
                                library: library.clone(),
                                artist: item.clone(),
                            },
                            name: name.clone(),
                        },
                        0,
                        &self.applied_jellyfin_proxy(),
                    );
                }
            }
            return true;
        }
        if let Action::AddProviderPlaylist {
            playlist:
                ItemReference::Jellyfin {
                    source,
                    account,
                    item: playlist,
                },
            item:
                ItemReference::Jellyfin {
                    source: item_source,
                    account: item_account,
                    item,
                },
        } = action
        {
            if source == item_source
                && account == item_account
                && let Some(current) = self
                    .jellyfin
                    .source
                    .clone()
                    .filter(|current| &current.id == source && &current.user_id == account)
            {
                self.jellyfin.edit(
                    current,
                    crate::jellyfin::browser::PlaylistEdit::Add {
                        id: playlist.clone(),
                        items: vec![item.clone()],
                    },
                    &self.applied_jellyfin_proxy(),
                );
            } else {
                self.toast_error("Jellyfin account changed. Open the song again.");
            }
            return true;
        }
        if let Action::RenameProviderPlaylist {
            target:
                ItemReference::Jellyfin {
                    source,
                    account,
                    item,
                },
            name,
        } = action
        {
            if let Some(current) = self
                .jellyfin
                .source
                .clone()
                .filter(|current| &current.id == source && &current.user_id == account)
            {
                self.jellyfin.edit(
                    current,
                    crate::jellyfin::browser::PlaylistEdit::Rename {
                        id: item.clone(),
                        name: name.clone(),
                    },
                    &self.applied_jellyfin_proxy(),
                );
            } else {
                self.toast_error("Jellyfin account changed. Open the playlist again.");
            }
            return true;
        }
        if self.jellyfin_selected()
            && self.jellyfin.dialog_source.is_none()
            && let Action::CreatePlaylist {
                name,
                public,
                add_uris,
            } = action
        {
            if let Some(source) = self.jellyfin.source.clone() {
                self.jellyfin.edit(
                    source,
                    crate::jellyfin::browser::PlaylistEdit::Create {
                        name: name.clone(),
                        public: *public,
                        items: add_uris.clone(),
                    },
                    &self.applied_jellyfin_proxy(),
                );
            } else {
                self.toast_error("Connect to Jellyfin before creating a playlist.");
            }
            return true;
        }
        if let Some(source) = self.jellyfin.dialog_source.clone() {
            let edit = match action {
                Action::CreatePlaylist {
                    name,
                    public,
                    add_uris,
                } => Some(crate::jellyfin::browser::PlaylistEdit::Create {
                    name: name.clone(),
                    public: *public,
                    items: add_uris.clone(),
                }),
                Action::DeletePlaylist(id) => {
                    Some(crate::jellyfin::browser::PlaylistEdit::Delete { id: id.clone() })
                }
                _ => None,
            };
            if let Some(edit) = edit {
                if self
                    .jellyfin
                    .source
                    .as_ref()
                    .is_some_and(|current| current.same_identity(&source))
                {
                    self.jellyfin
                        .edit(source, edit, &self.applied_jellyfin_proxy());
                } else {
                    self.toast_error("Jellyfin account changed. Open the playlist again.");
                }
                return true;
            }
        }
        if matches!(action, Action::ShowDialog(_)) {
            self.jellyfin.dialog_source = self
                .jellyfin_selected()
                .then(|| self.jellyfin.source.clone())
                .flatten();
        }
        if matches!(action, Action::CloseDialog) {
            self.jellyfin.dialog_source = None;
        }
        if !self.jellyfin_selected() {
            return false;
        }
        match action {
            Action::Open(Page::Home) => {
                self.jellyfin
                    .browse(View::Libraries, 0, &self.applied_jellyfin_proxy())
            }
            Action::Open(Page::Settings) => {
                self.jellyfin
                    .browse(View::Settings, 0, &self.applied_jellyfin_proxy())
            }
            Action::Open(Page::Search) | Action::FocusSearch => {
                if matches!(action, Action::FocusSearch) {
                    self.search.focus_requested = true;
                }
                if !self.global_search_presentation().page_active {
                    self.submit_provider_search(self.jellyfin.search_input.clone());
                }
            }
            Action::Search(query) => {
                self.jellyfin.search_input = query.clone();
                self.submit_provider_search(query.clone());
            }
            Action::SetSearchFilter(filter) => {
                self.jellyfin.search_kind = match filter {
                    SearchFilter::Songs => Some(Kind::Song),
                    SearchFilter::Artists => Some(Kind::Artist),
                    SearchFilter::Albums => Some(Kind::Album),
                    SearchFilter::Playlists => Some(Kind::Playlist),
                    _ => None,
                };
                self.submit_provider_search(self.jellyfin.search_input.clone());
            }
            Action::Back | Action::Forward => self.jellyfin.navigate_history(
                matches!(action, Action::Back),
                &self.applied_jellyfin_proxy(),
            ),
            Action::EndSearchRun => {}
            _ => return false,
        }
        true
    }

    pub(crate) fn tick_global_search(
        &mut self,
        ctx: &egui::Context,
        debounce: std::time::Duration,
    ) {
        let typed = match self.selected_provider() {
            Provider::Spotify => self.search.typed_at,
            Provider::Jellyfin => self.jellyfin.search_typed_at,
        };
        let Some(typed) = typed else {
            return;
        };
        if typed.elapsed() < debounce {
            ctx.request_repaint_after(debounce - typed.elapsed());
            return;
        }
        match self.selected_provider() {
            Provider::Spotify => {
                self.search.typed_at = None;
                let query = self.search.query.trim().to_string();
                // A pasted link waits for Enter rather than being searched
                // for as words.
                if crate::link::parse(&query).is_none() {
                    self.run_typed_search(query);
                }
            }
            Provider::Jellyfin => {
                self.jellyfin.search_typed_at = None;
                self.submit_provider_search(self.jellyfin.search_input.clone());
            }
        }
    }

    pub(crate) fn apply_provider_library(&mut self, action: LibraryAction) {
        if self.jellyfin.provider != Provider::Jellyfin {
            return;
        }
        match action {
            LibraryAction::Shelf(shelf) => {
                let kind = match shelf {
                    crate::settings::LibraryShelf::Artists => {
                        crate::jellyfin::catalog::Kind::Artist
                    }
                    crate::settings::LibraryShelf::Playlists => {
                        crate::jellyfin::catalog::Kind::Playlist
                    }
                    _ => crate::jellyfin::catalog::Kind::Album,
                };
                self.jellyfin
                    .set_shelf(kind, &self.applied_jellyfin_proxy());
            }
            LibraryAction::Filter(value) => self.jellyfin.catalog.filter = value,
            LibraryAction::Sort(value)
                if matches!(
                    value,
                    crate::settings::LibrarySort::Name
                        | crate::settings::LibrarySort::Library
                        | crate::settings::LibrarySort::RecentlyPlayed
                ) =>
            {
                self.jellyfin.catalog.sort = Some(value)
            }
            LibraryAction::Collection(id) => self
                .apply_jellyfin_action(crate::jellyfin::browser::BrowserAction::SelectLibrary(id)),
            LibraryAction::Refresh => {
                if let Some(library) = self.jellyfin.catalog.library.clone() {
                    let filter = self.jellyfin.catalog.filter.clone();
                    let sort = self.jellyfin.catalog.sort;
                    self.jellyfin
                        .select_catalog(library, &self.applied_jellyfin_proxy());
                    self.jellyfin.catalog.filter = filter;
                    self.jellyfin.catalog.sort = sort;
                }
            }
            _ => {}
        }
    }
}

pub(crate) mod detail;
pub use detail::ProviderRowActions;
pub mod queue;
