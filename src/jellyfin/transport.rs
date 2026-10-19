//! Jellyfin's independent album transport. The UI changes intent; one worker
//! owns protected credentials, HTTP input, decoding and the audio device.
use super::{Item, browser::Source};
use crate::media::{
    decode::AudioDecoder,
    http::HttpRangeSource,
    pipeline::{BufferedDecoder, Readiness},
};
use crate::playback_output::OutputRequest;
use crate::sink::{AudioControl, RodioSink};
use crate::vis::Tapped;
use crate::{backend::Waker, paths::AppDirs, player::EngineConfig, provider::Provider};
use librespot_playback::{
    audio_backend::Sink, convert::Converter, decoder::AudioPacket, mixer::VolumeGetter,
};
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub struct Snapshot {
    pub source_id: Option<String>,
    pub item: Option<Item>,
    pub tracks: Arc<Vec<Item>>,
    pub queued: Arc<Vec<Item>>,
    pub manual_current: bool,
    pub queue_complete: bool,
    pub index: usize,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub playing: bool,
    pub loading: bool,
    pub finished: bool,
    pub approximate_seek: bool,
    pub error: Option<String>,
    pub volume: u8,
}

#[derive(Clone)]
struct Job {
    source: Source,
    album: String,
    item: Item,
    known_tracks: Option<Vec<Item>>,
    query: Option<super::catalog::Query>,
    config: EngineConfig,
    manual: bool,
    context_index: Option<usize>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Cursor {
    item: Item,
    index: usize,
    manual: bool,
}

#[derive(Serialize, Deserialize)]
struct SavedPlayback {
    version: u32,
    source: Source,
    album: String,
    #[serde(default)]
    query: Option<super::catalog::Query>,
    item: Item,
    tracks: Vec<Item>,
    queued: Vec<Item>,
    index: usize,
    manual: bool,
    position_ms: u64,
    finished: bool,
    queue_complete: bool,
    history: Vec<Cursor>,
}

const SAVED_LIMIT: u64 = 16 * 1024 * 1024;

struct State {
    epoch: u64,
    job: Option<Job>,
    template: Option<Job>,
    snapshot: Snapshot,
    request: Option<OutputRequest>,
    seek: Option<(u64, u64)>,
    seek_revision: u64,
    cancel: Arc<AtomicBool>,
    shutdown: bool,
    active_epoch: Option<u64>,
    history: Vec<Cursor>,
    restored: bool,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    volume: Arc<AtomicU64>,
    waker: Waker,
}

pub struct Transport {
    shared: Arc<Shared>,
    config: EngineConfig,
}

impl Transport {
    pub fn new(dirs: AppDirs, waker: Waker, config: EngineConfig, enabled: bool) -> Self {
        let volume = crate::app::volume_to_percent(config.initial_volume);
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                epoch: 0,
                job: None,
                template: None,
                snapshot: Snapshot {
                    volume,
                    ..Snapshot::default()
                },
                request: None,
                seek: None,
                seek_revision: 0,
                cancel: Arc::new(AtomicBool::new(false)),
                shutdown: false,
                active_epoch: None,
                history: Vec::new(),
                restored: !enabled,
            }),
            changed: Condvar::new(),
            volume: Arc::new(AtomicU64::new(level(volume).to_bits())),
            waker,
        });
        if enabled {
            let worker = shared.clone();
            let initial_config = config.clone();
            if std::thread::Builder::new()
                .name("spotifast-jellyfin-audio".into())
                .spawn(move || run(dirs, worker, initial_config))
                .is_err()
            {
                shared
                    .state
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .snapshot
                    .error = Some("Could not start Jellyfin playback.".into());
            }
        }
        Self { shared, config }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .snapshot
            .clone()
    }

    /// A fresh catalog result can enrich an older saved song's artwork without
    /// changing output ownership, decoder intent, queue order or position.
    pub(crate) fn refresh_metadata(&self, source: &Source, items: &[Item]) -> bool {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if !state
            .template
            .as_ref()
            .is_some_and(|job| &job.source == source)
        {
            return false;
        }
        let Some(current) = state.snapshot.item.as_ref() else {
            return false;
        };
        let Some(updated) = items
            .iter()
            .find(|item| item.id == current.id && *item != current)
            .cloned()
        else {
            return false;
        };
        if let Some(job) = &mut state.template {
            job.item = updated.clone();
        }
        if let Some(job) = &mut state.job {
            job.item = updated.clone();
        }
        state.snapshot.item = Some(updated);
        true
    }

    pub(crate) fn encode_session(&self) -> Result<Option<Vec<u8>>, String> {
        let state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if !state.restored {
            return Ok(None);
        }
        let saved = state
            .template
            .as_ref()
            .zip(state.snapshot.item.as_ref())
            .map(|(job, item)| SavedPlayback {
                version: 1,
                source: job.source.clone(),
                album: job.album.clone(),
                query: job.query.clone(),
                item: item.clone(),
                tracks: state.snapshot.tracks.as_ref().clone(),
                queued: state.snapshot.queued.as_ref().clone(),
                index: state.snapshot.index,
                manual: state.snapshot.manual_current,
                position_ms: state.snapshot.position_ms,
                finished: state.snapshot.finished,
                queue_complete: state.snapshot.queue_complete,
                history: state.history.clone(),
            });
        drop(state);
        let bytes =
            serde_json::to_vec(&saved).map_err(|_| "Could not encode Jellyfin playback state.")?;
        if bytes.len() as u64 > SAVED_LIMIT {
            return Err("Jellyfin playback state exceeds the save limit.".into());
        }
        Ok(Some(bytes))
    }

    pub(crate) fn epoch(&self) -> u64 {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .epoch
    }

    pub(crate) fn context(&self) -> Option<(Source, String)> {
        let state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        state
            .template
            .as_ref()
            .map(|job| (job.source.clone(), job.album.clone()))
    }

    pub(crate) fn recent_items(&self) -> Vec<Item> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .history
            .iter()
            .rev()
            .map(|cursor| cursor.item.clone())
            .collect()
    }

    pub fn controls_output(&self) -> bool {
        let state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        state.snapshot.item.is_some()
            && state
                .request
                .is_some_and(|request| self.config.output.is_current(request))
    }

    pub fn context_kind(&self) -> Option<crate::provider::MediaKind> {
        use super::catalog::Query;
        let state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        match state.template.as_ref()?.query.as_ref() {
            None => Some(crate::provider::MediaKind::Album),
            Some(Query::PlaylistSongs { .. }) => Some(crate::provider::MediaKind::Playlist),
            Some(Query::ArtistSongs { .. }) => Some(crate::provider::MediaKind::Artist),
            _ => None,
        }
    }

    pub fn play(&self, source: Source, album: String, item: Item, tracks: Option<Vec<Item>>) {
        let job = Job {
            source,
            album,
            item,
            known_tracks: tracks,
            query: None,
            config: self.config.clone(),
            manual: false,
            context_index: None,
        };
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        state.history.clear();
        select(&mut state, job);
        self.shared.changed.notify_one();
    }

    pub fn play_catalog(
        &self,
        source: Source,
        context: String,
        item: Item,
        query: Option<super::catalog::Query>,
        index: Option<usize>,
    ) {
        let job = Job {
            source,
            album: context,
            item: item.clone(),
            known_tracks: query.is_none().then(|| vec![item]),
            query,
            config: self.config.clone(),
            manual: false,
            context_index: index,
        };
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        state.history.clear();
        select(&mut state, job);
        self.shared.changed.notify_one();
    }

    pub fn toggle(&self) {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.snapshot.item.is_none() {
            return;
        }
        state.snapshot.playing = !state.snapshot.playing;
        if !state.snapshot.playing {
            state.snapshot.loading = false;
        }
        if state.snapshot.playing {
            if state.active_epoch != Some(state.epoch) && state.job.is_none() {
                if let Some(mut job) = state.template.clone() {
                    let position = if state.snapshot.finished {
                        0
                    } else {
                        state.snapshot.position_ms
                    };
                    job.known_tracks = state
                        .snapshot
                        .queue_complete
                        .then(|| state.snapshot.tracks.as_ref().clone());
                    select(&mut state, job);
                    state.seek_revision += 1;
                    state.seek = Some((state.seek_revision, position));
                    state.snapshot.position_ms = position;
                }
            } else {
                state.request = Some(self.config.output.request());
                state.snapshot.error = None;
            }
        }
        self.shared.changed.notify_one();
    }

    pub fn seek(&self, position_ms: u64) {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        let position = position_ms.min(state.snapshot.duration_ms.saturating_sub(1));
        state.seek_revision += 1;
        state.seek = Some((state.seek_revision, position));
        state.snapshot.position_ms = position;
        state.snapshot.finished = false;
        self.shared.changed.notify_one();
    }

    pub fn step(&self, forward: bool) {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if !state.snapshot.queue_complete {
            return;
        }
        if !forward && state.snapshot.position_ms > 3000 {
            drop(state);
            self.seek(0);
            return;
        }
        advance(&mut state, forward);
        self.shared.changed.notify_one();
    }

    pub fn play_index(&self, index: usize) {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if !state.snapshot.queue_complete {
            return;
        }
        if let Some(item) = state.snapshot.tracks.get(index).cloned()
            && let Some(mut job) = state.template.clone()
        {
            remember_current(&mut state);
            state.snapshot.queued = Arc::default();
            job.item = item;
            job.manual = false;
            job.context_index = Some(index);
            job.known_tracks = Some(state.snapshot.tracks.as_ref().clone());
            select(&mut state, job);
            self.shared.changed.notify_one();
        }
    }

    pub fn enqueue(&self, source: &Source, item: Item) {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if !state
            .template
            .as_ref()
            .is_some_and(|job| &job.source == source)
        {
            state.snapshot.error =
                Some("Play a track from this Jellyfin account before adding to its queue.".into());
            return;
        }
        if state.snapshot.queued.len() >= 1000 {
            state.snapshot.error = Some("The Jellyfin queue is full (1,000 tracks).".into());
            return;
        }
        Arc::make_mut(&mut state.snapshot.queued).push(item);
        state.snapshot.error = None;
    }

    /// Apply row edits only while the same immutable queue snapshot is current.
    /// This guards repeated occurrences and account switches during a menu action.
    pub(crate) fn edit_manual(
        &self,
        source: &Source,
        expected: &Arc<Vec<Item>>,
        index: usize,
        to: Option<usize>,
    ) {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if !state
            .template
            .as_ref()
            .is_some_and(|job| &job.source == source)
            || !Arc::ptr_eq(expected, &state.snapshot.queued)
            || index >= state.snapshot.queued.len()
        {
            return;
        }
        let queue = Arc::make_mut(&mut state.snapshot.queued);
        let item = queue.remove(index);
        if let Some(to) = to {
            let to = to.min(queue.len());
            queue.insert(to, item);
        }
    }

    /// Remove selected occurrences together, against the snapshot the UI drew.
    pub(crate) fn remove_manual(
        &self,
        source: &Source,
        expected: &Arc<Vec<Item>>,
        indices: &[usize],
    ) {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if !state
            .template
            .as_ref()
            .is_some_and(|job| &job.source == source)
            || !Arc::ptr_eq(expected, &state.snapshot.queued)
            || indices.iter().any(|index| *index >= expected.len())
        {
            return;
        }
        let mut indices = indices.to_vec();
        indices.sort_unstable();
        indices.dedup();
        let queue = Arc::make_mut(&mut state.snapshot.queued);
        for index in indices.into_iter().rev() {
            queue.remove(index);
        }
    }

    pub fn clear_queue(&self) {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .snapshot
            .queued = Arc::default();
    }

    pub fn play_queued(&self, index: usize) {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if !state.snapshot.queue_complete {
            return;
        }
        if let Some(item) = state.snapshot.queued.get(index).cloned()
            && let Some(mut job) = state.template.clone()
        {
            remember_current(&mut state);
            Arc::make_mut(&mut state.snapshot.queued).drain(..=index);
            job.album = item.album_id.clone().unwrap_or(job.album);
            job.item = item;
            job.manual = true;
            job.context_index = Some(state.snapshot.index);
            job.known_tracks = Some(state.snapshot.tracks.as_ref().clone());
            select(&mut state, job);
            self.shared.changed.notify_one();
        }
    }

    pub fn set_volume(&self, volume: u8) {
        let volume = volume.min(100);
        self.shared
            .volume
            .store(level(volume).to_bits(), Ordering::Relaxed);
        self.shared
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .snapshot
            .volume = volume;
    }

    pub fn resume(&self) {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(mut job) = state.template.clone() {
            let position = if state.snapshot.finished {
                0
            } else {
                state.snapshot.position_ms
            };
            job.config = self.config.clone();
            job.known_tracks = state
                .snapshot
                .queue_complete
                .then(|| state.snapshot.tracks.as_ref().clone());
            select(&mut state, job);
            state.seek_revision += 1;
            state.seek = Some((state.seek_revision, position));
            state.snapshot.position_ms = position;
            self.shared.changed.notify_one();
        }
    }

    pub fn stop(&self) {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        state.cancel.store(true, Ordering::SeqCst);
        state.epoch += 1;
        state.restored = true;
        state.job = None;
        state.template = None;
        state.history.clear();
        state.request = None;
        state.seek = None;
        let volume = state.snapshot.volume;
        state.snapshot = Snapshot {
            volume,
            ..Snapshot::default()
        };
        self.shared.changed.notify_one();
    }

    pub fn suspend(&self) {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        state.cancel.store(true, Ordering::SeqCst);
        state.epoch += 1;
        state.job = None;
        state.request = None;
        state.snapshot.playing = false;
        state.snapshot.loading = false;
        self.shared.changed.notify_one();
    }

    pub fn configure(&mut self, config: EngineConfig) {
        self.set_volume(crate::app::volume_to_percent(config.initial_volume));
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(template) = &mut state.template {
            template.config = config.clone();
        }
        if let Some(job) = &mut state.job {
            job.config = config.clone();
        }
        self.config = config;
    }

    pub fn reconfigure(&mut self, config: EngineConfig) {
        let active = self.controls_output();
        let playing = self.snapshot().playing;
        self.configure(config);
        if active {
            self.resume();
            if !playing {
                self.toggle();
            }
        }
    }

    #[cfg(any(test, feature = "demo"))]
    pub fn demo_ready(&self, position_ms: u64, playing: bool) {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        state.snapshot.loading = false;
        state.snapshot.playing = playing;
        state.snapshot.position_ms = position_ms;
        state.active_epoch = Some(state.epoch);
    }
}

