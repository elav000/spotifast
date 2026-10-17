//! Bounded, seekable original-audio input. Use only on a decoder worker.
use crate::settings::ProxyConfig;
use reqwest::{Request, Url, header};
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};
use symphonia::core::io::MediaSource;

pub const RANGE_BYTES: usize = 256 * 1024;
/// About six seconds of CD-quality WAV, with one more range fetched ahead.
const PLAYBACK_RANGE_BYTES: usize = 1024 * 1024;

#[derive(Default)]
pub struct Metrics {
    requests: AtomicU64,
    cache_hits: AtomicU64,
    bytes: AtomicU64,
    peak: AtomicU64,
}

#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub requests: u64,
    pub cache_hits: u64,
    pub bytes: u64,
    pub peak_range_bytes: u64,
}

impl Metrics {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            requests: self.requests.load(Ordering::Relaxed),
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
            peak_range_bytes: self.peak.load(Ordering::Relaxed),
        }
    }
}

/// Validated, bounded HTTP ranges. Playback retains the current range and at
/// most one range fetched ahead. Failed ranges never replace the current buffer
/// or advance the cursor. No credentials enter error strings.
pub struct HttpRangeSource {
    http: reqwest::blocking::Client,
    url: Url,
    headers: header::HeaderMap,
    buffer: Vec<u8>,
    buffer_start: u64,
    position: u64,
    length: u64,
    validator: Option<(header::HeaderName, header::HeaderValue)>,
    cacheable_validator: bool,
    pub metrics: Arc<Metrics>,
    cancel: Arc<AtomicBool>,
    range_bytes: usize,
    request_timeout: Duration,
    read_ahead: bool,
    cache: Option<super::cache::Binding>,
    pending: Option<(u64, std::thread::JoinHandle<io::Result<Vec<u8>>>)>,
}

impl HttpRangeSource {
    pub fn new(request: Request, proxy: &ProxyConfig, cancel: Arc<AtomicBool>) -> io::Result<Self> {
        Self::build(request, proxy, cancel, false, None)
    }

    /// Fetch the next bounded range while the decoder/output consumes this one.
    pub fn buffered(
        request: Request,
        proxy: &ProxyConfig,
        cancel: Arc<AtomicBool>,
    ) -> io::Result<Self> {
        Self::build(request, proxy, cancel, true, None)
    }

    pub fn cached(
        request: Request,
        proxy: &ProxyConfig,
        cancel: Arc<AtomicBool>,
        cache: Option<super::cache::Binding>,
    ) -> io::Result<Self> {
        Self::build(request, proxy, cancel, true, cache)
    }

    fn build(
        request: Request,
        proxy: &ProxyConfig,
        cancel: Arc<AtomicBool>,
        read_ahead: bool,
        cache: Option<super::cache::Binding>,
    ) -> io::Result<Self> {
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
            cacheable_validator: false,
            metrics: Arc::default(),
            cancel,
            range_bytes: if read_ahead {
                PLAYBACK_RANGE_BYTES
            } else {
                RANGE_BYTES
            },
            request_timeout: Duration::from_secs(20),
            read_ahead,
            cache,
            pending: None,
        };
        source.headers.remove(header::RANGE);
        source.headers.remove(header::IF_RANGE);
        if source.cache.is_some() {
            // A one-byte authenticated probe validates authorization and the
            // entity without downloading the first cached megabyte again.
            let range_bytes = source.range_bytes;
            source.range_bytes = 1;
            source.fetch(0, true)?;
            source.range_bytes = range_bytes;
            source.fetch(0, false)?;
        } else {
            source.fetch(0, true)?;
        }
        source.prefetch_next();
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

    fn fill(&mut self, start: u64) -> io::Result<()> {
        if self
            .pending
            .as_ref()
            .is_some_and(|(next, _)| *next == start)
        {
            let (_, pending) = self.pending.take().unwrap();
            let bytes = pending
                .join()
                .map_err(|_| failure("Audio prefetch failed."))??;
            self.check_cancel()?;
            self.buffer = bytes;
            self.buffer_start = start;
            Ok(())
        } else {
            // A seek never waits for an unrelated speculative request and never
            // splices its bytes into the new position.
            self.fetch(start, false)
        }
    }

    fn prefetch_next(&mut self) {
        if !self.read_ahead || self.cancel.load(Ordering::SeqCst) {
            return;
        }
        let next = self.buffer_start + self.buffer.len() as u64;
        if let Some((start, pending)) = &self.pending {
            if *start == next || !pending.is_finished() {
                return;
            }
            self.pending.take();
        }
        if next >= self.length {
            return;
        }
        let mut reader = Self {
            http: self.http.clone(),
            url: self.url.clone(),
            headers: self.headers.clone(),
            buffer: Vec::new(),
            buffer_start: 0,
            position: next,
            length: self.length,
            validator: self.validator.clone(),
            cacheable_validator: self.cacheable_validator,
            metrics: self.metrics.clone(),
            cancel: self.cancel.clone(),
            range_bytes: self.range_bytes,
            request_timeout: self.request_timeout,
            read_ahead: false,
            cache: self.cache.clone(),
            pending: None,
        };
        if let Ok(pending) = std::thread::Builder::new()
            .name("media-prefetch".into())
            .spawn(move || {
                reader.fetch(next, false)?;
                Ok(reader.buffer)
            })
        {
            self.pending = Some((next, pending));
        }
    }

