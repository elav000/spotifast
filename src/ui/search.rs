//! The Search page.

use std::sync::Arc;

use egui::{Align, CornerRadius, Layout, Rect, Sense, UiBuilder, Vec2, pos2, vec2};

use crate::api::models::{Artist, ArtistRef, PlayableItem, SearchResults, pick_image};
use crate::app::App;
use crate::i18n::gettext;
use crate::model::{Action, Loadable, Page, RowContext, SearchFilter};
use crate::theme::{self, Icon};

use super::widgets::{self, TrackRow};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let provider = app.global_search_presentation().provider;
    show_for_provider(app, ui, provider);
}

pub fn combined(app: &mut App, ui: &mut egui::Ui) {
    let query = app.search.query.trim().to_owned();
    if query.is_empty() && app.search.typed_at.is_none() {
        widgets::empty_state(
            ui,
            &app.palette,
            Icon::Search,
            &gettext(app.locale, "Search"),
            &gettext(
                app.locale,
                "Find songs, artists, albums, playlists, and podcasts.",
            ),
        );
        return;
    }

    let palette = app.palette;
    let labels: Vec<_> = SearchFilter::ALL
        .iter()
        .map(|filter| (*filter, filter.label(app.locale)))
        .collect();
    let options: Vec<(SearchFilter, &str)> = labels
        .iter()
        .map(|(filter, label)| (*filter, label.as_ref()))
        .collect();
    if let Some(filter) = widgets::chips(ui, &palette, &options, app.search.filter) {
        app.actions.push(Action::SetSearchFilter(filter));
    }
    ui.add_space(16.0);

    theme::section_title(ui, &app.palette, "Spotify");
    ui.add_space(6.0);
    let pending = app.search.catalogue_pending || app.search.playlists_pending;
    if pending {
        widgets::loading_row(ui, &palette, app.locale);
    }
    if let Some(error) = app.search.error.clone() {
        widgets::error_row(ui, app, &error, None);
    }
    if let Loadable::Loaded(results) = app.search.results.clone() {
        ui.push_id("combined-spotify", |ui| match app.search.filter {
            SearchFilter::All => {
                let items = app.spotify_search_items(&results, SearchFilter::All);
                combined_result_rows(app, ui, &items, "Spotify");
            }
            SearchFilter::Podcasts => shows_grid(app, ui, &results),
            SearchFilter::Episodes => episodes(app, ui, &results, 12),
            filter => {
                let items = app.spotify_search_items(&results, filter);
                super::library::item_grid(app, ui, &items);
            }
        });
    } else if !pending && app.search.error.is_none() {
        widgets::skeleton_rows(ui, &palette, app.locale, 3);
    }

    let sources = app.jellyfin.combined_search.sources.clone();
    for source in sources {
        ui.add_space(28.0);
        let label = if source.source.nickname.trim().is_empty() {
            source.source.server_url.as_str()
        } else {
            source.source.nickname.trim()
        };
        theme::section_title(ui, &app.palette, label);
        ui.add_space(6.0);
        if source.loading {
            widgets::loading_row(ui, &app.palette, app.locale);
        }
        if let Some(error) = &source.error {
            widgets::error_row(ui, app, error, None);
        }
        let Some(sections) = source.sections else {
            continue;
        };
        let limit = if app.search.filter == SearchFilter::All {
            1
        } else {
            12
        };
        let cards: Vec<_> = sections
            .into_iter()
            .flat_map(|(kind, page)| {
                page.items
                    .into_iter()
                    .take(limit)
                    .map(move |item| (kind, item))
            })
            .map(|(kind, item)| app.jellyfin_search_item(&source.source, &item, kind))
            .collect();
        ui.push_id(("combined-source", &source.source.id), |ui| {
            if app.search.filter == SearchFilter::All {
                combined_result_rows(app, ui, &cards, label);
            } else {
                super::library::item_grid(app, ui, &cards);
            }
        });
    }
}

fn combined_result_rows(
    app: &mut App,
    ui: &mut egui::Ui,
    items: &[crate::provider::LibraryItem],
    source: &str,
) {
    for item in items {
        ui.push_id(("combined-result", item.reference.uri()), |ui| {
            combined_result_row(app, ui, item, source);
        });
    }
}

