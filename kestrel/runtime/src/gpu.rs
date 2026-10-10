//! GPU objects (textures, meshes) are owned by Kestrel values holding a
//! `GpuHandle`. Dropping one calls `kestrel_rt_gpu_drop` with its id, which the
//! runtime hands to the hook the window crate sets. 0 means no object.
use std::sync::OnceLock;

static DROP_HOOK: OnceLock<fn(i64)> = OnceLock::new();

/// Sets what dropping a GPU object does. The first hook set stays.
pub fn set_drop_hook(hook: fn(i64)) {
    let _ = DROP_HOOK.set(hook);
}

/// Called by compiled code when a value holding a `GpuHandle` is dropped. It may
/// run on any thread (a `par for` pool thread too), so a hook must be thread-safe.
#[no_mangle]
pub extern "C" fn kestrel_rt_gpu_drop(id: i64) {
    if id != 0 {
        if let Some(hook) = DROP_HOOK.get() {
            hook(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI64, Ordering};

    use super::*;

    static LAST: AtomicI64 = AtomicI64::new(0);
    static CALLS: AtomicI64 = AtomicI64::new(0);

    fn hook(id: i64) {
        LAST.store(id, Ordering::SeqCst);
        CALLS.fetch_add(1, Ordering::SeqCst);
    }

    #[test]
    fn dropping_hands_the_id_to_the_first_hook_and_ignores_zero() {
        kestrel_rt_gpu_drop(5); // no hook yet: nothing happens
        set_drop_hook(hook);
        set_drop_hook(|_| panic!("the first hook stays"));
        kestrel_rt_gpu_drop(0);
        assert_eq!(CALLS.load(Ordering::SeqCst), 0, "0 means no object");
        kestrel_rt_gpu_drop(7);
        assert_eq!((LAST.load(Ordering::SeqCst), CALLS.load(Ordering::SeqCst)), (7, 1));
    }
}
