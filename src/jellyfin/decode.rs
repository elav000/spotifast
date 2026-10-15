//! Original FLAC, MP3 and PCM/WAV decoding into the existing stereo 44.1 kHz
//! DSP path. No network or decoding work may run on the render thread.
use crate::resample::Resampler;
use symphonia::core::{
    audio::SampleBuffer,
    codecs::{Decoder, DecoderOptions},
    errors::Error,
    formats::{FormatOptions, FormatReader, SeekMode, SeekTo},
    io::{MediaSource, MediaSourceStream},
    meta::{Limit, MetadataOptions},
    probe::Hint,
};

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
            return Err("This playback spike supports mono/stereo audio from 8 to 192 kHz.".into());
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

    pub fn seek_ms(&mut self, milliseconds: u64) -> Result<(), String> {
        let timestamp = milliseconds.saturating_mul(u64::from(self.sample_rate)) / 1000;
        let seeked = self
            .format
            .seek(
                SeekMode::Accurate,
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
            let decoded = self
                .decoder
                .decode(&packet)
                .map_err(|_| "Audio decoding failed.".to_owned())?;
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
