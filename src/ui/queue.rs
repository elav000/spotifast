//! The playback queue, as a page or as a side panel.

use std::sync::Arc;

use egui::{Align, Frame, Layout, Margin, Rect, UiBuilder};

use crate::app::App;
use crate::i18n::{Locale, gettext};
use crate::model::{Action, DragTrack, QueueTab, RowContext};
use crate::theme::{self, Icon};

use super::widgets::{self, TrackRow};
use crate::provider::queue::{QueueEntry, QueuePresentation, QueueStatus};

pub fn page(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    ui.add_space(8.0);
    // The queue refreshes on track changes, additions, and while visible.
    let view = app.playback_queue();
    let offer_save = view.save.is_some();
    ui.horizontal(|ui| {
        theme::text(
            ui,
            gettext(app.locale, "Queue"),
            theme::bold(28.0),
            palette.text,
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if save_button(ui, &palette, offer_save, app.locale)
                && let Some(action) = &view.save
            {
                app.actions.push(action.clone());
            }
        });
    });
    ui.add_space(12.0);
    contents(app, ui, false, &view);
}

pub fn side_panel(app: &mut App, ui: &mut egui::Ui) {
    let view = app.playback_queue();
    let palette = app.palette;
    let fit = super::yielding_panel(
        ui.ctx(),
        "queue-panel",
        theme::SIDE_PANEL_MIN_WIDTH..=560.0,
        app.settings.queue_width,
        ui.available_width() - super::topbar::least_width(ui.ctx()),
    );
    let panel = egui::Panel::right("queue-panel")
        .resizable(true)
        .default_size(app.settings.queue_width)
        .size_range(fit.range.clone())
        .show_separator_line(false)
        .frame(
            Frame::new()
                .fill(palette.panel)
                .inner_margin(Margin::symmetric(12, 12)),
        );
    let response = panel.show(ui, |ui| {
        let window_controls = super::window_controls_reservation(
            ui.ctx(),
            app.show_queue_panel,
            app.show_lyrics_panel,
            ui.available_width(),
        );
        ui.add_space(window_controls.queue_top);
        // Measure buttons first and give the remaining width to the chips.
        // Without `shrink_left`, wrapped chips can overlap the close button.
        let tab = app.queue_tab;
        let offer_save = tab == QueueTab::Queue && view.save.is_some();
        let mut picked = None;
        let mut close = false;
        let mut save = false;
        egui::Sides::new().shrink_left().show(
            ui,
            |ui| {
                ui.add_space(4.0);
                picked = widgets::chips(
                    ui,
                    &palette,
                    &[
                        (QueueTab::Queue, &gettext(app.locale, "Queue")),
                        (QueueTab::Recents, &gettext(app.locale, "Recent")),
                    ],
                    tab,
                );
            },
            |ui| {
                close = theme::icon_button(
                    ui,
                    Icon::X,
                    18.0,
                    palette.secondary,
                    palette.text,
                    &gettext(app.locale, "Close"),
                )
                .clicked();
                save = save_button(ui, &palette, offer_save, app.locale);
            },
        );
        if let Some(tab) = picked {
            app.actions.push(Action::SetQueueTab(tab));
        }
        if close {
            app.actions.push(Action::ToggleQueuePanel);
        }
        if save && let Some(action) = &view.save {
            app.actions.push(action.clone());
        }
        ui.add_space(8.0);
        // Lazy load recents when tab becomes visible.
        if app.queue_tab == QueueTab::Recents
            && !view.recent.loading
            && !view.recent.complete
            && view.recent.rows.is_empty()
            && view.recent.error.is_none()
            && let Some(action) = &view.recent.load
        {
            app.actions.push(action.clone());
        }
        crate::autoscroll::show(
            ui,
            egui::ScrollArea::vertical()
                .id_salt("queue-panel-scroll")
                .auto_shrink([false, false])
                // Keep rows and headings clear of the scroll bar.
                .content_margin(egui::Margin {
                    left: 0,
                    right: 12,
                    top: 0,
                    bottom: 0,
                }),
            egui::Vec2b::new(false, true),
            |ui| match app.queue_tab {
                QueueTab::Queue => contents(app, ui, true, &view),
                QueueTab::Recents => recents_contents(app, ui, &view),
            },
        );
    });
    let width = response.response.rect.width();
    // Adopt the drawn width only from the person's own drag. The panel
    // also draws clamped copies of widths chosen elsewhere (the
    // fullscreen column shares this setting below the panel minimum),
    // and those must never overwrite the remembered width.
    if (width - app.settings.queue_width).abs() > 1.0
        && super::panel_dragged(ui.ctx(), "queue-panel")
    {
        app.settings.queue_width = width;
        app.actions.push(Action::SettingsChanged);
    }
}