impl crate::app::App {
    pub(crate) fn jellyfin_now_playing(&self) -> Option<crate::app::NowPlaying> {
        if !self.jellyfin_player.controls_output() {
            return None;
        }
        let state = self.jellyfin_player.snapshot();
        let item = state.item?;
        let audio_format = item.audio_format();
        let artwork = self
            .jellyfin
            .source
            .as_ref()
            .filter(|source| Some(source.id.as_str()) == state.source_id.as_deref())
            .and_then(|source| super::artwork::Reference::for_item(source, &item))
            .map(|reference| reference.uri());
        Some(crate::app::NowPlaying {
            provider: Provider::Jellyfin,
            local: true,
            device_name: None,
            device_kind: None,
            uri: format!("jellyfin:{}:{}", state.source_id?, item.id),
            id: None,
            title: item.name.unwrap_or_else(|| "Untitled".into()),
            subtitle: item.artists.join(", "),
            artists: item
                .artists
                .into_iter()
                .map(|name| crate::api::models::ArtistRef {
                    id: None,
                    name,
                    uri: None,
                })
                .collect(),
            album_name: item.album.unwrap_or_default(),
            album_id: None,
            audio_format,
            show_id: None,
            art_url: artwork.clone(),
            art_small: artwork,
            duration_ms: u32::try_from(state.duration_ms).unwrap_or(u32::MAX),
            position_ms: u32::try_from(state.position_ms).unwrap_or(u32::MAX),
            playing: state.playing,
            loading: state.loading,
            shuffle: false,
            repeat: crate::player::RepeatMode::Off,
            volume_percent: state.volume,
            can_control: true,
            can_set_volume: true,
            is_episode: false,
            resuming: false,
        })
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().unwrap_or_else(|p| p.into_inner());
        state.shutdown = true;
        state.cancel.store(true, Ordering::SeqCst);
        self.shared.changed.notify_one();
    }
}

