//! Arenas: chunked bump allocators. Compiled code allocates from the
//! current arena, the top of a per-thread stack. The program arena sits
//! at the bottom, made on first use and freed when the thread ends.

use std::alloc::Layout;
use std::cell::RefCell;
use std::sync::atomic::{AtomicI64, Ordering};

use crate::blocks;

const BLOCK: usize = 64 * 1024;

pub struct Arena {
    blocks: Vec<(*mut u8, Layout)>,
    /// Free space in the newest small block: `next..end`.
    next: usize,
    end: usize,
    /// Arenas handed over by `par for` threads, freed with this one. Boxed:
    /// lists and maps point at them, so they must never move.
    #[allow(clippy::vec_box)]
    children: Vec<Box<Arena>>,
    /// Whether dropping this arena poisons its blocks (see `blocks::poison`) instead of recycling them.
    poison: bool,
}

impl Default for Arena {
    fn default() -> Self {
        Self::new()
    }
}

impl Arena {
    /// A new arena that poisons its freed blocks if `KESTREL_POISON=1` was set at start.
    pub fn new() -> Self {
        Self::with_poison(blocks::poison_enabled())
    }

    /// A new arena that poisons its freed blocks (or recycles them), whatever the setting.
    pub fn with_poison(poison: bool) -> Self {
        Arena { blocks: Vec::new(), next: 0, end: 0, children: Vec::new(), poison }
    }

    /// `size` zeroed bytes aligned to `align` (a power of two).
    pub fn alloc(&mut self, size: usize, align: usize) -> *mut u8 {
        let p = self.alloc_uninit(size, align);
        if size > 0 {
            // SAFETY: `p` has room for `size` bytes (blocks are recycled, so not already zero).
            unsafe { std::ptr::write_bytes(p, 0, size) };
        }
        p
    }

    /// `size` bytes aligned to `align`, contents unspecified: for memory
    /// that's written before it's read (copies, a list's spare capacity).
    pub fn alloc_uninit(&mut self, size: usize, align: usize) -> *mut u8 {
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
            return self.block(size, align.max(16)).0;
        }
        let (base, cap) = self.block(BLOCK, 16);
        let base = base as usize;
        let start = base.next_multiple_of(align);
        self.next = start + size;
        self.end = base + cap;
        start as *mut u8
    }

    /// Keeps `child` (a `par for` thread's arena) alive as long as `self`.
    /// Lists and maps made in it still point at it to grow, so the arena
    /// itself must live on, not just its blocks.
    pub fn adopt(&mut self, child: Box<Arena>) {
        self.children.push(child);
    }

    /// The size of every block this arena holds, and those of the arenas it adopted.
    pub(crate) fn bytes(&self) -> usize {
        let own: usize = self.blocks.iter().map(|(_, layout)| layout.size()).sum();
        own + self.children.iter().map(|child| child.bytes()).sum::<usize>()
    }

    /// A block of at least `size` bytes, and how big it really is.
    fn block(&mut self, size: usize, align: usize) -> (*mut u8, usize) {
        let (p, layout) = blocks::take(size, align);
        self.blocks.push((p, layout));
        (p, layout.size())
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        for (p, layout) in self.blocks.drain(..) {
            if self.poison {
                blocks::poison(p, layout);
            } else {
                blocks::give(p, layout);
            }
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
pub(crate) fn program_arena() -> *mut Arena {
    if let Some(a) = crate::par::job_program_arena() {
        return a;
    }
    STACK.with_borrow_mut(|s| &mut **s.program.get_or_insert_with(Box::default) as *mut Arena)
}

/// The size of the program arena's blocks: what `KESTREL_ARENA_REPORT=1` reports.
pub(crate) fn program_bytes() -> usize {
    // SAFETY: the program arena lives until the program ends.
    unsafe { (*program_arena()).bytes() }
}

/// The arenas made by `kestrel_rt_arena_new` and not freed yet: `Arena()` values and `arena { }`
/// blocks. `KESTREL_ARENA_COUNT=1` reports it at exit, so tests can tell that every arena a
/// program made was dropped exactly once. Arenas that live until the program ends on purpose
/// (the heap constants', made by `kestrel_rt_arena_new_static`) and the runtime's own are left out.
static ALIVE: AtomicI64 = AtomicI64::new(0);

/// How many counted arenas are alive now (see `ALIVE`).
pub(crate) fn arenas_alive() -> i64 {
    ALIVE.load(Ordering::Relaxed)
}

/// A new, empty arena that nothing counts: for arenas the runtime makes for itself and then
/// keeps (see `Arena::adopt`).
pub(crate) fn new_uncounted() -> *mut Arena {
    Box::into_raw(Box::default())
}

/// A new, empty arena (`Arena()`). It allocates nothing until first used.
#[no_mangle]
pub extern "C" fn kestrel_rt_arena_new() -> *mut Arena {
    ALIVE.fetch_add(1, Ordering::Relaxed);
    new_uncounted()
}

/// A new, empty arena that is never freed (the heap constants') and that `ALIVE` leaves out.
#[no_mangle]
pub extern "C" fn kestrel_rt_arena_new_static() -> *mut Arena {
    new_uncounted()
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
    ALIVE.fetch_sub(1, Ordering::Relaxed);
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
pub extern "C" fn kestrel_rt_alloc(count: i64, elem_size: i64, align: i64, line: i32, file: *const crate::panic::FileName) -> *mut u8 {
    if count < 0 {
        crate::panic::fail(&format!("negative array length {count} on line {line}{}", crate::panic::of_file(file)));
    }
    let Some(size) = count.checked_mul(elem_size).and_then(|s| usize::try_from(s).ok()) else {
        crate::panic::fail(&format!("an array of {count} elements is too large (line {line}{})", crate::panic::of_file(file)));
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
    let dst = unsafe { (*kestrel_rt_current_arena()).alloc_uninit(n, align as usize) };
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
    // SAFETY: a list's arena outlives the list. Spare capacity is never read
    // before it's written, so it needn't be zeroed. Inside a `par for`, other
    // threads may be allocating in the same arena.
    let p = crate::par::shared_growth(l.arena, || unsafe { (*l.arena).alloc_uninit(size, align as usize) });
    if l.len > 0 {
        // SAFETY: the old buffer holds `len` elements; the new one is fresh and bigger.
        unsafe { std::ptr::copy_nonoverlapping(l.ptr, p, (l.len * elem_size) as usize) };
    }
    l.ptr = p;
    l.cap = cap;
}

#[cfg(test)]
mod tests;