fn combined_result_row(
    app: &mut App,
    ui: &mut egui::Ui,
    item: &crate::provider::LibraryItem,
    source: &str,
) {
    const HEIGHT: f32 = 64.0;
    const ART: f32 = 48.0;
    let palette = app.palette;
    let kind = match item.kind {
        crate::provider::MediaKind::Album => gettext(app.locale, "Album"),
        crate::provider::MediaKind::Artist => gettext(app.locale, "Artist"),
        crate::provider::MediaKind::Playlist => gettext(app.locale, "Playlist"),
        crate::provider::MediaKind::Song => gettext(app.locale, "Song"),
    };
    let accessibility_label = format!("{}, {}, {source}", item.name, kind);
    let metadata =
        if item.subtitle.trim().is_empty() || item.subtitle == kind || item.subtitle == source {
            format!("{kind} • {source}")
        } else {
            format!("{kind} • {} • {source}", item.subtitle)
        };
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            ui.is_enabled(),
            accessibility_label.clone(),
        )
    });
    if response.gained_focus() {
        response.scroll_to_me(None);
    }
    if ui.is_rect_visible(rect) {
        if response.hovered() || response.has_focus() {
            ui.painter().rect_filled(
                rect,
                CornerRadius::same(theme::RADIUS),
                palette.surface_hover,
            );
        }
        let art = Rect::from_center_size(
            pos2(rect.left() + 8.0 + ART / 2.0, rect.center().y),
            Vec2::splat(ART),
        );
        widgets::paint_cover(
            ui,
            &palette,
            item.artwork.as_deref(),
            art,
            if item.round { ART / 2.0 } else { 4.0 },
            Icon::Music,
            Some(app.backend.art()),
        );
        let left = art.right() + 12.0;
        let right = rect.right() - if item.play.is_some() { 48.0 } else { 12.0 };
        let title = widgets::ellipsized(
            ui,
            &item.name,
            theme::semibold(15.0),
            palette.text,
            (right - left).max(1.0),
            1,
        );
        let subtitle = widgets::ellipsized(
            ui,
            &metadata,
            theme::regular(12.5),
            palette.secondary,
            (right - left).max(1.0),
            1,
        );
        ui.painter().galley(
            pos2(left, rect.center().y - title.size().y - 1.0),
            title,
            palette.text,
        );
        ui.painter().galley(
            pos2(left, rect.center().y + 3.0),
            subtitle,
            palette.secondary,
        );
    }

    let mut played = false;
    if let Some(play) = &item.play {
        let play_rect = Rect::from_center_size(
            pos2(rect.right() - 24.0, rect.center().y),
            Vec2::splat(36.0),
        );
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(play_rect)
                .layout(Layout::centered_and_justified(egui::Direction::LeftToRight)),
        );
        let playing = item.current && app.believed_playing();
        let tooltip = gettext(app.locale, if playing { "Pause" } else { "Play" });
        if theme::icon_button(
            &mut child,
            if playing {
                Icon::PauseFilled
            } else {
                Icon::PlayFilled
            },
            17.0,
            palette.secondary,
            palette.text,
            &tooltip,
        )
        .clicked()
        {
            app.actions.push(if item.current {
                Action::TogglePlay
            } else {
                play.clone()
            });
            played = true;
        }
    }
    if response.clicked() && !played {
        app.actions.push(item.open.clone());
    }
    if !item.menu.is_empty() {
        egui::Popup::context_menu(&response)
            .id(ui.make_persistent_id(("combined-result-menu", item.reference.uri())))
            .frame(widgets::menu_frame(&palette))
            .show(|ui| {
                for (label, action) in &item.menu {
                    if widgets::menu_item(ui, &palette, None, label) {
                        app.actions.push(action.clone());
                    }
                }
            });
    }
    theme::focus_ring(ui, &response);
}

