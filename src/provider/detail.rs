//! Provider-owned presentation data and actions for detail pages.
use crate::{
    app::App,
    i18n::gettext,
    model::{Action, Dialog},
};
use std::sync::Arc;

/// Actions and state supplied by a provider for a shared track row.
pub struct ProviderRowActions {
    pub navigation: Vec<(String, Action)>,
    pub artist_navigation: Vec<(String, Action)>,
    pub provider: super::Provider,
    pub play: Action,
    pub enqueue: Option<Action>,
    pub queue_label: String,
    pub can_enqueue: bool,
    pub queue_drag: Option<crate::model::ProviderQueueDrag>,
    pub current: bool,
    pub playing: bool,
    pub loading: bool,
    pub menu: Vec<(String, Action)>,
    pub playlists: Arc<Vec<(super::ItemReference, String)>>,
    pub playlist_item: super::ItemReference,
    pub playlist_loading: bool,
    pub playlist_error: Option<String>,
    pub create_playlist: Option<Action>,
}

pub(crate) struct ProviderDetailTrack {
    pub track: super::TrackPresentation,
    pub index: usize,
    pub actions: ProviderRowActions,
}

pub(crate) struct ProviderDetailHeader {
    pub title: String,
    pub kind: String,
    pub artwork: Option<String>,
    pub description: Option<String>,
    pub artists: Vec<(String, Option<Action>)>,
    pub year: Option<(String, Option<String>)>,
    pub songs: Option<(u32, Option<u64>)>,
    pub origin: Option<String>,
    pub round: bool,
}

pub(crate) struct DetailPrimaryAction {
    pub label: String,
    pub action: Option<Action>,
    pub pending: bool,
    pub playing: bool,
    pub disabled_reason: Option<String>,
}

pub(crate) struct DetailButton {
    pub label: String,
    pub action: Action,
}

pub(crate) struct DetailMenuItem {
    pub label: String,
    pub icon: crate::theme::Icon,
    pub action: Action,
    pub separator_before: bool,
    pub enabled: bool,
}

pub(crate) struct DetailControls {
    pub primary: Option<DetailPrimaryAction>,
    pub buttons: Vec<DetailButton>,
    pub menu: Vec<DetailMenuItem>,
    pub compact: bool,
}

fn catalog_matches(item: &crate::jellyfin::Item, filter: &str) -> bool {
    let filter = filter.trim().to_lowercase();
    item.name
        .as_deref()
        .unwrap_or_default()
        .to_lowercase()
        .contains(&filter)
        || item
            .artists
            .iter()
            .any(|artist| artist.to_lowercase().contains(&filter))
}

pub(crate) fn provider_artist_navigation(
    source: &crate::jellyfin::browser::Source,
    library: Option<&str>,
    item: &crate::jellyfin::Item,
) -> Vec<(String, Action)> {
    provider_artist_links(source, library, &item.artist_items)
}

pub(crate) fn provider_artist_links(
    source: &crate::jellyfin::browser::Source,
    library: Option<&str>,
    artists: &[crate::jellyfin::ArtistItem],
) -> Vec<(String, Action)> {
    artists
        .iter()
        .filter_map(|artist| {
            artist.name.as_ref().map(|name| {
                (
                    name.clone(),
                    Action::OpenProviderArtist {
                        artist: crate::provider::ItemReference::Jellyfin {
                            source: source.id.clone(),
                            account: source.user_id.clone(),
                            item: artist.id.clone(),
                        },
                        name: name.clone(),
                        library: library.map(str::to_owned),
                    },
                )
            })
        })
        .collect()
}

