//! Arenas: chunked bump allocators. Compiled code allocates from the
//! current arena, the top of a per-thread stack. The program arena sits
//! at the bottom, made on first use and freed when the thread ends.

use std::alloc::{alloc_zeroed, dealloc, handle_alloc_error, Layout};
use std::cell::RefCell;

const BLOCK: usize = 64 * 1024;

pub struct Arena {
    blocks: Vec<(*mut u8, Layout)>,
    /// Free space in the newest small block: `next..end`.
    next: usize,
    end: usize,
}

impl Default for Arena {
    fn default() -> Self {
        Self::new()
    }
}

impl Arena {
    pub fn new() -> Self {
        Arena { blocks: Vec::new(), next: 0, end: 0 }
    }

    /// `size` zeroed bytes aligned to `align` (a power of two).
    pub fn alloc(&mut self, size: usize, align: usize) -> *mut u8 {
        if size == 0 {
            // Never read or written; it only needs to be non-null and aligned.
            return align as *mut u8;
        }
        let start = self.next.next_multiple_of(align);
        if self.next != 0 && start + size <= self.end {
            self.next = start + size;
            return start as *mut u8;
        }
        // A big request gets a block of its own, so the current one keeps its free space.
        if size > BLOCK / 4 {
            return self.block(size, align.max(16));
        }
        let base = self.block(BLOCK, 16) as usize;
        let start = base.next_multiple_of(align);
        self.next = start + size;
        self.end = base + BLOCK;
        start as *mut u8
    }

    fn block(&mut self, size: usize, align: usize) -> *mut u8 {
        let layout = Layout::from_size_align(size, align).unwrap_or_else(|_| too_large());
        // SAFETY: `layout` has a non-zero size.
        let p = unsafe { alloc_zeroed(layout) };
        if p.is_null() {
            handle_alloc_error(layout);
        }
        self.blocks.push((p, layout));
        p
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        for (p, layout) in self.blocks.drain(..) {
            // SAFETY: each block was allocated with exactly this layout.
            unsafe { dealloc(p, layout) };
        }
    }
}

fn too_large() -> ! {
    crate::panic::fail("out of memory: allocation is too large");
}

struct Stack {
    program: Option<Box<Arena>>,
    /// Arenas pushed by `arena { }` / `in x { }`, innermost last.
    pushed: Vec<*mut Arena>,
}

thread_local! {
    static STACK: RefCell<Stack> = const { RefCell::new(Stack { program: None, pushed: Vec::new() }) };
}

/// The arena allocations currently go to.
#[no_mangle]
pub extern "C" fn kestrel_rt_current_arena() -> *mut Arena {
    STACK.with_borrow_mut(|s| match s.pushed.last() {
        Some(&a) => a,
        None => &mut **s.program.get_or_insert_with(Box::default) as *mut Arena,
    })
}

/// The program arena, which lives until the program ends.
fn program_arena() -> *mut Arena {
    STACK.with_borrow_mut(|s| &mut **s.program.get_or_insert_with(Box::default) as *mut Arena)
}

/// A new, empty arena (`Arena()`). It allocates nothing until first used.
#[no_mangle]
pub extern "C" fn kestrel_rt_arena_new() -> *mut Arena {
    Box::into_raw(Box::default())
}

/// Frees an arena and everything allocated in it. Null does nothing.
///
/// # Safety
/// `a` must be null or come from `kestrel_rt_arena_new`, not yet freed.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_arena_free(a: *mut Arena) {
    if a.is_null() {
        return;
    }
    if STACK.with_borrow(|s| s.pushed.contains(&a)) {
        crate::panic::fail("an arena was freed while allocating into it (its owner was replaced inside `in`)");
    }
    // SAFETY: the caller guarantees `a` is a live arena from `kestrel_rt_arena_new`.
    drop(unsafe { Box::from_raw(a) });
}

/// Makes `a` the current arena (`arena { }` / `in x { }`).
#[no_mangle]
pub extern "C" fn kestrel_rt_arena_push(a: *mut Arena) {
    STACK.with_borrow_mut(|s| s.pushed.push(a));
}

/// Undoes the latest push and returns that arena.
#[no_mangle]
pub extern "C" fn kestrel_rt_arena_pop() -> *mut Arena {
    STACK.with_borrow_mut(|s| s.pushed.pop().expect("pops match pushes"))
}

/// Room for `count` zeroed elements of `elem_size` bytes in the current arena.
#[no_mangle]
pub extern "C" fn kestrel_rt_alloc(count: i64, elem_size: i64, align: i64, line: i32) -> *mut u8 {
    if count < 0 {
        crate::panic::fail(&format!("negative array length {count} on line {line}"));
    }
    let Some(size) = count.checked_mul(elem_size).and_then(|s| usize::try_from(s).ok()) else {
        crate::panic::fail(&format!("an array of {count} elements is too large (line {line})"));
    };
    // SAFETY: the current arena is alive while it's on the stack.
    unsafe { (*kestrel_rt_current_arena()).alloc(size, align as usize) }
}

/// A copy of `bytes` bytes at `src`, in the current arena.
///
/// # Safety
/// `src` must be valid for reading `bytes` bytes.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_dup(src: *const u8, bytes: i64, align: i64) -> *mut u8 {
    let n = bytes as usize;
    // SAFETY: the current arena is alive while it's on the stack.
    let dst = unsafe { (*kestrel_rt_current_arena()).alloc(n, align as usize) };
    if n > 0 {
        // SAFETY: the caller guarantees `src`; `dst` is fresh, so they can't overlap.
        unsafe { std::ptr::copy_nonoverlapping(src, dst, n) };
    }
    dst
}

/// A `list[T]` value, as compiled code lays it out.
#[repr(C)]
pub struct List {
    pub ptr: *mut u8,
    pub len: i64,
    pub cap: i64,
    /// Where the list grows. Null (a zero-filled list) means the program
    /// arena: nothing says how long the list lives, and that one outlives all.
    pub arena: *mut Arena,
}

/// Doubles a full list's capacity (at least 4), moving its elements into
/// new memory from the list's own arena.
///
/// # Safety
/// `list` must point to a valid `List` whose elements are `elem_size` bytes.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_list_grow(list: *mut List, elem_size: i64, align: i64) {
    // SAFETY: the caller guarantees `list`.
    let l = unsafe { &mut *list };
    if l.arena.is_null() {
        l.arena = program_arena();
    }
    let cap = (l.cap * 2).max(4);
    let size = cap.checked_mul(elem_size).and_then(|s| usize::try_from(s).ok()).unwrap_or_else(|| too_large());
    // SAFETY: a list's arena outlives the list.
    let p = unsafe { (*l.arena).alloc(size, align as usize) };
    if l.len > 0 {
        // SAFETY: the old buffer holds `len` elements; the new one is fresh and bigger.
        unsafe { std::ptr::copy_nonoverlapping(l.ptr, p, (l.len * elem_size) as usize) };
    }
    l.ptr = p;
    l.cap = cap;
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let program = kestrel_rt_current_arena();
        let a = kestrel_rt_arena_new();
        kestrel_rt_arena_push(a);
        assert_eq!(kestrel_rt_current_arena(), a);
        let p = kestrel_rt_alloc(4, 8, 8, 0);
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
}
