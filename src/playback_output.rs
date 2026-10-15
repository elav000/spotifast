//! Exclusive local output, independent of the provider being browsed.
//! Claims and guarded audio writes run on playback threads, never the UI.
use crate::{provider::Provider, sink::AudioControl};
use librespot_playback::audio_backend::SinkResult;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Default, Debug)]
pub struct LocalOutput {
    state: Mutex<State>,
    requested: AtomicU64,
}

/// One explicit play intent. Reserving it is nonblocking and does not silence
/// the current owner while replacement audio is still loading.
#[derive(Clone, Copy, Debug)]
pub struct OutputRequest(u64);

#[derive(Default)]
struct State {
    generation: u64,
    owner: Option<(Provider, Arc<AudioControl>)>,
    interruptible: bool,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutputState")
            .field("generation", &self.generation)
            .field(
                "provider",
                &self.owner.as_ref().map(|(provider, _)| provider),
            )
            .finish()
    }
}

/// Identifies one activation, including replacements within the same provider.
#[derive(Clone, Copy)]
pub(crate) struct Lease(u64);

/// Held through an audio write so takeover cannot race an already checked
/// packet. Network access and decoding must happen before obtaining it.
pub(crate) struct WriteGuard<'a> {
    _state: MutexGuard<'a, State>,
}

impl LocalOutput {
    pub fn request(&self) -> OutputRequest {
        OutputRequest(self.requested.fetch_add(1, Ordering::SeqCst) + 1)
    }

    pub fn is_current(&self, request: OutputRequest) -> bool {
        self.requested.load(Ordering::SeqCst) == request.0
    }

    pub fn owner(&self) -> Option<Provider> {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .owner
            .as_ref()
            .map(|(provider, _)| *provider)
    }

    pub(crate) fn claim(
        &self,
        provider: Provider,
        control: Arc<AudioControl>,
        interruptible: bool,
    ) -> SinkResult<Lease> {
        self.claim_requested(provider, control, interruptible, self.request())
    }

    pub(crate) fn claim_requested(
        &self,
        provider: Provider,
        control: Arc<AudioControl>,
        interruptible: bool,
        request: OutputRequest,
    ) -> SinkResult<Lease> {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if self.requested.load(Ordering::SeqCst) != request.0 {
            return Err(librespot_playback::audio_backend::SinkError::NotConnected(
                "Playback was superseded by a newer play request.".into(),
            ));
        }
        if let Some((_, previous)) = &state.owner
            && !Arc::ptr_eq(previous, &control)
        {
            if !state.interruptible {
                return Err(librespot_playback::audio_backend::SinkError::ConnectionRefused(
                    "Stop the current player before switching providers with this audio backend.".into(),
                ));
            }
            // Fade and discard before granting the new writer. Optional
            // backends must stop on their own thread before a takeover.
            previous.relinquish_output();
        }
        state.generation += 1;
        state.owner = Some((provider, control));
        state.interruptible = interruptible;
        Ok(Lease(state.generation))
    }

    pub(crate) fn writing(&self, lease: Lease) -> Option<WriteGuard<'_>> {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        (state.owner.is_some() && state.generation == lease.0)
            .then_some(WriteGuard { _state: state })
    }

    pub(crate) fn release(&self, lease: Lease) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.generation == lease.0 {
            state.owner = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(output: &LocalOutput, provider: Provider) -> Lease {
        output
            .claim(provider, AudioControl::new(100), true)
            .unwrap()
    }

    #[test]
    fn a_slow_load_cannot_take_output_from_a_newer_play_request() {
        let output = LocalOutput::default();
        let first = claim(&output, Provider::Spotify);
        let loading = output.request();
        assert!(output.writing(first).is_some());
        let newer = claim(&output, Provider::Spotify);
        assert!(
            output
                .claim_requested(Provider::Jellyfin, AudioControl::new(100), true, loading,)
                .is_err()
        );
        assert!(output.writing(newer).is_some());
        assert_eq!(output.owner(), Some(Provider::Spotify));
    }

    #[test]
    fn delayed_releases_and_same_provider_replacements_cannot_reclaim_output() {
        let output = LocalOutput::default();
        let spotify = claim(&output, Provider::Spotify);
        let jellyfin = claim(&output, Provider::Jellyfin);
        assert!(output.writing(spotify).is_none());
        output.release(spotify);
        assert_eq!(output.owner(), Some(Provider::Jellyfin));
        let replacement = claim(&output, Provider::Jellyfin);
        assert!(output.writing(jellyfin).is_none());
        output.release(jellyfin);
        assert!(output.writing(replacement).is_some());
        output.release(replacement);
        assert_eq!(output.owner(), None);
    }

    #[test]
    fn takeover_waits_for_an_already_admitted_write() {
        use std::{sync::mpsc, time::Duration};
        let output = Arc::new(LocalOutput::default());
        let old = claim(&output, Provider::Spotify);
        let writing = output.writing(old).unwrap();
        let (ready, entered) = mpsc::channel();
        let (claimed, finished) = mpsc::channel();
        let next = output.clone();
        let thread = std::thread::spawn(move || {
            ready.send(()).unwrap();
            let lease = claim(&next, Provider::Jellyfin);
            claimed.send(lease).unwrap();
        });
        entered.recv().unwrap();
        assert!(matches!(
            finished.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        drop(writing);
        let new = finished.recv_timeout(Duration::from_secs(2)).unwrap();
        thread.join().unwrap();
        assert!(output.writing(old).is_none());
        assert!(output.writing(new).is_some());
    }

    #[test]
    fn alternate_backend_must_stop_before_a_takeover() {
        let output = LocalOutput::default();
        let old = output
            .claim(Provider::Spotify, AudioControl::new(100), false)
            .unwrap();
        assert!(
            output
                .claim(Provider::Jellyfin, AudioControl::new(100), true)
                .is_err()
        );
        assert_eq!(output.owner(), Some(Provider::Spotify));
        assert!(output.writing(old).is_some());
        output.release(old);
        let new = claim(&output, Provider::Jellyfin);
        assert!(output.writing(new).is_some());
    }
}