impl App {
    pub(crate) fn spotify_context_primary(
        &self,
        uri: &str,
        view: Option<Arc<[String]>>,
        filtered: bool,
        radio: bool,
        plays_shown_rows: bool,
    ) -> DetailPrimaryAction {
        let context_here = self.playing_context_uri().as_deref() == Some(uri);
        let playing = context_here && self.believed_playing();
        let can_start = view.as_ref().is_none_or(|uris| !uris.is_empty());
        let play_view = view.is_some()
            && (!self.playing_context_shuffle() || filtered || radio || plays_shown_rows);
        let action = if !(context_here || can_start) {
            None
        } else if context_here {
            Some(Action::TogglePlay)
        } else if let Some(uris) = view.filter(|_| play_view) {
            Some(Action::PlayFromRow {
                context: crate::model::RowContext::View {
                    uris,
                    context_uri: uri.to_owned(),
                    editable_playlist: None,
                },
                uri: String::new(),
                index: 0,
            })
        } else {
            Some(Action::PlayContext {
                uri: uri.to_owned(),
                offset_uri: None,
                offset_index: None,
            })
        };
        DetailPrimaryAction {
            label: crate::i18n::gettext(self.locale, if playing { "Pause" } else { "Play" })
                .into_owned(),
            action,
            pending: self.play_pending(uri),
            playing,
            disabled_reason: Some(
                crate::i18n::gettext(self.locale, "No playable songs in this view").into_owned(),
            ),
        }
    }

    pub(crate) fn spotify_artist_controls(
        &self,
        artist: &crate::api::models::Artist,
    ) -> DetailControls {
        use crate::theme::Icon;
        let following = self.is_saved(&artist.uri).unwrap_or(false);
        DetailControls {
            primary: Some(DetailPrimaryAction {
                label: crate::i18n::gettext(self.locale, "Play").into_owned(),
                action: Some(Action::PlayContext {
                    uri: artist.uri.clone(),
                    offset_uri: None,
                    offset_index: None,
                }),
                pending: self.play_pending(&artist.uri),
                playing: false,
                disabled_reason: None,
            }),
            buttons: vec![DetailButton {
                label: crate::i18n::pgettext(
                    self.locale,
                    "artist",
                    if following { "Following" } else { "Follow" },
                )
                .into_owned(),
                action: Action::ToggleSaved(artist.uri.clone()),
            }],
            menu: vec![
                DetailMenuItem {
                    label: crate::i18n::gettext(self.locale, "Play").into_owned(),
                    icon: Icon::Play,
                    action: Action::PlayContext {
                        uri: artist.uri.clone(),
                        offset_uri: None,
                        offset_index: None,
                    },
                    separator_before: false,
                    enabled: true,
                },
                DetailMenuItem {
                    label: crate::i18n::pgettext(
                        self.locale,
                        "artist",
                        if following { "Unfollow" } else { "Follow" },
                    )
                    .into_owned(),
                    icon: if following {
                        Icon::CircleX
                    } else {
                        Icon::CirclePlus
                    },
                    action: Action::ToggleSaved(artist.uri.clone()),
                    separator_before: false,
                    enabled: true,
                },
                DetailMenuItem {
                    label: crate::i18n::gettext(self.locale, "Go to artist radio").into_owned(),
                    icon: Icon::Radio,
                    action: Action::Open(crate::model::Page::Radio(artist.uri.clone())),
                    separator_before: true,
                    enabled: true,
                },
                DetailMenuItem {
                    label: crate::i18n::gettext(self.locale, "Copy link").into_owned(),
                    icon: Icon::Copy,
                    action: Action::CopyLink(artist.uri.clone()),
                    separator_before: false,
                    enabled: true,
                },
                DetailMenuItem {
                    label: crate::i18n::gettext(self.locale, "Open in Spotify").into_owned(),
                    icon: Icon::ExternalLink,
                    action: Action::OpenInSpotify(artist.uri.clone()),
                    separator_before: false,
                    enabled: true,
                },
            ],
            compact: false,
        }
    }

