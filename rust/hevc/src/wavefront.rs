//! Running a picture's rows across threads as a wavefront, each two coding
//! tree blocks behind the row above.
//!
//! The threads are rayon's pool when the `threads` feature is on and the
//! decoder was made with more than one; otherwise the rows run on the caller,
//! in order.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};

use crate::error::{Error, Result};

const FAILED: u32 = u32::MAX;

/// How far one row has got, in coding tree blocks, for the row below to wait
/// on. The store and the loads are sequentially consistent, which is what
/// makes the row's samples and maps visible to the reader (see `shared`) and
/// what lets an advance skip the notify when no one waits.
pub struct Progress {
    done: AtomicU32,
    waiters: AtomicU32,
    lock: Mutex<()>,
    cv: Condvar,
}

impl Default for Progress {
    fn default() -> Self {
        Progress { done: AtomicU32::new(0), waiters: AtomicU32::new(0), lock: Mutex::new(()), cv: Condvar::new() }
    }
}

impl Progress {
    pub fn reset(&self) {
        self.done.store(0, Ordering::SeqCst);
    }

    /// `n` coding tree blocks are done.
    pub fn advance(&self, n: u32) {
        self.done.store(n, Ordering::SeqCst);
        if self.waiters.load(Ordering::SeqCst) > 0 {
            let _g = self.lock.lock();
            self.cv.notify_all();
        }
    }

    /// The row will not finish: let whoever waits on it give up.
    pub fn fail(&self) {
        self.advance(FAILED);
    }

    /// Waits for `n` coding tree blocks; false if the row failed instead.
    ///
    /// The rows run in step, so the block is usually there or about to be: the
    /// wait looks a few hundred times before it sleeps, which is a fraction of
    /// a microsecond, and sleeps rather than spin for the rest.
    pub fn wait_for(&self, n: u32) -> bool {
        let mut v = self.done.load(Ordering::SeqCst);
        let mut spins = 0;
        while v < n && spins < 256 {
            std::hint::spin_loop();
            spins += 1;
            v = self.done.load(Ordering::SeqCst);
        }
        if v < n {
            self.waiters.fetch_add(1, Ordering::SeqCst);
            let mut g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                v = self.done.load(Ordering::SeqCst);
                if v >= n {
                    break;
                }
                g = self.cv.wait(g).unwrap_or_else(|e| e.into_inner());
            }
            drop(g);
            self.waiters.fetch_sub(1, Ordering::SeqCst);
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
