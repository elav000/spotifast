//! Provider identities and presentation data for shared views.
//! Browsing selection remains independent of playback ownership and catalogs.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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

/// Navigation is supplied as data; the existing left panel owns its rendering.
pub struct NavigationItem {
    pub label: String,
    pub home: bool,
    pub selected: bool,
    pub action: crate::model::Action,
}

impl crate::app::App {
    pub fn provider_navigation(&self) -> Option<Vec<NavigationItem>> {
        use crate::jellyfin::browser::{BrowserAction, View};
        use crate::model::Action;
        if self.jellyfin.provider != Provider::Jellyfin || self.jellyfin.source.is_none() {
            return None;
        }
        let mut entries = vec![NavigationItem {
            label: crate::i18n::gettext(self.locale, "Home").into_owned(),
            home: true,
            selected: matches!(self.jellyfin.view, View::Libraries),
            action: Action::Jellyfin(BrowserAction::Browse {
                view: View::Libraries,
                start: 0,
            }),
        }];
        entries.extend(self.jellyfin.libraries.iter().map(|library| {
            let selected = matches!(
                &self.jellyfin.view,
                View::Albums { library_id, .. } | View::Tracks { library_id, .. }
                    if library_id == &library.id
            );
            NavigationItem {
                label: library.name.clone(),
                home: false,
                selected,
                action: Action::Jellyfin(BrowserAction::Browse {
                    view: View::Albums {
                        library_id: library.id.clone(),
                        name: library.name.clone(),
                    },
                    start: 0,
                }),
            }
        }));
        Some(entries)
    }
}
