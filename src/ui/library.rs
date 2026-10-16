//! Full-page library grids: albums, artists, podcasts, episodes.

use crate::api::models::{join_names, pick_image};
use crate::app::App;
use crate::i18n::gettext;
use crate::model::{Action, Page};
use crate::theme::{self, Icon};

use super::widgets;

pub fn show(app: &mut App, ui: &mut egui::Ui, page: Page) {
    let palette = app.palette;
    ui.add_space(8.0);
    let locale = app.locale;
    let (title, empty_title, empty_body) = match page {
        Page::Albums => (
            gettext(locale, "Albums"),
            gettext(locale, "No saved albums"),
            gettext(locale, "Saved albums appear here."),
        ),
        Page::Artists => (
            gettext(locale, "Artists"),
            gettext(locale, "No followed artists"),
            gettext(locale, "Followed artists appear here."),
        ),
        Page::Podcasts => (
            gettext(locale, "Podcasts"),
            gettext(locale, "No podcasts yet"),
            gettext(locale, "Followed podcasts appear here."),
        ),
        _ => (
            gettext(locale, "Episodes"),
            gettext(locale, "No saved episodes"),
            gettext(locale, "Saved episodes appear here."),
        ),
    };
    theme::text(ui, title.as_ref(), theme::bold(28.0), palette.text);
    ui.add_space(14.0);
    match page {
        Page::Albums => {
            let card_height = widgets::card_row_height(ui);
            let count = app.library.albums.items.len();
            widgets::virtual_wrapped_cards(ui, count, card_height, |ui, index| {
                let album = app.library.albums.items[index].album.clone();
                let id = album.id.clone();
                let uri = album.uri.clone();
                let subtitle = join_names(album.artists.iter().map(|artist| artist.name.as_str()));
                let context_here = app.playing_context_uri().as_deref() == Some(uri.as_str());
                let card_art = app.art_px(widgets::CARD_ART);
                let card = item_card(
                    app,
                    ui,
                    &crate::provider::LibraryItem {
                        reference: crate::provider::ItemReference::Spotify { uri: uri.clone() },
                        name: album.name.clone(),
                        subtitle: subtitle.to_string(),
                        artwork: pick_image(&album.images, card_art).map(str::to_owned),
                        open: Action::Open(Page::Album(id)),
                        play: Some(Action::PlayContext {
                            uri,
                            offset_uri: None,
                            offset_index: None,
                        }),
                        menu: Vec::new(),
                        kind: crate::provider::MediaKind::Album,
                        round: false,
                        selected: false,
                        current: context_here,
                    },
                );
                egui::Popup::context_menu(&card.response)
                    .id(ui.make_persistent_id(("library-album-menu", &album.uri)))
                    .frame(widgets::menu_frame(&palette))
                    .show(|ui| widgets::context_menu_items(ui, app, &album.uri, &album.name, None));
            });
            let list = &app.library.albums;
            let (loading, error, can_load, empty) = (
                list.loading,
                list.error.clone(),
                list.can_load_more(),
                list.items.is_empty() && list.loaded_once,
            );
            footer(
                app,
                ui,
                page,
                loading,
                error,
                can_load,
                empty,
                &empty_title,
                &empty_body,
                Icon::Disc,
            );
        }
        Page::Artists => {
            let card_height = widgets::card_row_height(ui);
            let count = app.library.artists.items.len();
            let artist_label = gettext(locale, "Artist");
            widgets::virtual_wrapped_cards(ui, count, card_height, |ui, index| {
                let artist = app.library.artists.items[index].clone();
                let id = artist.id.clone();
                let uri = artist.uri.clone();
                let context_here = app.playing_context_uri().as_deref() == Some(uri.as_str());
                let card_art = app.art_px(widgets::CARD_ART);
                let card = item_card(
                    app,
                    ui,
                    &crate::provider::LibraryItem {
                        reference: crate::provider::ItemReference::Spotify { uri: uri.clone() },
                        name: artist.name.clone(),
                        subtitle: artist_label.to_string(),
                        artwork: pick_image(&artist.images, card_art).map(str::to_owned),
                        open: Action::Open(Page::Artist(id)),
                        play: Some(Action::PlayContext {
                            uri,
                            offset_uri: None,
                            offset_index: None,
                        }),
                        menu: Vec::new(),
                        kind: crate::provider::MediaKind::Artist,
                        round: true,
                        selected: false,
                        current: context_here,
                    },
                );
                egui::Popup::context_menu(&card.response)
                    .id(ui.make_persistent_id(("library-artist-menu", &artist.uri)))
                    .frame(widgets::menu_frame(&palette))
                    .show(|ui| {
                        widgets::context_menu_items(ui, app, &artist.uri, &artist.name, None)
                    });
            });
            let list = &app.library.artists;
            let (loading, error, can_load, empty) = (
                list.loading,
                list.error.clone(),
                list.can_load_more(),
                list.items.is_empty() && list.loaded_once,
            );
            footer(
                app,
                ui,
                page,
                loading,
                error,
                can_load,
                empty,
                &empty_title,
                &empty_body,
                Icon::Users,
            );
        }
        Page::Podcasts => {
            let card_height = widgets::card_row_height(ui);
            let count = app.library.shows.items.len();
            widgets::virtual_wrapped_cards(ui, count, card_height, |ui, index| {
                let show = app.library.shows.items[index].show.clone();
                let id = show.id.clone();
                let card_art = app.art_px(widgets::CARD_ART);
                let card = widgets::card(
                    ui,
                    app,
                    pick_image(&show.images, card_art),
                    &show.name,
                    &show.publisher,
                    widgets::CardCover::default(),
                );
                if card.clicked {
                    app.actions.push(Action::Open(Page::Show(id)));
                }
                egui::Popup::context_menu(&card.response)
                    .id(ui.make_persistent_id(("library-show-menu", &show.uri)))
                    .frame(widgets::menu_frame(&palette))
                    .show(|ui| widgets::context_menu_items(ui, app, &show.uri, &show.name, None));
            });
            let list = &app.library.shows;
            let (loading, error, can_load, empty) = (
                list.loading,
                list.error.clone(),
                list.can_load_more(),
                list.items.is_empty() && list.loaded_once,
            );
            footer(
                app,
                ui,
                page,
                loading,
                error,
                can_load,
                empty,
                &empty_title,
                &empty_body,
                Icon::Mic,
            );
        }
        _ => {
            let episodes: Vec<_> = app
                .library
                .episodes
                .items
                .iter()
                .map(|saved| saved.episode.clone())
                .collect();
            widgets::virtual_rows(
                ui,
                episodes.len(),
                super::show::EPISODE_ROW_HEIGHT,
                |ui, index| {
                    super::show::episode_row(app, ui, &episodes[index], None);
                },
            );
            let list = &app.library.episodes;
            let (loading, error, can_load, empty) = (
                list.loading,
                list.error.clone(),
                list.can_load_more(),
                list.items.is_empty() && list.loaded_once,
            );
            footer(
                app,
                ui,
                page,
                loading,
                error,
                can_load,
                empty,
                &empty_title,
                &empty_body,
                Icon::Bookmark,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn footer(
    app: &mut App,
    ui: &mut egui::Ui,
    page: Page,
    loading: bool,
    error: Option<String>,
    can_load: bool,
    empty: bool,
    empty_title: &str,
    empty_body: &str,
    icon: Icon,
) {
    let palette = app.palette;
    if !app.network_reachable && empty {
        widgets::empty_state(
            ui,
            &palette,
            Icon::WifiOff,
            &gettext(app.locale, "You're offline"),
            &gettext(
                app.locale,
                "Spotifast will reconnect automatically when your connection returns.",
            ),
        );
        return;
    }
    if loading {
        ui.add_space(8.0);
        widgets::loading_row(ui, &palette, app.locale);
    }
    if let Some(error) = error {
        widgets::error_row(ui, app, &error, Some(page.clone()));
    }
    if empty && !loading {
        widgets::empty_state(ui, &palette, icon, empty_title, empty_body);
    }
    widgets::load_more_when_near_end(ui, app, page, can_load && !loading);
}

/// A catalog card with provider-owned identity and actions. The same card and
/// playback interaction are used by saved libraries and provider search pages.
pub fn item_card(
    app: &mut App,
    ui: &mut egui::Ui,
    item: &crate::provider::LibraryItem,
) -> widgets::CardResponse {
    let playing = item.current && app.believed_playing();
    let cover = widgets::CardCover {
        circular: item.round,
        playable: item.play.is_some(),
        playing,
    };
    let card = widgets::card(
        ui,
        app,
        item.artwork.as_deref(),
        &item.name,
        &item.subtitle,
        cover,
    );
    if card.play
        && let Some(action) = &item.play
    {
        app.actions.push(if item.current {
            Action::TogglePlay
        } else {
            action.clone()
        });
    }
    if card.clicked {
        app.actions.push(item.open.clone());
    }
    if !item.menu.is_empty() {
        egui::Popup::context_menu(&card.response)
            .id(ui.make_persistent_id(("provider-catalog-menu", item.reference.uri())))
            .frame(widgets::menu_frame(&app.palette))
            .show(|ui| {
                for (label, action) in &item.menu {
                    if widgets::menu_item(ui, &app.palette, None, label) {
                        app.actions.push(action.clone());
                    }
                }
            });
    }
    card
}

pub fn item_grid(app: &mut App, ui: &mut egui::Ui, items: &[crate::provider::LibraryItem]) {
    let height = widgets::card_row_height(ui);
    widgets::virtual_wrapped_cards(ui, items.len(), height, |ui, index| {
        item_card(app, ui, &items[index]);
    });
}
