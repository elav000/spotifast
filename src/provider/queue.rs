//! Playback-owned presentation for the existing queue panel and page. Browsing
//! selection never chooses which provider's queue or history the panel shows.
use super::{ItemReference, Provider, TrackPresentation};
use crate::{
    api::models::PlayableItem,
    app::App,
    model::{Action, Loadable},
    ui::widgets::ProviderRowActions,
};
use std::sync::Arc;

fn jellyfin_context_upcoming(state: &crate::jellyfin::transport::Snapshot) -> Vec<usize> {
    let len = state.tracks.len();
    let natural: Vec<_> = (0..len).collect();
    let order = if crate::player::queue::PlaybackOrder::from(state.context_order.as_ref().clone())
        .is_valid_for(len)
    {
        state.context_order.as_slice()
    } else {
        natural.as_slice()
    };
    if state.repeat == crate::player::RepeatMode::Track {
        return (state.index < len)
            .then_some(state.index)
            .into_iter()
            .collect();
    }
    let Some(position) = order.iter().position(|index| *index == state.index) else {
        return Vec::new();
    };
    let mut upcoming = order[position + 1..].to_vec();
    if state.repeat == crate::player::RepeatMode::Context {
        upcoming.extend_from_slice(&order[..=position]);
    }
    upcoming
}

pub enum QueueEntry {
    Spotify(Box<PlayableItem>),
    Provider(Box<(TrackPresentation, ProviderRowActions)>),
}

pub enum QueueStatus {
    Ready,
    Loading,
    Failed(String),
}

pub enum QueueRemoval {
    Spotify,
    Jellyfin {
        source: crate::jellyfin::browser::Source,
        expected: Arc<Vec<crate::jellyfin::Item>>,
    },
}

pub struct RecentPresentation {
    pub rows: Vec<(QueueEntry, Option<String>)>,
    pub loading: bool,
    pub error: Option<String>,
    pub complete: bool,
    pub loaded_once: bool,
    pub load: Option<Action>,
    pub reload: Option<Action>,
}

pub struct QueuePresentation {
    pub removal: Option<QueueRemoval>,
    pub add: Vec<(String, Action)>,
    pub playlists: Vec<(String, Action)>,
    pub status: QueueStatus,
    pub current: Option<QueueEntry>,
    pub rows: Vec<QueueEntry>,
    pub manual_len: usize,
    pub context: Option<(String, Option<Action>)>,
    pub pending: bool,
    pub clear: Option<Action>,
    pub save: Option<Action>,
    pub spotify_drag: bool,
    pub recent: RecentPresentation,
}

impl QueuePresentation {
    /// The same displayed occurrences are used by either queue view. A batch
    /// removal must be atomic so duplicate songs cannot shift under later edits.
    pub fn remove_rows(&self, indices: impl IntoIterator<Item = usize>) -> Option<Action> {
        let indices: Vec<_> = indices
            .into_iter()
            .filter(|index| *index < self.manual_len)
            .collect();
        if indices.is_empty() {
            return None;
        }
        match self.removal.as_ref()? {
            QueueRemoval::Spotify => {
                let rows = indices
                    .into_iter()
                    .filter_map(|index| match self.rows.get(index)? {
                        QueueEntry::Spotify(item) => Some((index, item.uri().to_owned())),
                        _ => None,
                    })
                    .collect();
                Some(Action::RemoveFromQueue { rows })
            }
            QueueRemoval::Jellyfin { source, expected } => Some(Action::Jellyfin(
                crate::jellyfin::browser::BrowserAction::RemoveManual {
                    source: source.clone(),
                    expected: expected.clone(),
                    indices,
                },
            )),
        }
    }
}

impl App {
    pub fn queue_uses_spotify(&self) -> bool {
        !self.jellyfin_player.controls_output()
            && (!self.jellyfin_selected() || self.is_connected() || self.local.is_active())
    }

