//! Shared occurrence queue used by local playback providers.
//!
//! The rows are kept behind an `Arc` so a view can retain the exact snapshot
//! it drew while playback state changes. Mutations use copy-on-write, keeping
//! stale row actions from targeting a later occurrence by accident.

use std::ops::Deref;
use std::sync::Arc;

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
}