    pub(crate) fn spotify_album_menu(&self, uri: &str, name: &str) -> Vec<DetailMenuItem> {
        use crate::theme::Icon;
        let saved = self.is_saved(uri).unwrap_or(false);
        vec![
            DetailMenuItem {
                label: crate::i18n::gettext(self.locale, "Play").into_owned(),
                icon: Icon::Play,
                action: Action::PlayContext {
                    uri: uri.to_owned(),
                    offset_uri: None,
                    offset_index: None,
                },
                separator_before: false,
                enabled: true,
            },
            DetailMenuItem {
                label: crate::i18n::gettext(self.locale, "Shuffle play").into_owned(),
                icon: Icon::Shuffle,
                action: Action::ShufflePlay(uri.to_owned()),
                separator_before: false,
                enabled: true,
            },
            DetailMenuItem {
                label: crate::i18n::gettext(self.locale, "Add to queue").into_owned(),
                icon: Icon::ListEnd,
                action: Action::AddToQueue {
                    uri: uri.to_owned(),
                    label: name.to_owned(),
                },
                separator_before: false,
                enabled: true,
            },
            DetailMenuItem {
                label: crate::i18n::gettext(
                    self.locale,
                    if saved {
                        "Remove from Your Library"
                    } else {
                        "Add to Your Library"
                    },
                )
                .into_owned(),
                icon: if saved {
                    Icon::CircleX
                } else {
                    Icon::CirclePlus
                },
                action: Action::ToggleSaved(uri.to_owned()),
                separator_before: false,
                enabled: true,
            },
            DetailMenuItem {
                label: crate::i18n::gettext(self.locale, "Go to album radio").into_owned(),
                icon: Icon::Radio,
                action: Action::Open(crate::model::Page::Radio(uri.to_owned())),
                separator_before: true,
                enabled: true,
            },
            DetailMenuItem {
                label: crate::i18n::gettext(self.locale, "Copy link").into_owned(),
                icon: Icon::Copy,
                action: Action::CopyLink(uri.to_owned()),
                separator_before: false,
                enabled: true,
            },
            DetailMenuItem {
                label: crate::i18n::gettext(self.locale, "Open in Spotify").into_owned(),
                icon: Icon::ExternalLink,
                action: Action::OpenInSpotify(uri.to_owned()),
                separator_before: false,
                enabled: true,
            },
        ]
    }

