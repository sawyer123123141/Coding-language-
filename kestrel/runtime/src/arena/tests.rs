//! Tests of the arenas, the arena stack and list growth.

use super::*;

/// Tests that make or free counted arenas hold this, so each can read the count without the
/// others changing it.
static COUNTED: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn allocations_are_aligned_zeroed_and_distinct() {
    let mut a = Arena::new();
    let p = a.alloc(3, 1);
    let q = a.alloc(8, 8);
    assert_eq!(q as usize % 8, 0);
    assert!(q as usize >= p as usize + 3);
    // SAFETY: both were just allocated with these sizes.
    unsafe {
        assert_eq!(std::slice::from_raw_parts(q, 8), &[0; 8]);
        *p = 7;
    }
    let big = a.alloc(BLOCK * 2, 16);
    assert_eq!(big as usize % 16, 0);
    // The small block's free space survives a big request.
    let r = a.alloc(4, 4);
    assert!(r as usize > q as usize && (r as usize) < q as usize + BLOCK);
}

#[test]
fn freed_blocks_are_reused_and_still_come_back_zeroed() {
    let first = {
        let mut a = Arena::with_poison(false);
        let p = a.alloc(BLOCK, 16);
        // SAFETY: `p` holds `BLOCK` bytes.
        unsafe { std::ptr::write_bytes(p, 0xAB, BLOCK) };
        p
    };
    let mut b = Arena::with_poison(false);
    let q = b.alloc(BLOCK, 16);
    assert_eq!(q, first, "the freed block is reused");
    // SAFETY: `q` holds `BLOCK` bytes.
    assert!(unsafe { std::slice::from_raw_parts(q, BLOCK) }.iter().all(|&x| x == 0));
}

#[test]
fn a_poisoned_arena_fills_its_freed_blocks_with_db_and_keeps_them() {
    let (p, big) = {
        let mut a = Arena::with_poison(true);
        let p = a.alloc(100, 1);
        // SAFETY: `p` holds 100 bytes.
        unsafe { std::ptr::write_bytes(p, 1, 100) };
        let big = a.alloc(BLOCK * 2, 16);
        // SAFETY: `big` holds `BLOCK * 2` bytes.
        unsafe { std::ptr::write_bytes(big, 1, BLOCK * 2) };
        (p, big)
    };
    // SAFETY: a poisoned arena leaves its blocks allocated, so they can still be read.
    unsafe {
        assert!(std::slice::from_raw_parts(p, 100).iter().all(|&x| x == 0xDB), "the small block is poisoned");
        assert!(std::slice::from_raw_parts(big, BLOCK * 2).iter().all(|&x| x == 0xDB), "and the big one");
    }
    // And neither goes to the next arena, poisoned or not.
    for poison in [true, false] {
        let mut b = Arena::with_poison(poison);
        let (q, r) = (b.alloc(100, 1), b.alloc(BLOCK * 2, 16));
        assert_ne!(q, p);
        assert_ne!(r, big);
    }
}

#[test]
fn a_poisoned_arena_counts_its_bytes_like_any_other() {
    let (mut poisoned, mut plain) = (Arena::with_poison(true), Arena::with_poison(false));
    for a in [&mut poisoned, &mut plain] {
        a.alloc(10, 1);
        a.alloc(BLOCK * 2, 16);
    }
    assert_eq!(poisoned.bytes(), plain.bytes());
}

#[test]
fn bytes_counts_every_block_including_adopted_ones() {
    let mut a = Arena::new();
    assert_eq!(a.bytes(), 0);
    a.alloc(10, 1);
    assert_eq!(a.bytes(), BLOCK);
    a.alloc(BLOCK * 2, 16);
    assert_eq!(a.bytes(), BLOCK * 3);
    let mut child = Box::new(Arena::new());
    child.alloc(10, 1);
    a.adopt(child);
    assert_eq!(a.bytes(), BLOCK * 4);
}

#[test]
fn many_small_allocations_span_blocks() {
    let mut a = Arena::new();
    for _ in 0..10_000 {
        let p = a.alloc(24, 8);
        assert_eq!(p as usize % 8, 0);
    }
    assert!(a.blocks.len() >= 3);
}

#[test]
fn list_grow_keeps_elements() {
    let _counted = COUNTED.lock().unwrap_or_else(|e| e.into_inner());
    let mut l = List { ptr: std::ptr::null_mut(), len: 0, cap: 0, arena: std::ptr::null_mut() };
    // SAFETY: `l` is a valid list of i32s throughout.
    unsafe {
        // Growing inside a temporary arena still uses the program arena.
        let temp = kestrel_rt_arena_new();
        kestrel_rt_arena_push(temp);
        kestrel_rt_list_grow(&mut l, 4, 4);
        kestrel_rt_arena_pop();
        kestrel_rt_arena_free(temp);
        assert_eq!(l.cap, 4);
        assert_eq!(l.arena, program_arena());
        for i in 0..4 {
            *(l.ptr as *mut i32).add(i) = i as i32 * 10;
        }
        l.len = 4;
        kestrel_rt_list_grow(&mut l, 4, 4);
        assert_eq!(l.cap, 8);
        assert_eq!(std::slice::from_raw_parts(l.ptr as *const i32, 4), &[0, 10, 20, 30]);
    }
}

