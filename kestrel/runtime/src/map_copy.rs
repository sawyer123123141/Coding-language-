//! A map assigned over another reuses the target's entries and index table when they have room
//! (storage-reusing assignment: see KIR's `lower::assign_into`).

use crate::map::{Map, LIVE};

/// Copies the live entries of the map at `src` into the map at `dst`'s own entries, in order and
/// compacted, sets `dst`'s count, and rebuilds its index table: 1. When `dst` is being looped over
/// (`locks > 0`, so the loop must see a new map, as today's copy gives it), or its entries or index
/// table are too small for `src`'s live keys, it gives 0 and changes nothing. Keys and values are
/// copied as bytes: compiled code then gives heap keys and values their own copies. `src` is only
/// read, and nothing is allocated, so the map keeps its arena.
///
/// A map is never copied into itself (lowering copies as today when the two places may overlap);
/// if it were, it would get 0.
///
/// # Safety
/// `dst` and `src` must be valid maps whose entries are `entry_size` bytes and `entry_align`-aligned.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_map_copy_into(dst: *mut Map, src: *const Map, entry_size: i64, entry_align: i64) -> i8 {
    // The entries go into the target's own array, which is aligned for them already.
    let _ = entry_align;
    if std::ptr::eq(dst, src) {
        return 0;
    }
    let (s, d) = (&*src, &mut *dst);
    if d.locks > 0 || d.cap < s.count || d.slot_count < slots_for(s.count) {
        return 0;
    }
    // At most `s.count` entries are written, so never past `d.cap`. (Two maps never share entries;
    // `copy` would still be safe if they did, since an entry only moves down.)
    let (mut i, mut j) = (0, 0);
    while i < s.used && j < s.count {
        let e = s.entry(i, entry_size);
        if *e.add(LIVE) != 0 {
            std::ptr::copy(e, d.entry(j, entry_size), entry_size as usize);
            j += 1;
        }
        i += 1;
    }
    d.count = j;
    d.used = j;
    // A map with no entries has no table either (and takes no key, so it holds none).
    if d.slot_count > 0 {
        d.rebuild(entry_size);
    }
    1
}

