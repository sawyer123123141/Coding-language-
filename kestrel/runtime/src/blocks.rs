//! Memory blocks for arenas, recycled per thread. A freed arena's blocks
//! are kept for the next arena instead of going back to the OS, so code
//! that makes and frees arenas over and over (every frame, every chunk)
//! reuses warm memory instead of faulting in fresh pages each time.

use std::alloc::{alloc, dealloc, handle_alloc_error, Layout};
use std::cell::RefCell;

/// At most this many bytes of spare blocks are kept per thread.
const KEEP: usize = 256 * 1024 * 1024;

struct Spares {
    blocks: Vec<(*mut u8, Layout)>,
    bytes: usize,
}

impl Drop for Spares {
    fn drop(&mut self) {
        for (p, layout) in self.blocks.drain(..) {
            // SAFETY: each spare was allocated with exactly this layout.
            unsafe { dealloc(p, layout) };
        }
    }
}

thread_local! {
    static SPARES: RefCell<Spares> = const { RefCell::new(Spares { blocks: Vec::new(), bytes: 0 }) };
}

/// A block of at least `size` bytes aligned to `align`, and its real
/// layout. Its contents are garbage.
pub(crate) fn take(size: usize, align: usize) -> (*mut u8, Layout) {
    // The smallest spare that fits, unless even that one is wastefully big.
    let reused = SPARES
        .try_with(|s| {
            let mut s = s.borrow_mut();
            let fits = |l: &Layout| l.size() >= size && l.size() <= size.saturating_mul(4) && l.align() >= align;
            let (i, _) = s.blocks.iter().enumerate().filter(|(_, (_, l))| fits(l)).min_by_key(|(_, (_, l))| l.size())?;
            let block = s.blocks.swap_remove(i);
            s.bytes -= block.1.size();
            Some(block)
        })
        .ok()
        .flatten();
    if let Some(block) = reused {
        return block;
    }
    let layout = Layout::from_size_align(size, align).unwrap_or_else(|_| crate::panic::fail("out of memory: allocation is too large"));
    // SAFETY: callers never ask for zero bytes.
    let p = unsafe { alloc(layout) };
    if p.is_null() {
        handle_alloc_error(layout);
    }
    (p, layout)
}

/// Hands a block back: kept for reuse by this thread, or freed.
pub(crate) fn give(p: *mut u8, layout: Layout) {
    let kept = SPARES
        .try_with(|s| {
            let mut s = s.borrow_mut();
            let keep = s.bytes + layout.size() <= KEEP;
            if keep {
                s.blocks.push((p, layout));
                s.bytes += layout.size();
            }
            keep
        })
        .unwrap_or(false);
    if !kept {
        // SAFETY: `p` was allocated with `layout` (by `take`).
        unsafe { dealloc(p, layout) };
    }
}
