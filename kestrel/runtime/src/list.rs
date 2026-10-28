//! Lists: removing an item and opening a slot for one. The compiled code reads and writes the
//! items itself; it calls these to move them. See `arena` for `List` and for growing.

use crate::arena::{kestrel_rt_list_grow, List};
use crate::panic::{kestrel_rt_panic_index, FileName};

/// Removes item `i`, moving the later items down one place. The vacated last slot is zeroed
/// and the length goes down by one. Compiled code drops an owning item before calling this.
/// Panics unless `0 <= i < len`.
///
/// # Safety
/// `list` must point to a valid `List` whose items are `elem_size` bytes.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_list_remove(list: *mut List, i: i64, elem_size: i64, line: i32, file: *const FileName) {
    // SAFETY: the caller guarantees `list`.
    let (ptr, len) = unsafe { ((*list).ptr, (*list).len) };
    if i < 0 || i >= len {
        kestrel_rt_panic_index(i, len, line, file);
    }
    let size = elem_size as usize;
    let tail = (len - 1 - i) as usize * size;
    // SAFETY: items `i..len` are in the list's buffer; `copy` handles the overlap, and the last
    // slot is the one left over once the tail has moved down.
    unsafe {
        let at = ptr.add(i as usize * size);
        std::ptr::copy(at.add(size), at, tail);
        std::ptr::write_bytes(at.add(tail), 0, size);
        (*list).len = len - 1;
    }
}

/// Opens a slot for a new item at index `i`, moving the items from `i` up one place (growing
/// the list if it is full), and returns the slot, for the caller to store the item in. The
/// length goes up by one. `i == len` adds at the end. Panics unless `0 <= i <= len`.
///
/// # Safety
/// `list` must point to a valid `List` whose items are `elem_size` bytes.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_list_insert(
    list: *mut List,
    i: i64,
    elem_size: i64,
    align: i64,
    line: i32,
    file: *const FileName,
) -> *mut u8 {
    // SAFETY: the caller guarantees `list`.
    let len = unsafe { (*list).len };
    if i < 0 || i > len {
        kestrel_rt_panic_index(i, len, line, file);
    }
    // SAFETY: as above; growing keeps the items and gives the list a bigger buffer.
    unsafe {
        if len >= (*list).cap {
            kestrel_rt_list_grow(list, elem_size, align);
        }
    }
    let size = elem_size as usize;
    // SAFETY: the buffer now has room for `len + 1` items.
    unsafe {
        let at = (*list).ptr.add(i as usize * size);
        std::ptr::copy(at, at.add(size), (len - i) as usize * size);
        // The slot is about to be stored to; zeroed meanwhile, it holds no stale copy of an item.
        std::ptr::write_bytes(at, 0, size);
        (*list).len = len + 1;
        at
    }
}

/// Empties the list: zeroes every item, so nothing stale can be read through a pointer to the
/// list, and sets the length to 0. The capacity stays. Compiled code drops owning items first.
///
/// # Safety
/// `list` must point to a valid `List` whose items are `elem_size` bytes.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_list_clear(list: *mut List, elem_size: i64) {
    // SAFETY: the caller guarantees `list`; its buffer holds `len` items.
    unsafe {
        let len = (*list).len as usize;
        if len > 0 {
            std::ptr::write_bytes((*list).ptr, 0, len * elem_size as usize);
        }
        (*list).len = 0;
    }
}

