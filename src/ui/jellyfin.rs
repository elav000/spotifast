//! Provider selection and a separate, read-only Jellyfin browser.
use crate::{
    app::App,
    jellyfin::browser::{BrowserAction as A, Provider, Rows, View},
    model::Action,
    theme,
};
use egui::{Align, CornerRadius, Frame, Layout, Margin, RichText, Sense, Stroke, vec2};

fn emit(app: &mut App, action: A) {
    app.actions.push(Action::Jellyfin(action));
}

pub fn provider_bar(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    egui::Panel::top("provider-selector")
        .exact_size(46.0 + theme::titlebar_inset(ui.ctx()))
        .resizable(false)
        .frame(
            Frame::new()
                .fill(palette.panel)
                .inner_margin(Margin::symmetric(20, 6)),
        )
        .show(ui, |ui| {
            ui.add_space(theme::titlebar_inset(ui.ctx()));
            fastframe_titlebar::drag(ui, ui.max_rect());
            ui.horizontal(|ui| {
                ui.label(RichText::new("Spotifast").strong().color(palette.text));
                ui.add_space(16.0);
                for (provider, label) in [
                    (Provider::Jellyfin, "Jellyfin"),
                    (Provider::Spotify, "Spotify"),
                ] {
                    if ui
                        .selectable_label(app.jellyfin.provider == provider, label)
                        .clicked()
                    {
                        emit(app, A::Select(provider));
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new("Your music, your providers")
                            .small()
                            .color(palette.secondary),
                    );
                });
            });
        });
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    if app.jellyfin.code.is_some() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs(1));
    }
    egui::CentralPanel::default().frame(Frame::new().fill(palette.window).inner_margin(24))
        .show(ui, |ui| {
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
            if let Some(error) = &browser.error {
                ui.label(RichText::new(error).color(palette.danger));
                if browser.rows.is_some() { ui.label("Showing the last saved results."); }
                ui.add_space(10.0);
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
            } else {
                ui.horizontal_wrapped(|ui| {
                    if ui.selectable_label(browser.view == View::Libraries, "Libraries").clicked() {
                        actions.push(A::Browse { view: View::Libraries, start: 0 });
                    }
                    match &browser.view {
                        View::Libraries => {},
                        View::Albums { name, .. } => { ui.label("/"); ui.label(name); },
                        View::Tracks { library_id, library_name, name, .. } => {
                            ui.label("/");
                            if ui.link(library_name).clicked() {
                                actions.push(A::Browse { view: View::Albums { library_id: library_id.clone(), name: library_name.clone() }, start: 0 });
                            }
                            ui.label("/"); ui.label(name);
                        }
                    }
                });
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    let mut filter = browser.filter.clone();
                    if ui.add(egui::TextEdit::singleline(&mut filter).hint_text("Filter this page…").desired_width(240.0)).changed() {
                        actions.push(A::Filter(filter));
                    }
                    if browser.busy { ui.spinner(); ui.label("Refreshing…"); }
                    else if browser.cached { ui.label("Saved results"); }
                });
                ui.add_space(16.0);
                let filter = browser.filter.to_lowercase();
                egui::ScrollArea::vertical().id_salt(("jellyfin-rows", &browser.view, browser.start)).show(ui, |ui| {
                    match &browser.rows {
                        Some(Rows::Libraries(libraries)) => {
                            for library in libraries.iter().filter(|item| item.name.to_lowercase().contains(&filter)) {
                                let response = card(ui, &palette, &library.name, "Music library", ui.available_width().min(520.0));
                                if response.clicked() { actions.push(A::Browse { view: View::Albums { library_id: library.id.clone(), name: library.name.clone() }, start: 0 }); }
                                ui.add_space(10.0);
                            }
                            if libraries.is_empty() { ui.label("No music libraries are available to this account."); }
                        }
                        Some(Rows::Items { items, start, total }) => {
                            ui.label(RichText::new(format!("{}–{} of {}", if items.is_empty() { 0 } else { start + 1 }, start + items.len() as u32, total)).small().color(palette.secondary));
                            ui.add_space(10.0);
                            let tracks = matches!(browser.view, View::Tracks { .. });
                            if tracks { ui.label(RichText::new("Browsing preview · Playback is coming next").small().color(palette.secondary)); ui.add_space(12.0); }
                            for item in items.iter().filter(|item| {
                                item.name.as_deref().unwrap_or("").to_lowercase().contains(&filter)
                                    || item.artists.iter().any(|artist| artist.to_lowercase().contains(&filter))
                            }) {
                                let name = item.name.as_deref().unwrap_or("Untitled");
                                if tracks {
                                    Frame::new().fill(palette.panel).corner_radius(6).inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
                                        ui.set_min_width((ui.available_width() - 28.0).max(0.0));
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new(item.index_number.map(|n| n.to_string()).unwrap_or_default()).color(palette.secondary));
                                            ui.vertical(|ui| { ui.label(RichText::new(name).strong()); ui.label(RichText::new(item.artists.join(", ")).small().color(palette.secondary)); });
                                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                                if let Some(ms) = item.duration_ms() { ui.label(format!("{}:{:02}", ms / 60000, ms / 1000 % 60)); }
                                            });
                                        });
                                    });
                                } else {
                                    let response = card(ui, &palette, name, &item.artists.join(", "), ui.available_width().min(720.0));
                                    if response.clicked() && let View::Albums { library_id, name: library_name } = &browser.view {
                                        actions.push(A::Browse { view: View::Tracks { library_id: library_id.clone(), library_name: library_name.clone(), album_id: item.id.clone(), name: name.into() }, start: 0 });
                                    }
                                }
                                ui.add_space(8.0);
                            }
                            if items.is_empty() { ui.label("No items in this view."); }
                            ui.add_space(12.0);
                            ui.horizontal(|ui| {
                                if ui.add_enabled(*start > 0 && !browser.busy, egui::Button::new("Previous page")).clicked() {
                                    actions.push(A::Browse { view: browser.view.clone(), start: start.saturating_sub(100) });
                                }
                                if ui.add_enabled(start.saturating_add(items.len() as u32) < *total && !items.is_empty() && !browser.busy, egui::Button::new("Next page")).clicked() {
                                    actions.push(A::Browse { view: browser.view.clone(), start: start + items.len() as u32 });
                                }
                            });
                        }
                        None if browser.busy => { ui.label("Loading your music…"); },
                        None => { ui.label("Choose Refresh to try again."); },
                    }
                });
            }
            for action in actions { emit(app, action); }
        });
}

fn card(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    title: &str,
    subtitle: &str,
    width: f32,
) -> egui::Response {
    let response = Frame::new()
        .fill(palette.panel)
        .stroke(Stroke::new(1.0, palette.outline))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(16)
        .show(ui, |ui| {
            ui.set_min_size(vec2((width - 32.0).max(0.0), 48.0));
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(46.0, 46.0), Sense::hover());
                ui.painter().rect_filled(rect, 8, palette.surface_active);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    title.chars().next().unwrap_or('♪'),
                    theme::bold(22.0),
                    palette.accent,
                );
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(title).strong().color(palette.text));
                    if !subtitle.is_empty() {
                        ui.label(RichText::new(subtitle).small().color(palette.secondary));
                    }
                });
            });
        })
        .response
        .interact(Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), title)
    });
    theme::focus_ring(ui, &response);
    response
}
