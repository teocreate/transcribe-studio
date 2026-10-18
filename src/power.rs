//! Keeps Windows from sleeping while a long job is running.
//!
//! `SetThreadExecutionState` with `ES_CONTINUOUS` is scoped to the calling
//! thread and is reset when that thread exits, so the request is held by a
//! dedicated keeper thread that lives exactly as long as the lock.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};

use windows_sys::Win32::System::Power::SetThreadExecutionState;

const ES_AWAYMODE_REQUIRED: u32 = 0x0000_0040;
const ES_CONTINUOUS: u32 = 0x8000_0000;
const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;

/// Number of live `SleepBlocker` guards. Nested jobs keep the lock alive.
static HOLDERS: AtomicUsize = AtomicUsize::new(0);
/// Set to true when the keeper thread should drop the request and exit.
static RELEASE: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

/// RAII guard: while alive, the machine will not go to sleep on idle.
///
/// The display is deliberately *not* held awake — a multi-hour transcription
/// should not burn the screen, it just must not be suspended mid-run.
pub struct SleepBlocker {
    active: bool,
}

impl SleepBlocker {
    /// Acquires the lock. `enabled == false` produces an inert guard so callers
    /// can wrap a job unconditionally and let the setting decide.
    pub fn acquire(enabled: bool) -> Self {
        if !enabled {
            return Self { active: false };
        }
        if HOLDERS.fetch_add(1, Ordering::SeqCst) == 0 {
            *RELEASE.0.lock().unwrap() = false;
            std::thread::Builder::new()
                .name("sleep-blocker".into())
                .spawn(keeper)
                .ok();
        }
        Self { active: true }
    }

    /// True when this process currently holds a wake lock.
    pub fn is_active() -> bool {
        HOLDERS.load(Ordering::SeqCst) > 0
    }
}

impl Drop for SleepBlocker {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        if HOLDERS.fetch_sub(1, Ordering::SeqCst) == 1 {
            let (lock, cv) = &RELEASE;
            *lock.lock().unwrap() = true;
            cv.notify_all();
        }
    }
}

fn keeper() {
    // Away-mode keeps background work running even when the machine is told to
    // "sleep"; it is not honoured on every system, so fall back to the plain
    // system request rather than leaving no lock at all.
    let requested = unsafe {
        SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_AWAYMODE_REQUIRED)
    };
    if requested == 0 {
        unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
    }

    let (lock, cv) = &RELEASE;
    let mut released = lock.lock().unwrap();
    while !*released {
        released = cv.wait(released).unwrap();
    }

    unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
}
