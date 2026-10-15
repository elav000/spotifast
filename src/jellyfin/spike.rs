//! A standalone playback experiment, not the app playback coordinator.
use super::{
    Client, Token,
    decode::AudioDecoder,
    range::{HttpRangeSource, Snapshot},
};
use crate::{
    eq,
    settings::ProxyConfig,
    sink::{AudioControl, RodioSink},
    vis::{AudioTap, Tapped},
};
use librespot_playback::{
    audio_backend::Sink, convert::Converter, decoder::AudioPacket, mixer::VolumeGetter,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64},
};
use std::time::{Duration, Instant};

pub struct Report {
    pub title: String,
    pub container: Option<String>,
    pub codec: Option<String>,
    pub sample_rate: u32,
    pub channels: usize,
    pub first_pcm_ms: u128,
    pub seek_ms: u128,
    pub approximate_seek: bool,
    pub duration_ms: u64,
    pub file_bytes: u64,
    pub bytes_at_first_pcm: u64,
    pub transfer: Snapshot,
    pub visualizer_received_samples: bool,
}

struct Volume(f64);
impl VolumeGetter for Volume {
    fn attenuation_factor(&self) -> f64 {
        self.0
    }
}

pub(crate) fn run_for_user(
    client: Client,
    token: Token,
    library_name: Option<&str>,
    proxy: &ProxyConfig,
    audible: bool,
    user: &str,
    container: Option<&str>,
) -> Result<Report, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "Could not start the probe runtime.")?;
    let item = runtime.block_on(async {
        let libraries = client
            .music_libraries(user, &token)
            .await
            .map_err(|error| error.to_string())?;
        let library = libraries
            .iter()
            .find(|library| library_name.is_none_or(|name| library.name.eq_ignore_ascii_case(name)))
            .ok_or("No matching music library.")?;
        if let Some(container) = container {
            for start in (0..500).step_by(100) {
                let page = client
                    .audio_tracks(user, &library.id, start, &token)
                    .await
                    .map_err(|error| error.to_string())?;
                let complete = page.start_index + page.items.len() as u32
                    >= page.total_record_count
                    || page.items.is_empty();
                if let Some(item) = page.items.into_iter().find(|item| {
                    item.container
                        .as_deref()
                        .is_some_and(|kind| kind.eq_ignore_ascii_case(container))
                }) {
                    return Ok(item);
                }
                if complete {
                    break;
                }
            }
            return Err("No matching format in the first 500 tracks of this library.".into());
        }
        let albums = client
            .albums(user, &library.id, 0, &token)
            .await
            .map_err(|error| error.to_string())?;
        let album = albums
            .items
            .first()
            .ok_or("The music library has no albums.")?;
        let tracks = client
            .album_tracks(user, &album.id, 0, &token)
            .await
            .map_err(|error| error.to_string())?;
        tracks
            .items
            .into_iter()
            .next()
            .ok_or_else(|| "The first album has no tracks.".to_owned())
    })?;
    drop(runtime);
    let started = Instant::now();
    let source = HttpRangeSource::new(
        client
            .stream_request(&item.id, &token, None)
            .map_err(|error| error.to_string())?,
        proxy,
        Arc::new(AtomicBool::new(false)),
    )
    .map_err(|error| error.to_string())?;
    let metrics = source.metrics.clone();
    let file_bytes = source.length();
    let mut decoder = AudioDecoder::open(Box::new(source))?;
    let first = decoder.next_pcm()?.ok_or("Audio is empty.")?;
    let first_pcm_ms = started.elapsed().as_millis();
    let bytes_at_first_pcm = metrics.snapshot().bytes;
    let duration_ms = decoder
        .duration_ms()
        .or_else(|| item.duration_ms())
        .ok_or("Audio duration is unavailable for this seek probe.")?;
    let approximate_seek = decoder.approximate_seeks();
    let rate = decoder.sample_rate();
    let channels = decoder.channels();
    let control = AudioControl::new(100);
    let tap = AudioTap::new();
    let equalizer = eq::shared();
    if let Ok(mut settings) = equalizer.lock() {
        settings.on = true;
        settings.preamp_db = -6.0;
    }
    let inner: Box<dyn Sink> = if audible {
        Box::new(RodioSink::new(
            None,
            Arc::new(|_| {}),
            Box::new(Volume(0.15)),
            100,
            control.clone(),
        ))
    } else {
        Box::new(Discard)
    };
    let mut sink = Tapped::new(
        inner,
        control.clone(),
        tap.clone(),
        Box::new(Volume(0.15)),
        false,
        equalizer,
        Arc::new(AtomicU64::new(1.0f64.to_bits())),
    );
    let mut converter = Converter::new(None);
    sink.start().map_err(|error| error.to_string())?;
    let playback = (|| {
        let mut seen = segment(&mut decoder, Some(first), &mut sink, &mut converter, &tap)?;
        sink.stop().map_err(|error| error.to_string())?;
        if audible {
            std::thread::sleep(Duration::from_millis(300));
        }
        control.interrupt();
        let seek = Instant::now();
        decoder.seek_ms(duration_ms * 9 / 10)?;
        let pcm = decoder.next_pcm()?.ok_or("Seek produced no audio.")?;
        let seek_ms = seek.elapsed().as_millis();
        control.track_changed();
        sink.start().map_err(|error| error.to_string())?;
        seen |= segment(&mut decoder, Some(pcm), &mut sink, &mut converter, &tap)?;
        Ok::<_, String>((seen, seek_ms))
    })();
    let stopped = sink.stop().map_err(|error| error.to_string());
    let (seen, seek_ms) = playback?;
    stopped?;
    let codec = item
        .media_streams
        .as_ref()
        .and_then(|streams| {
            streams
                .iter()
                .find(|stream| stream.kind.as_deref() == Some("Audio"))
        })
        .and_then(|stream| stream.codec.clone());
    Ok(Report {
        title: item.name.unwrap_or_else(|| "Untitled".into()),
        container: item.container,
        codec,
        sample_rate: rate,
        channels,
        first_pcm_ms,
        seek_ms,
        approximate_seek,
        duration_ms,
        file_bytes,
        bytes_at_first_pcm,
        transfer: metrics.snapshot(),
        visualizer_received_samples: seen,
    })
}