pub fn show_for_provider(app: &mut App, ui: &mut egui::Ui, source: crate::provider::Provider) {
    if source == crate::provider::Provider::Jellyfin {
        provider(app, ui);
        return;
    }
    let palette = app.palette;
    if app.search.committed.is_empty() && app.search.typed_at.is_none() {
        recent(app, ui);
        return;
    }
    ui.add_space(4.0);
    let labels: Vec<_> = SearchFilter::ALL
        .iter()
        .map(|f| (*f, f.label(app.locale)))
        .collect();
    let options: Vec<(SearchFilter, &str)> = labels
        .iter()
        .map(|(filter, label)| (*filter, label.as_ref()))
        .collect();
    if let Some(filter) = widgets::chips(ui, &palette, &options, app.search.filter) {
        app.actions.push(Action::SetSearchFilter(filter));
    }
    ui.add_space(12.0);
    let pending = app.search.catalogue_pending || app.search.playlists_pending;
    if pending {
        widgets::loading_row(ui, &palette, app.locale);
    }
    if let Some(error) = app.search.error.clone() {
        widgets::error_row(ui, app, &error, None);
    }
    let results = match &app.search.results {
        Loadable::Loaded(results) => results.clone(),
        Loadable::Loading | Loadable::NotLoaded => {
            if !pending {
                widgets::skeleton_rows(ui, &palette, app.locale, 6);
            }
            return;
        }
        Loadable::Failed(error) => {
            let error = error.clone();
            if app.search.error.is_none() {
                widgets::error_row(ui, app, &error, None);
            }
            return;
        }
    };
    if results.is_empty() && (pending || app.search.error.is_some()) {
        return;
    }
    if results.is_empty() {
        widgets::empty_state(
            ui,
            &palette,
            Icon::Search,
            // Translators: {query} is the text the user searched for.
            &gettext(app.locale, "No results for “{query}”")
                .replace("{query}", &app.search.committed),
            &gettext(app.locale, "Check the spelling, or try fewer words."),
        );
        return;
    }
    match app.search.filter {
        SearchFilter::All => all(app, ui, &results),
        SearchFilter::Songs => songs(app, ui, &results, usize::MAX),
        SearchFilter::Artists => artists_grid(app, ui, &results),
        SearchFilter::Albums => albums_grid(app, ui, &results),
        SearchFilter::Playlists => playlists_grid(app, ui, &results),
        SearchFilter::Podcasts => shows_grid(app, ui, &results),
        SearchFilter::Episodes => episodes(app, ui, &results, usize::MAX),
    }
}

fn recent(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    ui.add_space(6.0);
    if !app.network_reachable && app.settings.search_history.is_empty() {
        widgets::empty_state(
            ui,
            &palette,
            Icon::WifiOff,
            &gettext(app.locale, "You're offline"),
            &gettext(
                app.locale,
                "Connect to the internet to search songs, artists, and playlists.",
            ),
        );
        return;
    }
    if app.settings.search_history.is_empty() {
        widgets::empty_state(
            ui,
            &palette,
            Icon::Search,
            &gettext(app.locale, "Search Spotify"),
            &gettext(
                app.locale,
                "Find songs, artists, albums, playlists, and podcasts.",
            ),
        );
        return;
    }
    theme::section_title(ui, &palette, &gettext(app.locale, "Recent searches"));
    ui.add_space(6.0);
    let history = app.settings.search_history.clone();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        for query in &history {
            let (response, forget) = theme::soft_button_dismiss(ui, &palette, Icon::Clock, query);
            if forget {
                app.actions.push(Action::ForgetSearch(query.clone()));
            } else if response.clicked() {
                app.actions.push(Action::Search(query.clone()));
            }
        }
    });
}

