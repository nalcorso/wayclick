// SPDX-License-Identifier: MIT
use std::sync::{Mutex, MutexGuard};

/// Extension trait for [`Mutex`] that recovers from poison instead of panicking.
///
/// Peripheral mutexes (logger ring-buffer, event-bus subscriber list, device
/// tracking map, uinput file handle) should use [`lock_or_recover`] so that a
/// bug in one component does not cascade into a daemon-wide panic.
///
/// **Do not use this for the core [`Engine`] mutex.** If the engine panics
/// mid-mutation its fields may be inconsistent; the correct response there is a
/// clean process exit, not continued operation on corrupted state. The engine
/// lock therefore keeps `unwrap()` intentionally.
pub trait MutexExt<T> {
    /// Acquire the lock, recovering silently if the mutex is poisoned.
    ///
    /// When poison is detected the guard value is returned as-is. Rust's
    /// default panic hook has already written the original backtrace to
    /// stderr, so no additional logging is needed here.
    fn lock_or_recover(&self) -> MutexGuard<'_, T>;
}

impl<T> MutexExt<T> for Mutex<T> {
    fn lock_or_recover(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|p| p.into_inner())
    }
}