    fn cache_validator_key(&self) -> Option<Vec<u8>> {
        if !self.cacheable_validator {
            return None;
        }
        let (name, value) = self.validator.as_ref()?;
        let mut key = name.as_str().as_bytes().to_vec();
        key.push(b':');
        key.extend_from_slice(value.as_bytes());
        Some(key)
    }

    fn fetch(&mut self, start: u64, initial: bool) -> io::Result<()> {
        self.check_cancel()?;
        // Every new reader validates its first range online. Only a current
        // strong entity validator can unlock persisted ranges for this session.
        if !initial
            && let (Some(cache), Some(validator)) = (&self.cache, self.cache_validator_key())
            && let Some(bytes) = cache.read(
                &validator,
                self.length,
                start,
                self.range_bytes
                    .min(self.length.saturating_sub(start) as usize),
            )
        {
            self.check_cancel()?;
            self.metrics.cache_hits.fetch_add(1, Ordering::Relaxed);
            self.buffer = bytes;
            self.buffer_start = start;
            return Ok(());
        }
        let end = start.saturating_add(self.range_bytes as u64 - 1);
        let mut request = self
            .http
            .get(self.url.clone())
            // Request-level timeout also reaches the async body deadline. The
            // blocking client timeout alone restarts on every body read.
            .timeout(self.request_timeout)
            .headers(self.headers.clone())
            .header(header::ACCEPT_ENCODING, "identity")
            .header(header::RANGE, format!("bytes={start}-{end}"));
        if initial {
            request = request.header(header::CACHE_CONTROL, "no-cache");
        }
        if self.cacheable_validator
            && let Some((_, value)) = &self.validator
        {
            request = request.header(header::IF_RANGE, value);
        }
        self.metrics.requests.fetch_add(1, Ordering::Relaxed);
        let started = Instant::now();
        let mut response = request
            .send()
            .map_err(|_| failure("Audio request failed."))?;
        let headers_elapsed = started.elapsed();
        // Reject 200 even when the server ignores Range: do not buffer a whole
        // file, or silently splice a changed file into an existing decoder.
        if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(failure(&format!(
                "Audio needs HTTP range support (HTTP {}).",
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
        if !cache_storage_allowed(response.headers()) {
            self.cache = None;
        }
        if initial {
            self.cacheable_validator = cacheable_validator(response.headers());
            self.validator = response
                .headers()
                .get(header::ETAG)
                .filter(|value| strong_etag(value.as_bytes()))
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
            log::warn!(
                "audio range interrupted: start={start} expected_bytes={expected} elapsed_ms={}",
                started.elapsed().as_millis()
            );
            failure(
                "Audio range was interrupted or timed out. Retry from the last playback position.",
            )
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
        if started.elapsed() >= Duration::from_secs(2) {
            log::warn!(
                "audio range delayed: start={start} bytes={} headers_ms={} total_ms={}",
                bytes.len(),
                headers_elapsed.as_millis(),
                started.elapsed().as_millis()
            );
        }
        self.metrics
            .bytes
            .fetch_add(bytes.len() as u64, Ordering::Relaxed);
        self.metrics
            .peak
            .fetch_max(bytes.len() as u64, Ordering::Relaxed);
        if self.range_bytes > 1
            && let (Some(cache), Some(validator)) = (&self.cache, self.cache_validator_key())
        {
            let disk_started = Instant::now();
            let _ = cache.write(&validator, total, start, &bytes);
            if disk_started.elapsed() >= Duration::from_millis(250) {
                log::warn!(
                    "audio cache write delayed: elapsed_ms={}",
                    disk_started.elapsed().as_millis()
                );
            }
        }
        self.buffer = bytes;
        self.buffer_start = start;
        self.length = total;
        Ok(())
    }
}

fn cache_storage_allowed(headers: &header::HeaderMap) -> bool {
    !headers.get_all(header::CACHE_CONTROL).iter().any(|value| {
        value.to_str().map_or(true, |value| {
            value.split(',').any(|directive| {
                directive
                    .split('=')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .eq_ignore_ascii_case("no-store")
            })
        })
    })
}

// RFC 9110 8.8.2.2 permits strong date validators when clock/precision
// ambiguity can be excluded. Retain the conservative 60-second separation.
fn cacheable_validator(headers: &header::HeaderMap) -> bool {
    if let Some(etag) = headers.get(header::ETAG) {
        return strong_etag(etag.as_bytes());
    }
    let parse = |name| {
        let bytes = headers.get(name)?.as_bytes();
        if bytes.len() != 29 || !bytes.ends_with(b" GMT") {
            return None;
        }
        jiff::fmt::rfc2822::DateTimeParser::new()
            .parse_timestamp(bytes)
            .ok()
            .map(|time| time.as_second())
    };
    match (parse(header::DATE), parse(header::LAST_MODIFIED)) {
        (Some(date), Some(modified)) => date.checked_sub(modified).is_some_and(|age| age >= 60),
        _ => false,
    }
}

fn strong_etag(value: &[u8]) -> bool {
    value.len() >= 2
        && value[0] == b'"'
        && value[value.len() - 1] == b'"'
        && value[1..value.len() - 1]
            .iter()
            .all(|byte| *byte == 0x21 || (0x23..=0x7e).contains(byte) || *byte >= 0x80)
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
    if first > last || last >= total || last - first >= PLAYBACK_RANGE_BYTES as u64 {
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
            let start = self.position / self.range_bytes as u64 * self.range_bytes as u64;
            self.fill(start)?;
        }
        self.prefetch_next();
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
