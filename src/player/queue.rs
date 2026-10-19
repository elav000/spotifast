//! Shared occurrence queue used by local playback providers.
//!
//! The rows are kept behind an `Arc` so a view can retain the exact snapshot
//! it drew while playback state changes. Mutations use copy-on-write, keeping
//! stale row actions from targeting a later occurrence by accident.

use super::RepeatMode;
use std::ops::Deref;
use std::sync::Arc;

/// Stable context row order used by local playback shuffle and queue display.
/// The serialized form remains a plain array of context indices.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct PlaybackOrder(Vec<usize>);

impl PlaybackOrder {
    pub fn for_context(len: usize, current: usize, shuffle: bool) -> Self {
        let mut order: Vec<_> = (0..len).collect();
        if shuffle {
            order.retain(|index| *index != current);
            use rand::seq::SliceRandom;
            order.shuffle(&mut rand::rng());
            if current < len {
                order.insert(0, current);
            }
        }
        Self(order)
    }

    pub fn is_valid_for(&self, len: usize) -> bool {
        if self.0.len() != len {
            return false;
        }
        let mut seen = vec![false; len];
        self.0.iter().all(|index| {
            if *index >= len || seen[*index] {
                false
            } else {
                seen[*index] = true;
                true
            }
        })
    }

    pub fn rotate_to(&mut self, index: usize) -> bool {
        let Some(position) = self.0.iter().position(|row| *row == index) else {
            return false;
        };
        self.0.rotate_left(position);
        true
    }

    pub fn into_vec(self) -> Vec<usize> {
        self.0
    }
}

impl Deref for PlaybackOrder {
    type Target = [usize];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<Vec<usize>> for PlaybackOrder {
    fn from(order: Vec<usize>) -> Self {
        Self(order)
    }
}

/// The ordered tracks and active position for a local playback context.
/// Serde field names preserve the existing flattened Jellyfin session shape.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(bound(deserialize = "T: serde::Deserialize<'de>"))]
pub struct LocalPlaybackContext<T> {
    #[serde(default)]
    pub tracks: Vec<T>,
    #[serde(default)]
    pub index: usize,
    #[serde(default)]
    #[serde(rename = "manual")]
    pub current_is_manual: bool,
    #[serde(default)]
    #[serde(rename = "shuffle_order")]
    pub order: PlaybackOrder,
}

impl<T> Default for LocalPlaybackContext<T> {
    fn default() -> Self {
        Self {
            tracks: Vec::new(),
            index: 0,
            current_is_manual: false,
            order: PlaybackOrder::default(),
        }
    }
}

impl<T> LocalPlaybackContext<T> {
    pub fn new(
        tracks: Vec<T>,
        index: usize,
        current_is_manual: bool,
        order: PlaybackOrder,
    ) -> Self {
        Self {
            tracks,
            index,
            current_is_manual,
            order,
        }
    }

    pub fn set_cursor(&mut self, index: usize, manual: bool) {
        self.index = index;
        self.current_is_manual = manual;
    }

    pub fn set_tracks(&mut self, tracks: Vec<T>) {
        self.tracks = tracks;
    }
}

impl<T: Clone> LocalPlaybackContext<T> {
    pub fn cursor(&self, item: T) -> PlaybackCursor<T> {
        PlaybackCursor {
            item,
            index: self.index,
            manual: self.current_is_manual,
        }
    }
}

/// Provider-neutral state owned by a local playback controller. Provider
/// adapters still resolve and execute the selected item; this value owns the
/// local context, manual occurrences and Previous history together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalPlaybackController<T> {
    pub context: LocalPlaybackContext<T>,
    pub manual: ManualQueue<T>,
    pub history: PlaybackHistory<T>,
}

impl<T> Default for LocalPlaybackController<T> {
    fn default() -> Self {
        Self {
            context: LocalPlaybackContext::default(),
            manual: ManualQueue::default(),
            history: PlaybackHistory::default(),
        }
    }
}

