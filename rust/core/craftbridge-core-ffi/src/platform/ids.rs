//! The small ids the app carries a window back to us with (plan §20): between the JNI call that takes a
//! window in and `attach_window` taking it out — or `release` giving it up — the window sits in a table
//! under one of these ids, so a `jlong` is all the app's code ever holds.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// Live handles held for the app, each under an id: a positive number no other live entry has. Zero is
/// never handed out — it is the answer that there is no handle. An id taken out for an attach comes back
/// under itself when the attach failed, so the app can retry or release without holding a second id.
pub(crate) struct IdTable<T> {
    /// The live handles by the order they were first inserted: an id is its position here, plus one, and
    /// a handle taken out leaves its slot for the handle that comes back to it.
    slots: Mutex<Vec<Option<T>>>,
    /// One past the biggest id ever handed out: every id is below it, and slots grow as ids go.
    next_id: AtomicU64,
}

impl<T> IdTable<T> {
    /// An empty table: no id handed out, no handle held.
    pub(crate) const fn new() -> Self {
        Self { slots: Mutex::new(Vec::new()), next_id: AtomicU64::new(1) }
    }

    /// Hold `value` under an id no other live entry has and answer that id. When no further id can be
    /// named — an impossible event before a process could hold that many handles — the answer is zero and
    /// `value` is dropped, so nothing is held under an id the app was never given.
    pub(crate) fn insert(&self, value: T) -> i64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (Ok(answer), Some(slot)) = (i64::try_from(id), id.checked_sub(1).and_then(|below| usize::try_from(below).ok())) else {
            return 0;
        };
        let mut slots = self.lock();
        while slots.len() <= slot {
            slots.push(None);
        }
        match slots.get_mut(slot) {
            Some(entry) => {
                *entry = Some(value);
                answer
            }
            None => 0,
        }
    }

    /// Take the handle the id names out of the table, leaving its slot. An id that names none — never
    /// handed out, or already taken out — answers `None`.
    pub(crate) fn take(&self, id: i64) -> Option<T> {
        let slot = usize::try_from(id.checked_sub(1)?).ok()?;
        self.lock().get_mut(slot)?.take()
    }

    /// Put the handle the id named back under that same id, as a failed attach does so the app still holds
    /// what it held. It answers `false`, having taken `value` to give it up, when the table has no slot
    /// for that id.
    pub(crate) fn put(&self, id: i64, value: T) -> bool {
        let Some(below) = id.checked_sub(1) else {
            return false;
        };
        let Ok(slot) = usize::try_from(below) else {
            return false;
        };
        match self.lock().get_mut(slot) {
            Some(entry) => {
                *entry = Some(value);
                true
            }
            None => false,
        }
    }

    /// The table's slots, taken as they are even from a lock a panicked holder left: a poisoned lock here
    /// still holds its entries whole, and dropping them would lose app windows over an unrelated panic.
    fn lock(&self) -> MutexGuard<'_, Vec<Option<T>>> {
        self.slots.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::IdTable;

    /// An id is positive — zero is the answer that names nothing, so it is never handed out — and no two
    /// live entries carry the same one.
    #[test]
    fn ids_are_positive_and_distinct() {
        let table = IdTable::new();
        let first = table.insert("a");
        let second = table.insert("b");
        assert!(first > 0 && second > 0, "an id is never the zero that names nothing");
        assert_ne!(first, second);
        assert_eq!(table.take(first), Some("a"));
        assert_eq!(table.take(second), Some("b"));
    }

    /// Taking is what consumes an id: a second take of the same one answers `None`, an unknown id answers
    /// `None`, and zero — the no-id — is unknown in every table.
    #[test]
    fn an_id_names_its_entry_until_taken_and_names_nothing_else() {
        let table = IdTable::new();
        let id = table.insert("a");
        assert_eq!(table.take(id), Some("a"));
        assert_eq!(table.take(id), None, "a second take of an id gives nothing up");
        assert_eq!(table.take(id + 100), None);
        assert_eq!(table.take(0), None, "zero is the no-id in every table");
        assert_eq!(table.take(-3), None);
    }

    /// A failed attach puts its window back under the same id, so a retry — or the release after it —
    /// holds exactly what it held before; putting to an id no slot knows fails, and the handle is handed
    /// back to no one — dropped — rather than going in under some other id.
    #[test]
    fn a_failed_attach_gets_its_id_back() {
        let table = IdTable::new();
        let id = table.insert("a");
        assert_eq!(table.take(id), Some("a"));
        assert!(table.put(id, "a"));
        assert_eq!(table.take(id), Some("a"), "the same id serves the same window again");
        assert!(!table.put(id + 100, "lost"), "an id no slot knows takes nothing in");
        assert_eq!(table.take(id + 100), None, "and nothing went in under a neighbour");
    }
}
