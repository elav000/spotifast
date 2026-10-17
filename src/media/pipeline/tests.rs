use super::*;
use std::{
    sync::{atomic::AtomicUsize, mpsc},
    time::{Duration, Instant},
};

struct Fixture {
    calls: Arc<AtomicUsize>,
    gate: Option<mpsc::Receiver<()>>,
    entered: Option<mpsc::Sender<()>>,
    position: f64,
    remaining: usize,
    fail: bool,
}
impl Decode for Fixture {
    fn next(&mut self) -> Result<Option<Vec<f64>>, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(entered) = self.entered.take() {
            entered.send(()).unwrap();
        }
        if let Some(gate) = self.gate.take() {
            gate.recv_timeout(Duration::from_secs(3)).unwrap();
        }
        if self.remaining == 0 {
            return if self.fail {
                Err("input interrupted".into())
            } else {
                Ok(None)
            };
        }
        self.remaining -= 1;
        Ok(Some(vec![self.position; 88200]))
    }
    fn seek(&mut self, position: u64) -> Result<(), String> {
        self.position = position as f64;
        self.remaining = 1;
        Ok(())
    }
}
fn fixture(remaining: usize) -> Fixture {
    Fixture {
        calls: Arc::new(AtomicUsize::new(0)),
        gate: None,
        entered: None,
        position: 1.0,
        remaining,
        fail: false,
    }
}
fn buffered(fixture: Fixture) -> BufferedDecoder {
    BufferedDecoder::spawn(fixture, Arc::new(AtomicBool::new(false)), None, false).unwrap()
}
fn wait_for(decoder: &BufferedDecoder, predicate: impl Fn(&State) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut state = decoder.shared.state.lock().unwrap();
    while !predicate(&state) {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(
            !left.is_zero(),
            "decoder worker did not reach expected state"
        );
        state = decoder.shared.changed.wait_timeout(state, left).unwrap().0;
    }
}

#[test]
fn paused_consumption_has_bounded_read_ahead_and_resumes_in_order() {
    let input = fixture(20);
    let calls = input.calls.clone();
    let mut decoder = buffered(input);
    wait_for(&decoder, |s| s.frames == AHEAD_FRAMES);
    assert_eq!(calls.load(Ordering::SeqCst), 8);
    assert!(matches!(decoder.poll().unwrap(), Readiness::Packet(p) if p.iter().all(|v| *v == 1.0)));
    assert!(decoder.shared.state.lock().unwrap().frames <= AHEAD_FRAMES);
}

#[test]
fn seek_is_nonblocking_and_discards_an_old_in_flight_decode() {
    let (release, gate) = mpsc::channel();
    let (entered, decoding) = mpsc::channel();
    let mut input = fixture(2);
    input.gate = Some(gate);
    input.entered = Some(entered);
    let mut decoder = buffered(input);
    decoding.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(matches!(decoder.poll().unwrap(), Readiness::Pending));
    decoder.seek_ms(500);
    decoder.seek_ms(900);
    assert!(matches!(decoder.poll().unwrap(), Readiness::Pending));
    release.send(()).unwrap();
    wait_for(&decoder, |s| {
        s.frames >= START_FRAMES || s.terminal.is_some()
    });
    assert!(
        matches!(decoder.poll().unwrap(), Readiness::Packet(p) if p.iter().all(|v| *v == 900.0))
    );
}

#[test]
fn buffered_prefix_drains_before_an_error_and_seek_can_retry() {
    let mut input = fixture(1);
    input.fail = true;
    let mut decoder = buffered(input);
    wait_for(&decoder, |s| s.terminal.is_some());
    let mut samples = 0;
    loop {
        match decoder.poll() {
            Ok(Readiness::Packet(p)) => samples += p.len(),
            Err(error) => {
                assert_eq!(error, "input interrupted");
                break;
            }
            _ => panic!("buffered prefix lost"),
        }
    }
    assert_eq!(samples, 88200);
    decoder.seek_ms(123);
    wait_for(&decoder, |s| {
        s.frames >= START_FRAMES || s.terminal.is_some()
    });
    assert!(matches!(decoder.poll().unwrap(), Readiness::Packet(p) if p[0] == 123.0));
}

#[test]
fn short_tail_and_end_do_not_wait_for_a_full_cushion() {
    struct Short(bool);
    impl Decode for Short {
        fn next(&mut self) -> Result<Option<Vec<f64>>, String> {
            Ok(if std::mem::replace(&mut self.0, true) {
                None
            } else {
                Some(vec![0.25; 200])
            })
        }
        fn seek(&mut self, _: u64) -> Result<(), String> {
            Ok(())
        }
    }
    let mut decoder =
        BufferedDecoder::spawn(Short(false), Arc::new(AtomicBool::new(false)), None, false)
            .unwrap();
    wait_for(&decoder, |s| s.terminal.is_some());
    assert!(matches!(decoder.poll().unwrap(), Readiness::Packet(p) if p == vec![0.25; 200]));
    assert!(matches!(decoder.poll().unwrap(), Readiness::End));
    let enclosing_track = decoder.shared.cancel.clone();
    drop(decoder);
    assert!(
        !enclosing_track.load(Ordering::SeqCst),
        "normal EOF must reach the queue controller"
    );
}