fn all(app: &mut App, ui: &mut egui::Ui, results: &SearchResults) {
    let palette = app.palette;
    let locale = app.locale;
    let query = app.search.committed.to_lowercase();
    let top_artist = results
        .artists
        .as_ref()
        .and_then(|page| page.items.first())
        .filter(|artist| {
            artist.name.to_lowercase() == query
                || results.tracks.as_ref().is_none_or(|t| t.items.is_empty())
        });
    let top_art = app.art_px(TOP_RESULT_ART);
    let wide = ui.available_width() > 720.0;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 24.0;
        let top_width = if wide {
            (ui.available_width() * 0.36).clamp(240.0, 380.0)
        } else {
            ui.available_width()
        };
        ui.vertical(|ui| {
            ui.set_width(top_width);
            theme::section_title(ui, &palette, &gettext(locale, "Top result"));
            ui.add_space(4.0);
            if let Some(artist) = top_artist {
                top_result(
                    app,
                    ui,
                    pick_image(&artist.images, top_art),
                    &artist.name,
                    TopResultSubtitle::Text(&gettext(locale, "Artist")),
                    true,
                    Some(artist.uri.clone()),
                    Page::Artist(artist.id.clone()),
                    |ui, app| {
                        widgets::context_menu_items(ui, app, &artist.uri, &artist.name, None);
                    },
                );
            } else if let Some(track) = results.tracks.as_ref().and_then(|page| page.items.first())
            {
                let page = track
                    .album
                    .as_ref()
                    .map(|album| Page::Album(album.id.clone()))
                    .unwrap_or(Page::Search);
                top_result(
                    app,
                    ui,
                    track.image(top_art),
                    &track.name,
                    TopResultSubtitle::SongArtists(&track.artists),
                    false,
                    Some(track.uri.clone()),
                    page,
                    |ui, app| {
                        widgets::item_menu(
                            ui,
                            app,
                            &PlayableItem::Track(track.clone()),
                            None,
                            None,
                        );
                    },
                );
            } else if let Some(album) = results.albums.as_ref().and_then(|page| page.items.first())
            {
                top_result(
                    app,
                    ui,
                    pick_image(&album.images, top_art),
                    &album.name,
                    TopResultSubtitle::Text(
                        // Translators: {artists} is the album's artist names.
                        &gettext(locale, "Album • {artists}").replace(
                            "{artists}",
                            &crate::api::models::join_names(
                                album.artists.iter().map(|a| a.name.as_str()),
                            ),
                        ),
                    ),
                    false,
                    Some(album.uri.clone()),
                    Page::Album(album.id.clone()),
                    |ui, app| {
                        widgets::context_menu_items(ui, app, &album.uri, &album.name, None);
                    },
                );
            } else if let Some(playlist) = results
                .playlists
                .as_ref()
                .and_then(|page| page.items.first())
            {
                top_result(
                    app,
                    ui,
                    pick_image(&playlist.images, top_art),
                    &playlist.name,
                    TopResultSubtitle::Text(
                        // Translators: {owner} is the name of the playlist's owner.
                        &gettext(locale, "Playlist • {owner}")
                            .replace("{owner}", playlist.owner_name()),
                    ),
                    false,
                    Some(playlist.uri.clone()),
                    Page::Playlist(playlist.id.clone()),
                    |ui, app| {
                        let owned = app.user_id().is_some_and(|id| playlist.owned_by(id));
                        widgets::context_menu_items(
                            ui,
                            app,
                            &playlist.uri,
                            &playlist.name,
                            owned.then_some(playlist),
                        );
                    },
                );
            } else if let Some(show) = results.shows.as_ref().and_then(|page| page.items.first()) {
                top_result(
                    app,
                    ui,
                    pick_image(&show.images, top_art),
                    &show.name,
                    TopResultSubtitle::Text(
                        // Translators: {publisher} is the podcast's publisher.
                        &gettext(locale, "Podcast • {publisher}")
                            .replace("{publisher}", &show.publisher),
                    ),
                    false,
                    Some(show.uri.clone()),
                    Page::Show(show.id.clone()),
                    |ui, app| {
                        widgets::context_menu_items(ui, app, &show.uri, &show.name, None);
                    },
                );
            }
        });
        if wide {
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                songs(app, ui, results, 4);
            });
        }
    });
    if !wide {
        ui.add_space(12.0);
        songs(app, ui, results, 4);
    }
    ui.add_space(8.0);
    shelf_artists(app, ui, results);
    shelf_albums(app, ui, results);
    shelf_playlists(app, ui, results);
    shelf_shows(app, ui, results);
    if results
        .episodes
        .as_ref()
        .is_some_and(|page| !page.items.is_empty())
    {
        theme::section_title(ui, &palette, &gettext(locale, "Episodes"));
        ui.add_space(4.0);
        episodes(app, ui, results, 4);
    }
}