/// Copies `src_len` items of `elem_size` bytes from `src` into the buffer of the list or array at
/// `dst`, and sets its length to `src_len`: storage-reusing assignment of items that hold no heap
/// memory. Only the first two words of `dst` (the buffer and the length) are touched, so an
/// array's header, which has only those, can be passed too; a list keeps its capacity and arena.
/// `ptr::copy` is used, so even overlapping memory would be copied correctly.
///
/// # Safety
/// `dst` must point to a valid list or array header whose buffer has room for `src_len` items
/// (compiled code checks the capacity first), and `src` must hold `src_len` items.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_copy_items(dst: *mut List, src: *const u8, src_len: i64, elem_size: i64) {
    let bytes = src_len.max(0) as usize * elem_size.max(0) as usize;
    // SAFETY: the caller guarantees `dst` and `src`, and room for the items; no reference to the
    // whole header is made, so an array's two-word header is never read past. With no bytes to
    // copy the buffer may be null, so nothing is copied.
    unsafe {
        if bytes > 0 {
            std::ptr::copy(src, (*dst).ptr, bytes);
        }
        (*dst).len = src_len;
    }
}

/// Finishes a loop that removed items as it went: `kept` items are packed at the front, and the
/// items from `next` on, which the loop didn't reach, move down after them. The slots they leave
/// are zeroed and the length becomes `kept` plus their count. When `next` is past the length (the
/// list shrank while the loop ran) the length becomes at most `kept`. Nothing outside the list's
/// first `len` items is touched.
///
/// # Safety
/// `list` must point to a valid `List` whose items are `elem_size` bytes.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_list_compact(list: *mut List, kept: i64, next: i64, elem_size: i64) {
    // SAFETY: the caller guarantees `list`.
    let (ptr, len) = unsafe { ((*list).ptr, (*list).len) };
    if next > len {
        // SAFETY: as above.
        unsafe { (*list).len = len.min(kept.max(0)) };
        return;
    }
    // The loop keeps at most the items it passed; anything else would be a bug, kept harmless.
    let next = next.max(0);
    let kept = kept.clamp(0, next);
    let size = elem_size as usize;
    let tail = (len - next) as usize;
    let new_len = kept as usize + tail;
    // SAFETY: `kept <= next <= len`, so both ranges are inside the list's first `len` items;
    // `copy` handles the overlap, and the zeroed slots `new_len..len` are the ones the tail left.
    unsafe {
        if tail > 0 && kept < next {
            std::ptr::copy(ptr.add(next as usize * size), ptr.add(kept as usize * size), tail * size);
        }
        if new_len < len as usize {
            std::ptr::write_bytes(ptr.add(new_len * size), 0, (len as usize - new_len) * size);
        }
        (*list).len = new_len as i64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list_of(items: &[i32]) -> List {
        let mut l = List { ptr: std::ptr::null_mut(), len: 0, cap: 0, arena: std::ptr::null_mut() };
        for &x in items {
            // SAFETY: `l` is a valid list of i32s throughout.
            unsafe {
                let slot = kestrel_rt_list_insert(&mut l, l.len, 4, 4, 0, std::ptr::null()) as *mut i32;
                *slot = x;
            }
        }
        l
    }

    fn items(l: &List) -> Vec<i32> {
        // SAFETY: `len` items are initialised.
        unsafe { std::slice::from_raw_parts(l.ptr as *const i32, l.len as usize).to_vec() }
    }

    #[test]
    fn insert_at_the_end_and_in_the_middle_keeps_the_order() {
        let mut l = list_of(&[10, 30]);
        assert_eq!(items(&l), [10, 30]);
        // SAFETY: `l` is a valid list of i32s.
        unsafe {
            *(kestrel_rt_list_insert(&mut l, 1, 4, 4, 0, std::ptr::null()) as *mut i32) = 20;
            *(kestrel_rt_list_insert(&mut l, 0, 4, 4, 0, std::ptr::null()) as *mut i32) = 5;
        }
        assert_eq!(items(&l), [5, 10, 20, 30]);
    }

    #[test]
    fn insert_grows_a_full_list() {
        let mut l = list_of(&[1, 2, 3, 4]);
        assert_eq!(l.cap, 4);
        // SAFETY: `l` is a valid list of i32s.
        unsafe { *(kestrel_rt_list_insert(&mut l, 2, 4, 4, 0, std::ptr::null()) as *mut i32) = 9 };
        assert_eq!(items(&l), [1, 2, 9, 3, 4]);
        assert_eq!(l.cap, 8);
    }

    #[test]
    fn remove_moves_the_tail_down_and_zeroes_the_vacated_slot() {
        let mut l = list_of(&[1, 2, 3, 4]);
        // SAFETY: `l` is a valid list of i32s.
        unsafe {
            kestrel_rt_list_remove(&mut l, 1, 4, 0, std::ptr::null());
            assert_eq!(items(&l), [1, 3, 4]);
            // The old last slot is cleared, so nothing stale can be read through it.
            assert_eq!(*(l.ptr as *const i32).add(3), 0);
            kestrel_rt_list_remove(&mut l, 2, 4, 0, std::ptr::null());
            kestrel_rt_list_remove(&mut l, 0, 4, 0, std::ptr::null());
            kestrel_rt_list_remove(&mut l, 0, 4, 0, std::ptr::null());
        }
        assert_eq!(items(&l), Vec::<i32>::new());
        assert_eq!(l.cap, 4, "the capacity stays");
    }

    #[test]
    fn clear_zeroes_every_item_and_keeps_the_capacity() {
        let mut l = list_of(&[7, 8, 9]);
        let ptr = l.ptr as *const i32;
        // SAFETY: `l` is a valid list of i32s; the buffer stays allocated.
        unsafe {
            kestrel_rt_list_clear(&mut l, 4);
            assert_eq!(l.len, 0);
            assert_eq!(l.cap, 4);
            assert_eq!(std::slice::from_raw_parts(ptr, 3), &[0, 0, 0]);
            kestrel_rt_list_clear(&mut l, 4);
        }
        assert_eq!(l.len, 0);
    }

    #[test]
    fn items_of_other_sizes_move_whole() {
        // 12-byte items: three i32s each.
        let mut l = List { ptr: std::ptr::null_mut(), len: 0, cap: 0, arena: std::ptr::null_mut() };
        // SAFETY: `l` is a valid list of 12-byte items; each slot is written in full.
        unsafe {
            for k in 0..3i32 {
                let slot = kestrel_rt_list_insert(&mut l, l.len, 12, 4, 0, std::ptr::null()) as *mut i32;
                for part in 0..3 {
                    *slot.add(part) = k * 10 + part as i32;
                }
            }
            kestrel_rt_list_remove(&mut l, 0, 12, 0, std::ptr::null());
            assert_eq!(std::slice::from_raw_parts(l.ptr as *const i32, 6), &[10, 11, 12, 20, 21, 22]);
        }
    }

    #[test]
    fn copy_items_fills_the_buffer_and_keeps_the_capacity_and_arena() {
        let mut dst = list_of(&[1, 2, 3, 4, 5]);
        let (ptr, cap) = (dst.ptr, dst.cap);
        let src = [7i32, 8];
        // SAFETY: `dst` has room for 2 items; `src` holds 2 i32s.
        unsafe { kestrel_rt_copy_items(&mut dst, src.as_ptr() as *const u8, 2, 4) };
        assert_eq!(items(&dst), [7, 8]);
        assert_eq!((dst.ptr, dst.cap), (ptr, cap), "the buffer is reused");
        let longer = [9i32, 10, 11, 12, 13];
        // SAFETY: as above, 5 items fit in the capacity of 8.
        unsafe { kestrel_rt_copy_items(&mut dst, longer.as_ptr() as *const u8, 5, 4) };
        assert_eq!(items(&dst), [9, 10, 11, 12, 13]);
    }

    #[test]
    fn copy_items_of_nothing_leaves_a_null_buffer_alone() {
        let mut dst = List { ptr: std::ptr::null_mut(), len: 0, cap: 0, arena: std::ptr::null_mut() };
        // SAFETY: nothing is copied, so a null buffer and source are fine.
        unsafe { kestrel_rt_copy_items(&mut dst, std::ptr::null(), 0, 4) };
        assert_eq!((dst.len, dst.ptr), (0, std::ptr::null_mut()));
    }

    #[test]
    fn copy_items_into_an_array_touches_only_its_two_words() {
        // An array's header is `{ptr, len}`; the words after it belong to something else.
        let mut buf = [0i32; 3];
        let mut words: [i64; 4] = [buf.as_mut_ptr() as i64, 3, -1, -2];
        let src = [4i32, 5];
        // SAFETY: `words` starts like a list header; only its first two words are used.
        unsafe { kestrel_rt_copy_items(words.as_mut_ptr() as *mut List, src.as_ptr() as *const u8, 2, 4) };
        assert_eq!(&words[1..], [2, -1, -2]);
        assert_eq!(buf, [4, 5, 0]);
    }

    #[test]
    fn compact_moves_the_unvisited_tail_down_and_zeroes_what_it_leaves() {
        // A loop kept 1 of the first 3 items (packed at the front, the rest zeroed) and stopped.
        let mut l = list_of(&[1, 0, 0, 4, 5]);
        // SAFETY: `l` is a valid list of i32s; the buffer stays allocated.
        unsafe {
            kestrel_rt_list_compact(&mut l, 1, 3, 4);
            assert_eq!(items(&l), [1, 4, 5]);
            assert_eq!(std::slice::from_raw_parts(l.ptr as *const i32, 5), &[1, 4, 5, 0, 0]);
        }
    }

    #[test]
    fn compact_after_a_full_pass_keeps_only_the_kept_items() {
        let mut l = list_of(&[1, 3, 0, 0]);
        // SAFETY: as above.
        unsafe {
            kestrel_rt_list_compact(&mut l, 2, 4, 4);
            assert_eq!(items(&l), [1, 3]);
            kestrel_rt_list_compact(&mut l, 0, 0, 4);
            assert_eq!(items(&l), [1, 3], "nothing reached, nothing removed");
            let mut empty = list_of(&[]);
            kestrel_rt_list_compact(&mut empty, 0, 0, 4);
            assert_eq!(empty.len, 0);
        }
    }

    #[test]
    fn compact_moves_items_added_during_the_loop_too() {
        // The loop read a length of 3 and kept 1; two items were added meanwhile (the list grew).
        let mut l = list_of(&[7, 0, 0, 8, 9]);
        // SAFETY: as above.
        unsafe { kestrel_rt_list_compact(&mut l, 1, 3, 4) };
        assert_eq!(items(&l), [7, 8, 9]);
    }

    #[test]
    fn compact_of_a_list_that_shrank_only_lowers_the_length() {
        let mut l = list_of(&[1, 2]);
        // SAFETY: as above; the loop had passed 4 items of a list now 2 long.
        unsafe {
            kestrel_rt_list_compact(&mut l, 3, 4, 4);
            assert_eq!(items(&l), [1, 2], "kept is past the length: nothing changes");
            kestrel_rt_list_compact(&mut l, 1, 4, 4);
            assert_eq!(items(&l), [1]);
        }
    }

    #[test]
    fn compact_moves_whole_items_of_other_sizes() {
        let mut l = List { ptr: std::ptr::null_mut(), len: 0, cap: 0, arena: std::ptr::null_mut() };
        // SAFETY: `l` is a valid list of 12-byte items; each slot is written in full.
        unsafe {
            for k in 0..4i32 {
                let slot = kestrel_rt_list_insert(&mut l, l.len, 12, 4, 0, std::ptr::null()) as *mut i32;
                for part in 0..3 {
                    *slot.add(part) = if k == 1 { 0 } else { k * 10 + part as i32 };
                }
            }
            // Item 0 kept in place, item 1 removed (zeroed), items 2 and 3 not reached.
            kestrel_rt_list_compact(&mut l, 1, 2, 12);
            assert_eq!(l.len, 3);
            assert_eq!(std::slice::from_raw_parts(l.ptr as *const i32, 12), &[0, 1, 2, 20, 21, 22, 30, 31, 32, 0, 0, 0]);
        }
    }
}
