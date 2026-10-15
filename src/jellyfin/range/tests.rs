use super::*;
use std::net::{TcpListener, TcpStream};
use std::sync::Mutex;

#[derive(Clone, Copy)]
enum Reply {
    Good,
    WholeFile,
    BadRange,
    Truncated,
    Changed,
    Redirect,
}

struct Server {
    address: String,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    requests: Arc<Mutex<Vec<String>>>,
}

impl Server {
    fn new(bytes: Vec<u8>, replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
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
                assert!(last - first < RANGE_BYTES);
                recorded.lock().unwrap().push(request);
                let reply = sequence.next().unwrap_or(Reply::Good);
                let body = if matches!(reply, Reply::WholeFile) {
                    &bytes[..]
                } else {
                    &bytes[first..=last]
                };
                let status = match reply {
                    Reply::WholeFile => 200,
                    Reply::Redirect => 302,
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
                let response = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nContent-Range: bytes {range_first}-{last}/{}\r\nETag: \"{etag}\"\r\nLocation: http://127.0.0.1:1/leak\r\nConnection: close\r\n\r\n",
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
                    let _ = socket.write_all(body);
                }
            }
        });
        Self {
            address,
            stop,
            thread: Some(thread),
            requests,
        }
    }
    fn source(&self) -> io::Result<HttpRangeSource> {
        let client = crate::jellyfin::Client::new(
            &format!("http://{}", self.address),
            "test-device",
            &ProxyConfig::Off,
        )
        .unwrap();
        let token = crate::jellyfin::Token::new("dummy-audio-token".into()).unwrap();
        HttpRangeSource::new(
            client.stream_request("track1", &token, None).unwrap(),
            &ProxyConfig::Off,
            Arc::new(AtomicBool::new(false)),
        )
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
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
    assert!(source.read(&mut byte).is_err());
    assert_eq!(source.stream_position().unwrap(), RANGE_BYTES as u64);
    source.read_exact(&mut byte).unwrap();
    assert_eq!(byte[0], (RANGE_BYTES % 251) as u8);
    assert_eq!(source.metrics.snapshot().requests, 3);
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
    let mut decoder = crate::jellyfin::decode::AudioDecoder::open(Box::new(source)).unwrap();
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
        let mut decoder = crate::jellyfin::decode::AudioDecoder::open(Box::new(source)).unwrap();
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
            crate::jellyfin::decode::AudioDecoder::open(Box::new(server.source().unwrap()))
                .unwrap();
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