enum TopResultSubtitle<'a> {
    Text(&'a str),
    SongArtists(&'a [ArtistRef]),
}

/// The top result's cover size.
const TOP_RESULT_ART: f32 = 96.0;

#[allow(clippy::too_many_arguments)]
fn top_result(
    app: &mut App,
    ui: &mut egui::Ui,
    image: Option<&str>,
    title: &str,
    subtitle: TopResultSubtitle<'_>,
    round: bool,
    play_uri: Option<String>,
    page: Page,
    menu: impl FnOnce(&mut egui::Ui, &mut App),
) {
    let palette = app.palette;
    let mut subtitle_clicked = false;
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), 232.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = ui.rect_contains_pointer(rect);
        let fill = if hovered {
            palette.surface_hover
        } else {
            palette.surface
        };
        ui.painter()
            .rect_filled(rect, CornerRadius::same(theme::RADIUS), fill);
        let image_rect =
            Rect::from_min_size(rect.min + vec2(20.0, 20.0), Vec2::splat(TOP_RESULT_ART));
        widgets::paint_shadow(ui, &palette, image_rect, if round { 48.0 } else { 6.0 });
        widgets::paint_cover(
            ui,
            &palette,
            image,
            image_rect,
            if round { 48.0 } else { 6.0 },
            if round { Icon::User } else { Icon::Music },
            Some(app.backend.art()),
        );
        let text_clip = Rect::from_min_max(
            pos2(rect.left() + 20.0, image_rect.bottom() + 12.0),
            pos2(rect.right() - 20.0, rect.bottom()),
        );
        let painter = ui.painter().with_clip_rect(text_clip);
        crate::bidi::paint_line(
            &painter,
            text_clip.left(),
            text_clip.right(),
            text_clip.top() + 16.0,
            title,
            theme::bold(26.0),
            palette.text,
        );
        match subtitle {
            TopResultSubtitle::SongArtists(artists) => {
                let subtitle_rect = Rect::from_min_max(
                    pos2(text_clip.left(), text_clip.top() + 36.0),
                    pos2(text_clip.right(), text_clip.top() + 56.0),
                );
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(subtitle_rect)
                        .layout(Layout::left_to_right(Align::Center)),
                );
                child.set_clip_rect(subtitle_rect.intersect(ui.clip_rect()));
                child.spacing_mut().item_spacing.x = 0.0;
                theme::text(
                    &mut child,
                    // Translators: Precedes the song's artist names, which follow as links.
                    gettext(app.locale, "Song • ").as_ref(),
                    theme::regular(13.5),
                    palette.secondary,
                );
                subtitle_clicked = widgets::artist_links(
                    &mut child,
                    app,
                    artists,
                    theme::regular(13.5),
                    palette.secondary,
                );
            }
            TopResultSubtitle::Text(subtitle) => {
                crate::bidi::paint_line(
                    &painter,
                    text_clip.left(),
                    text_clip.right(),
                    text_clip.top() + 46.0,
                    subtitle,
                    theme::regular(13.5),
                    palette.secondary,
                );
            }
        }
        if hovered && let Some(uri) = &play_uri {
            let button = Rect::from_center_size(
                pos2(rect.right() - 44.0, rect.bottom() - 44.0),
                Vec2::splat(48.0),
            );
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(button)
                    .layout(Layout::centered_and_justified(egui::Direction::LeftToRight)),
            );
            if theme::circle_button(
                &mut child,
                Icon::PlayFilled,
                48.0,
                palette.accent,
                palette.accent_hover,
                palette.on_accent,
                &gettext(app.locale, "Play"),
            )
            .clicked()
            {
                if uri.starts_with("spotify:track:") {
                    app.actions.push(Action::PlayUris {
                        uris: vec![uri.clone()],
                        index: 0,
                    });
                } else {
                    app.actions.push(Action::PlayContext {
                        uri: uri.clone(),
                        offset_uri: None,
                        offset_index: None,
                    });
                }
            }
        }
    }
    if response.clicked() && !subtitle_clicked && page != Page::Search {
        app.actions.push(match page {
            Page::Artist(id) => crate::provider::spotify_artist_action(&id, title),
            page => Action::Open(page),
        });
    }
    egui::Popup::context_menu(&response)
        .frame(widgets::menu_frame(&palette))
        .show(|ui| menu(ui, app));
}

fn songs(app: &mut App, ui: &mut egui::Ui, results: &SearchResults, limit: usize) {
    let palette = app.palette;
    let Some(page) = &results.tracks else {
        return;
    };
    if page.items.is_empty() {
        return;
    }
    theme::section_title(ui, &palette, &gettext(app.locale, "Songs"));
    ui.add_space(4.0);
    let items: Vec<PlayableItem> = page
        .items
        .iter()
        .cloned()
        .map(PlayableItem::Track)
        .collect();
    for (index, item) in items.iter().take(limit).enumerate() {
        // A search result plays on its own, and Spotify continues with what
        // follows it (autoplay), as the Spotify apps do; the other results are
        // not a queue. A one-song list loads as that song's own context.
        let context = RowContext::Uris(Arc::from([item.uri().to_string()]));
        widgets::track_row(
            ui,
            app,
            TrackRow {
                index,
                number: None,
                item,
                context: &context,
                show_cover: true,
                show_album: limit == usize::MAX,
                added_at: None,
                added_by: None,
                show_added_by: false,
                show_plays: false,
                compact: limit != usize::MAX,
                thin: false,
                shift: 0.0,
                picked: false,
                picked_songs: &[],
                picked_positions: &[],
                position: None,
            },
        );
    }
}

