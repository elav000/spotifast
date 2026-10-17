//! Provider identities and presentation data for shared views.
//! Browsing selection remains independent of playback ownership and catalogs.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Provider {
    Spotify,
    #[default]
    Jellyfin,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaKind {
    #[default]
    Album,
    Artist,
    Playlist,
    Song,
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
        let name = item.name.clone().unwrap_or_else(|| "Untitled".into());
        let view = match kind {
            Kind::Artist => View::Catalog {
                query: Query::ArtistAlbums {
                    library: library_id.clone(),
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
            _ => View::Tracks {
                library_id,
                library_name: library.map(|lib| lib.name.clone()).unwrap_or_default(),
                album_id: item.id.clone(),
                name: name.clone(),
            },
        };
        let play_view = if kind == Kind::Artist {
            View::Catalog {
                query: Query::ArtistSongs {
                    library: library.map(|lib| lib.id.clone()).unwrap_or_default(),
                    artist: item.id.clone(),
                },
                name: name.clone(),
            }
        } else {
            view.clone()
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
            play: Some(Action::Jellyfin(BrowserAction::PlayAlbum(play_view))),
            kind,
            round: kind == Kind::Artist,
            selected: self.jellyfin.view.context_id() == Some(item.id.as_str()),
            current: self.jellyfin_player.controls_output()
                && context
                    .as_ref()
                    .is_some_and(|(current, context)| current == source && context == &item.id),
        }
    }

    pub fn provider_search_active(&self) -> bool {
        self.jellyfin_selected()
            && matches!(
                self.jellyfin.view,
                crate::jellyfin::browser::View::Search { .. }
            )
    }

    pub(crate) fn submit_provider_search(&mut self, query: String) {
        if let Some(library) = &self.jellyfin.catalog.library {
            let view = crate::jellyfin::browser::View::Search {
                library: library.id.clone(),
                term: query.trim().to_owned(),
                kind: self.jellyfin.search_kind,
            };
            self.jellyfin
                .browse(view, 0, &self.applied_jellyfin_proxy());
        }
    }

    pub(crate) fn apply_provider_navigation(&mut self, action: &crate::model::Action) -> bool {
        use crate::{
            jellyfin::{browser::View, catalog::Kind},
            model::{Action, Page, SearchFilter},
        };
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
                if self.jellyfin.source.as_ref() == Some(&source) {
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
                if !self.provider_search_active() {
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

pub mod queue;
