//! Shared occurrence queue used by local playback providers.
//!
//! The rows are kept behind an `Arc` so a view can retain the exact snapshot
//! it drew while playback state changes. Mutations use copy-on-write, keeping
//! stale row actions from targeting a later occurrence by accident.

use super::RepeatMode;
use std::ops::Deref;
use std::sync::Arc;

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
    forward: bool,
    context_order: &[usize],
    repeat: RepeatMode,
) -> QueueTransition<T> {
    if repeat == RepeatMode::Track {
        return if forward {
            QueueTransition::Play(current)
        } else {
            QueueTransition::Restart
        };
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
            true,
            &[],
            RepeatMode::Off,
        );
        assert_eq!(first, QueueTransition::Play(cursor("queued", 0, true)));
        assert!(manual.is_empty());
        let second = transition(
            &tracks,
            cursor("queued", 0, true),
            &mut manual,
            &mut history,
            true,
            &[],
            RepeatMode::Off,
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
            true,
            &[2, 0, 1],
            RepeatMode::Off,
        );
        assert_eq!(shuffled, QueueTransition::Play(cursor("a", 0, false)));

        let mut history = PlaybackHistory::from(vec![cursor("queued", 2, true)]);
        let previous = transition(
            &tracks,
            cursor("b", 1, true),
            &mut manual,
            &mut history,
            false,
            &[],
            RepeatMode::Off,
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
            true,
            &[],
            RepeatMode::Track,
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
            true,
            &[],
            RepeatMode::Context,
        );
        assert_eq!(repeat_context, QueueTransition::Play(cursor("a", 0, false)));
    }
}
