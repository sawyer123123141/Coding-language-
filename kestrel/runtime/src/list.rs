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
}
