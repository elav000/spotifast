//! Provider selection, separate Jellyfin browsing and playback actions.
use crate::{
    app::App,
    jellyfin::browser::{BrowserAction as A, Provider, Rows, View},
    model::Action,
    theme,
};
use egui::{Align, Frame, Layout, RichText, vec2};

fn emit(app: &mut App, action: A) {
    app.actions.push(Action::Jellyfin(action));
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    if app.jellyfin.source.is_some() {
        connected(app, ui);
        return;
    }
    let palette = app.palette;
    if app.jellyfin.code.is_some() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs(1));
    }
    egui::CentralPanel::default().frame(Frame::new().fill(palette.window).inner_margin(24))
        .show(ui, |ui| {
            ui.add_space(super::provider_titlebar_inset(ui.ctx()));
            super::sidebar::reveal_button(app, ui);
            let browser = &app.jellyfin;
            let mut actions = Vec::new();
            ui.horizontal(|ui| {
                ui.label(RichText::new("Jellyfin").font(theme::bold(30.0)).color(palette.text));
                if browser.source.is_some() {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Sign out").clicked() { actions.push(A::SignOut); }
                        if ui.add_enabled(!browser.busy, egui::Button::new("Refresh")).clicked() {
                            actions.push(A::Browse { view: browser.view.clone(), start: browser.start });
                        }
                    });
                }
            });
            ui.add_space(6.0);
            if let Some(source) = &browser.source {
                ui.label(RichText::new(&source.server_url).color(palette.secondary));
            } else {
                ui.label(RichText::new("Connect your music server. No Spotify account needed.").color(palette.secondary));
            }
            ui.add_space(18.0);
            if app.jellyfin_player.snapshot().item.is_some() && !app.jellyfin_player.controls_output()
                && ui.button("Resume Jellyfin").clicked() { actions.push(A::Resume); }
            if let Some(error) = &browser.error {
                ui.label(RichText::new(error).color(palette.danger));
                if browser.rows.is_some() { ui.label("Showing the last saved results."); }
                ui.add_space(10.0);
            }
            if let Some(error) = app.jellyfin_player.snapshot().error {
                ui.label(RichText::new(error).color(palette.danger));
            }
            if browser.source.is_none() {
                Frame::new().fill(palette.panel).corner_radius(12).inner_margin(24).show(ui, |ui| {
                    ui.set_max_width(520.0);
                    ui.label(RichText::new("Your Jellyfin server").strong().size(19.0));
                    ui.add_space(12.0);
                    let mut server = browser.server_input.clone();
                    let input = ui.add_enabled(!browser.busy, egui::TextEdit::singleline(&mut server)
                        .hint_text("http://your-server:8096").desired_width(440.0));
                    if input.changed() { actions.push(A::ServerInput(server)); }
                    if let Some(code) = &browser.code {
                        ui.add_space(16.0);
                        ui.label("In your signed-in Jellyfin client, open Quick Connect and enter:");
                        ui.add_space(12.0);
                        ui.label(RichText::new(code).font(theme::bold(36.0)).color(palette.accent));
                        ui.label("Waiting for approval…");
                        if ui.button("Cancel").clicked() { actions.push(A::Cancel); }
                    } else if browser.busy {
                        ui.horizontal(|ui| { ui.spinner(); ui.label("Connecting…"); });
                        if ui.button("Cancel").clicked() { actions.push(A::Cancel); }
                    } else {
                        ui.add_space(12.0);
                        if ui.add_enabled(!browser.server_input.trim().is_empty(), egui::Button::new("Connect with Quick Connect")).clicked()
                            || (input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                            actions.push(A::Connect);
                        }
                        ui.add_space(10.0);
                        ui.label(RichText::new("Approve in Jellyfin. Your access token is saved in the system credential store.").small().color(palette.secondary));
                    }
                });
            }
            for action in actions { emit(app, action); }
        });
}