fn level(volume: u8) -> f64 {
    (f64::from(volume) / 100.0).powi(3)
}
struct Volume(Arc<AtomicU64>);
impl VolumeGetter for Volume {
    fn attenuation_factor(&self) -> f64 {
        f64::from_bits(self.0.load(Ordering::Relaxed))
    }
}

fn select(state: &mut State, job: Job) {
    state.restored = true;
    state.cancel.store(true, Ordering::SeqCst);
    state.cancel = Arc::new(AtomicBool::new(false));
    state.epoch += 1;
    state.request = Some(job.config.output.request());
    state.seek = None;
    let tracks = job
        .known_tracks
        .clone()
        .unwrap_or_else(|| vec![job.item.clone()]);
    let same_source = state
        .template
        .as_ref()
        .is_some_and(|previous| previous.source == job.source);
    let queued = if same_source {
        state.snapshot.queued.clone()
    } else {
        Arc::default()
    };
    if !same_source {
        state.history.clear();
    }
    let index = job
        .context_index
        .filter(|index| {
            job.manual
                || tracks.get(*index).is_some_and(|item| {
                    item.id == job.item.id
                        && item.details.playlist_item_id == job.item.details.playlist_item_id
                })
        })
        .or_else(|| {
            (!job.manual)
                .then(|| {
                    tracks.iter().position(|item| {
                        item.id == job.item.id
                            && item.details.playlist_item_id == job.item.details.playlist_item_id
                    })
                })
                .flatten()
        })
        .unwrap_or(if job.manual { state.snapshot.index } else { 0 });
    state.snapshot = Snapshot {
        source_id: Some(job.source.id.clone()),
        queue_complete: job.known_tracks.is_some(),
        item: Some(job.item.clone()),
        tracks: Arc::new(tracks),
        queued,
        manual_current: job.manual,
        index,
        duration_ms: job.item.duration_ms().unwrap_or(0),
        playing: true,
        loading: true,
        volume: state.snapshot.volume,
        ..Snapshot::default()
    };
    state.template = Some(job.clone());
    state.job = Some(job);
}

fn remember_current(state: &mut State) {
    if let Some(item) = state.snapshot.item.clone() {
        if state.history.len() == 1000 {
            state.history.remove(0);
        }
        state.history.push(Cursor {
            item,
            index: state.snapshot.index,
            manual: state.snapshot.manual_current,
        });
    }
}

fn select_cursor(state: &mut State, cursor: Cursor) {
    if let Some(mut job) = state.template.clone() {
        job.album = cursor.item.album_id.clone().unwrap_or(job.album);
        job.item = cursor.item;
        job.manual = cursor.manual;
        job.context_index = Some(cursor.index);
        job.known_tracks = Some(state.snapshot.tracks.as_ref().clone());
        select(state, job);
    }
}