impl<T: Clone> LocalPlaybackController<T> {
    pub fn transition(
        &mut self,
        current: PlaybackCursor<T>,
        policy: TransitionPolicy<'_>,
    ) -> QueueTransition<T> {
        transition(
            &self.context.tracks,
            current,
            &mut self.manual,
            &mut self.history,
            policy,
        )
    }
}

/// A playback location that can be revisited by the Previous command.
///
/// Context rows use their stable position in the context. Manually queued
/// rows retain that same context position so returning from them resumes the
/// album or playlist after the song that was interrupted.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlaybackCursor<T> {
    pub item: T,
    pub index: usize,
    pub manual: bool,
}

/// Bounded backward-playback history shared by local provider controllers.
///
/// This is distinct from Recent history, which is a user-facing record of
/// played items rather than a stack used to implement Previous.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct PlaybackHistory<T> {
    cursors: Vec<PlaybackCursor<T>>,
}

impl<T> Default for PlaybackHistory<T> {
    fn default() -> Self {
        Self {
            cursors: Vec::new(),
        }
    }
}

impl<T> PlaybackHistory<T> {
    pub const LIMIT: usize = 1000;

    pub fn as_slice(&self) -> &[PlaybackCursor<T>] {
        &self.cursors
    }

    pub fn len(&self) -> usize {
        self.cursors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cursors.is_empty()
    }

    pub fn push(&mut self, cursor: PlaybackCursor<T>) {
        if self.cursors.len() == Self::LIMIT {
            self.cursors.remove(0);
        }
        self.cursors.push(cursor);
    }

    pub fn pop(&mut self) -> Option<PlaybackCursor<T>> {
        self.cursors.pop()
    }

    pub fn clear(&mut self) {
        self.cursors.clear();
    }

    pub fn into_vec(self) -> Vec<PlaybackCursor<T>> {
        self.cursors
    }
}

impl<T> From<Vec<PlaybackCursor<T>>> for PlaybackHistory<T> {
    fn from(cursors: Vec<PlaybackCursor<T>>) -> Self {
        let start = cursors.len().saturating_sub(Self::LIMIT);
        Self {
            cursors: cursors.into_iter().skip(start).collect(),
        }
    }
}

/// The result of asking the local queue policy to move in either direction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueTransition<T> {
    Play(PlaybackCursor<T>),
    Restart,
    Finished,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransitionPolicy<'a> {
    pub forward: bool,
    pub natural_end: bool,
    pub context_order: &'a [usize],
    pub repeat: RepeatMode,
}

impl<'a> TransitionPolicy<'a> {
    pub fn next(context_order: &'a [usize], repeat: RepeatMode) -> Self {
        Self {
            forward: true,
            natural_end: false,
            context_order,
            repeat,
        }
    }

    pub fn ended(context_order: &'a [usize], repeat: RepeatMode) -> Self {
        Self {
            forward: true,
            natural_end: true,
            context_order,
            repeat,
        }
    }

    pub fn previous(context_order: &'a [usize], repeat: RepeatMode) -> Self {
        Self {
            forward: false,
            natural_end: false,
            context_order,
            repeat,
        }
    }
}

