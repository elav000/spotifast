//! Spotify's implementation of the provider playback lifecycle.

use crate::{
    api::models::PlayableItem, app::App, i18n::gettext, provider::QueueItemLifecycle,
    soloist::Status,
};

pub(super) fn activate(app: &mut App) {
    app.start_configured_soloist();
}

pub(super) fn prepare(app: &mut App) -> QueueItemLifecycle {
    if app.mixed_spotify_target().is_some() {
        return QueueItemLifecycle::Ready;
    }
    if !app.settings.soloist_enabled || matches!(app.soloist_status, Status::NeedsKey) {
        return QueueItemLifecycle::Failed(
            gettext(
                app.locale,
                "The configured Spotify player is not ready. Skipped this queued song.",
            )
            .into_owned(),
        );
    }
    if matches!(app.soloist_status, Status::Failed(_)) && app.soloist_retry_at.is_none() {
        return QueueItemLifecycle::Failed(
            gettext(
                app.locale,
                "The configured Spotify player could not start. Skipped this queued song.",
            )
            .into_owned(),
        );
    }
    app.start_configured_soloist();
    QueueItemLifecycle::Preparing
}

pub(super) fn execute(app: &mut App, item: PlayableItem) {
    app.play_spotify_queue_item(item);
}