fn advance(state: &mut State, forward: bool) {
    let target = if forward {
        if !state.snapshot.queued.is_empty() {
            Some(Cursor {
                item: Arc::make_mut(&mut state.snapshot.queued).remove(0),
                index: state.snapshot.index,
                manual: true,
            })
        } else {
            state.snapshot.index.checked_add(1).and_then(|index| {
                state
                    .snapshot
                    .tracks
                    .get(index)
                    .cloned()
                    .map(|item| Cursor {
                        item,
                        index,
                        manual: false,
                    })
            })
        }
    } else {
        state.history.pop().or_else(|| {
            state.snapshot.index.checked_sub(1).and_then(|index| {
                state
                    .snapshot
                    .tracks
                    .get(index)
                    .cloned()
                    .map(|item| Cursor {
                        item,
                        index,
                        manual: false,
                    })
            })
        })
    };
    if let Some(target) = target {
        if forward {
            remember_current(state);
        } else if state.snapshot.manual_current
            && let Some(item) = state.snapshot.item.clone()
        {
            Arc::make_mut(&mut state.snapshot.queued).insert(0, item);
        }
        select_cursor(state, target);
    } else if forward {
        state.cancel.store(true, Ordering::SeqCst);
        state.epoch += 1;
        state.job = None;
        state.snapshot.playing = false;
        state.snapshot.loading = false;
        state.snapshot.finished = true;
        state.snapshot.position_ms = state.snapshot.duration_ms;
    } else {
        state.seek_revision += 1;
        state.seek = Some((state.seek_revision, 0));
        state.snapshot.position_ms = 0;
    }
}

fn restore(dirs: &AppDirs, shared: &Shared, config: &EngineConfig) {
    use std::io::Read;
    let saved = (|| {
        let file = std::fs::File::open(dirs.state.join("jellyfin-playback.json")).ok()?;
        let mut bytes = Vec::new();
        file.take(SAVED_LIMIT + 1).read_to_end(&mut bytes).ok()?;
        if bytes.len() as u64 > SAVED_LIMIT {
            return None;
        }
        let saved: SavedPlayback =
            serde_json::from_slice::<Option<SavedPlayback>>(&bytes).ok()??;
        let source = crate::backend::jellyfin::saved_playback_source(dirs).ok()?;
        if saved.version != 1
            || saved.source != source
            || saved.tracks.is_empty()
            || saved.tracks.len() > 10000
            || saved.queued.len() > 1000
            || saved.history.len() > 1000
            || saved.index >= saved.tracks.len()
            || (!saved.manual && saved.tracks[saved.index].id != saved.item.id)
            || !saved
                .tracks
                .iter()
                .chain(&saved.queued)
                .chain(std::iter::once(&saved.item))
                .all(|item| super::validate_id(&item.id).is_ok())
            || !saved.history.iter().all(|cursor| {
                cursor.index < saved.tracks.len() && super::validate_id(&cursor.item.id).is_ok()
            })
        {
            return None;
        }
        Some(saved)
    })();
    let Some(saved) = saved else {
        return;
    };
    let mut state = shared.state.lock().unwrap_or_else(|p| p.into_inner());
    if state.epoch != 0 || state.template.is_some() {
        return;
    }
    state.template = Some(Job {
        source: saved.source.clone(),
        album: saved.album,
        query: saved.query,
        item: saved.item.clone(),
        known_tracks: saved.queue_complete.then(|| saved.tracks.clone()),
        config: config.clone(),
        manual: saved.manual,
        context_index: Some(saved.index),
    });
    state.history = saved.history;
    let volume = state.snapshot.volume;
    state.snapshot = Snapshot {
        source_id: Some(saved.source.id),
        item: Some(saved.item),
        tracks: Arc::new(saved.tracks),
        queued: Arc::new(saved.queued),
        index: saved.index,
        manual_current: saved.manual,
        position_ms: saved.position_ms,
        duration_ms: state
            .template
            .as_ref()
            .and_then(|job| job.item.duration_ms())
            .unwrap_or(0),
        finished: saved.finished,
        queue_complete: saved.queue_complete,
        volume,
        ..Snapshot::default()
    };
    shared.waker.wake();
}

fn run(dirs: AppDirs, shared: Arc<Shared>, initial_config: EngineConfig) {
    restore(&dirs, &shared, &initial_config);
    shared
        .state
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .restored = true;
    shared.waker.wake();
    loop {
        let (job, epoch, cancel) = {
            let mut state = shared.state.lock().unwrap_or_else(|p| p.into_inner());
            while state.job.is_none() && !state.shutdown {
                state = shared
                    .changed
                    .wait(state)
                    .unwrap_or_else(|p| p.into_inner());
            }
            if state.shutdown {
                return;
            }
            state.active_epoch = Some(state.epoch);
            (state.job.take().unwrap(), state.epoch, state.cancel.clone())
        };
        let result = session(&dirs, &shared, &job, epoch, cancel.clone());
        let mut state = shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.epoch != epoch || cancel.load(Ordering::SeqCst) {
            continue;
        }
        state.active_epoch = None;
        match result {
            Ok(true) if state.snapshot.playing => advance(&mut state, true),
            Ok(_) => {
                state.snapshot.playing = false;
                state.snapshot.loading = false;
            }
            Err(error) => {
                log::warn!("Jellyfin playback stopped: {error}");
                state.snapshot.error = Some(error);
                state.snapshot.playing = false;
                state.snapshot.loading = false;
            }
        }
        shared.waker.wake();
    }
}

