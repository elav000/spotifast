//! Jellyfin's implementation of the provider playback lifecycle.

use crate::{app::App, i18n::gettext, jellyfin::Item, provider::QueueItemLifecycle};

pub(super) fn activate(_app: &mut App) {}

pub(super) fn prepare(app: &App, source: &str, account: &str) -> QueueItemLifecycle {
    if app.jellyfin_source(source, account).is_some() {
        QueueItemLifecycle::Ready
    } else {
        QueueItemLifecycle::Failed(
            gettext(
                app.locale,
                "This queued song's Jellyfin server or account is no longer available. Skipped it.",
            )
            .into_owned(),
        )
    }
}

pub(super) fn execute(app: &mut App, source: &str, account: &str, item: Item) {
    app.play_jellyfin_queue_item(source, account, item);
}