fn artist_card(app: &mut App, ui: &mut egui::Ui, artist: &Artist) {
    let context_here = app.playing_context_uri().as_deref() == Some(artist.uri.as_str());
    let playing_here = context_here && app.believed_playing();
    let card_art = app.art_px(widgets::CARD_ART);
    let card = widgets::card(
        ui,
        app,
        pick_image(&artist.images, card_art),
        &artist.name,
        &gettext(app.locale, "Artist"),
        widgets::CardCover::portrait(playing_here),
    );
    if card.play {
        if context_here {
            app.actions.push(Action::TogglePlay);
        } else {
            app.actions.push(Action::PlayContext {
                uri: artist.uri.clone(),
                offset_uri: None,
                offset_index: None,
            });
        }
    }
    if card.clicked {
        app.actions.push(crate::provider::spotify_artist_action(
            &artist.id,
            artist.name.clone(),
        ));
    }
    egui::Popup::context_menu(&card.response)
        .id(ui.make_persistent_id(("search-artist-menu", &artist.uri)))
        .frame(widgets::menu_frame(&app.palette))
        .show(|ui| {
            widgets::context_menu_items(ui, app, &artist.uri, &artist.name, None);
        });
}

fn shelf_artists(app: &mut App, ui: &mut egui::Ui, results: &SearchResults) {
    let palette = app.palette;
    let Some(page) = &results.artists else { return };
    if page.items.is_empty() {
        return;
    }
    widgets::shelf(
        ui,
        &palette,
        "search-artists",
        &gettext(app.locale, "Artists"),
        |ui| {
            for artist in &page.items {
                artist_card(app, ui, artist);
            }
        },
    );
}

fn artists_grid(app: &mut App, ui: &mut egui::Ui, results: &SearchResults) {
    let Some(page) = &results.artists else { return };
    widgets::grid(ui, |ui| {
        for artist in &page.items {
            artist_card(app, ui, artist);
        }
    });
}

fn album_card(app: &mut App, ui: &mut egui::Ui, album: &crate::api::models::Album) {
    let subtitle = format!(
        "{} • {}",
        album.year().unwrap_or(""),
        crate::api::models::join_names(album.artists.iter().map(|a| a.name.as_str()))
    );
    let context_here = app.playing_context_uri().as_deref() == Some(album.uri.as_str());
    let playing_here = context_here && app.believed_playing();
    let card_art = app.art_px(widgets::CARD_ART);
    let card = widgets::card(
        ui,
        app,
        pick_image(&album.images, card_art),
        &album.name,
        subtitle.trim_start_matches(" • "),
        widgets::CardCover::square(playing_here),
    );
    if card.play {
        if context_here {
            app.actions.push(Action::TogglePlay);
        } else {
            app.actions.push(Action::PlayContext {
                uri: album.uri.clone(),
                offset_uri: None,
                offset_index: None,
            });
        }
    }
    if card.clicked {
        app.actions
            .push(Action::Open(Page::Album(album.id.clone())));
    }
    egui::Popup::context_menu(&card.response)
        .id(ui.make_persistent_id(("search-album-menu", &album.uri)))
        .frame(widgets::menu_frame(&app.palette))
        .show(|ui| {
            widgets::context_menu_items(ui, app, &album.uri, &album.name, None);
        });
}

fn shelf_albums(app: &mut App, ui: &mut egui::Ui, results: &SearchResults) {
    let palette = app.palette;
    let Some(page) = &results.albums else { return };
    if page.items.is_empty() {
        return;
    }
    widgets::shelf(
        ui,
        &palette,
        "search-albums",
        &gettext(app.locale, "Albums"),
        |ui| {
            for album in &page.items {
                album_card(app, ui, album);
            }
        },
    );
}

fn albums_grid(app: &mut App, ui: &mut egui::Ui, results: &SearchResults) {
    let Some(page) = &results.albums else { return };
    widgets::grid(ui, |ui| {
        for album in &page.items {
            album_card(app, ui, album);
        }
    });
}