/// Fullscreen queue column width bounds, shared with the lyrics layout
/// so both agree without duplicating numbers.
pub(crate) const FULLSCREEN_COLUMN_MIN: f32 = 240.0;
pub(crate) const FULLSCREEN_COLUMN_MAX: f32 = 380.0;

/// The column width a drag gesture works from: the drawn width, not the
/// saved one. The saved side-panel width can sit outside this column's
/// range, and starting there would swallow the first movement back
/// inside it.
pub(crate) fn fullscreen_column_drag_width(saved: f32, delta_x: f32) -> f32 {
    let drawn = saved.clamp(FULLSCREEN_COLUMN_MIN, FULLSCREEN_COLUMN_MAX);
    (drawn - delta_x).clamp(FULLSCREEN_COLUMN_MIN, FULLSCREEN_COLUMN_MAX)
}

/// The queue as the right column of lyrics fullscreen: the same rows as
/// the side panel, without the resizable panel chrome. The caller provides
/// the rect; everything here draws inside it.
pub fn fullscreen_column(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    // Drag strip on the column's left edge: same persisted width the side
    // panel uses, so both follow each resize. Explicit id, stable across
    // frames for the whole drag gesture.
    let strip = egui::Rect::from_min_size(
        egui::pos2(ui.max_rect().left() - 4.0, ui.max_rect().top()),
        egui::vec2(8.0, ui.max_rect().height()),
    );
    let grip = ui.interact(
        strip,
        egui::Id::new("queue-column-resize"),
        egui::Sense::drag(),
    );
    if grip.hovered() || grip.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn);
    }
    // Gutter between the divider and the content, mirroring the side
    // panel's frame margin, so headings never touch the edge.
    let mut inner = ui.new_child(UiBuilder::new().max_rect(Rect::from_min_max(
        egui::pos2(ui.max_rect().left() + 12.0, ui.max_rect().top()),
        ui.max_rect().max,
    )));
    let ui = &mut inner;
    // The handle stays invisible until hovered, then fades a slim edge in:
    // the theme text color, so it reads white on dark and black on bright.
    let show = ui.ctx().animate_value_with_time(
        egui::Id::new("queue-column-edge"),
        if grip.hovered() || grip.dragged() {
            1.0
        } else {
            0.0
        },
        0.15,
    );
    if show > 0.0 {
        ui.painter().vline(
            strip.center().x,
            strip.y_range(),
            egui::Stroke::new(1.0, palette.text.linear_multiply(show)),
        );
    }
    if grip.dragged() {
        let width = fullscreen_column_drag_width(app.settings.queue_width, grip.drag_delta().x);
        if (width - app.settings.queue_width).abs() > 0.5 {
            app.settings.queue_width = width;
            app.actions.push(Action::SettingsChanged);
        }
    }
    let tab = app.queue_tab;
    let offer_save = tab == QueueTab::Queue && !app.queue_playlist_uris().is_empty();
    let mut picked = None;
    let mut close = false;
    let mut save = false;
    egui::Sides::new().shrink_left().show(
        ui,
        |ui| {
            ui.add_space(4.0);
            picked = widgets::chips(
                ui,
                &palette,
                &[
                    (QueueTab::Queue, &gettext(app.locale, "Queue")),
                    (QueueTab::Recents, &gettext(app.locale, "Recent")),
                ],
                tab,
            );
        },
        |ui| {
            close = theme::icon_button(
                ui,
                Icon::X,
                18.0,
                palette.secondary,
                palette.text,
                &gettext(app.locale, "Close"),
            )
            .clicked();
            save = save_button(ui, &palette, offer_save, app.locale);
        },
    );
    if let Some(tab) = picked {
        app.actions.push(Action::SetQueueTab(tab));
    }
    if close {
        app.actions.push(Action::ToggleQueuePanel);
    }
    if save {
        app.actions.push(Action::SaveQueueAsPlaylist);
    }
    ui.add_space(8.0);
    // Lazy load recents when tab becomes visible.
    if app.queue_tab == QueueTab::Recents
        && !app.recents.loading
        && !app.recents.complete
        && app.recents.items.is_empty()
        && app.recents.error.is_none()
    {
        app.actions.push(Action::LoadMoreRecents);
    }
    crate::autoscroll::show(
        ui,
        egui::ScrollArea::vertical()
            .id_salt("queue-fullscreen-scroll")
            .auto_shrink([false, false])
            // Keep rows and headings clear of the scroll bar.
            .content_margin(egui::Margin {
                left: 0,
                right: 12,
                top: 0,
                bottom: 0,
            }),
        egui::Vec2b::new(false, true),
        |ui| match app.queue_tab {
            QueueTab::Queue => contents(app, ui, true),
            QueueTab::Recents => recents_contents(app, ui),
        },
    );
}

