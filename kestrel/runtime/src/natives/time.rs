//! The `time` module's natives (`driver/std/time.kes`), each `kestrel_time_<name>`.
//!
//! `now` counts from `start::started()`, which `kestrel_rt_start` fixes before `main` runs. None of
//! them stops the program: a clock before 1970 gives a negative `unix`, and a wait that is not a
//! positive number of seconds returns at once.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// `time.now() -> f64`: seconds since the program started, never going backwards.
#[no_mangle]
pub extern "C" fn kestrel_time_now() -> f64 {
    crate::start::started().elapsed().as_secs_f64()
}

/// `time.unix() -> i64`: whole seconds since the start of 1970 (UTC), by the system clock.
#[no_mangle]
pub extern "C" fn kestrel_time_unix() -> i64 {
    unix_seconds(SystemTime::now())
}

/// Whole seconds from 1970 to `at`, negative before it (rounded toward 1970).
fn unix_seconds(at: SystemTime) -> i64 {
    match at.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_secs()).unwrap_or(i64::MAX),
        Err(before) => i64::try_from(before.duration().as_secs()).map_or(i64::MIN, |s| -s),
    }
}

/// `time.sleep(seconds: f64)`: waits that long. Zero, a negative number or NaN returns at once,
/// and a wait too long to measure (infinity) waits as long as the system can.
#[no_mangle]
pub extern "C" fn kestrel_time_sleep(seconds: f64) {
    if let Some(wait) = wait_for(seconds) {
        std::thread::sleep(wait);
    }
}

/// How long `time.sleep(seconds)` waits, or `None` when it returns at once.
fn wait_for(seconds: f64) -> Option<Duration> {
    if seconds.is_nan() || seconds <= 0.0 {
        return None;
    }
    Some(Duration::try_from_secs_f64(seconds).unwrap_or(Duration::MAX))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant, UNIX_EPOCH};

    use super::{kestrel_time_now, kestrel_time_sleep, kestrel_time_unix, unix_seconds, wait_for};

    #[test]
    fn now_grows_and_sleep_waits_at_least_its_argument() {
        let t1 = kestrel_time_now();
        let wall = Instant::now();
        kestrel_time_sleep(0.02);
        let t2 = kestrel_time_now();
        assert!(wall.elapsed() >= Duration::from_millis(20));
        assert!(t1 >= 0.0 && t2 - t1 >= 0.02, "{t1} {t2}");
    }

    #[test]
    fn a_wait_that_is_not_positive_returns_at_once() {
        for seconds in [0.0, -0.0, -1.0, f64::NAN, -f64::NAN, f64::NEG_INFINITY] {
            assert_eq!(wait_for(seconds), None, "{seconds}");
        }
        let wall = Instant::now();
        kestrel_time_sleep(-5.0);
        kestrel_time_sleep(f64::NAN);
        assert!(wall.elapsed() < Duration::from_secs(1));
        assert_eq!(wait_for(1.5), Some(Duration::from_millis(1500)));
        assert_eq!(wait_for(f64::INFINITY), Some(Duration::MAX));
        assert_eq!(wait_for(1e300), Some(Duration::MAX));
    }

    #[test]
    fn unix_counts_whole_seconds_either_side_of_1970() {
        assert_eq!(unix_seconds(UNIX_EPOCH), 0);
        assert_eq!(unix_seconds(UNIX_EPOCH + Duration::from_millis(2500)), 2);
        assert_eq!(unix_seconds(UNIX_EPOCH - Duration::from_millis(2500)), -2);
        assert!(kestrel_time_unix() > 1_700_000_000);
    }
}