fn playlist_card(app: &mut App, ui: &mut egui::Ui, playlist: &crate::api::models::Playlist) {
    let context_here = app.playing_context_uri().as_deref() == Some(playlist.uri.as_str());
    let playing_here = context_here && app.believed_playing();
    let card_art = app.art_px(widgets::CARD_ART);
    let card = widgets::card(
        ui,
        app,
        pick_image(&playlist.images, card_art),
        &playlist.name,
        // Translators: {owner} is the name of the playlist's owner.
        &gettext(app.locale, "By {owner}").replace("{owner}", playlist.owner_name()),
        widgets::CardCover::square(playing_here),
    );
    if card.play {
        if context_here {
            app.actions.push(Action::TogglePlay);
        } else {
            app.actions.push(Action::PlayContext {
                uri: playlist.uri.clone(),
                offset_uri: None,
                offset_index: None,
            });
        }
    }
    if card.clicked {
        app.actions
            .push(Action::Open(Page::Playlist(playlist.id.clone())));
    }
    egui::Popup::context_menu(&card.response)
        .id(ui.make_persistent_id(("search-playlist-menu", &playlist.uri)))
        .frame(widgets::menu_frame(&app.palette))
        .show(|ui| {
            let owned = app.user_id().is_some_and(|id| playlist.owned_by(id));
            widgets::context_menu_items(
                ui,
                app,
                &playlist.uri,
                &playlist.name,
                owned.then_some(playlist),
            );
        });
}

fn shelf_playlists(app: &mut App, ui: &mut egui::Ui, results: &SearchResults) {
    let palette = app.palette;
    let Some(page) = &results.playlists else {
        return;
    };
    if page.items.is_empty() {
        return;
    }
    widgets::shelf(
        ui,
        &palette,
        "search-playlists",
        &gettext(app.locale, "Playlists"),
        |ui| {
            for playlist in &page.items {
                playlist_card(app, ui, playlist);
            }
        },
    );
}

fn playlists_grid(app: &mut App, ui: &mut egui::Ui, results: &SearchResults) {
    let Some(page) = &results.playlists else {
        return;
    };
    widgets::grid(ui, |ui| {
        for playlist in &page.items {
            playlist_card(app, ui, playlist);
        }
    });
}

fn show_card(app: &mut App, ui: &mut egui::Ui, show: &crate::api::models::Show) {
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
        app.actions.push(Action::Open(Page::Show(show.id.clone())));
    }
    egui::Popup::context_menu(&card.response)
        .id(ui.make_persistent_id(("search-show-menu", &show.uri)))
        .frame(widgets::menu_frame(&app.palette))
        .show(|ui| {
            widgets::context_menu_items(ui, app, &show.uri, &show.name, None);
        });
}

fn shelf_shows(app: &mut App, ui: &mut egui::Ui, results: &SearchResults) {
    let palette = app.palette;
    let Some(page) = &results.shows else { return };
    if page.items.is_empty() {
        return;
    }
    widgets::shelf(
        ui,
        &palette,
        "search-shows",
        &gettext(app.locale, "Podcasts"),
        |ui| {
            for show in &page.items {
                show_card(app, ui, show);
            }
        },
    );
}

fn shows_grid(app: &mut App, ui: &mut egui::Ui, results: &SearchResults) {
    let Some(page) = &results.shows else { return };
    widgets::grid(ui, |ui| {
        for show in &page.items {
            show_card(app, ui, show);
        }
    });
}

fn episodes(app: &mut App, ui: &mut egui::Ui, results: &SearchResults, limit: usize) {
    let Some(page) = &results.episodes else {
        return;
    };
    for episode in page.items.iter().take(limit) {
        super::show::episode_row(app, ui, episode, None);
    }
}

#[allow(dead_code)]
fn align_right(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    ui.with_layout(Layout::right_to_left(Align::Center), add);
}

