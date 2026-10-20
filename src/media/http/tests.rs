use super::*;
use std::net::{TcpListener, TcpStream};
use std::sync::Mutex;

#[derive(Clone, Copy)]
enum Reply {
    Good,
    Blocked,
    WholeFile,
    BadRange,
    Truncated,
    Changed,
    Redirect,
    NoValidator,
    WeakValidator,
    Unauthorized,
    Dated,
    FreshDated,
    NoStore,
    Drip,
    ServerError,
}

struct Server {
    address: String,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    requests: Arc<Mutex<Vec<String>>>,
    release: Arc<(Mutex<bool>, std::sync::Condvar)>,
}

impl Server {
    fn new(bytes: Vec<u8>, replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let release = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let gate = release.clone();
        let thread = std::thread::spawn(move || {
            let mut sequence = replies.into_iter();
            loop {
                let (mut socket, _) = listener.accept().unwrap();
                if done.load(Ordering::SeqCst) {
                    break;
                }
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    if socket.read_exact(&mut byte).is_err() {
                        break;
                    }
                    request.push(byte[0]);
                    assert!(request.len() < 16 * 1024);
                }
                let request = String::from_utf8(request).unwrap();
                let lower = request.to_lowercase();
                let range = lower
                    .lines()
                    .find_map(|line| line.strip_prefix("range: bytes="))
                    .unwrap();
                let (first, last) = range.split_once('-').unwrap();
                let first = first.parse::<usize>().unwrap();
                let last = last.parse::<usize>().unwrap().min(bytes.len() - 1);
                assert!(last - first < PLAYBACK_RANGE_BYTES);
                recorded.lock().unwrap().push(request);
                let reply = sequence.next().unwrap_or(Reply::Good);
                if matches!(reply, Reply::Blocked) {
                    let (lock, ready) = &*gate;
                    let mut released = lock.lock().unwrap();
                    while !*released {
                        released = ready.wait(released).unwrap();
                    }
                }
                let body = if matches!(reply, Reply::WholeFile) {
                    &bytes[..]
                } else {
                    &bytes[first..=last]
                };
                let status = match reply {
                    Reply::WholeFile => 200,
                    Reply::Redirect => 302,
                    Reply::Unauthorized => 401,
                    Reply::ServerError => 503,
                    _ => 206,
                };
                let etag = if matches!(reply, Reply::Changed) {
                    "v2"
                } else {
                    "v1"
                };
                let range_first = if matches!(reply, Reply::BadRange) {
                    first + 1
                } else {
                    first
                };
                let validator = match reply {
                    Reply::NoValidator => String::new(),
                    Reply::NoStore => "ETag: \"v1\"\r\nCache-Control: private, NO-STORE\r\n".into(),
                    Reply::Dated => "Date: Thu, 07 Nov 2024 12:00:00 GMT\r\nLast-Modified: Thu, 07 Nov 2024 11:00:00 GMT\r\n".into(),
                    Reply::FreshDated => "Date: Thu, 07 Nov 2024 12:00:00 GMT\r\nLast-Modified: Thu, 07 Nov 2024 12:00:00 GMT\r\n".into(),
                    Reply::WeakValidator => "ETag: W/\"v1\"\r\n".into(),
                    _ => format!("ETag: \"{etag}\"\r\n"),
                };
                let response = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nContent-Range: bytes {range_first}-{last}/{}\r\n{validator}Location: http://127.0.0.1:1/leak\r\nConnection: close\r\n\r\n",
                    body.len(),
                    bytes.len()
                );
                use std::io::Write;
                if socket.write_all(response.as_bytes()).is_ok() {
                    let body = if matches!(reply, Reply::Truncated) {
                        &body[..body.len() / 2]
                    } else {
                        body
                    };
                    if matches!(reply, Reply::Drip) {
                        for chunk in body.chunks(4096) {
                            if socket.write_all(chunk).is_err() {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(15));
                        }
                    } else {
                        let _ = socket.write_all(body);
                    }
                }
            }
        });
        Self {
            address,
            stop,
            thread: Some(thread),
            requests,
            release,
        }
    }
    fn unblock(&self) {
        let (lock, ready) = &*self.release;
        *lock.lock().unwrap() = true;
        ready.notify_all();
    }
    fn source(&self) -> io::Result<HttpRangeSource> {
        self.source_mode(false)
    }
    fn source_cached(&self, cache: super::super::cache::Binding) -> io::Result<HttpRangeSource> {
        let mut request = Request::new(
            reqwest::Method::GET,
            format!("http://{}/audio", self.address).parse().unwrap(),
        );
        request.headers_mut().insert(
            "authorization",
            header::HeaderValue::from_static("dummy-audio-token"),
        );
        HttpRangeSource::cached(
            request,
            &ProxyConfig::Off,
            Arc::new(AtomicBool::new(false)),
            Some(cache),
        )
    }
    fn source_mode(&self, buffered: bool) -> io::Result<HttpRangeSource> {
        let mut request = Request::new(
            reqwest::Method::GET,
            format!("http://{}/audio", self.address).parse().unwrap(),
        );
        request.headers_mut().insert(
            "authorization",
            header::HeaderValue::from_static("dummy-audio-token"),
        );
        let open = if buffered {
            HttpRangeSource::buffered
        } else {
            HttpRangeSource::new
        };
        open(request, &ProxyConfig::Off, Arc::new(AtomicBool::new(false)))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.unblock();
        let _ = TcpStream::connect(&self.address);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

fn bytes() -> Vec<u8> {
    (0..RANGE_BYTES * 5).map(|i| (i % 251) as u8).collect()
}

#[test]
fn bounded_ranges_seek_without_downloading_the_middle_of_a_file() {
    let bytes = bytes();
    let server = Server::new(bytes.clone(), vec![]);
    let mut source = server.source().unwrap();
    assert_eq!(source.metrics.snapshot().bytes, RANGE_BYTES as u64);
    let mut first = [0; 16];
    source.read_exact(&mut first).unwrap();
    assert_eq!(first, bytes[..16]);
    source.seek(SeekFrom::End(-16)).unwrap();
    let mut last = [0; 16];
    source.read_exact(&mut last).unwrap();
    assert_eq!(last, bytes[bytes.len() - 16..]);
    assert_eq!(source.metrics.snapshot().requests, 2);
    assert!(source.metrics.snapshot().bytes < bytes.len() as u64);
    assert!(source.metrics.snapshot().peak_range_bytes <= RANGE_BYTES as u64);
    let requests = server.requests.lock().unwrap();
    assert!(
        requests
            .iter()
            .all(|request| request.contains("dummy-audio-token"))
    );
    assert!(requests.iter().all(|request| {
        !request
            .lines()
            .next()
            .unwrap()
            .contains("dummy-audio-token")
    }));
    assert!(requests[1].to_lowercase().contains("if-range: \"v1\""));
}

#[test]
fn rejects_ignored_ranges_malformed_ranges_and_redirects_without_exposing_credentials() {
    for reply in [Reply::WholeFile, Reply::BadRange, Reply::Redirect] {
        let server = Server::new(bytes(), vec![reply]);
        let error = server.source().err().unwrap().to_string();
        assert!(!error.contains("dummy-audio-token"));
        assert!(!error.contains(&server.address));
        assert_eq!(server.requests.lock().unwrap().len(), 1);
    }
}

#[test]
fn interrupted_range_can_be_retried_without_advancing_or_splicing_partial_bytes() {
    let server = Server::new(bytes(), vec![Reply::Good, Reply::Truncated, Reply::Good]);
    let mut source = server.source().unwrap();
    source.seek(SeekFrom::Start(RANGE_BYTES as u64)).unwrap();
    let mut byte = [0];
    source.read_exact(&mut byte).unwrap();
    assert_eq!(byte[0], (RANGE_BYTES % 251) as u8);
    assert_eq!(source.stream_position().unwrap(), RANGE_BYTES as u64 + 1);
    assert_eq!(source.metrics.snapshot().requests, 3);
}

#[test]
fn transient_server_failure_is_retried_but_retries_are_bounded() {
    let server = Server::new(bytes(), vec![Reply::ServerError, Reply::Good]);
    let source = server.source().unwrap();
    assert_eq!(source.metrics.snapshot().requests, 2);

    let server = Server::new(
        bytes(),
        vec![
            Reply::Good,
            Reply::Truncated,
            Reply::Truncated,
            Reply::Truncated,
        ],
    );
    let mut source = server.source().unwrap();
    source.seek(SeekFrom::Start(RANGE_BYTES as u64)).unwrap();
    assert!(source.read(&mut [0]).is_err());
    assert_eq!(source.stream_position().unwrap(), RANGE_BYTES as u64);
    assert_eq!(source.buffer_start, 0);
    assert_eq!(source.metrics.snapshot().requests, 4);
}

#[test]
fn changed_entity_is_rejected_instead_of_combining_different_audio() {
    let server = Server::new(bytes(), vec![Reply::Good, Reply::Changed]);
    let mut source = server.source().unwrap();
    source.seek(SeekFrom::Start(RANGE_BYTES as u64)).unwrap();
    assert!(
        source
            .read(&mut [0])
            .unwrap_err()
            .to_string()
            .contains("changed")
    );
}

#[test]
fn cancellation_interrupts_read_and_seek_without_another_request() {
    let server = Server::new(bytes(), vec![]);
    let mut source = server.source().unwrap();
    source.cancel.store(true, Ordering::SeqCst);
    assert_eq!(
        source.read(&mut [0]).unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(
        source.seek(SeekFrom::Start(0)).unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(source.metrics.snapshot().requests, 1);
}

#[test]
fn flac_decodes_before_whole_file_arrives_and_seeks_near_the_end() {
    let bytes = include_bytes!("../../testdata/jellyfin-noise.flac");
    assert!(bytes.len() > RANGE_BYTES * 2);
    let server = Server::new(bytes.to_vec(), vec![]);
    let source = server.source().unwrap();
    let metrics = source.metrics.clone();
    let mut decoder = crate::media::decode::AudioDecoder::open(Box::new(source)).unwrap();
    let first = decoder.next_pcm().unwrap().unwrap();
    assert!(!first.is_empty());
    assert!(first.iter().any(|sample| sample.abs() > 0.001));
    assert!(metrics.snapshot().bytes < bytes.len() as u64);
    assert_eq!(decoder.duration_ms(), Some(8000));
    decoder.seek_ms(7500).unwrap();
    let mut frames = 0;
    while let Some(pcm) = decoder.next_pcm().unwrap() {
        frames += pcm.len() / 2;
    }
    assert_eq!(frames, 22050);
    assert!(metrics.snapshot().peak_range_bytes <= RANGE_BYTES as u64);
}

#[test]
fn byte_probe_supports_wav_and_mp3_without_an_extension_hint() {
    for bytes in [
        include_bytes!("../../testdata/jellyfin-tone.wav").as_slice(),
        include_bytes!("../../testdata/jellyfin-tone.mp3").as_slice(),
    ] {
        let server = Server::new(bytes.to_vec(), vec![]);
        let source = server.source().unwrap();
        let mut decoder = crate::media::decode::AudioDecoder::open(Box::new(source)).unwrap();
        let mut samples = 0;
        while let Some(pcm) = decoder.next_pcm().unwrap() {
            assert!(pcm.iter().all(|sample| sample.is_finite()));
            samples += pcm.len();
        }
        assert!(samples > 80000);
        assert!(samples < 100000);
    }
}

#[test]
fn wav_resampling_preserves_the_tail_and_wav_mp3_seek_produces_audio() {
    for bytes in [
        include_bytes!("../../testdata/jellyfin-tone.wav").as_slice(),
        include_bytes!("../../testdata/jellyfin-tone.mp3").as_slice(),
    ] {
        let server = Server::new(bytes.to_vec(), vec![]);
        let mut decoder =
            crate::media::decode::AudioDecoder::open(Box::new(server.source().unwrap())).unwrap();
        let mut frames = 0;
        while let Some(pcm) = decoder.next_pcm().unwrap() {
            frames += pcm.len() / 2;
        }
        if decoder.sample_rate() == 48000 {
            assert_eq!(frames, 44100);
        }
        decoder.seek_ms(500).unwrap();
        let mut frames = 0;
        while let Some(pcm) = decoder.next_pcm().unwrap() {
            frames += pcm.len() / 2;
        }
        assert!((20000..26000).contains(&frames), "{frames}");
    }
}

#[test]
fn flac_seeks_return_the_same_samples_as_sequential_decoding() {
    let server = Server::new(
        include_bytes!("../../testdata/jellyfin-noise.flac").to_vec(),
        vec![],
    );
    let mut decoder =
        crate::media::decode::AudioDecoder::open(Box::new(server.source().unwrap())).unwrap();
    let mut original = Vec::new();
    while let Some(pcm) = decoder.next_pcm().unwrap() {
        original.extend(pcm);
    }
    // Alternate directions and use a time inside a FLAC block, not just its
    // boundary. Counting remaining frames alone would miss wrong audio.
    for milliseconds in [7123, 1250, 4000] {
        decoder.seek_ms(milliseconds).unwrap();
        let mut sought = Vec::new();
        while let Some(pcm) = decoder.next_pcm().unwrap() {
            sought.extend(pcm);
        }
        let first_sample = (milliseconds * 44100 / 1000) as usize * 2;
        assert_eq!(sought, original[first_sample..], "seek {milliseconds} ms");
    }
}

#[test]
fn resampled_wav_seek_preserves_sample_alignment_after_filter_startup() {
    let server = Server::new(
        include_bytes!("../../testdata/jellyfin-tone.wav").to_vec(),
        vec![],
    );
    let mut decoder =
        crate::media::decode::AudioDecoder::open(Box::new(server.source().unwrap())).unwrap();
    let mut original = Vec::new();
    while let Some(pcm) = decoder.next_pcm().unwrap() {
        original.extend(pcm);
    }
    decoder.seek_ms(500).unwrap();
    let mut sought = Vec::new();
    while let Some(pcm) = decoder.next_pcm().unwrap() {
        sought.extend(pcm);
    }
    let expected = &original[44100..];
    assert_eq!(sought.len(), expected.len());
    // Resetting the sinc filter intentionally drops its pre-seek history.
    // Beyond that short startup, the resampled signal must align exactly.
    for (actual, expected) in sought[256..].iter().zip(&expected[256..]) {
        assert!((actual - expected).abs() < 0.00001);
    }
}

fn await_prefetch(source: &HttpRangeSource) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !source.pending.as_ref().unwrap().1.is_finished() {
        assert!(
            std::time::Instant::now() < deadline,
            "bounded prefetch completed"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn playback_bytes() -> Vec<u8> {
    (0..PLAYBACK_RANGE_BYTES * 4)
        .map(|i| (i % 251) as u8)
        .collect()
}

#[test]
fn buffered_playback_fetches_ahead_before_consumption_and_reuses_the_ready_range() {
    let bytes = playback_bytes();
    let server = Server::new(bytes.clone(), vec![]);
    let mut source = server.source_mode(true).unwrap();
    await_prefetch(&source);
    assert_eq!(source.stream_position().unwrap(), 0);
    assert_eq!(source.buffer.len(), PLAYBACK_RANGE_BYTES);
    assert_eq!(
        source.metrics.snapshot().bytes,
        (PLAYBACK_RANGE_BYTES * 2) as u64
    );
    let mut out = vec![0; PLAYBACK_RANGE_BYTES + 16];
    source.read_exact(&mut out).unwrap();
    assert_eq!(out, bytes[..out.len()]);
    assert_eq!(source.buffer_start, PLAYBACK_RANGE_BYTES as u64);
    assert!(source.metrics.snapshot().peak_range_bytes <= PLAYBACK_RANGE_BYTES as u64);
    let requests = server.requests.lock().unwrap();
    let next = format!("range: bytes={}-", PLAYBACK_RANGE_BYTES);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.to_lowercase().contains(&next))
            .count(),
        1
    );
}

#[test]
fn buffered_seek_discards_unrelated_prefetch_and_preserves_exact_bytes() {
    let bytes = playback_bytes();
    let server = Server::new(bytes.clone(), vec![]);
    let mut source = server.source_mode(true).unwrap();
    await_prefetch(&source);
    source.seek(SeekFrom::End(-16)).unwrap();
    let mut out = [0; 16];
    source.read_exact(&mut out).unwrap();
    assert_eq!(out, bytes[bytes.len() - 16..]);
    source
        .seek(SeekFrom::Start(PLAYBACK_RANGE_BYTES as u64 + 37))
        .unwrap();
    source.read_exact(&mut out).unwrap();
    assert_eq!(
        out,
        bytes[PLAYBACK_RANGE_BYTES + 37..PLAYBACK_RANGE_BYTES + 53]
    );
}

#[test]
fn failed_prefetch_preserves_current_range_cursor_and_validation_on_retry() {
    let server = Server::new(
        playback_bytes(),
        vec![Reply::Good, Reply::Changed, Reply::Good],
    );
    let mut source = server.source_mode(true).unwrap();
    await_prefetch(&source);
    source
        .seek(SeekFrom::Start(PLAYBACK_RANGE_BYTES as u64))
        .unwrap();
    assert!(
        source
            .read(&mut [0])
            .unwrap_err()
            .to_string()
            .contains("changed")
    );
    assert_eq!(source.buffer_start, 0);
    assert_eq!(
        source.stream_position().unwrap(),
        PLAYBACK_RANGE_BYTES as u64
    );
    let mut out = [0];
    source.read_exact(&mut out).unwrap();
    assert_eq!(out[0], (PLAYBACK_RANGE_BYTES % 251) as u8);
    await_prefetch(&source);
    let requests = source.metrics.snapshot().requests;
    source.cancel.store(true, Ordering::SeqCst);
    assert_eq!(
        source.read(&mut out).unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(
        source.seek(SeekFrom::Start(0)).unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(source.metrics.snapshot().requests, requests);
}

#[test]
fn current_audio_remains_readable_while_the_next_network_range_is_blocked() {
    let bytes = playback_bytes();
    let server = Server::new(bytes.clone(), vec![Reply::Good, Reply::Blocked]);
    let mut source = server.source_mode(true).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while server.requests.lock().unwrap().len() < 2 {
        assert!(
            std::time::Instant::now() < deadline,
            "next range requested before consuming current audio"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!source.pending.as_ref().unwrap().1.is_finished());
    let mut out = [0; 8192];
    source.read_exact(&mut out).unwrap();
    assert_eq!(out, bytes[..out.len()]);
    assert!(!source.pending.as_ref().unwrap().1.is_finished());
    server.unblock();
    await_prefetch(&source);
    source
        .seek(SeekFrom::Start(PLAYBACK_RANGE_BYTES as u64))
        .unwrap();
    source.read_exact(&mut out).unwrap();
    assert_eq!(
        out,
        bytes[PLAYBACK_RANGE_BYTES..PLAYBACK_RANGE_BYTES + out.len()]
    );
}

#[test]
fn repeat_playback_validates_online_then_uses_cached_ranges_and_seeks() {
    use crate::media::cache::{Identity, Policy};
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/media-http-cache-tests")
        .join(format!("repeat-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(root.with_extension("streams"));
    let cache = Policy::new(Some(&root), Some(16 * 1024 * 1024))
        .http
        .unwrap();
    let identity = Identity {
        provider: "fixture-http".into(),
        source: "source".into(),
        account: "a".into(),
        item: "item".into(),
        representation: "original".into(),
    };
    let bytes = playback_bytes();
    let server = Server::new(bytes.clone(), vec![]);
    let mut cold = server
        .source_cached(cache.clone().bind(identity.clone()))
        .unwrap();
    let mut out = Vec::new();
    cold.read_to_end(&mut out).unwrap();
    assert_eq!(out, bytes);
    assert_eq!(cold.metrics.snapshot().cache_hits, 0);
    let mut warm = server
        .source_cached(cache.clone().bind(identity.clone()))
        .unwrap();
    out.clear();
    warm.read_to_end(&mut out).unwrap();
    assert_eq!(out, bytes);
    assert_eq!(
        warm.metrics.snapshot().requests,
        1,
        "one-byte online validation only"
    );
    assert_eq!(warm.metrics.snapshot().bytes, 1);
    assert_eq!(warm.metrics.snapshot().cache_hits, 4);
    warm.seek(SeekFrom::Start(12345)).unwrap();
    let mut seek = [0; 16];
    warm.read_exact(&mut seek).unwrap();
    assert_eq!(seek, bytes[12345..12361]);
    await_prefetch(&warm);
    assert_eq!(
        warm.metrics.snapshot().requests,
        1,
        "seeking reuses aligned cached ranges"
    );
    // A different account must refill from the network even for the same item.
    let mut other = identity;
    other.account = "b".into();
    let mut other = server.source_cached(cache.bind(other)).unwrap();
    out.clear();
    other.read_to_end(&mut out).unwrap();
    assert_eq!(other.metrics.snapshot().cache_hits, 0);
    let _ = std::fs::remove_dir_all(root.with_extension("streams"));
}

#[test]
fn changed_entity_never_reuses_old_audio_from_the_disk_cache() {
    use crate::media::cache::{Identity, Policy};
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/media-http-cache-tests")
        .join(format!("changed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(root.with_extension("streams"));
    let binding = Policy::new(Some(&root), Some(16 * 1024 * 1024))
        .http
        .unwrap()
        .bind(Identity {
            provider: "fixture".into(),
            source: "s".into(),
            account: "a".into(),
            item: "i".into(),
            representation: "original".into(),
        });
    let bytes = playback_bytes();
    let server = Server::new(bytes.clone(), vec![]);
    let mut cold = server.source_cached(binding.clone()).unwrap();
    let mut out = Vec::new();
    cold.read_to_end(&mut out).unwrap();
    drop(cold);
    let mut changed_bytes = bytes;
    changed_bytes.fill(99);
    let changed = Server::new(changed_bytes.clone(), vec![Reply::Changed; 6]);
    let mut warm = changed.source_cached(binding).unwrap();
    out.clear();
    warm.read_to_end(&mut out).unwrap();
    assert_eq!(out, changed_bytes);
    assert_eq!(warm.metrics.snapshot().cache_hits, 0);
    let _ = std::fs::remove_dir_all(root.with_extension("streams"));
}

#[test]
fn uncacheable_responses_and_revoked_auth_cannot_use_disk_cache() {
    use crate::media::cache::{Identity, Policy};
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/media-http-cache-tests")
        .join(format!("validators-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(root.with_extension("streams"));
    let binding = Policy::new(Some(&root), Some(16 * 1024 * 1024))
        .http
        .unwrap()
        .bind(Identity {
            provider: "fixture".into(),
            source: "s".into(),
            account: "a".into(),
            item: "i".into(),
            representation: "original".into(),
        });
    for reply in [
        Reply::NoValidator,
        Reply::WeakValidator,
        Reply::FreshDated,
        Reply::NoStore,
    ] {
        let server = Server::new(playback_bytes(), vec![reply; 10]);
        for _ in 0..2 {
            let mut reader = server.source_cached(binding.clone()).unwrap();
            reader.read_to_end(&mut Vec::new()).unwrap();
            assert_eq!(reader.metrics.snapshot().cache_hits, 0);
        }
    }
    let server = Server::new(playback_bytes(), vec![]);
    let mut reader = server.source_cached(binding.clone()).unwrap();
    reader.read_to_end(&mut Vec::new()).unwrap();
    let denied = Server::new(playback_bytes(), vec![Reply::Unauthorized]);
    let error = denied.source_cached(binding).err().unwrap().to_string();
    assert!(error.contains("401"));
    assert_eq!(denied.requests.lock().unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(root.with_extension("streams"));
}

#[test]
fn corrupt_disk_ranges_refetch_and_repair_without_changing_decoded_bytes() {
    use crate::media::cache::{Identity, Policy};
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/media-http-cache-tests")
        .join(format!("repair-{}", std::process::id()));
    let disk = root.with_extension("streams");
    let _ = std::fs::remove_dir_all(&disk);
    let binding = Policy::new(Some(&root), Some(16 * 1024 * 1024))
        .http
        .unwrap()
        .bind(Identity {
            provider: "fixture".into(),
            source: "s".into(),
            account: "a".into(),
            item: "i".into(),
            representation: "original".into(),
        });
    let bytes = playback_bytes();
    let server = Server::new(bytes.clone(), vec![]);
    let mut cold = server.source_cached(binding.clone()).unwrap();
    cold.read_to_end(&mut Vec::new()).unwrap();
    for entry in std::fs::read_dir(&disk).unwrap() {
        let path = entry.unwrap().path();
        let mut damaged = std::fs::read(&path).unwrap();
        *damaged.last_mut().unwrap() ^= 1;
        std::fs::write(path, damaged).unwrap();
    }
    let mut repair = server.source_cached(binding.clone()).unwrap();
    let mut out = Vec::new();
    repair.read_to_end(&mut out).unwrap();
    assert_eq!(out, bytes);
    assert_eq!(repair.metrics.snapshot().cache_hits, 0);
    let mut replay = server.source_cached(binding).unwrap();
    out.clear();
    replay.read_to_end(&mut out).unwrap();
    assert_eq!(out, bytes);
    assert_eq!(replay.metrics.snapshot().requests, 1);
    let _ = std::fs::remove_dir_all(disk);
}

#[test]
fn settled_last_modified_supports_cached_playback_without_an_etag() {
    use crate::media::cache::{Identity, Policy};
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/media-http-cache-tests")
        .join(format!("dated-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(root.with_extension("streams"));
    let binding = Policy::new(Some(&root), Some(16 * 1024 * 1024))
        .http
        .unwrap()
        .bind(Identity {
            provider: "fixture".into(),
            source: "s".into(),
            account: "a".into(),
            item: "i".into(),
            representation: "original".into(),
        });
    let bytes = playback_bytes();
    let server = Server::new(bytes.clone(), vec![Reply::Dated; 10]);
    let mut cold = server.source_cached(binding.clone()).unwrap();
    cold.read_to_end(&mut Vec::new()).unwrap();
    let mut warm = server.source_cached(binding).unwrap();
    let mut out = Vec::new();
    warm.read_to_end(&mut out).unwrap();
    assert_eq!(out, bytes);
    assert_eq!(warm.metrics.snapshot().requests, 1);
    assert_eq!(warm.metrics.snapshot().cache_hits, 4);
    assert!(server.requests.lock().unwrap().iter().any(|request| {
        request
            .to_lowercase()
            .contains("if-range: thu, 07 nov 2024 11:00:00 gmt")
    }));
    let _ = std::fs::remove_dir_all(root.with_extension("streams"));
}

#[test]
fn date_validator_rejects_clock_ambiguity_and_weak_etags() {
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::DATE,
        header::HeaderValue::from_static("Thu, 07 Nov 2024 12:00:00 GMT"),
    );
    for modified in [
        "Thu, 07 Nov 2024 12:00:00 GMT",
        "Thu, 07 Nov 2024 12:01:00 GMT",
        "invalid",
    ] {
        headers.insert(header::LAST_MODIFIED, modified.parse().unwrap());
        assert!(!cacheable_validator(&headers));
    }
    headers.insert(
        header::LAST_MODIFIED,
        header::HeaderValue::from_static("Thu, 07 Nov 2024 11:00:00 GMT"),
    );
    assert!(cacheable_validator(&headers));
    headers.insert(header::ETAG, header::HeaderValue::from_static("W/\"v1\""));
    assert!(!cacheable_validator(&headers));
    headers.remove(header::ETAG);
    headers.remove(header::DATE);
    assert!(!cacheable_validator(&headers));
}

#[test]
fn a_trickling_response_cannot_extend_the_total_range_deadline() {
    let server = Server::new(
        playback_bytes(),
        vec![Reply::Good, Reply::Drip, Reply::Drip, Reply::Drip],
    );
    let mut source = server.source().unwrap();
    source.request_timeout = Duration::from_millis(80);
    source.seek(SeekFrom::Start(RANGE_BYTES as u64)).unwrap();
    let start = Instant::now();
    assert!(source.read(&mut [0; 1]).is_err());
    assert!(start.elapsed() < Duration::from_millis(1200));
    assert_eq!(
        source.position, RANGE_BYTES as u64,
        "no partial range advances the decoder"
    );
    assert_eq!(source.buffer_start, 0);
    assert_eq!(source.metrics.snapshot().requests, 4);
}