fn session(
    dirs: &AppDirs,
    shared: &Shared,
    job: &Job,
    epoch: u64,
    cancel: Arc<AtomicBool>,
) -> Result<bool, String> {
    let (client, token) =
        crate::backend::jellyfin::saved_playback_session(dirs, &job.source, &job.config.proxy)?;
    let tracks = match &job.known_tracks {
        Some(tracks) => tracks.clone(),
        None => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| "Could not start album lookup.")?;
            runtime.block_on(async {
                let mut tracks = Vec::new();
                loop {
                    if cancel.load(Ordering::SeqCst) {
                        return Err("Playback cancelled.".to_owned());
                    }
                    let page = if let Some(query) = &job.query {
                        client
                            .query(&job.source.user_id, query, tracks.len() as u32, &token)
                            .await
                    } else {
                        client
                            .album_tracks(
                                &job.source.user_id,
                                &job.album,
                                tracks.len() as u32,
                                &token,
                            )
                            .await
                    }
                    .map_err(|error| error.to_string())?;
                    let done = page.items.is_empty()
                        || page.start_index + page.items.len() as u32 >= page.total_record_count;
                    tracks.extend(page.items);
                    if tracks.len() > 10000 {
                        return Err("This album exceeds the playback queue limit.".into());
                    }
                    if done {
                        return Ok(tracks);
                    }
                }
            })?
        }
    };
    {
        let mut state = shared.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.epoch != epoch {
            return Ok(false);
        }
        if !job.manual {
            state.snapshot.index = job
                .context_index
                .filter(|index| {
                    tracks.get(*index).is_some_and(|item| {
                        item.id == job.item.id
                            && item.details.playlist_item_id == job.item.details.playlist_item_id
                    })
                })
                .or_else(|| {
                    tracks.iter().position(|item| {
                        item.id == job.item.id
                            && item.details.playlist_item_id == job.item.details.playlist_item_id
                    })
                })
                .ok_or("Track is no longer available in this album.")?;
        }
        state.snapshot.tracks = Arc::new(tracks);
        state.snapshot.queue_complete = true;
    }
    let cache = crate::media::cache::Policy::new(
        job.config.audio_cache_dir.as_deref(),
        job.config.audio_cache_limit,
    )
    .http
    .map(|cache| {
        cache.bind(crate::media::cache::Identity {
            provider: "jellyfin".into(),
            source: format!("{}:{}", job.source.id, job.source.server_id),
            account: job.source.user_id.clone(),
            item: job.item.id.clone(),
            representation: "original".into(),
        })
    });
    let source = HttpRangeSource::cached(
        client
            .stream_request(&job.item.id, &token, None)
            .map_err(|error| error.to_string())?,
        &job.config.proxy,
        cancel.clone(),
        cache,
    )
    .map_err(|error| error.to_string())?;
    let transfer = source.metrics.clone();
    let file_length = source.length();
    let decoder = AudioDecoder::open(Box::new(source))?;
    let mut decoder = BufferedDecoder::new(decoder, cancel.clone())?;
    let control = AudioControl::new(job.config.buffer_ms);
    let inner = RodioSink::new(
        job.config.audio_device.clone(),
        Arc::new(|_| {}),
        Box::new(Volume(shared.volume.clone())),
        job.config.buffer_ms,
        control.clone(),
    );
    let mut sink = Tapped::new(
        Box::new(inner),
        control.clone(),
        job.config.tap.clone(),
        Box::new(Volume(shared.volume.clone())),
        false,
        job.config.eq.clone(),
        Arc::new(AtomicU64::new(1.0f64.to_bits())),
    )
    .with_output(job.config.output.clone(), Provider::Jellyfin, true);
    let result = play(
        shared,
        job,
        epoch,
        &cancel,
        &mut decoder,
        &control,
        &mut sink,
    );
    if let Err(error) = &result {
        let audio_stream = job.item.media_streams.as_deref().and_then(|streams| {
            streams
                .iter()
                .find(|stream| stream.kind.as_deref() == Some("Audio"))
        });
        let stats = transfer.snapshot();
        log::warn!(
            "Jellyfin item playback failed: item_id={} container={} codec={} sample_rate={} channels={} duration_ms={} file_bytes={} range_requests={} range_bytes={} cache_hits={} error={}",
            job.item.id,
            job.item.container.as_deref().unwrap_or("unknown"),
            audio_stream
                .and_then(|stream| stream.codec.as_deref())
                .unwrap_or("unknown"),
            audio_stream
                .and_then(|stream| stream.sample_rate)
                .unwrap_or(0),
            audio_stream.and_then(|stream| stream.channels).unwrap_or(0),
            job.item.duration_ms().unwrap_or(0),
            file_length,
            stats.requests,
            stats.bytes,
            stats.cache_hits,
            error,
        );
    }
    let stopped = sink.stop().map_err(|error| error.to_string());
    result.and_then(|ended| stopped.map(|()| ended))
}

