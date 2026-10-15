//! Bounded, seekable original-audio input. Use only on a decoder worker.
use crate::settings::ProxyConfig;
use reqwest::{Request, Url, header};
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;
use symphonia::core::io::MediaSource;

pub const RANGE_BYTES: usize = 256 * 1024;

#[derive(Default)]
pub struct Metrics {
    requests: AtomicU64,
    bytes: AtomicU64,
    peak: AtomicU64,
}

#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub requests: u64,
    pub bytes: u64,
    pub peak_range_bytes: u64,
}

impl Metrics {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            requests: self.requests.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
            peak_range_bytes: self.peak.load(Ordering::Relaxed),
        }
    }
}

/// One validated HTTP range is retained. A failed range never replaces the
/// previous buffer or advances the cursor. No credentials enter error strings.
pub struct HttpRangeSource {
    http: reqwest::blocking::Client,
    url: Url,
    headers: header::HeaderMap,
    buffer: Vec<u8>,
    buffer_start: u64,
    position: u64,
    length: u64,
    validator: Option<(header::HeaderName, header::HeaderValue)>,
    pub metrics: Arc<Metrics>,
    cancel: Arc<AtomicBool>,
}

impl HttpRangeSource {
    pub fn new(request: Request, proxy: &ProxyConfig, cancel: Arc<AtomicBool>) -> io::Result<Self> {
        let http = crate::http::blocking_builder(proxy)
            .map_err(|_| failure("Invalid proxy configuration."))?
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_gzip()
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|_| failure("Could not configure audio transport."))?;
        let mut source = Self {
            http,
            url: request.url().clone(),
            headers: request.headers().clone(),
            buffer: Vec::new(),
            buffer_start: 0,
            position: 0,
            length: 0,
            validator: None,
            metrics: Arc::default(),
            cancel,
        };
        source.headers.remove(header::RANGE);
        source.headers.remove(header::IF_RANGE);
        source.fetch(0, true)?;
        Ok(source)
    }

    pub fn length(&self) -> u64 {
        self.length
    }

    fn check_cancel(&self) -> io::Result<()> {
        if self.cancel.load(Ordering::SeqCst) {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Audio request cancelled.",
            ))
        } else {
            Ok(())
        }
    }

    fn fetch(&mut self, start: u64, initial: bool) -> io::Result<()> {
        self.check_cancel()?;
        let end = start.saturating_add(RANGE_BYTES as u64 - 1);
        let mut request = self
            .http
            .get(self.url.clone())
            .headers(self.headers.clone())
            .header(header::ACCEPT_ENCODING, "identity")
            .header(header::RANGE, format!("bytes={start}-{end}"));
        if let Some((_, value)) = &self.validator {
            request = request.header(header::IF_RANGE, value);
        }
        self.metrics.requests.fetch_add(1, Ordering::Relaxed);
        let mut response = request
            .send()
            .map_err(|_| failure("Jellyfin audio request failed."))?;
        // Reject 200 even when the server ignores Range: do not buffer a whole
        // file, or silently splice a changed file into an existing decoder.
        if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(failure(&format!(
                "Jellyfin audio needs HTTP range support (HTTP {}).",
                response.status().as_u16()
            )));
        }
        let range = response
            .headers()
            .get(header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| failure("Missing audio Content-Range."))?;
        let (first, last, total) = parse_range(range)?;
        if first != start
            || total == 0
            || last != end.min(total - 1)
            || (!initial && total != self.length)
        {
            return Err(failure("Invalid or changed audio Content-Range."));
        }
        if let Some((name, value)) = &self.validator
            && response
                .headers()
                .get(name)
                .is_some_and(|received| received != value)
        {
            return Err(failure("Audio changed during playback."));
        }
        if initial {
            self.validator = response
                .headers()
                .get(header::ETAG)
                .filter(|value| !value.as_bytes().starts_with(b"W/"))
                .cloned()
                .map(|value| (header::ETAG, value))
                .or_else(|| {
                    response
                        .headers()
                        .get(header::LAST_MODIFIED)
                        .cloned()
                        .map(|value| (header::LAST_MODIFIED, value))
                });
        }
        let expected = (last - first + 1) as usize;
        let mut bytes = vec![0; expected];
        response.read_exact(&mut bytes).map_err(|_| {
            failure("Audio range was interrupted. Retry from the last playback position.")
        })?;
        let mut extra = [0];
        if response
            .read(&mut extra)
            .map_err(|_| failure("Audio range was interrupted."))?
            != 0
        {
            return Err(failure("Oversized audio range."));
        }
        self.check_cancel()?;
        self.metrics
            .bytes
            .fetch_add(bytes.len() as u64, Ordering::Relaxed);
        self.metrics
            .peak
            .fetch_max(bytes.len() as u64, Ordering::Relaxed);
        self.buffer = bytes;
        self.buffer_start = start;
        self.length = total;
        Ok(())
    }
}

fn parse_range(value: &str) -> io::Result<(u64, u64, u64)> {
    let (extent, total) = value
        .strip_prefix("bytes ")
        .and_then(|value| value.split_once('/'))
        .ok_or_else(|| failure("Invalid audio Content-Range."))?;
    let (first, last) = extent
        .split_once('-')
        .ok_or_else(|| failure("Invalid audio Content-Range."))?;
    let first = first
        .parse::<u64>()
        .map_err(|_| failure("Invalid audio range start."))?;
    let last = last
        .parse::<u64>()
        .map_err(|_| failure("Invalid audio range end."))?;
    let total = total
        .parse::<u64>()
        .map_err(|_| failure("Unknown audio length."))?;
    if first > last || last >= total || last - first >= RANGE_BYTES as u64 {
        return Err(failure("Invalid audio range bounds."));
    }
    Ok((first, last, total))
}

impl Read for HttpRangeSource {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.check_cancel()?;
        if out.is_empty() || self.position >= self.length {
            return Ok(0);
        }
        if self.position < self.buffer_start
            || self.position >= self.buffer_start + self.buffer.len() as u64
        {
            self.fetch(self.position, false)?;
        }
        let offset = (self.position - self.buffer_start) as usize;
        let count = out.len().min(self.buffer.len() - offset);
        out[..count].copy_from_slice(&self.buffer[offset..offset + count]);
        self.position += count as u64;
        Ok(count)
    }
}

impl Seek for HttpRangeSource {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.check_cancel()?;
        let position = match from {
            SeekFrom::Start(position) => i128::from(position),
            SeekFrom::Current(offset) => i128::from(self.position) + i128::from(offset),
            SeekFrom::End(offset) => i128::from(self.length) + i128::from(offset),
        };
        self.position = u64::try_from(position)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid audio seek."))?;
        Ok(self.position)
    }
}

impl MediaSource for HttpRangeSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.length)
    }
}

fn failure(message: &str) -> io::Error {
    io::Error::other(message.to_owned())
}

#[cfg(test)]
mod tests;
