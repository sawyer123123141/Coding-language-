use std::sync::atomic::AtomicU8;

use super::*;

fn run(count: i64, body: Body, env: *const u8, threads: usize) -> Result<(), String> {
    run_job(count, body, env, threads)
}

extern "C" fn mark(env: *const u8, start: i64, end: i64) {
    let seen = env as *const AtomicU8;
    for i in start..end {
        // SAFETY: the test's env holds `count` counters.
        unsafe { (*seen.add(i as usize)).fetch_add(1, Ordering::Relaxed) };
    }
}

#[test]
fn every_index_runs_once() {
    for threads in [1, 2, 4, 16] {
        for count in [0i64, 1, 7, 1000, 100_003] {
            let seen: Vec<AtomicU8> = (0..count).map(|_| AtomicU8::new(0)).collect();
            run(count, mark, seen.as_ptr() as *const u8, threads).unwrap();
            assert!(seen.iter().all(|s| s.load(Ordering::Relaxed) == 1), "threads {threads}, count {count}");
        }
    }
}

extern "C" fn nested(env: *const u8, start: i64, end: i64) {
    for _ in start..end {
        // An inner job runs in order on this thread.
        run_job(10, mark, env, 8).unwrap();
    }
}

#[test]
fn nested_jobs_run_inline() {
    let seen: Vec<AtomicU8> = (0..10).map(|_| AtomicU8::new(0)).collect();
    run(25, nested, seen.as_ptr() as *const u8, 4).unwrap();
    assert!(seen.iter().all(|s| s.load(Ordering::Relaxed) == 25));
}

extern "C" fn allocate(env: *const u8, start: i64, end: i64) {
    let slots = env as *mut usize;
    for i in start..end {
        let p = arena::kestrel_rt_alloc(8, 8, 8, 0, std::ptr::null()) as *mut i64;
        // SAFETY: `p` holds 8 i64s; the env holds a slot per index.
        unsafe {
            *p = i * 3;
            *slots.add(i as usize) = p as usize;
        }
    }
}

#[test]
fn allocations_survive_the_loop() {
    let mut slots = vec![0usize; 5000];
    run(5000, allocate, slots.as_mut_ptr() as *const u8, 8).unwrap();
    // The threads' arenas are freed by now; their blocks belong to this thread's.
    let filler: Vec<*mut u8> = (0..5000).map(|_| arena::kestrel_rt_alloc(8, 8, 8, 0, std::ptr::null())).collect();
    assert!(!filler.is_empty());
    for (i, &p) in slots.iter().enumerate() {
        // SAFETY: each slot points at an i64 that should still be alive.
        assert_eq!(unsafe { *(p as *const i64) }, i as i64 * 3);
    }
}

extern "C" fn fail_on_pool(_env: *const u8, _start: i64, _end: i64) {
    let pool_thread = CURRENT.with_borrow(|c| c.as_ref().is_some_and(|(_, p)| *p));
    if pool_thread {
        fail_in_job("iteration failed");
    }
    // The caller waits for a pool thread to fail.
    while !CURRENT.with_borrow(|c| c.as_ref().is_some_and(|(j, _)| j.failed.load(Ordering::Acquire))) {
        std::thread::yield_now();
    }
}

#[test]
fn pool_failures_come_back_to_the_caller() {
    assert_eq!(run(100, fail_on_pool, std::ptr::null(), 4), Err("iteration failed".to_string()));
    // The pool still works afterwards.
    let seen: Vec<AtomicU8> = (0..100).map(|_| AtomicU8::new(0)).collect();
    run(100, mark, seen.as_ptr() as *const u8, 4).unwrap();
    assert!(seen.iter().all(|s| s.load(Ordering::Relaxed) == 1));
}

#[cfg(target_arch = "x86_64")]
mod float_mode {
    use std::sync::atomic::AtomicU32;

    use super::*;
    use crate::fpu;

    /// Every participating thread leaves its float mode on round-toward-zero and flushing.
    extern "C" fn scramble(_env: *const u8, _start: i64, _end: i64) {
        fpu::set(0x7FC0);
    }

    extern "C" fn record(env: *const u8, start: i64, end: i64) {
        let modes = env as *const AtomicU32;
        for i in start..end {
            // SAFETY: the test's env holds a slot per index.
            unsafe { (*modes.add(i as usize)).store(fpu::read(), Ordering::Relaxed) };
        }
    }

    #[test]
    fn pool_threads_run_with_the_float_mode_pinned() {
        fpu::pin();
        fpu::mark_started();
        run(100, scramble, std::ptr::null(), 4).unwrap();
        fpu::pin();
        let modes: Vec<AtomicU32> = (0..100).map(|_| AtomicU32::new(0)).collect();
        run(100, record, modes.as_ptr() as *const u8, 4).unwrap();
        assert!(modes.iter().all(|m| m.load(Ordering::Relaxed) == 0x1F80));
    }

    static CALLER_MODE: AtomicU32 = AtomicU32::new(0);

    /// The caller records its mode; pool threads wait until it has, so it takes part for certain.
    extern "C" fn record_on_caller(_env: *const u8, _start: i64, _end: i64) {
        if CURRENT.with_borrow(|c| c.as_ref().is_some_and(|(_, pool)| *pool)) {
            while CALLER_MODE.load(Ordering::Acquire) == 0 {
                std::thread::yield_now();
            }
        } else {
            CALLER_MODE.store(fpu::read(), Ordering::Release);
        }
    }

    #[test]
    fn a_host_callers_float_mode_is_left_alone() {
        std::thread::spawn(|| {
            assert!(!fpu::started());
            fpu::set(0x7FC0);
            run(100, record_on_caller, std::ptr::null(), 4).unwrap();
            let after = fpu::read();
            fpu::pin();
            assert_eq!(CALLER_MODE.load(Ordering::Acquire), 0x7FC0);
            assert_eq!(after, 0x7FC0);
        })
        .join()
        .unwrap();
    }
}
