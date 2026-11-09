//! Jellyfin's implementation of the provider playback lifecycle.

use crate::{
    app::{App, engine_config},
    i18n::gettext,
    jellyfin::Item,
    provider::{ProviderContinuation, ProviderQueueItem, QueueItemLifecycle},
};

pub(super) fn shuffle(app: &App) -> bool {
    app.jellyfin_player.snapshot().shuffle
}

pub(super) fn can_shuffle() -> bool {
    true
}

pub(super) fn capture_continuation(app: &App) -> Option<ProviderContinuation> {
    let state = app.jellyfin_player.snapshot();
    let rows: Vec<_> = state
        .context_upcoming()
        .into_iter()
        .filter_map(|index| state.tracks.get(index).cloned())
        .collect();
    app.jellyfin_player
        .context()
        .filter(|_| !rows.is_empty())
        .map(|(source, context)| {
            ProviderContinuation::jellyfin(
                &source,
                context,
                app.jellyfin_player.context_query(),
                rows,
            )
        })
}

pub(super) fn migrate_manual_queue(app: &mut App) -> Vec<ProviderQueueItem> {
    let state = app.jellyfin_player.snapshot();
    let Some((source, _)) = app.jellyfin_player.context() else {
        return Vec::new();
    };
    let items = state
        .queued
        .iter()
        .cloned()
        .map(|item| ProviderQueueItem::jellyfin(&source, item))
        .collect();
    app.jellyfin_player.clear_queue();
    items
}

pub(super) fn can_handoff(app: &App) -> bool {
    app.jellyfin_player.controls_output()
}

pub(super) fn set_handoff_pending(app: &App, pending: bool) {
    app.jellyfin_player.set_external_queue_pending(pending);
}

fn configure_playback(app: &mut App) {
    app.prepare_jellyfin_playback();
    app.jellyfin_player.configure(engine_config(
        &app.dirs,
        &app.settings,
        app.applied_jellyfin_proxy(),
        app.winamp.tap.clone(),
        app.winamp.eq.clone(),
        app.local_output.clone(),
    ));
}

pub(super) fn activate(app: &mut App) {
    // A credential store can be temporarily unavailable while the app is
    // restored. Selecting this provider is an explicit opportunity to retry
    // its active saved account without teaching shared navigation about
    // Jellyfin credentials.
    if app.jellyfin.error.is_some()
        && let Some(source) = app.jellyfin.source.clone()
    {
        app.jellyfin
            .activate_server(source.id, &app.applied_jellyfin_proxy());
    }
}

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
    let source = app
        .jellyfin_source(source, account)
        .expect("a prepared Jellyfin occurrence retains its source");
    configure_playback(app);
    let context = item.album_id.clone().unwrap_or_else(|| item.id.clone());
    app.jellyfin_player
        .play(source, context, item.clone(), Some(vec![item]));
}

pub(super) fn execute_continuation(
    app: &mut App,
    source: &str,
    account: &str,
    context: String,
    query: Option<crate::jellyfin::catalog::Query>,
    rows: Vec<Item>,
    index: usize,
) {
    let Some(item) = rows.get(index).cloned() else {
        return;
    };
    let source = app
        .jellyfin_source(source, account)
        .expect("a prepared Jellyfin continuation retains its source");
    let tracks = rows.into_iter().skip(index).collect();
    configure_playback(app);
    app.jellyfin_player
        .play_continuation(source, context, item, tracks, query);
}
