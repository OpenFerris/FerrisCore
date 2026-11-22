//! One generic owner-keyed registry, shared by the desk declaration stores.
//!
//! `workspaces` and `ui_extensions` each keep their app declarations in a
//! process-wide `OnceLock<RwLock<Vec<T>>>` plus a cheap `AtomicBool` gate, and
//! each had grown its own hand-written copy of that plumbing. This module owns
//! it once so the two stores cannot drift apart again.
//!
//! # Poisoning
//!
//! Every accessor recovers a poisoned lock with `into_inner()` instead of
//! re-panicking. A panic taken while a `OnceLock` initializer or an `RwLock`
//! critical section is held poisons the lock **for the whole process**, and
//! these registries are read from request paths (`GET /api/ui-extensions`,
//! `GET /api/modules`). Re-panicking through `.expect(...)` on every later read
//! would turn one transient failure into a permanently dead endpoint; taking
//! the inner value back keeps the store serving whatever entries were committed
//! before the poison. This is the reason the two stores no longer each guess at
//! a recovery policy — there is exactly one here, and it never panics.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{OnceLock, RwLock};

/// An append-mostly, process-wide registry of declarations.
///
/// `T` is the per-owner entry (`AppWorkspaces`, `AppUi`, …). The type is kept
/// deliberately small: it owns the storage, the single lock-recovery rule and
/// the cheap "has anything registered" flag. Each store keeps its own
/// registration *policy*, which is the seam that actually varies — replacing an
/// owner in place (`workspaces`) versus validating against the existing set
/// (`ui_extensions`) — so there is no shared trait and no `dyn` here.
pub struct OwnerRegistry<T> {
    entries: OnceLock<RwLock<Vec<T>>>,
    has_entries: AtomicBool,
}

impl<T> OwnerRegistry<T> {
    /// An empty registry. `const` so it can back a `static`.
    pub(crate) const fn new() -> Self {
        Self {
            entries: OnceLock::new(),
            has_entries: AtomicBool::new(false),
        }
    }

    fn lock(&self) -> &RwLock<Vec<T>> {
        self.entries.get_or_init(|| RwLock::new(Vec::new()))
    }

    /// Run `f` with exclusive access to the entries.
    ///
    /// A poisoned lock is recovered with [`into_inner`](std::sync::PoisonError::into_inner)
    /// rather than re-panicked: these stores sit on request paths, so turning a
    /// single poisoning panic into a permanent read failure is worse than
    /// reading the entries that were committed before it (see module docs).
    pub(crate) fn write<R>(&self, f: impl FnOnce(&mut Vec<T>) -> R) -> R {
        let mut entries = self
            .lock()
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut entries)
    }

    /// Run `f` with shared access to the entries (same poison recovery as
    /// [`Self::write`]).
    pub(crate) fn read<R>(&self, f: impl FnOnce(&[T]) -> R) -> R {
        let entries = self
            .lock()
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&entries)
    }

    /// A clone of every entry, read under one lock acquisition.
    pub(crate) fn snapshot(&self) -> Vec<T>
    where
        T: Clone,
    {
        self.read(<[T]>::to_vec)
    }

    /// Whether any entry has ever been registered — a cheap flag read that
    /// avoids taking the lock on the common read path.
    pub(crate) fn is_registered(&self) -> bool {
        self.has_entries.load(Ordering::Acquire)
    }

    /// Set the cheap flag after a successful registration.
    pub(crate) fn mark_registered(&self) {
        self.has_entries.store(true, Ordering::Release);
    }

    /// Whether the store currently holds no entries (takes the lock).
    ///
    /// Test-only introspection: production paths use [`Self::is_registered`],
    /// the cheap lock-free gate, and never need the exact length under lock.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.read(<[T]>::is_empty)
    }

    /// How many entries the store currently holds (takes the lock).
    ///
    /// Test-only — see [`Self::is_empty`].
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.read(<[T]>::len)
    }

    /// Drop every entry and clear the cheap flag. Test-only.
    #[cfg(test)]
    pub(crate) fn reset(&self) {
        self.write(Vec::clear);
        self.has_entries.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests;
