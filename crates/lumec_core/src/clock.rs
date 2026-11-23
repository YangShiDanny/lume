//! Injectable wall-clock and monotonic-clock source.
//!
//! Lume builtins and JIT-registered builtins both read the same thread-local
//! clock. Tests can replace it with a fake clock and no longer need `sleep`.

use std::cell::RefCell;
use std::sync::{Arc, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub trait Clock {
    /// Unix epoch time in milliseconds.
    fn wall_ms(&self) -> i64;
    /// Monotonic time in milliseconds since an unspecified epoch.
    fn monotonic_ms(&self) -> i64;
}

struct SystemClock;

impl Clock for SystemClock {
    fn wall_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or(0)
    }

    fn monotonic_ms(&self) -> i64 {
        static EPOCH: OnceLock<Instant> = OnceLock::new();
        let epoch = *EPOCH.get_or_init(Instant::now);
        epoch.elapsed().as_millis() as i64
    }
}

thread_local! {
    static CURRENT_CLOCK: RefCell<Option<Arc<dyn Clock>>> = const {
        RefCell::new(None)
    };
}

fn current_clock() -> Arc<dyn Clock> {
    CURRENT_CLOCK.with(|clock| {
        clock
            .borrow()
            .clone()
            .unwrap_or_else(|| Arc::new(SystemClock))
    })
}

pub fn wall_ms() -> i64 {
    current_clock().wall_ms()
}

pub fn monotonic_ms() -> i64 {
    current_clock().monotonic_ms()
}

/// Runs `body` with `clock` installed for the current thread.
///
/// The previous clock is restored even if `body` panics.
pub fn with_clock<T>(clock: Arc<dyn Clock>, body: impl FnOnce() -> T) -> T {
    struct RestoreClock(Option<Arc<dyn Clock>>);

    impl Drop for RestoreClock {
        fn drop(&mut self) {
            CURRENT_CLOCK.with(|current| {
                *current.borrow_mut() = self.0.take();
            });
        }
    }

    let previous = CURRENT_CLOCK.with(|current| current.borrow_mut().replace(clock));
    let _restore = RestoreClock(previous);
    body()
}

/// # Safety
///
/// Must be called with the JIT builtin ABI: `_args` is either null or points to
/// `count` readable `i64` values. This builtin ignores the pointer and rejects
/// any non-zero `count`.
pub unsafe extern "C" fn now_ms_builtin(_args: *const i64, count: u64) -> i64 {
    if count != 0 {
        return -1;
    }
    wall_ms()
}

/// # Safety
///
/// Must be called with the JIT builtin ABI: `_args` is either null or points to
/// `count` readable `i64` values. This builtin ignores the pointer and rejects
/// any non-zero `count`.
pub unsafe extern "C" fn mono_ms_builtin(_args: *const i64, count: u64) -> i64 {
    if count != 0 {
        return -1;
    }
    monotonic_ms()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeClock {
        wall: i64,
        monotonic: i64,
    }

    impl Clock for FakeClock {
        fn wall_ms(&self) -> i64 {
            self.wall
        }

        fn monotonic_ms(&self) -> i64 {
            self.monotonic
        }
    }

    #[test]
    fn injected_clock_is_scoped_to_the_thread() {
        let before = wall_ms();
        let fake = Arc::new(FakeClock {
            wall: 1_700_000_000_123,
            monotonic: 4321,
        });
        with_clock(fake, || {
            assert_eq!(wall_ms(), 1_700_000_000_123);
            assert_eq!(monotonic_ms(), 4321);
            assert_eq!(
                unsafe { now_ms_builtin(std::ptr::null(), 0) },
                1_700_000_000_123
            );
            assert_eq!(unsafe { mono_ms_builtin(std::ptr::null(), 0) }, 4321);
        });
        assert!(wall_ms() >= before);
    }
}