fn play(
    shared: &Shared,
    job: &Job,
    epoch: u64,
    cancel: &AtomicBool,
    decoder: &mut BufferedDecoder,
    control: &AudioControl,
    sink: &mut Tapped,
) -> Result<bool, String> {
    let mut converter = Converter::new(None);
    let mut running = false;
    let mut pcm = Vec::new();
    let mut offset = 0;
    let mut base_ms = 0;
    let mut frames = 0u64;
    let mut notified = Instant::now();
    loop {
        let (playing, seek, request) = {
            let mut state = shared.state.lock().unwrap_or_else(|p| p.into_inner());
            if state.shutdown || state.epoch != epoch || cancel.load(Ordering::SeqCst) {
                return Ok(false);
            }
            state.snapshot.duration_ms =
                decoder.duration_ms().unwrap_or(state.snapshot.duration_ms);
            state.snapshot.approximate_seek = decoder.approximate_seeks();
            (
                state.snapshot.playing,
                state.seek.take(),
                state.request.unwrap(),
            )
        };
        if !job.config.output.is_current(request) {
            return Ok(false);
        }
        if let Some((revision, position)) = seek {
            control.interrupt();
            sink.stop().map_err(|error| error.to_string())?;
            running = false;
            decoder.seek_ms(position);
            control.track_changed();
            pcm.clear();
            offset = 0;
            base_ms = position;
            frames = 0;
            let mut state = shared.state.lock().unwrap_or_else(|p| p.into_inner());
            if state.epoch == epoch && state.seek_revision == revision {
                state.snapshot.position_ms = position;
            }
        }
        if !playing {
            if running {
                sink.stop().map_err(|error| error.to_string())?;
                running = false;
            }
            let state = shared.state.lock().unwrap_or_else(|p| p.into_inner());
            if state.epoch == epoch
                && !state.shutdown
                && !state.snapshot.playing
                && state.seek.is_none()
            {
                drop(
                    shared
                        .changed
                        .wait(state)
                        .unwrap_or_else(|p| p.into_inner()),
                );
            }
            continue;
        }
        if !running {
            sink.set_output_request(request);
            sink.start_checked().map_err(|error| error.to_string())?;
            running = true;
        }
        if !sink.owns_output() {
            return Ok(false);
        }
        if offset == pcm.len() {
            pcm = match decoder.poll()? {
                Readiness::End => return Ok(true),
                Readiness::Packet(packet) => packet,
                Readiness::Pending => {
                    let mut state = shared.state.lock().unwrap_or_else(|p| p.into_inner());
                    if state.epoch == epoch && state.snapshot.playing {
                        let changed = !state.snapshot.loading;
                        state.snapshot.loading = true;
                        if changed {
                            shared.waker.wake();
                        }
                        drop(
                            shared
                                .changed
                                .wait_timeout(state, Duration::from_millis(20))
                                .unwrap_or_else(|p| p.into_inner()),
                        );
                    }
                    continue;
                }
            };
            offset = 0;
            // Fetching a range may have blocked; recheck current intent before
            // writing even the first packet of the newly arrived block.
            continue;
        }
        let end = (offset + 882).min(pcm.len());
        sink.write(
            AudioPacket::Samples(pcm[offset..end].to_vec()),
            &mut converter,
        )
        .map_err(|error| error.to_string())?;
        frames += ((end - offset) / 2) as u64;
        offset = end;
        {
            let mut state = shared.state.lock().unwrap_or_else(|p| p.into_inner());
            if state.epoch == epoch && state.seek.is_none() && state.snapshot.playing {
                state.snapshot.position_ms =
                    (base_ms + frames * 1000 / 44100).min(state.snapshot.duration_ms);
                state.snapshot.loading = false;
            }
        }
        if notified.elapsed() >= Duration::from_millis(100) {
            shared.waker.wake();
            notified = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transport() -> Transport {
        let dirs = AppDirs {
            config: "unused-config".into(),
            state: "unused-state".into(),
            cache: "unused-cache".into(),
        };
        let config = crate::app::engine_config(
            &dirs,
            &crate::settings::Settings::default(),
            crate::settings::ProxyConfig::Off,
            crate::vis::AudioTap::new(),
            crate::eq::shared(),
            Arc::default(),
        );
        Transport::new(dirs, Waker::default(), config, false)
    }

    fn source() -> Source {
        Source {
            id: "0123456789abcdef0123456789abcdef".into(),
            server_url: "http://127.0.0.1:1/".into(),
            server_id: "server1".into(),
            user_id: "user1".into(),
            device_id: "device1".into(),
        }
    }

    fn item(id: &str) -> Item {
        serde_json::from_value(
            serde_json::json!({ "Id": id, "Name": id, "Artists": [], "RunTimeTicks": 10_000_000 }),
        )
        .unwrap()
    }

    #[test]
    fn refreshed_artwork_metadata_preserves_playback_and_rejects_other_accounts() {
        let transport = transport();
        transport.play(
            source(),
            "album".into(),
            item("a"),
            Some(vec![item("a"), item("b")]),
        );
        transport.enqueue(&source(), item("queued"));
        transport.seek(500);
        let before = transport.snapshot();
        let epoch = transport.epoch();
        let mut updated = item("a");
        updated.image_tags = Some(std::collections::HashMap::from([(
            "Primary".into(),
            Some("cover-v2".into()),
        )]));
        let mut foreign = source();
        foreign.user_id = "otheruser".into();
        assert!(!transport.refresh_metadata(&foreign, &[updated.clone()]));
        assert!(transport.refresh_metadata(&source(), &[updated.clone()]));
        assert!(!transport.refresh_metadata(&source(), &[updated.clone()]));
        let after = transport.snapshot();
        assert_eq!(after.item, Some(updated));
        assert_eq!(after.position_ms, before.position_ms);
        assert_eq!(after.playing, before.playing);
        assert_eq!(after.queued, before.queued);
        assert_eq!(after.tracks, before.tracks);
        assert_eq!(transport.epoch(), epoch);
        assert!(transport.controls_output());
    }

    #[test]
    fn playback_configuration_uses_the_current_shared_volume() {
        let mut transport = transport();
        transport.set_volume(70);
        let mut config = transport.config.clone();
        config.initial_volume = crate::app::percent_to_volume(20);
        transport.configure(config);
        assert_eq!(transport.snapshot().volume, 20);
        assert_eq!(
            f64::from_bits(transport.shared.volume.load(Ordering::Relaxed)),
            level(20)
        );
    }

    #[test]
    fn playlist_playback_keeps_occurrence_and_query_through_pause_and_save() {
        let player = transport();
        let current_source = source();
        let mut first = item("same");
        first.details.playlist_item_id = Some("first".into());
        let mut second = first.clone();
        second.details.playlist_item_id = Some("second".into());
        let query = super::super::catalog::Query::PlaylistSongs {
            playlist: "list1".into(),
        };
        player.play_catalog(
            current_source,
            "list1".into(),
            second.clone(),
            Some(query.clone()),
            Some(101),
        );
        assert_eq!(
            player.snapshot().index,
            0,
            "a loading placeholder cannot have an out-of-bounds index"
        );
        let state = player.shared.state.lock().unwrap();
        assert_eq!(state.template.as_ref().unwrap().query, Some(query));
        assert_eq!(state.template.as_ref().unwrap().context_index, Some(101));
        drop(state);
        // A known queue uses the occurrence identity even when song IDs repeat.
        player.play(
            source(),
            "list1".into(),
            second,
            Some(vec![first, item("other"), {
                let mut item = item("same");
                item.details.playlist_item_id = Some("second".into());
                item
            }]),
        );
        assert_eq!(player.snapshot().index, 2);
    }

    #[test]
    fn ordered_album_navigation_is_independent_and_optimistic() {
        let transport = transport();
        let tracks = vec![item("a"), item("b"), item("c")];
        transport.play(
            source(),
            "album".into(),
            tracks[1].clone(),
            Some(tracks.clone()),
        );
        assert_eq!(transport.snapshot().item.unwrap().id, "b");
        assert!(transport.snapshot().loading);
        let old_cancel = transport.shared.state.lock().unwrap().cancel.clone();
        transport.step(true);
        assert!(old_cancel.load(Ordering::SeqCst));
        assert_eq!(transport.snapshot().item.unwrap().id, "c");
        transport.step(false);
        assert_eq!(transport.snapshot().item.unwrap().id, "b");
        assert_eq!(transport.snapshot().tracks.as_ref(), &tracks);
    }

    #[test]
    fn stop_cancels_loading_and_discards_account_scoped_queue_state() {
        let transport = transport();
        transport.play(source(), "album".into(), item("a"), Some(vec![item("a")]));
        let cancel = transport.shared.state.lock().unwrap().cancel.clone();
        transport.stop();
        assert!(cancel.load(Ordering::SeqCst));
        assert!(transport.snapshot().item.is_none());
        assert!(transport.snapshot().tracks.is_empty());
        assert!(!transport.controls_output());
    }

    #[test]
    fn queued_duplicates_play_before_the_album_without_consuming_its_position() {
        let transport = transport();
        let tracks = vec![item("a"), item("b"), item("c")];
        transport.play(
            source(),
            "album".into(),
            tracks[0].clone(),
            Some(tracks.clone()),
        );
        transport.enqueue(&source(), item("manual"));
        transport.enqueue(&source(), item("manual"));
        transport.step(true);
        assert_eq!(transport.snapshot().item.unwrap().id, "manual");
        assert!(transport.snapshot().manual_current);
        assert_eq!(transport.snapshot().index, 0);
        assert_eq!(transport.snapshot().queued.len(), 1);
        transport.step(true);
        assert_eq!(transport.snapshot().item.unwrap().id, "manual");
        assert!(transport.snapshot().queued.is_empty());
        transport.step(true);
        assert_eq!(transport.snapshot().item.unwrap().id, "b");
        assert!(!transport.snapshot().manual_current);
        transport.step(true);
        assert_eq!(transport.snapshot().item.unwrap().id, "c");
        transport.step(true);
        assert!(transport.snapshot().finished);
        assert!(!transport.snapshot().playing);
        transport.toggle();
        assert_eq!(transport.snapshot().position_ms, 0);
        assert!(transport.snapshot().playing);
    }

    #[test]
    fn previous_replays_manual_history_and_restores_forward_album_order() {
        let transport = transport();
        transport.play(
            source(),
            "album".into(),
            item("a"),
            Some(vec![item("a"), item("b")]),
        );
        transport.enqueue(&source(), item("manual"));
        transport.step(true);
        transport.step(false);
        assert_eq!(transport.snapshot().item.unwrap().id, "a");
        assert_eq!(transport.snapshot().queued[0].id, "manual");
        transport.step(true);
        assert_eq!(transport.snapshot().item.unwrap().id, "manual");
        transport.step(true);
        assert_eq!(transport.snapshot().item.unwrap().id, "b");
        transport.step(false);
        assert_eq!(transport.snapshot().item.unwrap().id, "manual");
        transport.step(true);
        assert_eq!(transport.snapshot().item.unwrap().id, "b");
    }

    #[test]
    fn clear_keeps_current_audio_and_album_and_new_albums_keep_manual_additions() {
        let transport = transport();
        transport.play(
            source(),
            "album".into(),
            item("a"),
            Some(vec![item("a"), item("b")]),
        );
        transport.enqueue(&source(), item("manual1"));
        transport.enqueue(&source(), item("manual2"));
        transport.step(true);
        transport.clear_queue();
        assert_eq!(transport.snapshot().item.unwrap().id, "manual1");
        transport.step(true);
        assert_eq!(transport.snapshot().item.unwrap().id, "b");
        transport.enqueue(&source(), item("manual3"));
        transport.play(
            source(),
            "other-album".into(),
            item("x"),
            Some(vec![item("x"), item("y")]),
        );
        transport.step(true);
        assert_eq!(transport.snapshot().item.unwrap().id, "manual3");
        transport.step(true);
        assert_eq!(transport.snapshot().item.unwrap().id, "y");
    }

    #[test]
    fn queue_row_selection_skips_only_the_rows_above_it_and_account_changes_clear_it() {
        let transport = transport();
        transport.play(
            source(),
            "album".into(),
            item("a"),
            Some(vec![item("a"), item("b")]),
        );
        for id in ["m1", "m2", "m3"] {
            transport.enqueue(&source(), item(id));
        }
        transport.play_queued(1);
        assert_eq!(transport.snapshot().item.unwrap().id, "m2");
        assert_eq!(transport.snapshot().queued.len(), 1);
        assert_eq!(transport.snapshot().queued[0].id, "m3");
        transport.step(true);
        assert_eq!(transport.snapshot().item.unwrap().id, "m3");
        transport.step(true);
        assert_eq!(transport.snapshot().item.unwrap().id, "b");
        let mut other = source();
        other.user_id = "other-user".into();
        transport.enqueue(&other, item("wrong-user"));
        assert!(transport.snapshot().queued.is_empty());
        assert!(transport.snapshot().error.is_some());
        transport.enqueue(&source(), item("m4"));
        transport.play(
            other,
            "new-account".into(),
            item("z"),
            Some(vec![item("z")]),
        );
        assert!(transport.snapshot().queued.is_empty());
    }

    #[test]
    fn selecting_an_album_queue_row_skips_manual_rows_and_reconfiguration_keeps_pause_position() {
        let mut transport = transport();
        transport.play(
            source(),
            "album".into(),
            item("a"),
            Some(vec![item("a"), item("b"), item("c")]),
        );
        transport.enqueue(&source(), item("manual"));
        transport.play_index(2);
        assert_eq!(transport.snapshot().item.unwrap().id, "c");
        assert!(transport.snapshot().queued.is_empty());
        transport.seek(500);
        transport.toggle();
        let mut config = transport.config.clone();
        config.buffer_ms = 200;
        transport.reconfigure(config);
        assert!(!transport.snapshot().playing);
        assert_eq!(transport.snapshot().position_ms, 500);
        let state = transport.shared.state.lock().unwrap();
        assert_eq!(state.job.as_ref().unwrap().config.buffer_ms, 200);
    }

    #[test]
    fn latest_seek_wins_and_resuming_an_ended_worker_queues_a_new_session() {
        let transport = transport();
        transport.play(source(), "album".into(), item("a"), Some(vec![item("a")]));
        transport.seek(100);
        transport.seek(500);
        assert_eq!(transport.snapshot().position_ms, 500);
        {
            let mut state = transport.shared.state.lock().unwrap();
            assert_eq!(state.seek.unwrap().1, 500);
            state.job = None;
            state.snapshot.playing = false;
        }
        transport.toggle();
        let state = transport.shared.state.lock().unwrap();
        assert!(state.job.is_some());
        assert!(state.snapshot.playing);
        assert_eq!(state.seek.unwrap().1, 500);
    }

    #[test]
    fn saved_queue_restores_paused_with_duplicates_and_the_original_album_position() {
        let original = transport();
        original.play(
            source(),
            "album".into(),
            item("a"),
            Some(vec![item("a"), item("b")]),
        );
        original.enqueue(&source(), item("manual"));
        original.enqueue(&source(), item("manual"));
        original.step(true);
        original.seek(400);
        original.toggle();
        let root = std::env::current_dir()
            .unwrap()
            .join("target/jellyfin-queue-restore")
            .join(format!("{:032x}", rand::random::<u128>()));
        std::fs::create_dir_all(&root).unwrap();
        let dirs = AppDirs {
            state: root.clone(),
            cache: root.clone(),
            config: root.clone(),
        };
        std::fs::write(
            root.join("jellyfin-account.json"),
            serde_json::to_vec(&source()).unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join("jellyfin-playback.json"),
            original.encode_session().unwrap().unwrap(),
        )
        .unwrap();
        let resumed = transport();
        restore(&dirs, &resumed.shared, &resumed.config);
        let snapshot = resumed.snapshot();
        assert!(!snapshot.playing);
        assert!(!resumed.controls_output());
        assert_eq!(snapshot.position_ms, 400);
        assert_eq!(snapshot.item.unwrap().id, "manual");
        assert!(snapshot.manual_current);
        assert_eq!(snapshot.index, 0);
        assert_eq!(snapshot.queued.len(), 1);
        resumed.resume();
        resumed.step(true);
        assert_eq!(resumed.snapshot().item.unwrap().id, "manual");
        resumed.step(true);
        assert_eq!(resumed.snapshot().item.unwrap().id, "b");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn queue_restore_cannot_cross_accounts_or_override_newer_intent() {
        let original = transport();
        original.play(source(), "album".into(), item("a"), Some(vec![item("a")]));
        let root = std::env::current_dir()
            .unwrap()
            .join("target/jellyfin-queue-restore")
            .join(format!("{:032x}", rand::random::<u128>()));
        std::fs::create_dir_all(&root).unwrap();
        let dirs = AppDirs {
            state: root.clone(),
            cache: root.clone(),
            config: root.clone(),
        };
        std::fs::write(
            root.join("jellyfin-playback.json"),
            original.encode_session().unwrap().unwrap(),
        )
        .unwrap();
        let mut other = source();
        other.user_id = "another-user".into();
        std::fs::write(
            root.join("jellyfin-account.json"),
            serde_json::to_vec(&other).unwrap(),
        )
        .unwrap();
        let resumed = transport();
        restore(&dirs, &resumed.shared, &resumed.config);
        assert!(resumed.snapshot().item.is_none());
        std::fs::write(
            root.join("jellyfin-account.json"),
            serde_json::to_vec(&source()).unwrap(),
        )
        .unwrap();
        std::fs::write(root.join("jellyfin-revoked"), &source().id).unwrap();
        restore(&dirs, &resumed.shared, &resumed.config);
        assert!(resumed.snapshot().item.is_none());
        std::fs::remove_file(root.join("jellyfin-revoked")).unwrap();
        resumed.play(
            source(),
            "album".into(),
            item("newer"),
            Some(vec![item("newer")]),
        );
        restore(&dirs, &resumed.shared, &resumed.config);
        assert_eq!(resumed.snapshot().item.unwrap().id, "newer");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pending_restoration_cannot_erase_saved_state_and_sign_out_encodes_an_empty_session() {
        let transport = transport();
        transport.shared.state.lock().unwrap().restored = false;
        assert!(transport.encode_session().unwrap().is_none());
        transport.play(source(), "album".into(), item("a"), Some(vec![item("a")]));
        assert!(transport.encode_session().unwrap().unwrap().len() > 4);
        transport.stop();
        assert_eq!(transport.encode_session().unwrap().unwrap(), b"null");
    }

    struct SeekAfterFirstWrite {
        shared: Arc<Shared>,
        sizes: Arc<Mutex<Vec<usize>>>,
    }

    impl Sink for SeekAfterFirstWrite {
        fn start(&mut self) -> librespot_playback::audio_backend::SinkResult<()> {
            Ok(())
        }
        fn stop(&mut self) -> librespot_playback::audio_backend::SinkResult<()> {
            Ok(())
        }
        fn write(
            &mut self,
            packet: AudioPacket,
            _: &mut Converter,
        ) -> librespot_playback::audio_backend::SinkResult<()> {
            let mut sizes = self.sizes.lock().unwrap();
            sizes.push(packet.samples().unwrap().len());
            if sizes.len() == 1 {
                let mut state = self.shared.state.lock().unwrap();
                state.seek_revision += 1;
                state.seek = Some((state.seek_revision, 500));
                state.snapshot.position_ms = 500;
            }
            Ok(())
        }
    }

    #[test]
    fn manual_row_edits_preserve_occurrences_and_reject_stale_or_other_account_actions() {
        let transport = transport();
        let source = source();
        transport.play(
            source.clone(),
            "album".into(),
            item("a"),
            Some(vec![item("a"), item("b")]),
        );
        for id in ["x", "x", "y"] {
            transport.enqueue(&source, item(id));
        }
        let expected = transport.snapshot().queued;
        let mut other = source.clone();
        other.user_id.push('x');
        transport.edit_manual(&other, &expected, 0, None);
        assert_eq!(transport.snapshot().queued.len(), 3);
        transport.edit_manual(&source, &expected, 0, Some(2));
        assert_eq!(
            transport
                .snapshot()
                .queued
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["x", "y", "x"]
        );
        transport.edit_manual(&source, &expected, 0, None);
        assert_eq!(
            transport.snapshot().queued.len(),
            3,
            "stale menu cannot remove a different occurrence"
        );
        let expected = transport.snapshot().queued;
        transport.edit_manual(&source, &expected, 0, None);
        assert_eq!(
            transport
                .snapshot()
                .queued
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["y", "x"]
        );
        assert_eq!(transport.snapshot().tracks.len(), 2);
        assert_eq!(transport.snapshot().item.unwrap().id, "a");
    }

    #[test]
    fn manual_batch_removal_is_atomic_and_rejects_stale_or_other_account_snapshots() {
        let transport = transport();
        let source = source();
        transport.play(
            source.clone(),
            "album".into(),
            item("a"),
            Some(vec![item("a"), item("b")]),
        );
        for id in ["x", "x", "y"] {
            transport.enqueue(&source, item(id));
        }
        let expected = transport.snapshot().queued;
        let mut other = source.clone();
        other.user_id.push('x');
        transport.remove_manual(&other, &expected, &[0, 2]);
        transport.remove_manual(&source, &expected, &[0, 99]);
        assert_eq!(transport.snapshot().queued.len(), 3);
        transport.remove_manual(&source, &expected, &[2, 0, 2]);
        assert_eq!(
            transport
                .snapshot()
                .queued
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["x"]
        );
        transport.remove_manual(&source, &expected, &[0]);
        assert_eq!(
            transport.snapshot().queued.len(),
            1,
            "a stale selection cannot delete the remaining duplicate"
        );
        assert_eq!(transport.snapshot().item.unwrap().id, "a");
        assert_eq!(transport.snapshot().tracks.len(), 2);
    }

    #[test]
    fn transport_seek_discards_unwritten_old_pcm_and_keeps_output_chunks_bounded() {
        let transport = transport();
        transport.play(source(), "album".into(), item("a"), Some(vec![item("a")]));
        let (job, epoch, cancel) = {
            let state = transport.shared.state.lock().unwrap();
            (
                state.job.clone().unwrap(),
                state.epoch,
                state.cancel.clone(),
            )
        };
        let decoder = AudioDecoder::open(Box::new(std::io::Cursor::new(
            include_bytes!("../testdata/jellyfin-tone.wav").to_vec(),
        )))
        .unwrap();
        let mut decoder = BufferedDecoder::new(decoder, cancel.clone()).unwrap();
        let sizes = Arc::new(Mutex::new(Vec::new()));
        let control = AudioControl::new(100);
        let mut sink = Tapped::new(
            Box::new(SeekAfterFirstWrite {
                shared: transport.shared.clone(),
                sizes: sizes.clone(),
            }),
            control.clone(),
            job.config.tap.clone(),
            Box::new(Volume(transport.shared.volume.clone())),
            false,
            job.config.eq.clone(),
            Arc::new(AtomicU64::new(1.0f64.to_bits())),
        )
        .with_output(job.config.output.clone(), Provider::Jellyfin, true);
        assert!(
            play(
                &transport.shared,
                &job,
                epoch,
                &cancel,
                &mut decoder,
                &control,
                &mut sink
            )
            .unwrap()
        );
        let sizes = sizes.lock().unwrap();
        assert!(sizes.iter().all(|size| *size <= 882));
        assert_eq!(sizes.iter().sum::<usize>(), 882 + 44100);
        assert_eq!(transport.snapshot().position_ms, 1000);
    }
}
