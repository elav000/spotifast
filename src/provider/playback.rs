//! Provider lifecycle hooks for stamped local queue occurrences. The shared
//! queue coordinates ordering and exact occurrences; providers own restoring,
//! readiness checks and execution.

use std::collections::HashSet;

use super::{Provider, ProviderContinuation, ProviderQueueItem, ProviderStamp};
use crate::app::App;

#[path = "playback/jellyfin.rs"]
mod jellyfin;
#[path = "playback/spotify.rs"]
mod spotify;

pub(crate) enum QueueItemLifecycle {
    Ready,
    Preparing,
    Failed(String),
}

impl App {
    /// Capture the active provider's remaining automatic context. The shared
    /// coordinator stores the opaque result but never inspects engine state.
    pub(crate) fn capture_active_provider_continuation(&self) -> Option<ProviderContinuation> {
        match self.playback_provider() {
            Some(Provider::Spotify) => spotify::capture_continuation(self),
            Some(Provider::Jellyfin) => jellyfin::capture_continuation(self),
            None => None,
        }
    }

    /// Move provider-native manual occurrences into the stamped shared queue.
    pub(crate) fn migrate_active_provider_queue(&mut self) -> Vec<ProviderQueueItem> {
        match self.playback_provider() {
            Some(Provider::Spotify) => spotify::migrate_manual_queue(self),
            Some(Provider::Jellyfin) => jellyfin::migrate_manual_queue(self),
            None => Vec::new(),
        }
    }

    /// Whether the active provider currently owns an output that may hand its
    /// natural end or explicit Next to the shared coordinator.
    pub(crate) fn active_provider_can_handoff(&self) -> bool {
        match self.playback_provider() {
            Some(Provider::Spotify) => spotify::can_handoff(self),
            Some(Provider::Jellyfin) => jellyfin::can_handoff(self),
            // A provider-owned output may already be selected while its first
            // now-playing snapshot is still pending. Ask the adapters rather
            // than making the coordinator guess which engine that is.
            None => spotify::can_handoff(self) || jellyfin::can_handoff(self),
        }
    }

    /// Tell provider engines whether an external stamped occurrence or saved
    /// context must take precedence at natural end.
    pub(crate) fn sync_provider_handoff_pending(&self, pending: bool) {
        spotify::set_handoff_pending(self, pending);
        jellyfin::set_handoff_pending(self, pending);
    }

    /// Ask the active playback provider whether a remote state transition is
    /// its natural-end signal. Shared queue code never names that provider's
    /// engine or credential lifecycle.
    pub(crate) fn provider_remote_natural_end(
        &self,
        next: Option<&crate::api::models::PlaybackState>,
    ) -> bool {
        match self.playback_provider() {
            Some(Provider::Spotify) => spotify::remote_natural_end(self, next),
            Some(Provider::Jellyfin) | None => false,
        }
    }

    /// Activate the selected catalog provider without exposing its concrete
    /// runtime to shared navigation code.
    pub(crate) fn activate_selected_provider(&mut self) {
        self.activate_provider(self.selected_provider());
    }

    fn activate_provider(&mut self, provider: Provider) {
        match provider {
            Provider::Spotify => spotify::activate(self),
            Provider::Jellyfin => jellyfin::activate(self),
        }
    }

    /// Let every provider represented by restored rows initialize what it
    /// needs. Implementations must not transfer playback merely to prepare.
    pub(crate) fn restore_provider_queue(&mut self) {
        let mut seen = HashSet::new();
        let stamps: Vec<_> = self
            .mixed_queue
            .iter()
            .map(ProviderQueueItem::stamp)
            .chain(
                self.provider_continuation
                    .iter()
                    .map(ProviderContinuation::stamp),
            )
            .filter(|stamp| seen.insert(stamp.clone()))
            .collect();
        for stamp in stamps {
            self.restore_queue_provider(&stamp);
        }
    }

    pub(crate) fn restore_queue_provider(&mut self, stamp: &ProviderStamp) {
        self.activate_provider(stamp.provider());
    }

    /// Whether a provider runtime is still required by the browsed catalog or
    /// any saved occurrence. Provider engines use this to decide whether a
    /// background retry still serves visible state.
    pub(crate) fn provider_runtime_needed(&self, provider: Provider) -> bool {
        self.selected_provider() == provider
            || self
                .mixed_queue
                .iter()
                .any(|item| item.stamp().provider() == provider)
            || self
                .provider_continuation
                .as_ref()
                .is_some_and(|continuation| continuation.stamp().provider() == provider)
    }

    /// Prepare one exact occurrence without consuming it. `Preparing` means
    /// the provider will later call the shared ready or failed continuation.
    pub(crate) fn prepare_provider_queue_item(
        &mut self,
        item: &ProviderQueueItem,
    ) -> QueueItemLifecycle {
        match item {
            ProviderQueueItem::Spotify { .. } => spotify::prepare(self),
            ProviderQueueItem::Jellyfin {
                source, account, ..
            } => jellyfin::prepare(self, source, account),
        }
    }

    /// Execute an occurrence only after its provider returned `Ready`.
    pub(crate) fn execute_provider_queue_item(&mut self, item: ProviderQueueItem) {
        match item {
            ProviderQueueItem::Spotify { item } => spotify::execute(self, item),
            ProviderQueueItem::Jellyfin {
                source,
                account,
                item,
            } => jellyfin::execute(self, &source, &account, item),
        }
    }

    pub(crate) fn prepare_provider_continuation(
        &mut self,
        continuation: &ProviderContinuation,
    ) -> QueueItemLifecycle {
        match continuation {
            ProviderContinuation::Spotify { .. } => spotify::prepare(self),
            ProviderContinuation::Jellyfin {
                source, account, ..
            } => jellyfin::prepare(self, source, account),
        }
    }

    pub(crate) fn execute_provider_continuation(
        &mut self,
        continuation: ProviderContinuation,
        index: usize,
    ) {
        match continuation {
            ProviderContinuation::Spotify {
                context_uri,
                shuffle,
                rows,
            } => spotify::execute_continuation(self, context_uri, shuffle, rows, index),
            ProviderContinuation::Jellyfin {
                source,
                account,
                context,
                query,
                rows,
            } => {
                jellyfin::execute_continuation(self, &source, &account, context, query, rows, index)
            }
        }
    }
}
