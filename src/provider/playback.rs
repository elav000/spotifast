//! Provider lifecycle hooks for stamped local queue occurrences. The shared
//! queue coordinates ordering and exact occurrences; providers own restoring,
//! readiness checks and execution.

use std::collections::HashSet;

use super::{Provider, ProviderQueueItem, ProviderStamp};
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
}