/// Apply provider-neutral Next/Previous ordering rules to a local queue.
///
/// `context_order` is a permutation of the context's stable row indices when
/// shuffle is enabled, or an empty slice for normal order. The provider still
/// resolves and plays the returned item through its own execution adapter.
pub fn transition<T: Clone>(
    tracks: &[T],
    current: PlaybackCursor<T>,
    manual: &mut ManualQueue<T>,
    history: &mut PlaybackHistory<T>,
    policy: TransitionPolicy<'_>,
) -> QueueTransition<T> {
    let TransitionPolicy {
        forward,
        natural_end,
        context_order,
        repeat,
    } = policy;
    if repeat == RepeatMode::Track {
        if forward && natural_end {
            return QueueTransition::Play(current);
        }
        if !forward {
            return QueueTransition::Restart;
        }
    }

    let order = if context_order.is_empty() {
        None
    } else {
        Some(context_order)
    };
    let context_index = |position: usize| -> Option<usize> {
        order
            .and_then(|order| order.get(position).copied())
            .or_else(|| order.is_none().then_some(position))
            .filter(|index| *index < tracks.len())
    };
    let current_position = order
        .and_then(|order| order.iter().position(|index| *index == current.index))
        .or_else(|| order.is_none().then_some(current.index));

    let target = if forward {
        if let Some(item) = manual.pop_front() {
            Some(PlaybackCursor {
                item,
                index: current.index,
                manual: true,
            })
        } else {
            current_position
                .and_then(|position| context_index(position.saturating_add(1)))
                .or_else(|| {
                    (repeat == RepeatMode::Context)
                        .then(|| context_index(0))
                        .flatten()
                })
                .and_then(|index| {
                    tracks.get(index).cloned().map(|item| PlaybackCursor {
                        item,
                        index,
                        manual: false,
                    })
                })
        }
    } else {
        history.pop().or_else(|| {
            current_position
                .and_then(|position| position.checked_sub(1))
                .and_then(context_index)
                .and_then(|index| {
                    tracks.get(index).cloned().map(|item| PlaybackCursor {
                        item,
                        index,
                        manual: false,
                    })
                })
        })
    };

    if let Some(target) = target {
        if forward {
            history.push(current);
        } else if current.manual {
            manual.insert(0, current.item);
        }
        QueueTransition::Play(target)
    } else if forward {
        QueueTransition::Finished
    } else {
        QueueTransition::Restart
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManualQueue<T> {
    rows: Arc<Vec<T>>,
}

impl<T> Default for ManualQueue<T> {
    fn default() -> Self {
        Self {
            rows: Arc::default(),
        }
    }
}

impl<T: Clone> ManualQueue<T> {
    pub fn as_slice(&self) -> &[T] {
        self.rows.as_slice()
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    pub fn snapshot(&self) -> Arc<Vec<T>> {
        Arc::clone(&self.rows)
    }

    pub fn is_snapshot(&self, snapshot: &Arc<Vec<T>>) -> bool {
        Arc::ptr_eq(&self.rows, snapshot)
    }

    pub fn push(&mut self, row: T) {
        Arc::make_mut(&mut self.rows).push(row);
    }

    pub fn extend(&mut self, rows: impl IntoIterator<Item = T>) {
        Arc::make_mut(&mut self.rows).extend(rows);
    }

    pub fn insert(&mut self, index: usize, row: T) {
        Arc::make_mut(&mut self.rows).insert(index, row);
    }

    pub fn remove(&mut self, index: usize) -> T {
        Arc::make_mut(&mut self.rows).remove(index)
    }

    pub fn pop_front(&mut self) -> Option<T> {
        (!self.is_empty()).then(|| self.remove(0))
    }

    pub fn remove_through(&mut self, index: usize) -> bool {
        if index >= self.len() {
            return false;
        }
        drop(Arc::make_mut(&mut self.rows).drain(..=index));
        true
    }

    pub fn clear(&mut self) {
        self.rows = Arc::default();
    }

    /// Move a row to its final index, preserving duplicate occurrences.
    pub fn move_to(&mut self, from: usize, to: usize) -> bool {
        if from >= self.len() || to >= self.len() || from == to {
            return false;
        }
        let rows = Arc::make_mut(&mut self.rows);
        let row = rows.remove(from);
        rows.insert(to, row);
        true
    }

    /// Remove several row occurrences as one action. Invalid indices reject
    /// the whole edit; repeated indices are treated as one selected row.
    pub fn remove_indices(&mut self, indices: &[usize]) -> bool {
        if indices.iter().any(|index| *index >= self.len()) {
            return false;
        }
        let mut indices = indices.to_vec();
        indices.sort_unstable();
        indices.dedup();
        let rows = Arc::make_mut(&mut self.rows);
        for index in indices.into_iter().rev() {
            rows.remove(index);
        }
        true
    }

    pub fn into_vec(self) -> Vec<T>
    where
        T: Clone,
    {
        Arc::unwrap_or_clone(self.rows)
    }
}

impl<T> Deref for ManualQueue<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        self.rows.as_slice()
    }
}

impl<T> AsRef<[T]> for ManualQueue<T> {
    fn as_ref(&self) -> &[T] {
        self.rows.as_slice()
    }
}

impl<T> From<Vec<T>> for ManualQueue<T> {
    fn from(rows: Vec<T>) -> Self {
        Self {
            rows: Arc::new(rows),
        }
    }
}

impl<T> FromIterator<T> for ManualQueue<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self::from(iter.into_iter().collect::<Vec<_>>())
    }
}

