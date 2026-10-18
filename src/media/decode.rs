//! Original FLAC, MP3 and PCM/WAV decoding into the existing stereo 44.1 kHz
//! DSP path. No network or decoding work may run on the render thread.
use crate::resample::Resampler;
use symphonia::core::{
    audio::SampleBuffer,
    codecs::{CODEC_TYPE_MP3, Decoder, DecoderOptions},
    errors::Error,
    formats::{FormatOptions, FormatReader, SeekMode, SeekTo},
    io::{MediaSource, MediaSourceStream},
    meta::{Limit, MetadataOptions},
    probe::Hint,
};

const MAX_STARTUP_DECODE_ERRORS: u16 = 64;

pub struct AudioDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track: u32,
    sample_rate: u32,
    channels: usize,
    frames: Option<u64>,
    resampler: Option<Resampler>,
    discard_before: u64,
    input_frames: u64,
    output_frames: u64,
    finished: bool,
    mp3: bool,
    startup_recovery: bool,
    startup_decode_errors: u16,
}

impl AudioDecoder {
    pub fn open(source: Box<dyn MediaSource>) -> Result<Self, String> {
        let stream = MediaSourceStream::new(source, Default::default());
        // Probe bytes, not the URL extension or a possibly stale catalog hint.
        let probed = symphonia::default::get_probe()
            .format(
                &Hint::new(),
                stream,
                &FormatOptions::default(),
                &MetadataOptions {
                    limit_metadata_bytes: Limit::Maximum(64 * 1024),
                    limit_visual_bytes: Limit::Maximum(0),
                },
            )
            .map_err(|_| "Unsupported or unreadable audio format.".to_owned())?;
        let format = probed.format;
        let track = format.default_track().ok_or("No audio track found.")?;
        let parameters = track.codec_params.clone();
        let id = track.id;
        let rate = parameters
            .sample_rate
            .ok_or("Audio sample rate is missing.")?;
        let channels = parameters
            .channels
            .ok_or("Audio channels are missing.")?
            .count();
        if !(8000..=192000).contains(&rate) || !(1..=2).contains(&channels) {
            return Err("Audio playback supports mono/stereo audio from 8 to 192 kHz.".into());
        }
        let decoder = symphonia::default::get_codecs()
            .make(&parameters, &DecoderOptions::default())
            .map_err(|_| "Unsupported audio codec.".to_owned())?;
        Ok(Self {
            format,
            decoder,
            track: id,
            sample_rate: rate,
            channels,
            frames: parameters.n_frames,
            resampler: Resampler::new(rate, 44100, 2),
            discard_before: 0,
            input_frames: 0,
            output_frames: 0,
            finished: false,
            mp3: parameters.codec == CODEC_TYPE_MP3,
            startup_recovery: true,
            startup_decode_errors: 0,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    pub fn channels(&self) -> usize {
        self.channels
    }
    pub fn duration_ms(&self) -> Option<u64> {
        self.frames
            .map(|frames| frames.saturating_mul(1000) / u64::from(self.sample_rate))
    }

    /// MP3 uses the demuxer's coarse seek to avoid scanning the whole file
    /// over HTTP. Its position is approximate; WAV and FLAC remain accurate.
    pub fn approximate_seeks(&self) -> bool {
        self.mp3
    }

    pub fn seek_ms(&mut self, milliseconds: u64) -> Result<(), String> {
        let timestamp = milliseconds.saturating_mul(u64::from(self.sample_rate)) / 1000;
        let seeked = self
            .format
            .seek(
                if self.mp3 {
                    SeekMode::Coarse
                } else {
                    SeekMode::Accurate
                },
                SeekTo::TimeStamp {
                    ts: timestamp,
                    track_id: self.track,
                },
            )
            .map_err(|_| "Could not seek this audio stream.".to_owned())?;
        self.discard_before = seeked.required_ts;
        self.decoder.reset();
        self.input_frames = 0;
        self.output_frames = 0;
        self.finished = false;
        self.resampler = Resampler::new(self.sample_rate, 44100, 2);
        self.startup_recovery = false;
        Ok(())
    }

    pub fn next_pcm(&mut self) -> Result<Option<Vec<f64>>, String> {
        if self.finished {
            return Ok(None);
        }
        loop {
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                Err(Error::IoError(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                    self.finished = true;
                    if let Some(mut resampler) = self.resampler.take() {
                        // Drain the existing sinc filter without adding extra
                        // playback time or clipping the final real samples.
                        let remaining = self.input_frames * 44100 / u64::from(self.sample_rate)
                            - self.output_frames;
                        let tail = resampler
                            .process(&[0.0; 128])
                            .into_iter()
                            .take(remaining as usize * 2)
                            .map(f64::from)
                            .collect::<Vec<_>>();
                        if !tail.is_empty() {
                            return Ok(Some(tail));
                        }
                    }
                    return Ok(None);
                }
                Err(_) => return Err(
                    "Audio input was interrupted or is invalid. Restart from the last position."
                        .into(),
                ),
            };
            if packet.track_id() != self.track {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    if self.startup_decode_errors > 0 {
                        log::warn!(
                            "MP3 startup recovered after skipping {} undecodable leading packets",
                            self.startup_decode_errors
                        );
                        self.startup_decode_errors = 0;
                    }
                    self.startup_recovery = false;
                    decoded
                }
                Err(error)
                    if self.mp3
                        && self.startup_recovery
                        && self.startup_decode_errors < MAX_STARTUP_DECODE_ERRORS =>
                {
                    if self.startup_decode_errors == 0 {
                        log::warn!(
                            "skipping undecodable leading MP3 packet: track={} timestamp={} duration={} error={error:?}",
                            self.track,
                            packet.ts(),
                            packet.dur(),
                        );
                    }
                    self.startup_decode_errors += 1;
                    continue;
                }
                Err(error) => {
                    // Keep the UI message stable, but retain the decoder's
                    // safe local error for playback reports.
                    log::warn!(
                        "audio packet decode failed: track={} timestamp={} duration={} error={error:?}",
                        self.track,
                        packet.ts(),
                        packet.dur(),
                    );
                    return Err("Audio decoding failed.".to_owned());
                }
            };
            if decoded.spec().rate != self.sample_rate
                || decoded.spec().channels.count() != self.channels
            {
                return Err("Audio format changed during playback.".into());
            }
            // FLAC blocks are bounded by the format, but also reject unusual
            // packets that would otherwise queue many seconds at once.
            if decoded.frames() > self.sample_rate as usize * 8 {
                return Err("Audio block exceeds the playback buffer limit.".into());
            }
            let mut buffer = SampleBuffer::<f64>::new(decoded.capacity() as u64, *decoded.spec());
            buffer.copy_interleaved_ref(decoded);
            let skip =
                self.discard_before
                    .saturating_sub(packet.ts())
                    .min((buffer.samples().len() / self.channels) as u64) as usize;
            let samples = &buffer.samples()[skip * self.channels..];
            if samples.is_empty() {
                continue;
            }
            self.discard_before = 0;
            self.input_frames += (samples.len() / self.channels) as u64;
            let stereo = match self.channels {
                1 => samples
                    .iter()
                    .flat_map(|sample| [*sample, *sample])
                    .collect(),
                _ => samples.to_vec(),
            };
            let pcm = match &mut self.resampler {
                Some(resampler) => resampler
                    .process(
                        &stereo
                            .iter()
                            .map(|sample| *sample as f32)
                            .collect::<Vec<_>>(),
                    )
                    .into_iter()
                    .map(f64::from)
                    .collect(),
                None => stereo,
            };
            self.output_frames += (pcm.len() / 2) as u64;
            if pcm.is_empty() {
                continue;
            }
            return Ok(Some(pcm));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AudioDecoder;
    use std::io::Cursor;

    #[test]
    fn skips_bounded_undecodable_mp3_leader_and_recovers() {
        let source = include_bytes!("../testdata/jellyfin-tone.mp3");
        let tag_size = source[6..10]
            .iter()
            .fold(0usize, |size, byte| (size << 7) | usize::from(byte & 0x7f));
        let frame_start = 10 + tag_size;
        // The fixture is MPEG-1 Layer III, 128 kbps at 44.1 kHz, without
        // padding: 144 * 128,000 / 44,100 gives a 417-byte frame.
        let frame_end = frame_start + 417;
        let mut invalid_frame = source[frame_start..frame_end].to_vec();
        let side_info = &mut invalid_frame[4..36];
        side_info.fill(0);
        // Make each of the four granule/channel payload lengths exceed the
        // frame's available main data, as in a damaged leading MP3 packet.
        for channel in 0..4 {
            for bit in 0..12 {
                let offset = 20 + channel * 59 + bit;
                side_info[offset / 8] |= 1 << (7 - offset % 8);
            }
        }

        let mut bytes = source[..frame_start].to_vec();
        for _ in 0..4 {
            bytes.extend_from_slice(&invalid_frame);
        }
        bytes.extend_from_slice(&source[frame_end..]);

        let mut decoder = AudioDecoder::open(Box::new(Cursor::new(bytes))).unwrap();
        let pcm = decoder
            .next_pcm()
            .expect("decoder should recover after the bad leading packets")
            .expect("valid audio follows the leading packets");
        assert!(!pcm.is_empty());
    }
}
