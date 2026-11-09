//! Spotify's implementation of the provider playback lifecycle.

use crate::{
    api::PlayRequest,
    api::models::{PlayableItem, PlaybackState},
    app::{App, Target},
    i18n::gettext,
    provider::{ProviderContinuation, ProviderQueueItem, QueueItemLifecycle},
    soloist::Status,
};

pub(super) fn shuffle(app: &App) -> bool {
    app.spotify_shuffle_wanted()
}

pub(super) fn can_shuffle() -> bool {
    true
}

pub(super) fn capture_continuation(app: &App) -> Option<ProviderContinuation> {
    let rows = app.spotify_context_upcoming();
    (!rows.is_empty()).then(|| {
        ProviderContinuation::spotify(
            app.playing_context_uri(),
            app.spotify_shuffle_wanted(),
            rows,
        )
    })
}

pub(super) fn migrate_manual_queue(app: &mut App) -> Vec<ProviderQueueItem> {
    app.take_spotify_manual_queue()
        .into_iter()
        .map(ProviderQueueItem::spotify)
        .collect()
}

pub(super) fn can_handoff(app: &App) -> bool {
    app.targets_own_soloist() || (app.local.is_active() && matches!(app.target(), Target::Local))
}

pub(super) fn set_handoff_pending(_app: &App, _pending: bool) {}

fn owned_target(app: &App) -> Option<Target> {
    if app.settings.soloist_enabled {
        return (matches!(app.soloist_status, Status::Ready { .. }))
            .then(|| app.soloist_device_id.clone())
            .flatten()
            .map(|id| Target::Remote(Some(id)));
    }
    (app.local_ready && app.local.connected).then_some(Target::Local)
}

/// Soloist is Spotify's app-owned remote engine. Its end-timed playback poll
/// is the provider's natural-end signal; explicit commands and their expected
/// track transitions have already updated the shared queue optimistically.
pub(super) fn remote_natural_end(app: &App, next: Option<&PlaybackState>) -> bool {
    if !app.provider_handoff_waiting()
        || !app.targets_own_soloist()
        || !app.remote_end_poll_recent()
        || app.remote_command_recent()
    {
        return false;
    }
    let Some(previous) = app.remote_fresh().filter(|remote| remote.state.is_playing) else {
        return false;
    };
    let previous_uri = previous.state.item.as_ref().map(PlayableItem::uri);
    let next_uri = next
        .and_then(|state| state.item.as_ref())
        .map(PlayableItem::uri);
    let commanded = app
        .expected_remote_track()
        .is_some_and(|expected| next_uri == Some(expected));
    !commanded && (next.is_none_or(|state| !state.is_playing) || next_uri != previous_uri)
}

pub(super) fn activate(app: &mut App) {
    app.start_configured_soloist();
}

pub(super) fn prepare(app: &mut App) -> QueueItemLifecycle {
    if owned_target(app).is_some() {
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
    let target = owned_target(app).expect("a prepared Spotify occurrence has an owned target");
    if let Target::Remote(Some(device)) = &target
        && app.soloist_device_id.as_deref() == Some(device.as_str())
    {
        app.selected_device = Some(device.clone());
    }
    app.play_request_on(
        PlayRequest::tracks(vec![item.uri().to_owned()]).starting_at_index(0),
        false,
        target,
    );
}

pub(super) fn execute_continuation(
    app: &mut App,
    context_uri: Option<String>,
    shuffle: bool,
    rows: Vec<PlayableItem>,
    index: usize,
) {
    let Some(item) = rows.get(index) else {
        return;
    };
    let target = owned_target(app).expect("a prepared Spotify continuation has an owned target");
    if let Target::Remote(Some(device)) = &target
        && app.soloist_device_id.as_deref() == Some(device.as_str())
    {
        app.selected_device = Some(device.clone());
    }
    let request = match context_uri {
        Some(context) => PlayRequest::context(context).starting_at_uri(item.uri()),
        None => PlayRequest::tracks(
            rows.into_iter()
                .skip(index)
                .map(|item| item.uri().to_owned())
                .collect(),
        )
        .starting_at_index(0),
    };
    app.play_request_on(request, shuffle, target);
}