fn provider(app: &mut App, ui: &mut egui::Ui) {
    use crate::jellyfin::{
        browser::{Rows, View},
        catalog::{Kind, Query},
    };
    let view = app.jellyfin.view.clone();
    let View::Search {
        library,
        term,
        kind,
    } = &view
    else {
        return;
    };
    let filter = match kind {
        Some(Kind::Song) => SearchFilter::Songs,
        Some(Kind::Artist) => SearchFilter::Artists,
        Some(Kind::Album) => SearchFilter::Albums,
        Some(Kind::Playlist) => SearchFilter::Playlists,
        None => SearchFilter::All,
    };
    let labels: Vec<_> = [
        SearchFilter::All,
        SearchFilter::Songs,
        SearchFilter::Artists,
        SearchFilter::Albums,
        SearchFilter::Playlists,
    ]
    .into_iter()
    .map(|filter| (filter, filter.label(app.locale)))
    .collect();
    let options: Vec<_> = labels
        .iter()
        .map(|(filter, label)| (*filter, label.as_ref()))
        .collect();
    if let Some(filter) = widgets::chips(ui, &app.palette, &options, filter) {
        app.actions.push(Action::SetSearchFilter(filter));
    }
    ui.add_space(8.0);
    let all_collections = gettext(app.locale, "All collections");
    let libraries = app.jellyfin.libraries.clone();
    let selected_collection = library
        .as_deref()
        .and_then(|id| libraries.iter().find(|candidate| candidate.id == id))
        .map(|library| library.name.as_str())
        .unwrap_or(all_collections.as_ref())
        .to_owned();
    let mut next_library = None;
    ui.horizontal(|ui| {
        theme::text(
            ui,
            gettext(app.locale, "Search in"),
            theme::regular(13.0),
            app.palette.dim,
        );
        let response = egui::ComboBox::from_id_salt("jellyfin-search-collection")
            .selected_text(&selected_collection)
            .width(180.0_f32.min(ui.available_width()))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(library.is_none(), all_collections.as_ref())
                    .clicked()
                    && library.is_some()
                {
                    next_library = Some(None);
                }
                for candidate in &libraries {
                    let selected = library.as_deref() == Some(candidate.id.as_str());
                    if ui.selectable_label(selected, &candidate.name).clicked() && !selected {
                        next_library = Some(Some(candidate.id.clone()));
                    }
                }
            });
        response.response.widget_info(|| {
            let mut info = egui::WidgetInfo::labeled(
                egui::WidgetType::ComboBox,
                ui.is_enabled(),
                gettext(app.locale, "Search collection"),
            );
            info.current_text_value = Some(selected_collection.clone());
            info
        });
    });
    if let Some(library) = next_library {
        app.actions.push(Action::Jellyfin(
            crate::jellyfin::browser::BrowserAction::Browse {
                view: View::Search {
                    library,
                    term: term.clone(),
                    kind: *kind,
                },
                start: 0,
            },
        ));
    }
    ui.add_space(12.0);
    if term.is_empty() {
        widgets::empty_state(
            ui,
            &app.palette,
            Icon::Search,
            "Search Jellyfin",
            "Find songs, artists, albums and playlists.",
        );
        return;
    }
    if app.jellyfin.busy {
        widgets::loading_row(ui, &app.palette, app.locale);
    }
    if let Some(error) = app.jellyfin.error.clone() {
        widgets::error_row(ui, app, &error, None);
    }
    let Some(Rows::Search(sections)) = app.jellyfin.rows.clone() else {
        return;
    };
    if sections.iter().all(|(_, page)| page.items.is_empty()) && !app.jellyfin.busy {
        widgets::empty_state(
            ui,
            &app.palette,
            Icon::Search,
            &format!("No results for “{term}”"),
            "Check the spelling, or try fewer words.",
        );
    }
    for (category, page) in sections {
        if page.items.is_empty() {
            continue;
        }
        let label = match category {
            Kind::Song => "Songs",
            Kind::Artist => "Artists",
            Kind::Album => "Albums",
            Kind::Playlist => "Playlists",
        };
        theme::section_title(ui, &app.palette, label);
        ui.add_space(8.0);
        ui.push_id(("provider-search-section", category), |ui| {
            if category == Kind::Song {
                let tracks = app.provider_detail_tracks(
                    &page.items,
                    page.start_index,
                    Some(Query::Search {
                        library: library.clone(),
                        term: term.clone(),
                        kind: category,
                    }),
                    "search",
                );
                super::collection::provider_tracks(app, ui, &tracks);
            } else {
                let cards: Vec<_> = page
                    .items
                    .iter()
                    .map(|item| app.jellyfin_library_item(item, category))
                    .collect();
                super::library::item_grid(app, ui, &cards);
            }
            let section_view = View::Search {
                library: library.clone(),
                term: term.clone(),
                kind: Some(category),
            };
            super::collection::provider_paging(
                app,
                ui,
                section_view,
                page.start_index,
                page.items.len(),
                page.total_record_count,
            );
        });
        ui.add_space(24.0);
    }
}