/// Saves the current and upcoming queue as a playlist.
fn save_button(
    ui: &mut egui::Ui,
    palette: &crate::theme::Palette,
    offer: bool,
    locale: Locale,
) -> bool {
    offer
        && theme::icon_button(
            ui,
            Icon::ListPlus,
            18.0,
            palette.secondary,
            palette.text,
            &gettext(locale, "Save as a playlist"),
        )
        .clicked()
}

/// Clears manual rows from the active local queue.
fn clear_button(app: &mut App, ui: &mut egui::Ui, view: &QueuePresentation) {
    let Some(action) = &view.clear else {
        return;
    };
    let palette = app.palette;
    if theme::icon_button(
        ui,
        Icon::Trash,
        18.0,
        palette.secondary,
        palette.text,
        &gettext(app.locale, "Clear queue"),
    )
    .clicked()
    {
        app.actions.push(action.clone());
    }
}

/// A song was dropped on the queue this frame: the primary button was
/// released while a drag was over the given rect.
fn queue_drop(ui: &egui::Ui, rect: egui::Rect) -> Option<Arc<DragTrack>> {
    if ui.rect_contains_pointer(rect)
        && ui.input(|input| input.pointer.button_released(egui::PointerButton::Primary))
    {
        egui::DragAndDrop::take_payload::<DragTrack>(ui.ctx())
    } else {
        None
    }
}

/// Space below the now-playing row and below Playing next, before the next
/// section's heading.
const SECTION_GAP: f32 = 14.0;

