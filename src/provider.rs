//! Provider identities and presentation data for shared views.
//! Browsing selection remains independent of playback ownership and catalogs.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Provider {
    Spotify,
    #[default]
    Jellyfin,
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
    pub shelves: Vec<crate::settings::LibraryShelf>,
    pub sorts: Vec<crate::settings::LibrarySort>,
}

#[derive(Clone, Debug)]
pub enum LibraryAction {
    Filter(String),
    Sort(crate::settings::LibrarySort),
    Collection(String),
    Refresh,
}

impl ItemReference {
    /// Account-scoped context keys also feed the existing recent-context order.
    pub fn context_key(&self) -> String {
        match self {
            Self::Spotify { uri } => uri.clone(),
            Self::Jellyfin {
                source,
                account,
                item,
            } => format!("provider:jellyfin:{source}:{account}:album:{item}"),
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
        let source = self.jellyfin.source.as_ref()?;
        let catalog = &self.jellyfin.catalog;
        let library = catalog.library.as_ref();
        let context = self.jellyfin_player.context();
        let items = catalog.items.iter().map(|item| {
            let reference = ItemReference::Jellyfin { source: source.id.clone(), account: source.user_id.clone(), item: item.id.clone() };
            LibraryItem {
                reference,
                name: item.name.clone().unwrap_or_else(|| "Untitled".into()),
                subtitle: item.artists.join(", "),
                artwork: crate::jellyfin::artwork::Reference::for_item(source, item).map(|art| art.uri()),
                open: Action::Jellyfin(BrowserAction::Browse { view: View::Tracks {
                    library_id: library.map(|lib| lib.id.clone()).unwrap_or_default(),
                    library_name: library.map(|lib| lib.name.clone()).unwrap_or_default(),
                    album_id: item.id.clone(), name: item.name.clone().unwrap_or_default(),
                }, start: 0 }),
                play: library.map(|library| Action::Jellyfin(BrowserAction::PlayAlbum(View::Tracks {
                    library_id: library.id.clone(), library_name: library.name.clone(),
                    album_id: item.id.clone(), name: item.name.clone().unwrap_or_default(),
                }))),
                selected: matches!(&self.jellyfin.view, View::Tracks { album_id, .. } if album_id == &item.id),
                current: self.jellyfin_player.controls_output() && context.as_ref().is_some_and(|(current, album)| current == source && album == &item.id),
            }
        }).collect();
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
            shelves: vec![crate::settings::LibraryShelf::Albums],
            sorts: vec![
                crate::settings::LibrarySort::Library,
                crate::settings::LibrarySort::RecentlyPlayed,
                crate::settings::LibrarySort::Name,
            ],
        })
    }

    pub(crate) fn apply_provider_library(&mut self, action: LibraryAction) {
        if self.jellyfin.provider != Provider::Jellyfin {
            return;
        }
        match action {
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