/// Presentation reuses the existing collection hero, cards and track renderer.
/// Moving the page data out temporarily follows the existing collection pages'
/// borrowing pattern; actions are still applied only after drawing.
fn connected(app: &mut App, ui: &mut egui::Ui) {
    use super::{collection, widgets};
    use crate::provider::{ItemReference, TrackPresentation};
    let rows = app.jellyfin.rows.take();
    let source = app
        .jellyfin
        .source
        .clone()
        .expect("connected page has a source");
    let view = app.jellyfin.view.clone();
    let busy = app.jellyfin.busy;
    let palette = app.palette;
    let state = app.jellyfin_player.snapshot();
    let controls_output = app.jellyfin_player.controls_output();
    let mut actions = Vec::new();
    egui::CentralPanel::default()
        .frame(
            Frame::new()
                .fill(palette.window)
                .inner_margin(widgets::PAGE_PADDING),
        )
        .show(ui, |ui| {
            ui.add_space(super::provider_titlebar_inset(ui.ctx()));
            ui.horizontal(|ui| {
                super::sidebar::reveal_button(app, ui);
                if ui.link("Libraries").clicked() {
                    actions.push(A::Browse {
                        view: View::Libraries,
                        start: 0,
                    });
                }
                match &view {
                    View::Libraries => {}
                    View::Albums { name, .. } => {
                        ui.label("/");
                        ui.label(name);
                    }
                    View::Tracks {
                        library_id,
                        library_name,
                        name,
                        ..
                    } => {
                        ui.label("/");
                        if ui.link(library_name).clicked() {
                            actions.push(A::Browse {
                                view: View::Albums {
                                    library_id: library_id.clone(),
                                    name: library_name.clone(),
                                },
                                start: 0,
                            });
                        }
                        ui.label("/");
                        ui.label(name);
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Sign out").clicked() {
                        actions.push(A::SignOut);
                    }
                    if ui
                        .add_enabled(!busy, egui::Button::new("Refresh"))
                        .clicked()
                    {
                        actions.push(A::Browse {
                            view: view.clone(),
                            start: app.jellyfin.start,
                        });
                    }
                });
            });
            if state.item.is_some() && !controls_output && ui.button("Resume Jellyfin").clicked() {
                actions.push(A::Resume);
            }
            if let Some(error) = &app.jellyfin.error {
                ui.label(RichText::new(error).color(palette.danger));
                if rows.is_some() {
                    ui.label("Showing the last saved results.");
                }
            }
            if let Some(error) = &state.error {
                ui.label(RichText::new(error).color(palette.danger));
            }
            ui.add_space(12.0);
            egui::ScrollArea::vertical()
                .id_salt(("jellyfin-page", &view, app.jellyfin.start))
                .show(ui, |ui| {
                    if let View::Tracks { name, .. } = &view {
                        let items = match &rows {
                            Some(Rows::Items { items, .. }) => items.as_slice(),
                            _ => &[],
                        };
                        let artwork = app
                            .jellyfin
                            .album
                            .as_ref()
                            .or_else(|| items.first())
                            .and_then(|item| {
                                crate::jellyfin::artwork::Reference::for_item(&source, item)
                            })
                            .map(|reference| reference.uri());
                        let artists = app
                            .jellyfin
                            .album
                            .as_ref()
                            .map(|album| album.artists.as_slice())
                            .or_else(|| items.first().map(|item| item.artists.as_slice()))
                            .unwrap_or_default()
                            .join(", ");
                        collection::hero(
                            app,
                            ui,
                            collection::Hero {
                                images: collection::HeroImages {
                                    image: artwork.as_deref(),
                                    ..Default::default()
                                },
                                liked: false,
                                kind: "Album".into(),
                                title: name,
                                description: None,
                                byline: vec![(artists, None), ("Jellyfin".into(), None)],
                                avatar: None,
                                round: false,
                            },
                        );
                    } else {
                        ui.heading(match &view {
                            View::Libraries => "Your Library",
                            _ => "Albums",
                        });
                        ui.add_space(12.0);
                    }
                    ui.horizontal(|ui| {
                        if matches!(&view, View::Tracks { .. }) {
                            let first = match &rows {
                                Some(Rows::Items {
                                    items, start: 0, ..
                                }) => items.first(),
                                _ => None,
                            };
                            ui.add_enabled_ui(!busy && first.is_some(), |ui| {
                                if theme::circle_button(
                                    ui,
                                    theme::Icon::PlayFilled,
                                    56.0,
                                    palette.accent,
                                    palette.accent_hover,
                                    palette.on_accent,
                                    "Play album",
                                )
                                .clicked()
                                    && let Some(first) = first
                                {
                                    actions.push(A::Play(first.id.clone()));
                                }
                            });
                            ui.add_space(12.0);
                        }
                        let mut filter = app.jellyfin.filter.clone();
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut filter)
                                    .id_salt("jellyfin-page-filter")
                                    .hint_text("Filter this page…")
                                    .desired_width(220.0),
                            )
                            .changed()
                        {
                            actions.push(A::Filter(filter));
                        }
                        if busy {
                            ui.spinner();
                            ui.label("Refreshing…");
                        } else if app.jellyfin.cached {
                            ui.label("Saved results");
                        }
                    });
                    ui.add_space(16.0);
                    let filter = app.jellyfin.filter.to_lowercase();
                    match &rows {
                        Some(Rows::Libraries(libraries)) => {
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing =
                                    vec2(widgets::CARD_GAP, widgets::CARD_GAP);
                                for library in libraries
                                    .iter()
                                    .filter(|library| library.name.to_lowercase().contains(&filter))
                                {
                                    let response = widgets::card(
                                        ui,
                                        app,
                                        None,
                                        &library.name,
                                        "Music library",
                                        widgets::CardCover::default(),
                                    );
                                    if response.clicked {
                                        actions.push(A::Browse {
                                            view: View::Albums {
                                                library_id: library.id.clone(),
                                                name: library.name.clone(),
                                            },
                                            start: 0,
                                        });
                                    }
                                }
                            });
                            if libraries.is_empty() {
                                ui.label("No music libraries are available to this account.");
                            }
                        }
                        Some(Rows::Items {
                            items,
                            start,
                            total,
                        }) => {
                            ui.label(
                                RichText::new(format!(
                                    "{}–{} of {}",
                                    if items.is_empty() { 0 } else { start + 1 },
                                    start + items.len() as u32,
                                    total
                                ))
                                .small()
                                .color(palette.secondary),
                            );
                            ui.add_space(10.0);
                            if matches!(&view, View::Tracks { .. }) {
                                for (index, item) in items
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, item)| matches_filter(item, &filter))
                                {
                                    let name = item.name.as_deref().unwrap_or("Untitled");
                                    let current = controls_output
                                        && state.source_id.as_deref() == Some(source.id.as_str())
                                        && state
                                            .item
                                            .as_ref()
                                            .is_some_and(|current| current.id == item.id);
                                    widgets::provider_track_row(
                                        ui,
                                        app,
                                        &TrackPresentation {
                                            reference: ItemReference::Jellyfin {
                                                source: source.id.clone(),
                                                account: source.user_id.clone(),
                                                item: item.id.clone(),
                                            },
                                            title: name.into(),
                                            artists: item.artists.clone(),
                                            album: item.album.clone().unwrap_or_default(),
                                            artwork: crate::jellyfin::artwork::Reference::for_item(
                                                &source, item,
                                            )
                                            .map(|reference| reference.uri()),
                                            duration_ms: item.duration_ms().unwrap_or(0),
                                            number: item.index_number,
                                        },
                                        *start as usize + index,
                                        &widgets::ProviderRowActions {
                                            provider: Provider::Jellyfin,
                                            play: Action::Jellyfin(A::Play(item.id.clone())),
                                            enqueue: Some(Action::Jellyfin(A::Queue(
                                                item.id.clone(),
                                            ))),
                                            queue_label: format!("Add {name} to Jellyfin queue"),
                                            can_enqueue: state.item.is_some(),
                                            current,
                                            playing: state.playing,
                                            loading: state.loading,
                                        },
                                    );
                                }
                            } else {
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing =
                                        vec2(widgets::CARD_GAP, widgets::CARD_GAP);
                                    for item in
                                        items.iter().filter(|item| matches_filter(item, &filter))
                                    {
                                        let name = item.name.as_deref().unwrap_or("Untitled");
                                        let artwork =
                                            crate::jellyfin::artwork::Reference::for_item(
                                                &source, item,
                                            )
                                            .map(|reference| reference.uri());
                                        let response = widgets::card(
                                            ui,
                                            app,
                                            artwork.as_deref(),
                                            name,
                                            &item.artists.join(", "),
                                            widgets::CardCover::default(),
                                        );
                                        if response.clicked
                                            && let View::Albums {
                                                library_id,
                                                name: library_name,
                                            } = &view
                                        {
                                            actions.push(A::Browse {
                                                view: View::Tracks {
                                                    library_id: library_id.clone(),
                                                    library_name: library_name.clone(),
                                                    album_id: item.id.clone(),
                                                    name: name.into(),
                                                },
                                                start: 0,
                                            });
                                        }
                                    }
                                });
                            }
                            if items.is_empty() {
                                ui.label("No items in this view.");
                            }
                            ui.add_space(12.0);
                            ui.horizontal(|ui| {
                                if ui
                                    .add_enabled(
                                        *start > 0 && !busy,
                                        egui::Button::new("Previous page"),
                                    )
                                    .clicked()
                                {
                                    actions.push(A::Browse {
                                        view: view.clone(),
                                        start: start.saturating_sub(100),
                                    });
                                }
                                if ui
                                    .add_enabled(
                                        start.saturating_add(items.len() as u32) < *total
                                            && !items.is_empty()
                                            && !busy,
                                        egui::Button::new("Next page"),
                                    )
                                    .clicked()
                                {
                                    actions.push(A::Browse {
                                        view: view.clone(),
                                        start: start + items.len() as u32,
                                    });
                                }
                            });
                        }
                        None if busy => {
                            ui.label("Loading your music…");
                        }
                        None => {
                            ui.label("Choose Refresh to try again.");
                        }
                    }
                });
        });
    app.jellyfin.rows = rows;
    for action in actions {
        emit(app, action);
    }
}