#[test]
fn pushed_arenas_take_allocations_until_popped() {
    let _counted = COUNTED.lock().unwrap_or_else(|e| e.into_inner());
    let program = kestrel_rt_current_arena();
    let a = kestrel_rt_arena_new();
    kestrel_rt_arena_push(a);
    assert_eq!(kestrel_rt_current_arena(), a);
    let p = kestrel_rt_alloc(4, 8, 8, 0, std::ptr::null());
    // SAFETY: `a` is live and was just used.
    assert!(unsafe { (*a).blocks.iter().any(|(b, l)| (*b as usize..*b as usize + l.size()).contains(&(p as usize))) });
    assert_eq!(kestrel_rt_arena_pop(), a);
    assert_eq!(kestrel_rt_current_arena(), program);
    // SAFETY: `a` came from `kestrel_rt_arena_new` and is no longer pushed.
    unsafe {
        kestrel_rt_arena_free(a);
        kestrel_rt_arena_free(std::ptr::null_mut());
    }
}

#[test]
fn dup_copies_into_the_current_arena() {
    let src = [1u8, 2, 3, 4, 5];
    // SAFETY: `src` holds 5 bytes.
    let p = unsafe { kestrel_rt_dup(src.as_ptr(), 5, 1) };
    assert_ne!(p as *const u8, src.as_ptr());
    // SAFETY: `p` was just filled with 5 bytes.
    assert_eq!(unsafe { std::slice::from_raw_parts(p, 5) }, &src);
}

#[test]
fn the_live_arena_count_goes_up_on_new_and_down_on_free() {
    let _counted = COUNTED.lock().unwrap_or_else(|e| e.into_inner());
    let before = arenas_alive();
    let (a, b) = (kestrel_rt_arena_new(), kestrel_rt_arena_new());
    assert_eq!(arenas_alive(), before + 2);
    // SAFETY: both came from `kestrel_rt_arena_new` and aren't pushed.
    unsafe {
        kestrel_rt_arena_free(a);
        assert_eq!(arenas_alive(), before + 1);
        // Freeing null frees nothing, so it counts nothing.
        kestrel_rt_arena_free(std::ptr::null_mut());
        assert_eq!(arenas_alive(), before + 1);
        kestrel_rt_arena_free(b);
    }
    assert_eq!(arenas_alive(), before);
    // The runtime's own arenas (a `par for` thread's) aren't counted.
    let own = new_uncounted();
    assert_eq!(arenas_alive(), before);
    // SAFETY: `own` is a live arena nothing else holds.
    drop(unsafe { Box::from_raw(own) });
    assert_eq!(arenas_alive(), before);
}

#[test]
fn a_list_grows_by_doubling_up_to_the_item_limit() {
    assert_eq!(grown_capacity(0), Some(4));
    assert_eq!(grown_capacity(3), Some(6));
    assert_eq!(grown_capacity(4), Some(8));
    assert_eq!(grown_capacity(MAX_ITEMS / 2), Some(MAX_ITEMS - 1));
    assert_eq!(grown_capacity(MAX_ITEMS / 2 + 1), Some(MAX_ITEMS));
    assert_eq!(grown_capacity(MAX_ITEMS - 1), Some(MAX_ITEMS));
    assert_eq!(grown_capacity(MAX_ITEMS), None);
}

#[test]
fn lists_and_arrays_fit_up_to_the_item_limit_and_no_further() {
    assert_eq!(MAX_ITEMS, 2_147_483_647);
    assert!(items_fit(0));
    assert!(items_fit(MAX_ITEMS));
    assert!(!items_fit(MAX_ITEMS + 1));
    assert!(!items_fit(-1));
}

/// What the child process of `going_past_the_item_limit_fails` does, named by this variable.
const LIMIT_CHILD: &str = "KESTREL_LIMIT_CHILD";

/// Child half of `going_past_the_item_limit_fails`; does nothing unless the parent set `LIMIT_CHILD`.
#[test]
fn item_limit_child() {
    let Some(what) = std::env::var_os(LIMIT_CHILD) else { return };
    let mut full = List { ptr: std::ptr::null_mut(), len: MAX_ITEMS, cap: MAX_ITEMS, arena: std::ptr::null_mut() };
    // SAFETY: each of these fails before it touches the list's items (there are none to touch).
    unsafe {
        match what.to_str().unwrap() {
            "grow" => kestrel_rt_list_grow(&mut full, 1, 1),
            "insert" => {
                crate::list::kestrel_rt_list_insert(&mut full, 0, 1, 1, 5, std::ptr::null());
            }
            "alloc" => {
                kestrel_rt_alloc(MAX_ITEMS + 1, 1, 1, 7, std::ptr::null());
            }
            other => panic!("unknown case {other}"),
        }
    }
    panic!("the item limit should have ended the process");
}

#[test]
fn going_past_the_item_limit_fails() {
    for (case, message) in [
        ("grow", "a list can't hold more than 2,147,483,647 items"),
        ("insert", "a list can't hold more than 2,147,483,647 items"),
        (
            "alloc",
            "an array can't hold more than 2,147,483,647 items; this one would have 2147483648 on line 7",
        ),
    ] {
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["arena::tests::item_limit_child", "--exact", "--nocapture"])
            .env(LIMIT_CHILD, case)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(101), "{case}: {stderr}");
        assert!(stderr.contains(message), "{case}: {stderr}");
    }
}
