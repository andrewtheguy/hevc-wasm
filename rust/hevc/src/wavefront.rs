//! Running a picture's work across threads: its coding tree block rows as a
//! wavefront, each two blocks behind the row above, and the in-loop filters'
//! bands side by side.
//!
//! The threads are rayon's pool when the `threads` feature is on and the
//! decoder was made with more than one; otherwise everything runs on the
//! caller, in order.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};

use crate::error::{Error, Result};

const FAILED: u32 = u32::MAX;

/// How far one row has got, in coding tree blocks, for the row below to wait
/// on. The store is a release and the load an acquire, which is what makes
/// the row's samples and maps visible to the reader (see `shared`).
pub struct Progress {
    done: AtomicU32,
    lock: Mutex<()>,
    cv: Condvar,
}

impl Default for Progress {
    fn default() -> Self {
        Progress { done: AtomicU32::new(0), lock: Mutex::new(()), cv: Condvar::new() }
    }
}

impl Progress {
    pub fn reset(&self) {
        self.done.store(0, Ordering::Relaxed);
    }

    /// `n` coding tree blocks are done.
    pub fn advance(&self, n: u32) {
        self.done.store(n, Ordering::Release);
        let _g = self.lock.lock();
        self.cv.notify_all();
    }

    /// The row will not finish: let whoever waits on it give up.
    pub fn fail(&self) {
        self.advance(FAILED);
    }

    /// Waits for `n` coding tree blocks; false if the row failed instead.
    pub fn wait_for(&self, n: u32) -> bool {
        let mut v = self.done.load(Ordering::Acquire);
        if v < n {
            let mut g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                v = self.done.load(Ordering::Acquire);
                if v >= n {
                    break;
                }
                g = self.cv.wait(g).unwrap_or_else(|e| e.into_inner());
            }
        }
        v != FAILED
    }
}

/// Runs `f` for rows `0..n` in order of claiming, on `threads` threads, each
/// row marking its progress as it goes and its failure if it fails. A row
/// that fails stops the rest, and the first error is returned.
pub fn run_rows(threads: usize, n: usize, progress: &[Progress], f: impl Fn(usize) -> Result<()> + Sync) -> Result<()> {
    for p in &progress[..n] {
        p.reset();
    }
    let worker = |next: &AtomicUsize, stop: &AtomicBool, first: &Mutex<Option<Error>>| loop {
        let row = next.fetch_add(1, Ordering::Relaxed);
        if row >= n || stop.load(Ordering::Relaxed) {
            // Rows this one would have decoded are not coming: say so to
            // whoever waits on them.
            if row < n {
                progress[row].fail();
            }
            if row >= n {
                return;
            }
            continue;
        }
        if let Err(e) = f(row) {
            progress[row].fail();
            stop.store(true, Ordering::Relaxed);
            first.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert(e);
        }
    };
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let first = Mutex::new(None);
    if threads > 1 {
        #[cfg(feature = "threads")]
        rayon::scope(|s| {
            for _ in 0..threads {
                s.spawn(|_| worker(&next, &stop, &first));
            }
        });
        #[cfg(not(feature = "threads"))]
        worker(&next, &stop, &first);
    } else {
        worker(&next, &stop, &first);
    }
    match first.into_inner().unwrap_or_else(|e| e.into_inner()) {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Runs `f` for `0..n`, independent of each other, on `threads` threads.
pub fn parallel_for(threads: usize, n: usize, f: impl Fn(usize) + Sync) {
    let next = AtomicUsize::new(0);
    let worker = || loop {
        let i = next.fetch_add(1, Ordering::Relaxed);
        if i >= n {
            return;
        }
        f(i);
    };
    if threads > 1 {
        #[cfg(feature = "threads")]
        rayon::scope(|s| {
            for _ in 0..threads {
                s.spawn(|_| worker());
            }
        });
        #[cfg(not(feature = "threads"))]
        worker();
    } else {
        worker();
    }
}
