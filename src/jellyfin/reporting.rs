//! Playback reporting policy, independent of browsing and selected provider.
use std::time::{Duration, Instant};

use super::{browser::Source, catalog::PlaybackEvent};

#[derive(Clone)]
pub struct Playback {
    pub source: Source,
    pub item: String,
    pub position_ms: u64,
    pub playing: bool,
    pub loading: bool,
}

pub struct Report {
    pub playback: Playback,
    pub event: PlaybackEvent,
}

#[derive(Default)]
pub struct Reporter {
    last: Option<(Playback, Instant)>,
}

impl Reporter {
    pub fn update(&mut self, current: Option<Playback>, now: Instant) -> Vec<Report> {
        let mut reports = Vec::new();
        let changed = self.last.as_ref().is_some_and(|(old, _)| {
            current
                .as_ref()
                .is_none_or(|new| !new.source.same_identity(&old.source) || new.item != old.item)
        });
        if changed && let Some((old, _)) = self.last.take() {
            reports.push(Report {
                playback: old,
                event: PlaybackEvent::Stop,
            });
        }
        let Some(current) = current else {
            return reports;
        };
        if let Some((last, sent)) = &mut self.last {
            if !current.loading
                && (current.playing != last.playing
                    || now.duration_since(*sent) >= Duration::from_secs(10))
            {
                reports.push(Report {
                    playback: current.clone(),
                    event: PlaybackEvent::Progress,
                });
                *sent = now;
            }
            *last = current;
        } else if current.playing && !current.loading {
            reports.push(Report {
                playback: current.clone(),
                event: PlaybackEvent::Start,
            });
            self.last = Some((current, now));
        }
        reports
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn playback() -> Playback {
        Playback {
            source: Source {
                id: "source".into(),
                nickname: "Test server".into(),
                server_url: "http://localhost:8096/".into(),
                server_id: "server".into(),
                user_id: "user".into(),
                device_id: "device".into(),
            },
            item: "song".into(),
            position_ms: 0,
            playing: true,
            loading: false,
        }
    }
    #[test]
    fn reports_start_pause_progress_and_stop_without_tying_them_to_navigation() {
        let mut reporter = Reporter::default();
        let now = Instant::now();
        let mut current = playback();
        assert!(matches!(
            reporter.update(Some(current.clone()), now)[0].event,
            PlaybackEvent::Start
        ));
        current.position_ms = 5000;
        assert!(
            reporter
                .update(Some(current.clone()), now + Duration::from_secs(5))
                .is_empty()
        );
        current.playing = false;
        assert!(matches!(
            reporter.update(Some(current.clone()), now + Duration::from_secs(6))[0].event,
            PlaybackEvent::Progress
        ));
        assert!(matches!(
            reporter.update(Some(current), now + Duration::from_secs(16))[0].event,
            PlaybackEvent::Progress
        ));
        let stopped = reporter.update(None, now + Duration::from_secs(17));
        assert!(matches!(stopped[0].event, PlaybackEvent::Stop));
        assert_eq!(stopped[0].playback.position_ms, 5000);
        assert!(
            reporter
                .update(None, now + Duration::from_secs(18))
                .is_empty()
        );
    }
    #[test]
    fn loading_and_paused_restore_do_not_start_sessions_and_account_changes_stop_old_first() {
        let now = Instant::now();
        let mut reporter = Reporter::default();
        let mut current = playback();
        current.loading = true;
        assert!(reporter.update(Some(current.clone()), now).is_empty());
        current.loading = false;
        current.playing = false;
        assert!(reporter.update(Some(current.clone()), now).is_empty());
        current.playing = true;
        reporter.update(Some(current.clone()), now);
        current.loading = true;
        assert!(
            reporter
                .update(Some(current.clone()), now + Duration::from_secs(1))
                .is_empty()
        );
        current.loading = false;
        current.source.user_id = "other".into();
        let reports = reporter.update(Some(current), now + Duration::from_secs(2));
        assert!(matches!(reports[0].event, PlaybackEvent::Stop));
        assert_eq!(reports[0].playback.source.user_id, "user");
        assert!(matches!(reports[1].event, PlaybackEvent::Start));
        assert_eq!(reports[1].playback.source.user_id, "other");
    }

    #[test]
    fn renaming_the_source_does_not_split_a_playback_reporting_session() {
        let mut reporter = Reporter::default();
        let now = Instant::now();
        let current = playback();
        assert!(matches!(
            reporter.update(Some(current.clone()), now)[0].event,
            PlaybackEvent::Start
        ));
        let mut renamed = current;
        renamed.source.nickname = "Upstairs".into();
        let reports = reporter.update(Some(renamed), now + Duration::from_secs(10));
        assert_eq!(reports.len(), 1);
        assert!(matches!(reports[0].event, PlaybackEvent::Progress));
        assert_eq!(reports[0].playback.source.nickname, "Upstairs");
    }
}