fn contents(app: &mut App, ui: &mut egui::Ui, compact: bool, view: &QueuePresentation) {
    let palette = app.palette;
    // Neither the Web API nor librespot can reorder or insert into a live
    // remote queue, so without local playback active every drop just
    // appends, same as the "Add to queue" menu item.
    let reorderable = view.spotify_drag && app.queue_locally_reorderable();
    if view.spotify_drag
        && !reorderable
        && let Some(track) = queue_drop(ui, ui.clip_rect())
    {
        app.actions.push(Action::QueueMany {
            songs: track
                .items
                .iter()
                .map(|item| (item.uri().to_string(), item.name().to_string()))
                .collect(),
        });
    }
    match &view.status {
        QueueStatus::Ready => {}
        QueueStatus::Loading => {
            widgets::loading_row(ui, &palette, app.locale);
            return;
        }
        QueueStatus::Failed(error) => {
            widgets::error_row(ui, app, error, Some(crate::model::Page::Queue));
            return;
        }
    }
    let current = view.current.as_ref();
    // Where the songs come from, above the song itself. A radio has no
    // page, so its name is plain text.
    if let Some((name, open)) = view.context.as_ref().filter(|_| current.is_some()) {
        ui.horizontal(|ui| {
            theme::text(
                ui,
                gettext(app.locale, "Playing from"),
                theme::regular(13.0),
                palette.secondary,
            );
            match open {
                Some(action) => {
                    if theme::link(ui, name, theme::semibold(13.0), palette.text).clicked() {
                        app.actions.push(action.clone());
                    }
                }
                None => {
                    theme::text(ui, name, theme::semibold(13.0), palette.text);
                }
            }
        });
        ui.add_space(10.0);
    }
    if let Some(current) = current.as_ref() {
        theme::text(
            ui,
            gettext(app.locale, "Now playing"),
            theme::semibold(14.0),
            palette.text,
        );
        ui.add_space(4.0);
        let context = match current {
            QueueEntry::Spotify(item) => RowContext::Uris(Arc::from([item.uri().to_string()])),
            _ => RowContext::Queue,
        };
        show_entry(
            app,
            ui,
            current,
            &context,
            widgets::ProviderTrackLayout {
                number: Some(1),
                show_cover: true,
                show_album: !compact,
                compact,
                shift: 0.0,
            },
            0,
            None,
        );
        ui.add_space(SECTION_GAP);
    }
    if view.rows.is_empty() {
        if view.pending {
            widgets::loading_row(ui, &palette, app.locale);
            return;
        }
        // Nothing queued at all yet: the only slot a drop could land on is
        // the first one.
        if reorderable && let Some(track) = queue_drop(ui, ui.clip_rect()) {
            app.actions.push(Action::InsertInQueue {
                items: track.items.clone(),
                position: 0,
            });
        }
        widgets::empty_state(
            ui,
            &palette,
            Icon::ListVideo,
            &gettext(app.locale, "Nothing queued"),
            &gettext(app.locale, "Queued songs appear here."),
        );
        return;
    }
    let row_height = if compact {
        theme::COMPACT_ROW_HEIGHT
    } else {
        theme::ROW_HEIGHT
    };
    let queue_len = view.rows.len();
    // The user's own songs get their own section on top; the playing
    // context's rows follow under the usual heading. One numbering runs
    // through both, because that is the order things play.
    let queued_len = view.manual_len.min(queue_len);
    if queued_len > 0 {
        // The trash sits with the songs it removes: only this section is
        // the user's to clear, the context below plays itself.
        ui.horizontal(|ui| {
            theme::text(
                ui,
                gettext(
                    app.locale,
                    // Translators: Songs added manually, before the current playlist or album continues.
                    "Playing next",
                ),
                theme::semibold(14.0),
                palette.text,
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                clear_button(app, ui, view);
            });
        });
        ui.add_space(4.0);
        if reorderable && egui::DragAndDrop::has_payload_of_type::<DragTrack>(ui.ctx()) {
            widgets::scroll_during_drag(ui);
        }
        let gap = ui.spacing().item_spacing.y;
        // Calculate the nearest drop slot from fixed row height because
        // virtualized rows are not all available during drawing.
        let list_top = ui.cursor().top();
        // Bound the hit test to Playing next's own rows and the space below
        // them, where the append-at-the-end slot sits. The scroll area's
        // clip rect also covers Next up below, which is never a drop target.
        let section_rect = egui::Rect::from_min_max(
            egui::pos2(ui.clip_rect().left(), list_top),
            egui::pos2(
                ui.clip_rect().right(),
                list_top + queued_len as f32 * (row_height + gap) + SECTION_GAP,
            ),
        );
        let move_slot = reorderable
            .then(|| {
                egui::DragAndDrop::payload::<DragTrack>(ui.ctx())?;
                if !ui.rect_contains_pointer(section_rect) {
                    return None;
                }
                let pos = ui
                    .ctx()
                    .pointer_latest_pos()
                    .filter(|pos| section_rect.contains(*pos))?;
                let row = (pos.y - list_top) / (row_height + gap);
                (row >= 0.0).then(|| (row.round() as usize).min(queued_len))
            })
            .flatten();
        widgets::virtual_rows(ui, queued_len, row_height + gap, |ui, index| {
            let width = ui.available_width();
            let shift = ui.ctx().animate_value_with_time(
                ui.id().with(("queue-move-shift", index)),
                match move_slot {
                    Some(slot) if index < slot => -4.0,
                    Some(_) => 4.0,
                    None => 0.0,
                },
                crate::motion::seconds(ui.ctx(), 0.12),
            );
            queue_row(app, ui, view, index, compact, shift);
            ui.allocate_space(egui::vec2(width, gap));
        });
        if let Some(slot) = move_slot {
            let y = list_top + slot as f32 * (row_height + gap);
            ui.painter().hline(
                ui.max_rect().x_range().shrink(8.0),
                y,
                egui::Stroke::new(2.0, palette.accent),
            );
            if ui.input(|input| input.pointer.button_released(egui::PointerButton::Primary))
                && let Some(track) = egui::DragAndDrop::take_payload::<DragTrack>(ui.ctx())
            {
                match track.from.as_ref() {
                    Some((origin, from)) if origin == "queue" => {
                        let from = *from as usize;
                        let uri = track.items[0].uri();
                        // Playback can advance mid-drag and shift queue
                        // indices; trust the recorded one only if it still
                        // names the dragged song, otherwise find where that
                        // song actually sits now. Not there at all means it
                        // left the queue, so there is nothing left to move.
                        let from = if app.manual_queue.get(from).map(String::as_str) == Some(uri) {
                            Some(from)
                        } else {
                            app.manual_queue.iter().position(|queued| queued == uri)
                        };
                        // A row dropped back on its own edges moves nothing.
                        if let Some(from) = from
                            && slot != from
                            && slot != from + 1
                        {
                            app.actions.push(Action::MoveInQueue { from, to: slot });
                        }
                    }
                    _ => {
                        app.actions.push(Action::InsertInQueue {
                            items: track.items.clone(),
                            position: slot,
                        });
                    }
                }
            }
        }
        ui.add_space(SECTION_GAP);
    }
    // With Playing next empty, the panel holds no drop target at all: every
    // row on it belongs to Next up, which plays from the context and is never
    // rewritten. The player bar's Queue button still takes the drop.
    if queue_len > queued_len {
        theme::text(
            ui,
            gettext(
                app.locale,
                // Translators: Upcoming songs from the current playlist or album, after manually queued songs.
                "Next up",
            ),
            theme::semibold(14.0),
            palette.text,
        );
        ui.add_space(4.0);
        widgets::virtual_rows(ui, queue_len - queued_len, row_height, |ui, index| {
            queue_row(app, ui, view, queued_len + index, compact, 0.0);
        });
    }
}