    pub(crate) fn provider_detail_controls(&self) -> Option<DetailControls> {
        use crate::jellyfin::{
            browser::{BrowserAction as A, Rows, View},
            catalog::Query,
        };
        let view = &self.jellyfin.view;
        if view.context_id().is_none() && !view.is_tracks() {
            return None;
        }
        let source = self.jellyfin.source.as_ref()?;
        let mut controls = DetailControls {
            primary: None,
            buttons: Vec::new(),
            menu: Vec::new(),
            compact: !view.is_detail(),
        };
        if view.is_tracks()
            && let Some(Rows::Items {
                items,
                start: 0,
                total,
            }) = &self.jellyfin.rows
        {
            let action = items.first().map(|first| {
                if matches!(view, View::Tracks { .. }) {
                    Action::Jellyfin(A::Play(first.id.clone()))
                } else {
                    Action::Jellyfin(A::PlayItem {
                        source: source.clone(),
                        item: Box::new(first.clone()),
                        context: view.context_id().unwrap_or("tracks").into(),
                        query: view.track_query().cloned(),
                        index: Some(0),
                    })
                }
            });
            controls.primary = Some(DetailPrimaryAction {
                label: crate::i18n::gettext(
                    self.locale,
                    if matches!(view, View::Tracks { .. }) {
                        "Play album"
                    } else {
                        "Play"
                    },
                )
                .into_owned(),
                action,
                pending: false,
                playing: false,
                disabled_reason: None,
            });
            if let Some(first) = items.first()
                && matches!(view, View::Tracks { .. })
            {
                use crate::theme::Icon;
                let queue_ready = items.len() as u32 == *total
                    && self.jellyfin_player.controls_output()
                    && self.jellyfin_player.snapshot().source_id.as_deref() == Some(&source.id);
                controls.menu.extend([
                    DetailMenuItem {
                        label: crate::i18n::gettext(self.locale, "Play").into_owned(),
                        icon: Icon::Play,
                        action: Action::Jellyfin(A::Play(first.id.clone())),
                        separator_before: false,
                        enabled: true,
                    },
                    DetailMenuItem {
                        label: crate::i18n::gettext(self.locale, "Shuffle play").into_owned(),
                        icon: Icon::Shuffle,
                        action: Action::Jellyfin(A::PlayShuffled(first.id.clone())),
                        separator_before: false,
                        enabled: true,
                    },
                    DetailMenuItem {
                        label: crate::i18n::gettext(self.locale, "Add to queue").into_owned(),
                        icon: Icon::ListEnd,
                        action: Action::Jellyfin(A::EnqueueItems {
                            source: source.clone(),
                            items: items.clone(),
                        }),
                        separator_before: false,
                        enabled: queue_ready,
                    },
                ]);
                if let Some(context) = self
                    .jellyfin
                    .album
                    .as_ref()
                    .filter(|item| view.context_id() == Some(item.id.as_str()))
                {
                    let favorite = context
                        .details
                        .user_data
                        .as_ref()
                        .and_then(|data| data.is_favorite)
                        .unwrap_or(false);
                    controls.menu.push(DetailMenuItem {
                        label: if favorite {
                            gettext(self.locale, "Remove from favorites")
                        } else {
                            gettext(self.locale, "Favorite")
                        }
                        .into_owned(),
                        icon: if favorite {
                            Icon::CircleX
                        } else {
                            Icon::CirclePlus
                        },
                        action: Action::Jellyfin(A::Favorite {
                            source: source.clone(),
                            item: context.id.clone(),
                            favorite: !favorite,
                        }),
                        separator_before: false,
                        enabled: !self.jellyfin.favorite_busy,
                    });
                }
                if let Some(id) = view.context_id() {
                    let url = format!(
                        "{}/web/#/details?id={id}&serverId={}",
                        source.server_url.trim_end_matches('/'),
                        source.server_id
                    );
                    controls.menu.extend([
                        DetailMenuItem {
                            label: crate::i18n::gettext(self.locale, "Copy link").into_owned(),
                            icon: Icon::Copy,
                            action: Action::CopyText(url.clone()),
                            separator_before: true,
                            enabled: true,
                        },
                        DetailMenuItem {
                            label: gettext(self.locale, "Open in Jellyfin").into_owned(),
                            icon: Icon::ExternalLink,
                            action: Action::OpenUrl(url),
                            separator_before: false,
                            enabled: true,
                        },
                    ]);
                }
            }
        }
        if let View::Catalog {
            query: Query::ArtistAlbums { library, artist },
            name,
        } = view
        {
            let songs = View::Catalog {
                query: Query::ArtistSongs {
                    library: library.clone(),
                    artist: artist.clone(),
                },
                name: name.clone(),
            };
            controls.primary = Some(DetailPrimaryAction {
                label: crate::i18n::gettext(self.locale, "Play artist").into_owned(),
                action: Some(Action::Jellyfin(A::PlayAlbum(songs.clone()))),
                pending: false,
                playing: false,
                disabled_reason: None,
            });
            controls.buttons.push(DetailButton {
                label: crate::i18n::gettext(self.locale, "Songs").into_owned(),
                action: Action::Jellyfin(A::Browse {
                    view: songs.clone(),
                    start: 0,
                }),
            });
            use crate::theme::Icon;
            controls.menu.push(DetailMenuItem {
                label: crate::i18n::gettext(self.locale, "Play").into_owned(),
                icon: Icon::Play,
                action: Action::Jellyfin(A::PlayAlbum(songs)),
                separator_before: false,
                enabled: true,
            });
            if let Some(context) = self
                .jellyfin
                .album
                .as_ref()
                .filter(|item| item.id.as_str() == artist.as_str())
            {
                let favorite = context
                    .details
                    .user_data
                    .as_ref()
                    .and_then(|data| data.is_favorite)
                    .unwrap_or(false);
                controls.menu.push(DetailMenuItem {
                    label: if favorite {
                        gettext(self.locale, "Remove from favorites")
                    } else {
                        gettext(self.locale, "Favorite")
                    }
                    .into_owned(),
                    icon: if favorite {
                        Icon::CircleX
                    } else {
                        Icon::CirclePlus
                    },
                    action: Action::Jellyfin(A::Favorite {
                        source: source.clone(),
                        item: context.id.clone(),
                        favorite: !favorite,
                    }),
                    separator_before: false,
                    enabled: !self.jellyfin.favorite_busy,
                });
            }
            let url = format!(
                "{}/web/#/details?id={artist}&serverId={}",
                source.server_url.trim_end_matches('/'),
                source.server_id
            );
            controls.menu.extend([
                DetailMenuItem {
                    label: crate::i18n::gettext(self.locale, "Copy link").into_owned(),
                    icon: Icon::Copy,
                    action: Action::CopyText(url.clone()),
                    separator_before: true,
                    enabled: true,
                },
                DetailMenuItem {
                    label: gettext(self.locale, "Open in Jellyfin").into_owned(),
                    icon: Icon::ExternalLink,
                    action: Action::OpenUrl(url),
                    separator_before: false,
                    enabled: true,
                },
            ]);
        }
        if let View::Catalog {
            query: Query::PlaylistSongs { playlist },
            name,
        } = view
        {
            controls.buttons.push(DetailButton {
                label: crate::i18n::gettext(self.locale, "Rename").into_owned(),
                action: Action::ShowDialog(Dialog::RenamePlaylist {
                    target: super::ItemReference::Jellyfin {
                        source: source.id.clone(),
                        account: source.user_id.clone(),
                        item: playlist.clone(),
                    },
                    name: name.clone(),
                }),
            });
            if self
                .jellyfin
                .album
                .as_ref()
                .is_none_or(|item| item.details.can_delete != Some(false))
            {
                controls.buttons.push(DetailButton {
                    label: crate::i18n::gettext(self.locale, "Delete playlist").into_owned(),
                    action: Action::ShowDialog(Dialog::ConfirmDeletePlaylist {
                        id: playlist.clone(),
                        name: name.clone(),
                        owned: true,
                    }),
                });
            }
        }
        controls.buttons.push(DetailButton {
            label: crate::i18n::gettext(self.locale, "Refresh").into_owned(),
            action: Action::Jellyfin(A::Browse {
                view: view.clone(),
                start: self.jellyfin.start,
            }),
        });
        Some(controls)
    }

