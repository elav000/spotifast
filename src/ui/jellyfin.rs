//! Provider selection, separate Jellyfin browsing and playback actions.
use crate::{app::App, jellyfin::browser::BrowserAction as A, model::Action, theme};
use egui::{Align, Frame, Layout, RichText};

fn emit(app: &mut App, action: A) {
    app.actions.push(Action::Jellyfin(action));
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    if app.jellyfin.source.is_some() {
        super::central(app, ui);
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
            let browser = &app.jellyfin;
            let mut actions = Vec::new();
            ui.horizontal(|ui| {
                ui.label(RichText::new("Jellyfin").font(theme::bold(30.0)).color(palette.text));
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Use Spotify").clicked() {
                        actions.push(A::Select(crate::provider::Provider::Spotify));
                    }
                });
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
                    if !browser.servers.is_empty() {
                        ui.label(RichText::new("Choose a Jellyfin server").strong().size(19.0));
                        ui.add_space(8.0);
                        for server in &browser.servers {
                            ui.horizontal(|ui| {
                                let name = if server.nickname.trim().is_empty() {
                                    &server.server_url
                                } else {
                                    &server.nickname
                                };
                                ui.label(name);
                                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                                    if ui.button("Use").clicked() {
                                        actions.push(A::ActivateServer(server.id.clone()));
                                    }
                                });
                            });
                        }
                        ui.add_space(16.0);
                        ui.separator();
                        ui.add_space(8.0);
                    }
                    ui.label(RichText::new(if browser.servers.is_empty() { "Your Jellyfin server" } else { "Add another server" }).strong().size(19.0));
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