impl<'a, T> IntoIterator for &'a ManualQueue<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<T: Clone> IntoIterator for ManualQueue<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_vec().into_iter()
    }
}

impl<T: PartialEq<U>, U> PartialEq<Vec<U>> for ManualQueue<T> {
    fn eq(&self, other: &Vec<U>) -> bool {
        self.as_ref() == other.as_slice()
    }
}

impl<T: PartialEq<U>, U> PartialEq<[U]> for ManualQueue<T> {
    fn eq(&self, other: &[U]) -> bool {
        self.as_ref() == other
    }
}

impl<T: PartialEq<U>, U, const N: usize> PartialEq<[U; N]> for ManualQueue<T> {
    fn eq(&self, other: &[U; N]) -> bool {
        self.as_ref() == other.as_slice()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_order_keeps_a_permutation_and_serializes_as_the_existing_array() {
        let mut order = PlaybackOrder::for_context(5, 2, true);
        assert!(order.is_valid_for(5));
        assert_eq!(order.first(), Some(&2));
        assert!(order.rotate_to(4));
        assert_eq!(order.first(), Some(&4));
        assert!(order.is_valid_for(5));
        assert!(!order.rotate_to(9));

        let json = serde_json::to_string(&order).unwrap();
        assert!(json.starts_with('['));
        assert_eq!(serde_json::from_str::<PlaybackOrder>(&json).unwrap(), order);
        assert!(!PlaybackOrder::from(vec![0, 0]).is_valid_for(2));
    }

    fn cursor(item: &'static str, index: usize, manual: bool) -> PlaybackCursor<&'static str> {
        PlaybackCursor {
            item,
            index,
            manual,
        }
    }

    #[test]
    fn duplicate_rows_keep_occurrence_order_and_old_snapshots() {
        let mut queue = ManualQueue::from(vec!["a", "b", "a"]);
        let old = queue.snapshot();
        assert!(queue.move_to(2, 0));
        assert_eq!(queue.as_ref(), &["a", "a", "b"]);
        assert_eq!(old.as_slice(), &["a", "b", "a"]);
        assert!(!queue.is_snapshot(&old));
    }

    #[test]
    fn selected_removal_is_atomic_and_removes_only_selected_occurrences() {
        let mut queue = ManualQueue::from(vec!["a", "b", "a", "c"]);
        let old = queue.snapshot();
        assert!(!queue.remove_indices(&[0, 9]));
        assert_eq!(queue.as_ref(), &["a", "b", "a", "c"]);
        assert!(queue.remove_indices(&[0, 2, 2]));
        assert_eq!(queue.as_ref(), &["b", "c"]);
        assert_eq!(old.as_slice(), &["a", "b", "a", "c"]);
    }

    #[test]
    fn playback_history_is_a_bounded_lifo_of_occurrences() {
        let mut history = PlaybackHistory::default();
        history.push(PlaybackCursor {
            item: "same",
            index: 1,
            manual: false,
        });
        history.push(PlaybackCursor {
            item: "same",
            index: 1,
            manual: true,
        });

        assert!(history.pop().unwrap().manual);
        assert!(!history.pop().unwrap().manual);
        assert!(history.pop().is_none());

        let mut bounded = PlaybackHistory::default();
        for index in 0..=PlaybackHistory::<usize>::LIMIT {
            bounded.push(PlaybackCursor {
                item: index,
                index,
                manual: false,
            });
        }
        assert_eq!(bounded.as_slice().len(), PlaybackHistory::<usize>::LIMIT);
        assert_eq!(bounded.as_slice()[0].item, 1);
        assert_eq!(bounded.pop().unwrap().item, PlaybackHistory::<usize>::LIMIT);

        let persisted = PlaybackHistory::from(vec![PlaybackCursor {
            item: "same".to_owned(),
            index: 3,
            manual: true,
        }]);
        let json = serde_json::to_string(&persisted).unwrap();
        assert_eq!(json, r#"[{"item":"same","index":3,"manual":true}]"#);
        assert_eq!(
            serde_json::from_str::<PlaybackHistory<String>>(&json).unwrap(),
            persisted
        );
    }

    #[test]
    fn next_prioritizes_manual_rows_then_continues_the_context() {
        let tracks = ["a", "b", "c"];
        let mut manual = ManualQueue::from(vec!["queued"]);
        let mut history = PlaybackHistory::default();
        let first = transition(
            &tracks,
            cursor("a", 0, false),
            &mut manual,
            &mut history,
            TransitionPolicy::next(&[], RepeatMode::Off),
        );
        assert_eq!(first, QueueTransition::Play(cursor("queued", 0, true)));
        assert!(manual.is_empty());
        let second = transition(
            &tracks,
            cursor("queued", 0, true),
            &mut manual,
            &mut history,
            TransitionPolicy::next(&[], RepeatMode::Off),
        );
        assert_eq!(second, QueueTransition::Play(cursor("b", 1, false)));
        assert_eq!(
            history.as_slice(),
            &[cursor("a", 0, false), cursor("queued", 0, true)]
        );
    }

    #[test]
    fn previous_restores_manual_current_and_shuffle_uses_context_positions() {
        let tracks = ["a", "b", "c"];
        let mut manual = ManualQueue::default();
        let mut history = PlaybackHistory::default();
        let shuffled = transition(
            &tracks,
            cursor("c", 2, false),
            &mut manual,
            &mut history,
            TransitionPolicy::next(&[2, 0, 1], RepeatMode::Off),
        );
        assert_eq!(shuffled, QueueTransition::Play(cursor("a", 0, false)));

        let mut history = PlaybackHistory::from(vec![cursor("queued", 2, true)]);
        let previous = transition(
            &tracks,
            cursor("b", 1, true),
            &mut manual,
            &mut history,
            TransitionPolicy::previous(&[], RepeatMode::Off),
        );
        assert_eq!(previous, QueueTransition::Play(cursor("queued", 2, true)));
        assert_eq!(manual.as_ref(), &["b"]);
    }

    #[test]
    fn repeat_modes_wrap_context_or_replay_current_without_consuming_manual() {
        let tracks = ["a", "b"];
        let mut history = PlaybackHistory::default();
        let mut manual = ManualQueue::from(vec!["queued"]);
        let repeat_track = transition(
            &tracks,
            cursor("b", 1, false),
            &mut manual,
            &mut history,
            TransitionPolicy::ended(&[], RepeatMode::Track),
        );
        assert_eq!(repeat_track, QueueTransition::Play(cursor("b", 1, false)));
        assert_eq!(manual.as_ref(), &["queued"]);
        assert!(history.as_slice().is_empty());

        manual.clear();
        let repeat_context = transition(
            &tracks,
            cursor("b", 1, false),
            &mut manual,
            &mut history,
            TransitionPolicy::ended(&[], RepeatMode::Context),
        );
        assert_eq!(repeat_context, QueueTransition::Play(cursor("a", 0, false)));
    }

    #[test]
    fn next_skips_repeat_track_while_end_of_track_repeats_it() {
        let tracks = ["a", "b"];
        let mut history = PlaybackHistory::default();
        let mut manual = ManualQueue::from(vec!["queued"]);
        let skipped = transition(
            &tracks,
            cursor("a", 0, false),
            &mut manual,
            &mut history,
            TransitionPolicy::next(&[], RepeatMode::Track),
        );
        assert_eq!(skipped, QueueTransition::Play(cursor("queued", 0, true)));
        let history_len = history.len();

        let repeated = transition(
            &tracks,
            cursor("a", 0, false),
            &mut manual,
            &mut history,
            TransitionPolicy::ended(&[], RepeatMode::Track),
        );
        assert_eq!(repeated, QueueTransition::Play(cursor("a", 0, false)));
        assert_eq!(history.len(), history_len);
    }
}