/// The size of the index table a map of `count` live keys needs, as `Map::reallocate` makes it for
/// entries of exactly that many: 0 for none.
fn slots_for(count: i64) -> i64 {
    if count <= 0 {
        0
    } else {
        (count as u64 * 2).next_power_of_two() as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{kestrel_rt_map_find, kestrel_rt_map_insert, kestrel_rt_map_remove};

    /// `map[int, int]`: the value is at 12, entries are 16 bytes.
    const SIZE: i64 = 16;
    const VAL: usize = 12;

    fn empty() -> Map {
        Map {
            entries: std::ptr::null_mut(),
            count: 0,
            used: 0,
            cap: 0,
            slots: std::ptr::null_mut(),
            slot_count: 0,
            locks: 0,
            arena: std::ptr::null_mut(),
        }
    }

    fn set(m: &mut Map, k: i32, v: i32) {
        unsafe {
            let e = kestrel_rt_map_insert(m, &k as *const i32 as *const u8, 1, SIZE, 8, 1, std::ptr::null());
            *(e.add(VAL) as *mut i32) = v;
        }
    }

    fn get(m: &Map, k: i32) -> Option<i32> {
        unsafe {
            let e = kestrel_rt_map_find(m, &k as *const i32 as *const u8, 1, SIZE);
            (!e.is_null()).then(|| *(e.add(VAL) as *const i32))
        }
    }

    fn remove(m: &mut Map, k: i32) {
        unsafe { kestrel_rt_map_remove(m, &k as *const i32 as *const u8, 1, SIZE) };
    }

    /// (key, value, live) of each entry below `used`, in order.
    fn listed(m: &Map) -> Vec<(i32, i32, u8)> {
        (0..m.used)
            .map(|i| unsafe {
                let e = m.entries.add((i * SIZE) as usize);
                (*(e.add(8) as *const i32), *(e.add(VAL) as *const i32), *e.add(4))
            })
            .collect()
    }

    /// A map of `keys` (in order) with value `k * 10` each.
    fn of(keys: impl IntoIterator<Item = i32>) -> Map {
        let mut m = empty();
        for k in keys {
            set(&mut m, k, k * 10);
        }
        m
    }

    /// Everything a map holds, byte for byte: its header, its entries up to `cap` and its table.
    fn bytes(m: &Map) -> Vec<u8> {
        let header = unsafe { std::slice::from_raw_parts(m as *const Map as *const u8, std::mem::size_of::<Map>()) };
        let mut out = header.to_vec();
        if m.cap > 0 {
            out.extend_from_slice(unsafe { std::slice::from_raw_parts(m.entries, (m.cap * SIZE) as usize) });
        }
        if m.slot_count > 0 {
            out.extend_from_slice(unsafe { std::slice::from_raw_parts(m.slots as *const u8, (m.slot_count * 4) as usize) });
        }
        out
    }

    #[test]
    fn copies_in_place_when_there_is_room() {
        // A target with room for 16 and keys of its own; a source with removed keys among its live ones.
        let mut dst = of(0..12);
        let (entries, cap, slots) = (dst.entries, dst.cap, dst.slots);
        assert_eq!(cap, 16);
        let mut src = of([50, 40, 30, 20, 10, 60, 70]);
        remove(&mut src, 40);
        remove(&mut src, 60);
        assert_eq!(unsafe { kestrel_rt_map_copy_into(&mut dst, &src, SIZE, 8) }, 1);
        assert_eq!((dst.count, dst.used, dst.cap, dst.locks), (5, 5, cap, 0));
        assert_eq!((dst.entries, dst.slots), (entries, slots), "the same arrays");
        let want: Vec<_> = [50, 30, 20, 10, 70].iter().map(|&k| (k, k * 10, 1)).collect();
        assert_eq!(listed(&dst), want);
        for k in [50, 30, 20, 10, 70] {
            assert_eq!(get(&dst, k), Some(k * 10), "key {k}");
        }
        for k in [40, 60, 0, 11] {
            assert_eq!(get(&dst, k), None, "key {k}");
        }
        // The source is unchanged, and the target grows and changes on its own.
        assert_eq!((src.count, src.used), (5, 7));
        for k in 100..120 {
            set(&mut dst, k, k);
        }
        set(&mut dst, 50, 1);
        assert_eq!((dst.count, get(&dst, 119), get(&dst, 50), get(&src, 50), get(&src, 119)), (25, Some(119), Some(1), Some(500), None));
        // An empty source empties the target.
        let none = empty();
        assert_eq!(unsafe { kestrel_rt_map_copy_into(&mut dst, &none, SIZE, 8) }, 1);
        assert_eq!((dst.count, dst.used, get(&dst, 50)), (0, 0, None));
        set(&mut dst, 3, 4);
        assert_eq!((dst.count, get(&dst, 3)), (1, Some(4)));
        // So does one with only removed keys, into a target with no entries yet.
        let mut gone = of([1, 2]);
        remove(&mut gone, 1);
        remove(&mut gone, 2);
        let mut zero = empty();
        assert_eq!(unsafe { kestrel_rt_map_copy_into(&mut zero, &gone, SIZE, 8) }, 1);
        assert_eq!((zero.count, zero.used, zero.cap, get(&zero, 1)), (0, 0, 0, None));
    }

    #[test]
    fn refuses_a_locked_or_small_map() {
        let src = of(0..6);
        // Locked: a loop runs over it.
        let mut locked = of(0..12);
        remove(&mut locked, 3);
        locked.locks = 1;
        let before = bytes(&locked);
        assert_eq!(unsafe { kestrel_rt_map_copy_into(&mut locked, &src, SIZE, 8) }, 0);
        assert_eq!(bytes(&locked), before);
        // Too little room: 4 entries (cap 8) take at most 8 keys.
        let mut small = of(0..4);
        assert_eq!(small.cap, 8);
        let big = of(100..109);
        let before = bytes(&small);
        assert_eq!(unsafe { kestrel_rt_map_copy_into(&mut small, &big, SIZE, 8) }, 0);
        assert_eq!(bytes(&small), before);
        // A map with no entries yet has no room for a key.
        let mut zero = empty();
        assert_eq!(unsafe { kestrel_rt_map_copy_into(&mut zero, &src, SIZE, 8) }, 0);
        assert_eq!(bytes(&zero), bytes(&empty()));
        // Removed keys in the source don't count against the room: 8 live of 12 used fit.
        let mut many = of(200..212);
        for k in 200..204 {
            remove(&mut many, k);
        }
        assert_eq!(unsafe { kestrel_rt_map_copy_into(&mut small, &many, SIZE, 8) }, 1);
        assert_eq!((small.count, small.cap, get(&small, 211)), (8, 8, Some(2110)));
    }
}
