//! Bounded decoded-audio read-ahead. Network/decoder waits never own playback
//! controls. Seek revisions discard data from an older in-flight decode.
use super::decode::AudioDecoder;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

const AHEAD_FRAMES: usize = 8 * 44100;
const START_FRAMES: usize = 44100;

trait Decode: Send + 'static {
    fn next(&mut self) -> Result<Option<Vec<f64>>, String>;
    fn seek(&mut self, position: u64) -> Result<(), String>;
}

impl Decode for AudioDecoder {
    fn next(&mut self) -> Result<Option<Vec<f64>>, String> {
        self.next_pcm()
    }
    fn seek(&mut self, position: u64) -> Result<(), String> {
        self.seek_ms(position)
    }
}

#[derive(Default)]
struct State {
    packets: VecDeque<Vec<f64>>,
    frames: usize,
    revision: u64,
    seek: Option<u64>,
    terminal: Option<Result<(), String>>,
    priming: bool,
    stopped: bool,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    cancel: Arc<AtomicBool>,
}

pub enum Readiness {
    Pending,
    Packet(Vec<f64>),
    End,
}

pub struct BufferedDecoder {
    shared: Arc<Shared>,
    duration: Option<u64>,
    approximate: bool,
}

impl BufferedDecoder {
    pub fn new(decoder: AudioDecoder, cancel: Arc<AtomicBool>) -> Result<Self, String> {
        let duration = decoder.duration_ms();
        let approximate = decoder.approximate_seeks();
        Self::spawn(decoder, cancel, duration, approximate)
    }

    fn spawn(
        decoder: impl Decode,
        cancel: Arc<AtomicBool>,
        duration: Option<u64>,
        approximate: bool,
    ) -> Result<Self, String> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                priming: true,
                ..State::default()
            }),
            changed: Condvar::new(),
            cancel,
        });
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("media-decode".into())
            .spawn(move || produce(decoder, worker))
            .map_err(|_| "Could not start audio buffering.".to_owned())?;
        Ok(Self {
            shared,
            duration,
            approximate,
        })
    }

    pub fn duration_ms(&self) -> Option<u64> {
        self.duration
    }
    pub fn approximate_seeks(&self) -> bool {
        self.approximate
    }

    /// Never waits for the decoder. A short track or an error drains its valid
    /// prefix without waiting for a full startup cushion.
    pub fn poll(&mut self) -> Result<Readiness, String> {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.seek.is_some()
            || (state.priming && state.frames < START_FRAMES && state.terminal.is_none())
        {
            return Ok(Readiness::Pending);
        }
        if let Some(packet) = state.packets.pop_front() {
            state.priming = false;
            state.frames -= packet.len() / 2;
            self.shared.changed.notify_one();
            return Ok(Readiness::Packet(packet));
        }
        if let Some(terminal) = &state.terminal {
            return terminal.clone().map(|()| Readiness::End);
        }
        state.priming = true;
        Ok(Readiness::Pending)
    }

    /// Invalidates queued and in-flight PCM immediately, even while input is
    /// blocked. The worker executes only the newest outstanding seek.
    pub fn seek_ms(&mut self, position: u64) {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        state.revision = state.revision.wrapping_add(1);
        state.seek = Some(position);
        state.packets.clear();
        state.frames = 0;
        state.terminal = None;
        state.priming = true;
        self.shared.changed.notify_one();
    }
}

impl Drop for BufferedDecoder {
    fn drop(&mut self) {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .stopped = true;
        self.shared.changed.notify_one();
        // A network read may still be finishing. Its request deadline and the
        // track's cancellation flag bound it. Do not set that shared flag here:
        // normal EOF/errors must still reach the transport's queue controller.
        // Playback controls never join a blocked input thread.
    }
}

fn produce(mut decoder: impl Decode, shared: Arc<Shared>) {
    loop {
        let mut state = shared.state.lock().unwrap_or_else(|p| p.into_inner());
        while !state.stopped
            && !shared.cancel.load(Ordering::SeqCst)
            && state.seek.is_none()
            && (state.frames >= AHEAD_FRAMES || state.terminal.is_some())
        {
            state = shared
                .changed
                .wait(state)
                .unwrap_or_else(|p| p.into_inner());
        }
        if state.stopped || shared.cancel.load(Ordering::SeqCst) {
            return;
        }
        let revision = state.revision;
        let seek = state.seek.take();
        drop(state);
        let result = if let Some(position) = seek {
            decoder.seek(position).and_then(|()| decoder.next())
        } else {
            decoder.next()
        };
        let mut state = shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.stopped || shared.cancel.load(Ordering::SeqCst) {
            return;
        }
        if state.revision != revision {
            continue;
        }
        match result {
            Ok(Some(packet)) if !packet.is_empty() => {
                // Split codec blocks so queued PCM has a strict eight-second
                // cap. At most one bounded codec block remains on this worker.
                for part in packet.chunks(8820) {
                    while state.frames + part.len() / 2 > AHEAD_FRAMES
                        && !state.stopped
                        && state.revision == revision
                        && !shared.cancel.load(Ordering::SeqCst)
                    {
                        state = shared
                            .changed
                            .wait(state)
                            .unwrap_or_else(|p| p.into_inner());
                    }
                    if state.stopped || shared.cancel.load(Ordering::SeqCst) {
                        return;
                    }
                    if state.revision != revision {
                        break;
                    }
                    state.frames += part.len() / 2;
                    state.packets.push_back(part.to_vec());
                    shared.changed.notify_all();
                }
            }
            Ok(Some(_)) => {}
            Ok(None) => state.terminal = Some(Ok(())),
            Err(error) => state.terminal = Some(Err(error)),
        }
        shared.changed.notify_all();
    }
}

#[cfg(test)]
mod tests;