fn matches_filter(item: &crate::jellyfin::Item, filter: &str) -> bool {
    item.name
        .as_deref()
        .unwrap_or("")
        .to_lowercase()
        .contains(filter)
        || item
            .artists
            .iter()
            .any(|artist| artist.to_lowercase().contains(filter))
}

pub fn queue_panel(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let state = app.jellyfin_player.snapshot();
    let mut actions = Vec::new();
    egui::Panel::right("jellyfin-queue")
        .default_size(300.0)
        .size_range(240.0..=500.0)
        .resizable(true)
        .frame(Frame::new().fill(palette.panel).inner_margin(16))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Jellyfin queue");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Close").clicked() {
                        actions.push(Action::ToggleQueuePanel);
                    }
                });
            });
            ui.add_space(12.0);
            if let Some(item) = &state.item {
                ui.label(RichText::new("Now playing").color(palette.secondary));
                ui.label(RichText::new(item.name.as_deref().unwrap_or("Untitled")).strong());
                ui.label(item.artists.join(", "));
                ui.add_space(18.0);
                if !state.queue_complete && state.loading {
                    ui.spinner();
                    ui.label("Loading the complete album…");
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if !state.queued.is_empty() {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Playing next").color(palette.secondary));
                            if ui.button("Clear queue").clicked() {
                                actions.push(Action::Jellyfin(A::ClearQueue));
                            }
                        });
                        for (index, item) in state.queued.iter().enumerate() {
                            if ui
                                .selectable_label(false, item.name.as_deref().unwrap_or("Untitled"))
                                .on_hover_text(item.artists.join(", "))
                                .clicked()
                            {
                                actions.push(Action::Jellyfin(A::PlayManual(index)));
                            }
                            ui.add_space(6.0);
                        }
                        ui.add_space(12.0);
                    }
                    ui.label(RichText::new("Next up · Album order").color(palette.secondary));
                    for (index, item) in state.tracks.iter().enumerate().skip(state.index + 1) {
                        let title = item.name.as_deref().unwrap_or("Untitled");
                        if ui
                            .selectable_label(false, title)
                            .on_hover_text(item.artists.join(", "))
                            .clicked()
                        {
                            actions.push(Action::Jellyfin(A::PlayQueue(index)));
                        }
                        ui.add_space(6.0);
                    }
                    if state.queue_complete && state.index + 1 >= state.tracks.len() {
                        ui.label("End of album");
                    }
                });
            } else {
                ui.label("Play an album or track to start your Jellyfin queue.");
            }
        });
    app.actions.extend(actions);
}