    pub(crate) fn spotify_artist_header(
        &self,
        artist: &crate::api::models::Artist,
    ) -> ProviderDetailHeader {
        let mut byline = Vec::new();
        if let Some(followers) = &artist.followers {
            byline.push((
                crate::i18n::ngettext(
                    self.locale,
                    "{count} follower",
                    "{count} followers",
                    u32::try_from(followers.total).unwrap_or(u32::MAX),
                )
                .replace("{count}", &crate::util::format_count(followers.total)),
                None,
            ));
        }
        if !artist.genres.is_empty() {
            byline.push((
                artist
                    .genres
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", "),
                None,
            ));
        }
        ProviderDetailHeader {
            title: artist.name.clone(),
            kind: crate::i18n::gettext(self.locale, "Artist").into_owned(),
            artwork: None,
            description: None,
            artists: byline,
            year: None,
            songs: None,
            origin: None,
            round: true,
        }
    }

    pub(crate) fn spotify_album_header(
        &self,
        album: &crate::api::models::Album,
        tracks: &crate::model::PagedList<crate::api::models::Track>,
    ) -> ProviderDetailHeader {
        let artists = album
            .artists
            .iter()
            .map(|artist| {
                (
                    artist.name.clone(),
                    artist
                        .id
                        .as_deref()
                        .map(|id| super::spotify_artist_action(id, artist.name.clone())),
                )
            })
            .collect();
        let year = album.year().map(|year| {
            let date = album
                .release_date
                .as_deref()
                .filter(|date| date.len() >= 10)
                .unwrap_or(year);
            (
                year.to_owned(),
                Some(crate::util::format_date(self.locale, date)),
            )
        });
        let count = album.total_tracks.unwrap_or(tracks.items.len() as u32);
        let duration = tracks.is_complete().then(|| {
            tracks
                .items
                .iter()
                .map(|track| track.duration_ms as u64)
                .sum()
        });
        ProviderDetailHeader {
            title: album.name.clone(),
            kind: self.album_kind_label(album).into_owned(),
            artwork: None,
            description: None,
            artists,
            year,
            songs: Some((count, duration)),
            origin: None,
            round: false,
        }
    }

    pub(crate) fn provider_detail_header(&self) -> Option<ProviderDetailHeader> {
        use crate::jellyfin::{
            browser::{Rows, View},
            catalog::Query,
        };
        let view = &self.jellyfin.view;
        view.context_id()?;
        let source = self.jellyfin.source.as_ref()?;
        let title = match view {
            View::Tracks { name, .. } | View::Catalog { name, .. } => name.clone(),
            _ => return None,
        };
        let item = self.jellyfin.album.as_ref().or_else(|| {
            if matches!(view, View::Tracks { .. }) {
                match &self.jellyfin.rows {
                    Some(Rows::Items { items, .. }) => items.first(),
                    _ => None,
                }
            } else {
                None
            }
        });
        let kind = match view {
            View::Catalog {
                query: Query::ArtistAlbums { .. } | Query::ArtistSongs { .. },
                ..
            } => super::MediaKind::Artist,
            View::Catalog {
                query: Query::PlaylistSongs { .. },
                ..
            } => super::MediaKind::Playlist,
            _ => super::MediaKind::Album,
        };
        let artist_items = item
            .map(|item| item.artist_items.as_slice())
            .filter(|artists| !artists.is_empty())
            .or_else(|| match &self.jellyfin.rows {
                Some(Rows::Items { items, .. }) => items
                    .iter()
                    .find(|item| !item.artist_items.is_empty())
                    .map(|item| item.artist_items.as_slice()),
                _ => None,
            });
        let library = self
            .jellyfin
            .catalog
            .library
            .as_ref()
            .map(|library| library.id.as_str());
        let links = if matches!(view, View::Tracks { .. }) {
            artist_items
                .map(|artists| provider_artist_links(source, library, artists))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let artists = if links.is_empty() {
            item.map(|item| vec![(item.artists.join(", "), None)])
                .unwrap_or_default()
        } else {
            links
                .into_iter()
                .map(|(name, action)| (name, Some(action)))
                .collect()
        };
        let year = matches!(view, View::Tracks { .. })
            .then(|| {
                item.and_then(|item| {
                    item.details
                        .production_year
                        .filter(|year| *year > 0)
                        .or_else(|| {
                            item.details
                                .premiere_date
                                .as_deref()
                                .and_then(|date| date.get(..4))
                                .and_then(|year| year.parse::<u32>().ok())
                                .filter(|year| *year > 0)
                        })
                })
            })
            .flatten();
        let songs = if matches!(view, View::Tracks { .. }) {
            match &self.jellyfin.rows {
                Some(Rows::Items {
                    items,
                    start: 0,
                    total,
                }) => {
                    let duration = (items.len() == *total as usize)
                        .then(|| {
                            items.iter().try_fold(0_u64, |total, item| {
                                total.checked_add(item.duration_ms()?)
                            })
                        })
                        .flatten();
                    Some((*total, duration))
                }
                _ => None,
            }
        } else {
            None
        };
        let round = kind == super::MediaKind::Artist;
        let kind = match kind {
            super::MediaKind::Artist => "Artist",
            super::MediaKind::Playlist => "Playlist",
            _ => "Album",
        };
        Some(ProviderDetailHeader {
            title,
            kind: crate::i18n::gettext(self.locale, kind).into_owned(),
            artwork: item
                .and_then(|item| crate::jellyfin::artwork::Reference::for_item(source, item))
                .map(|art| art.uri()),
            description: item.and_then(|item| item.details.overview.clone()),
            artists,
            year: year.map(|year| (year.to_string(), None)),
            songs,
            origin: Some("Jellyfin".into()),
            round,
        })
    }

    pub(crate) fn provider_detail_tracks(
        &self,
        items: &[crate::jellyfin::Item],
        start: u32,
        query: Option<crate::jellyfin::catalog::Query>,
        context: &str,
    ) -> Vec<ProviderDetailTrack> {
        use crate::{
            jellyfin::browser::BrowserAction as A,
            provider::{ItemReference, Provider, TrackPresentation},
        };
        let Some(source) = self.jellyfin.source.clone() else {
            return Vec::new();
        };
        let state = self.jellyfin_player.snapshot();
        let playlists = Arc::new(
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
                .collect::<Vec<_>>(),
        );

        let filter = self.jellyfin.filter.clone();
        let mut rows = Vec::new();
        for (index, item) in items
            .iter()
            .enumerate()
            .filter(|(_, item)| catalog_matches(item, &filter))
        {
            let title = item.name.clone().unwrap_or_else(|| "Untitled".into());
            let current = self.jellyfin_player.controls_output()
                && state.source_id.as_deref() == Some(&source.id)
                && state.item.as_ref().is_some_and(|current| {
                    current.id == item.id
                        && current.details.playlist_item_id == item.details.playlist_item_id
                });
            let play = if matches!(
                self.jellyfin.view,
                crate::jellyfin::browser::View::Tracks { .. }
            ) {
                Action::Jellyfin(A::Play(item.id.clone()))
            } else {
                Action::Jellyfin(A::PlayItem {
                    source: source.clone(),
                    item: Box::new(item.clone()),
                    context: context.into(),
                    query: query.clone(),
                    index: Some(start as usize + index),
                })
            };
            let artist_library = match &query {
                Some(
                    crate::jellyfin::catalog::Query::ArtistSongs { library, .. }
                    | crate::jellyfin::catalog::Query::Search { library, .. },
                ) => library.clone(),
                _ => self
                    .jellyfin
                    .catalog
                    .library
                    .as_ref()
                    .map(|library| library.id.clone()),
            };
            let artist_navigation =
                provider_artist_navigation(&source, artist_library.as_deref(), item);
            let mut menu = Vec::new();
            if let Some(crate::jellyfin::catalog::Query::PlaylistSongs { playlist }) = &query
                && let Some(entry) = &item.details.playlist_item_id
            {
                menu.push((
                    "Remove from this playlist".into(),
                    Action::Jellyfin(A::Edit {
                        source: source.clone(),
                        edit: crate::jellyfin::browser::PlaylistEdit::Remove {
                            id: playlist.clone(),
                            entries: vec![entry.clone()],
                        },
                    }),
                ));
                if start as usize + index > 0 {
                    menu.push((
                        "Move up".into(),
                        Action::Jellyfin(A::Edit {
                            source: source.clone(),
                            edit: crate::jellyfin::browser::PlaylistEdit::Move {
                                id: playlist.clone(),
                                entry: entry.clone(),
                                index: start + index as u32 - 1,
                            },
                        }),
                    ));
                }
                if index + 1 < items.len()
                    || matches!(&self.jellyfin.rows, Some(crate::jellyfin::browser::Rows::Items { total, .. }) if start as usize + index + 1 < *total as usize)
                {
                    menu.push((
                        "Move down".into(),
                        Action::Jellyfin(A::Edit {
                            source: source.clone(),
                            edit: crate::jellyfin::browser::PlaylistEdit::Move {
                                id: playlist.clone(),
                                entry: entry.clone(),
                                index: start + index as u32 + 1,
                            },
                        }),
                    ));
                }
            }
            rows.push(ProviderDetailTrack {
                track: TrackPresentation {
                    reference: ItemReference::Jellyfin {
                        source: source.id.clone(),
                        account: source.user_id.clone(),
                        item: item.id.clone(),
                    },
                    title: title.clone(),
                    artists: item.artists.clone(),
                    album: item.album.clone().unwrap_or_default(),
                    artwork: crate::jellyfin::artwork::Reference::for_item(&source, item)
                        .map(|art| art.uri()),
                    duration_ms: item.duration_ms().unwrap_or_default(),
                    number: item.index_number,
                    disc_number: item.parent_index_number,
                },
                index: start as usize + index,
                actions: ProviderRowActions {
                    navigation: Vec::new(),
                    artist_navigation,
                    provider: Provider::Jellyfin,
                    play,
                    enqueue: Some(Action::Jellyfin(A::Queue(item.id.clone()))),
                    queue_label: format!("Add {title} to Jellyfin queue"),
                    can_enqueue: state.item.is_some()
                        && state.source_id.as_deref() == Some(&source.id),
                    queue_drag: (state.source_id.as_deref() == Some(&source.id)).then(|| {
                        crate::model::ProviderQueueDrag {
                            source: source.clone(),
                            item: item.clone(),
                            origin: None,
                        }
                    }),
                    current,
                    playing: state.playing,
                    loading: state.loading,
                    menu,
                    playlists: playlists.clone(),
                    playlist_loading: self.jellyfin.playlists_loading,
                    playlist_error: self.jellyfin.playlists_error.clone(),
                    playlist_item: ItemReference::Jellyfin {
                        source: source.id.clone(),
                        account: source.user_id.clone(),
                        item: item.id.clone(),
                    },
                    create_playlist: Some(Action::ShowDialog(Dialog::CreatePlaylist {
                        name: String::new(),
                        public: false,
                        add_uris: vec![item.id.clone()],
                    })),
                },
            });
        }
        rows
    }
}