fn segment(
    decoder: &mut AudioDecoder,
    mut pcm: Option<Vec<f64>>,
    sink: &mut Tapped,
    converter: &mut Converter,
    tap: &AudioTap,
) -> Result<bool, String> {
    let mut frames = 0;
    let mut seen = false;
    loop {
        let next = match pcm.take() {
            Some(samples) => Some(samples),
            None => decoder.next_pcm()?,
        };
        let Some(samples) = next else {
            break;
        };
        // Ten-millisecond chunks retain the existing sink's short bounded
        // queue, rather than queuing twelve potentially large FLAC packets.
        for chunk in samples.chunks(882) {
            sink.write(AudioPacket::Samples(chunk.to_vec()), converter)
                .map_err(|error| error.to_string())?;
            frames += chunk.len() / 2;
            seen |= tap
                .window(512, 0)
                .iter()
                .any(|sample| sample.abs() > 0.00001);
            if frames >= 44100 * 3 {
                return Ok(seen);
            }
        }
    }
    Ok(seen)
}

struct Discard;
impl Sink for Discard {
    fn start(&mut self) -> librespot_playback::audio_backend::SinkResult<()> {
        Ok(())
    }
    fn stop(&mut self) -> librespot_playback::audio_backend::SinkResult<()> {
        Ok(())
    }
    fn write(
        &mut self,
        _: AudioPacket,
        _: &mut Converter,
    ) -> librespot_playback::audio_backend::SinkResult<()> {
        Ok(())
    }
}