fn recents_contents(app: &mut App, ui: &mut egui::Ui, view: &QueuePresentation) {
    let palette = app.palette;
    // Snapshot to avoid borrow issues while drawing. The rows are both
    // histories as one: what was played here, which Spotify is never told
    // about, and what Spotify knows of every other device.
    let items = &view.recent.rows;
    let loading = view.recent.loading;
    let error = view.recent.error.clone();
    let complete = view.recent.complete;
    let loaded_once = view.recent.loaded_once;

    if items.is_empty() {
        if loading {
            widgets::loading_row(ui, &palette, app.locale);
            return;
        }
        if let Some(err) = error {
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                theme::icon(ui, Icon::CircleAlert, 16.0, palette.danger);
                theme::text(ui, &err, theme::regular(13.0), palette.secondary);
                if theme::soft_button(
                    ui,
                    &palette,
                    Some(Icon::Refresh),
                    &gettext(app.locale, "Retry"),
                    false,
                )
                .clicked()
                    && let Some(action) = &view.recent.reload
                {
                    app.actions.push(action.clone());
                }
            });
            return;
        }
        if loaded_once {
            widgets::empty_state(
                ui,
                &palette,
                Icon::Clock,
                &gettext(app.locale, "No recent plays"),
                &gettext(app.locale, "Played songs appear here."),
            );
        } else {
            widgets::loading_row(ui, &palette, app.locale);
        }
        return;
    }

    // Show error inline if we have items but also an error on next page.
    if let Some(err) = error {
        ui.horizontal(|ui| {
            theme::icon(ui, Icon::CircleAlert, 14.0, palette.danger);
            theme::text(ui, &err, theme::regular(12.0), palette.secondary);
            if theme::soft_button(
                ui,
                &palette,
                Some(Icon::Refresh),
                &gettext(app.locale, "Retry"),
                false,
            )
            .clicked()
                && let Some(action) = &view.recent.load
            {
                app.actions.push(action.clone());
            }
        });
        ui.add_space(6.0);
    }

    let row_height = theme::COMPACT_ROW_HEIGHT;
    // Build PlayableItems on the fly; virtual_rows needs stable index.
    widgets::virtual_rows(ui, items.len(), row_height, |ui, index| {
        let (entry, played_at) = &items[index];
        let context = match entry {
            QueueEntry::Spotify(item) => RowContext::Uris(Arc::from([item.uri().to_string()])),
            _ => RowContext::Queue,
        };
        show_entry(
            app,
            ui,
            entry,
            &context,
            widgets::ProviderTrackLayout {
                number: None,
                show_cover: true,
                compact: true,
                ..Default::default()
            },
            index,
            played_at.as_deref(),
        );
    });

    // Footer: loading more or load more trigger
    if loading {
        ui.add_space(8.0);
        widgets::loading_row(ui, &palette, app.locale);
    } else if !complete {
        ui.add_space(8.0);
        // Auto-load when near end, plus manual button as fallback.
        let can_load = view.recent.load.is_some();
        // Check if scroll is near end (same heuristic as widgets::load_more_when_near_end)
        let clip = ui.clip_rect();
        let cursor = ui.cursor().top();
        if can_load
            && cursor - clip.bottom() < 900.0
            && let Some(action) = &view.recent.load
        {
            app.actions.push(action.clone());
        }
        if theme::soft_button(
            ui,
            &palette,
            Some(Icon::Refresh),
            &gettext(app.locale, "Load more"),
            false,
        )
        .clicked()
            && let Some(action) = &view.recent.load
        {
            app.actions.push(action.clone());
        }
    }
}