    pub fn playback_queue(&self) -> QueuePresentation {
        if !self.queue_uses_spotify() {
            return self.jellyfin_queue_presentation();
        }
        let queue = self.queue.get();
        let now = self.now_playing();
        let current = match now {
            Some(now) => queue
                .and_then(|q| q.currently_playing.clone())
                .filter(|item| item.uri() == now.uri)
                .or_else(|| self.now_playing_item()),
            None => queue.and_then(|q| q.currently_playing.clone()),
        };
        QueuePresentation {
            removal: self
                .queue_locally_reorderable()
                .then_some(QueueRemoval::Spotify),
            add: vec![
                (
                    crate::i18n::gettext(self.locale, "Search Spotify").into_owned(),
                    Action::FocusSearch,
                ),
                (
                    crate::i18n::gettext(self.locale, "Liked Songs").into_owned(),
                    Action::Open(crate::model::Page::LikedSongs),
                ),
            ],
            playlists: self
                .library
                .playlists
                .get()
                .map(|playlists| {
                    playlists
                        .iter()
                        .map(|playlist| {
                            (
                                playlist.name.clone(),
                                Action::PlayContext {
                                    uri: playlist.uri.clone(),
                                    offset_uri: None,
                                    offset_index: None,
                                },
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            status: match &self.queue {
                Loadable::Loaded(_) => QueueStatus::Ready,
                Loadable::Failed(error) => QueueStatus::Failed(error.clone()),
                _ => QueueStatus::Loading,
            },
            current: current.map(|item| QueueEntry::Spotify(Box::new(item))),
            rows: queue
                .map(|q| {
                    q.queue
                        .iter()
                        .cloned()
                        .map(|item| QueueEntry::Spotify(Box::new(item)))
                        .collect()
                })
                .unwrap_or_default(),
            manual_len: self.queued_rows_len(),
            context: self
                .playing_from()
                .map(|from| (from.name, from.page.map(Action::Open))),
            pending: false,
            clear: self.can_clear_queue().then_some(Action::ClearQueue),
            save: (!self.queue_playlist_uris().is_empty()).then_some(Action::SaveQueueAsPlaylist),
            spotify_drag: true,
            recent: RecentPresentation {
                rows: self
                    .recents_view
                    .iter()
                    .map(|entry| {
                        (
                            QueueEntry::Spotify(Box::new(PlayableItem::Track(entry.track.clone()))),
                            entry.played_at.clone(),
                        )
                    })
                    .collect(),
                loading: self.recents.loading,
                error: self.recents.error.clone(),
                complete: self.recents.complete,
                loaded_once: self.recents.loaded_once,
                load: self
                    .recents
                    .can_load_more()
                    .then_some(Action::LoadMoreRecents),
                reload: Some(Action::ReloadRecents),
            },
        }
    }

    fn jellyfin_queue_presentation(&self) -> QueuePresentation {
        use crate::jellyfin::browser::{BrowserAction as A, PlaylistEdit, View};
        let state = self.jellyfin_player.snapshot();
        // The transport retains its source even while another catalog is open.
        let context = self.jellyfin_player.context();
        let source = context
            .as_ref()
            .map(|(source, _)| source)
            .or(self.jellyfin.source.as_ref());
        let playlists = Arc::new(
            if let Some(source) =
                source.filter(|source| self.jellyfin.source.as_ref() == Some(*source))
            {
                self.jellyfin
                    .playlists
                    .iter()
                    .map(|playlist| {
                        (
                            ItemReference::Jellyfin {
                                source: source.id.clone(),
                                account: source.user_id.clone(),
                                item: playlist.id.clone(),
                            },
                            playlist.name.clone().unwrap_or_else(|| "Untitled".into()),
                        )
                    })
                    .collect()
            } else {
                Vec::new()
            },
        );
        let make = |item: &crate::jellyfin::Item,
                    play: Action,
                    current: bool,
                    origin: Option<usize>| {
            let source = source?;
            let reference = ItemReference::Jellyfin {
                source: source.id.clone(),
                account: source.user_id.clone(),
                item: item.id.clone(),
            };
            let title = item.name.clone().unwrap_or_else(|| "Untitled".into());
            Some(QueueEntry::Provider(Box::new((
                TrackPresentation {
                    reference: reference.clone(),
                    title: title.clone(),
                    artists: item.artists.clone(),
                    album: item.album.clone().unwrap_or_default(),
                    duration_ms: item.duration_ms().unwrap_or_default(),
                    number: item.index_number,
                    artwork: crate::jellyfin::artwork::Reference::for_item(source, item)
                        .map(|art| art.uri()),
                },
                ProviderRowActions {
                    navigation: item
                        .album_id
                        .as_ref()
                        .map(|album| {
                            (
                                crate::i18n::gettext(self.locale, "Song info").into_owned(),
                                Action::Jellyfin(A::OpenQueueContext {
                                    source: source.clone(),
                                    view: View::Tracks {
                                        library_id: self
                                            .jellyfin
                                            .catalog
                                            .library
                                            .as_ref()
                                            .map(|library| library.id.clone())
                                            .unwrap_or_default(),
                                        library_name: self
                                            .jellyfin
                                            .catalog
                                            .library
                                            .as_ref()
                                            .map(|library| library.name.clone())
                                            .unwrap_or_default(),
                                        album_id: album.clone(),
                                        name: item.album.clone().unwrap_or_else(|| "Album".into()),
                                    },
                                }),
                            )
                        })
                        .into_iter()
                        .collect(),
                    provider: Provider::Jellyfin,
                    play,
                    enqueue: Some(Action::Jellyfin(A::EnqueueItem {
                        source: source.clone(),
                        item: Box::new(item.clone()),
                    })),
                    queue_label: format!("Add {title} to Jellyfin queue"),
                    can_enqueue: state.item.is_some()
                        && state.source_id.as_deref() == Some(&source.id),
                    queue_drag: (state.source_id.as_deref() == Some(&source.id)).then(|| {
                        crate::model::ProviderQueueDrag {
                            source: source.clone(),
                            item: item.clone(),
                            origin: origin.map(|index| (state.queued.snapshot(), index)),
                        }
                    }),
                    current,
                    playing: state.playing,
                    loading: state.loading,
                    menu: Vec::new(),
                    playlists: playlists.clone(),
                    playlist_item: reference,
                    playlist_loading: self.jellyfin.playlists_loading,
                    playlist_error: self.jellyfin.playlists_error.clone(),
                    create_playlist: None,
                },
            ))))
        };
        let upcoming = jellyfin_context_upcoming(&state);
        let mut rows: Vec<_> = state
            .queued
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                make(
                    item,
                    Action::Jellyfin(A::PlayManual(index)),
                    false,
                    Some(index),
                )
            })
            .collect();
        for (index, row) in rows.iter_mut().enumerate() {
            if let (Some(source), QueueEntry::Provider(row)) = (source, row) {
                let edit = |to| {
                    Action::Jellyfin(A::EditManual {
                        source: source.clone(),
                        expected: state.queued.snapshot(),
                        index,
                        to,
                    })
                };
                row.1.menu.push((
                    crate::i18n::gettext(self.locale, "Remove from queue").into_owned(),
                    edit(None),
                ));
                if index > 0 {
                    row.1.menu.push(("Move up".into(), edit(Some(index - 1))));
                }
                if index + 1 < state.queued.len() {
                    row.1.menu.push(("Move down".into(), edit(Some(index + 1))));
                }
            }
        }
        let manual_len = rows.len();
        rows.extend(upcoming.iter().filter_map(|index| {
            state
                .tracks
                .get(*index)
                .and_then(|item| make(item, Action::Jellyfin(A::PlayQueue(*index)), false, None))
        }));
        let mut seen = std::collections::HashSet::new();
        let items: Vec<_> = state
            .item
            .iter()
            .chain(state.queued.iter())
            .chain(upcoming.iter().filter_map(|index| state.tracks.get(*index)))
            .filter(|item| seen.insert(item.id.clone()))
            .map(|item| item.id.clone())
            .collect();
        let save = source
            .filter(|_| !items.is_empty() && state.queue_complete)
            .map(|source| {
                Action::Jellyfin(A::Edit {
                    source: source.clone(),
                    edit: PlaylistEdit::Create {
                        name: self.queue_playlist_name(),
                        public: false,
                        items,
                    },
                })
            });
        let recent = self
            .jellyfin_player
            .recent_items()
            .iter()
            .filter_map(|item| {
                let source = source?;
                let action = Action::Jellyfin(A::PlayItem {
                    source: source.clone(),
                    item: Box::new(item.clone()),
                    context: item.album_id.clone().unwrap_or_else(|| item.id.clone()),
                    query: None,
                    index: None,
                });
                make(item, action, false, None).map(|entry| (entry, None))
            })
            .collect();
        QueuePresentation {
            removal: source.map(|source| QueueRemoval::Jellyfin {
                source: source.clone(),
                expected: state.queued.snapshot(),
            }),
            add: source
                .filter(|source| self.jellyfin.source.as_ref() == Some(*source))
                .map(|source| {
                    let library = self.jellyfin.catalog.library.as_ref();
                    let id = library
                        .map(|library| library.id.clone())
                        .unwrap_or_default();
                    vec![
                        (
                            "Search Jellyfin".into(),
                            Action::Jellyfin(A::OpenQueueContext {
                                source: source.clone(),
                                view: View::Search {
                                    library: id.clone(),
                                    term: String::new(),
                                    kind: None,
                                },
                            }),
                        ),
                        (
                            "Music library".into(),
                            Action::Jellyfin(A::OpenQueueContext {
                                source: source.clone(),
                                view: View::Albums {
                                    library_id: id,
                                    name: library
                                        .map(|library| library.name.clone())
                                        .unwrap_or_else(|| "Music".into()),
                                },
                            }),
                        ),
                    ]
                })
                .unwrap_or_default(),
            playlists: source
                .filter(|source| self.jellyfin.source.as_ref() == Some(*source))
                .map(|source| {
                    self.jellyfin
                        .playlists
                        .iter()
                        .map(|playlist| {
                            let name = playlist.name.clone().unwrap_or_else(|| "Playlist".into());
                            (
                                name.clone(),
                                Action::Jellyfin(A::PlayQueueContext {
                                    source: source.clone(),
                                    view: View::Catalog {
                                        query: crate::jellyfin::catalog::Query::PlaylistSongs {
                                            playlist: playlist.id.clone(),
                                        },
                                        name,
                                    },
                                }),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            status: QueueStatus::Ready,
            current: state.item.as_ref().and_then(|item| {
                make(
                    item,
                    if self.jellyfin_player.controls_output() {
                        Action::TogglePlay
                    } else {
                        Action::Jellyfin(A::Resume)
                    },
                    true,
                    None,
                )
            }),
            rows,
            manual_len,
            context: self.jellyfin_queue_context(&state),
            pending: state.item.is_some() && !state.queue_complete && state.loading,
            clear: (!state.queued.is_empty()).then_some(Action::Jellyfin(A::ClearQueue)),
            save,
            spotify_drag: false,
            recent: RecentPresentation {
                rows: recent,
                loading: false,
                error: None,
                complete: true,
                loaded_once: true,
                load: None,
                reload: None,
            },
        }
    }
    fn jellyfin_queue_context(
        &self,
        state: &crate::jellyfin::transport::Snapshot,
    ) -> Option<(String, Option<Action>)> {
        use crate::jellyfin::{
            browser::{BrowserAction as A, View},
            catalog::Query,
        };
        let (source, id) = self.jellyfin_player.context()?;
        let library = self.jellyfin.catalog.library.as_ref();
        let kind = self.jellyfin_player.context_kind()?;
        let (name, view) = match kind {
            super::MediaKind::Album => {
                let name = state
                    .tracks
                    .get(state.index)
                    .and_then(|item| item.album.clone())
                    .unwrap_or_else(|| "Album".into());
                (
                    name.clone(),
                    View::Tracks {
                        library_id: library.map(|lib| lib.id.clone()).unwrap_or_default(),
                        library_name: library.map(|lib| lib.name.clone()).unwrap_or_default(),
                        album_id: id,
                        name,
                    },
                )
            }
            super::MediaKind::Playlist => {
                let name = self
                    .jellyfin
                    .playlists
                    .iter()
                    .find(|item| item.id == id)
                    .and_then(|item| item.name.clone())
                    .unwrap_or_else(|| "Playlist".into());
                (
                    name.clone(),
                    View::Catalog {
                        query: Query::PlaylistSongs { playlist: id },
                        name,
                    },
                )
            }
            super::MediaKind::Artist => {
                let name = state
                    .tracks
                    .first()
                    .map(|item| item.artists.join(", "))
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| "Artist".into());
                (
                    name.clone(),
                    View::Catalog {
                        query: Query::ArtistAlbums {
                            library: library.map(|lib| lib.id.clone()).unwrap_or_default(),
                            artist: id,
                        },
                        name,
                    },
                )
            }
            _ => return None,
        };
        let open = (self.jellyfin.source.as_ref() == Some(&source))
            .then_some(Action::Jellyfin(A::OpenQueueContext { source, view }));
        Some((name, open))
    }
}