/// One row of the queue, numbered and indexed by its place in the whole
/// queue, whichever section it sits in. `shift` parts rows around the slot
/// a dragged row would land in, in the "Playing next" section only.
fn queue_row(
    app: &mut App,
    ui: &mut egui::Ui,
    view: &QueuePresentation,
    index: usize,
    compact: bool,
    shift: f32,
) {
    if let Some(entry) = view.rows.get(index) {
        show_entry(
            app,
            ui,
            entry,
            &RowContext::Queue,
            widgets::ProviderTrackLayout {
                number: Some(index + 1),
                show_cover: true,
                show_album: !compact,
                compact,
                shift,
            },
            index,
            None,
        );
    }
}

fn show_entry(
    app: &mut App,
    ui: &mut egui::Ui,
    entry: &QueueEntry,
    context: &RowContext,
    layout: widgets::ProviderTrackLayout,
    index: usize,
    added_at: Option<&str>,
) {
    match entry {
        QueueEntry::Spotify(item) => {
            widgets::track_row(
                ui,
                app,
                TrackRow {
                    index,
                    number: layout.number,
                    item,
                    context,
                    show_cover: layout.show_cover,
                    show_album: layout.show_album,
                    added_at,
                    added_by: None,
                    show_added_by: false,
                    compact: layout.compact,
                    thin: false,
                    shift: layout.shift,
                    picked: false,
                    picked_songs: &[],
                },
            );
        }
        QueueEntry::Provider(row) => {
            widgets::provider_track_row_with_layout(ui, app, &row.0, index, &row.1, layout);
        }
    }
}
