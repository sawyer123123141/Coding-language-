//! `map[K, V]`: hash maps that keep insertion order. Compiled code lays
//! out the header and the entries (see KIR's `map_layout`) and reads them
//! directly for `len`, loops and values; these functions find, add, remove
//! and copy entries.
//!
//! Entries sit in insertion order in one array, removed ones marked dead
//! until the array is compacted. An index table (open addressing, linear
//! probing) maps hashes to entry numbers.

use crate::arena::{kestrel_rt_current_arena, program_arena, Arena};
use crate::panic::fail;

/// A `map[K, V]` value, as compiled code lays it out.
#[repr(C)]
pub struct Map {
    pub entries: *mut u8,
    /// Live keys (`m.len`), at the same offset as a list's length.
    pub count: i64,
    /// Entries used, including removed ones.
    pub used: i64,
    pub cap: i64,
    pub slots: *mut i32,
    /// A power of two, at least twice `cap`; 0 before the first key.
    pub slot_count: i64,
    /// `for ... in` loops running over the map: no keys can be added
    /// (that may move the entries). Removing only marks an entry dead.
    pub locks: i64,
    /// Where the map grows. Null (a zero-filled map) means the program arena.
    pub arena: *mut Arena,
}

const EMPTY: i32 = -1;
const REMOVED: i32 = -2;
/// Entry fields: the key's hash (u32), whether it's live (u8), the key.
const HASH: usize = 0;
const LIVE: usize = 4;
const KEY: usize = 8;
/// Key kinds: 0, 1, 2 are integers (or bools) of 1, 4 and 8 bytes.
const STRING: i32 = 3;

fn key_size(kind: i32) -> usize {
    match kind {
        0 => 1,
        1 => 4,
        2 => 8,
        _ => 16,
    }
}

/// A key, read once: an integer's (or bool's) value, or a string's text.
enum Key<'a> {
    Int(u64),
    Text(&'a [u8]),
}

/// The integer of `kind` at `p`, or the text of the string at `p`.
///
/// # Safety
/// `p` must point at a key of this kind; a string's text must be valid.
unsafe fn read_key<'a>(p: *const u8, kind: i32) -> Key<'a> {
    match kind {
        0 => Key::Int(*p as u64),
        1 => Key::Int(std::ptr::read_unaligned(p as *const u32) as u64),
        2 => Key::Int(std::ptr::read_unaligned(p as *const u64)),
        STRING => {
            let ptr = std::ptr::read_unaligned(p as *const *const u8);
            let len = std::ptr::read_unaligned(p.add(8) as *const i64);
            Key::Text(if len <= 0 { &[] } else { std::slice::from_raw_parts(ptr, len as usize) })
        }
        other => unreachable!("map key kind {other}"),
    }
}

impl Key<'_> {
    fn hash(&self) -> u32 {
        match self {
            Key::Int(v) => (v.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 32) as u32,
            Key::Text(bytes) => {
                // FNV-1a
                let mut h: u64 = 0xcbf2_9ce4_8422_2325;
                for &b in *bytes {
                    h ^= b as u64;
                    h = h.wrapping_mul(0x0100_0000_01b3);
                }
                (h ^ (h >> 32)) as u32
            }
        }
    }

    /// Whether the key stored at `p` (of `kind`) is this one.
    ///
    /// # Safety
    /// As for `read_key`.
    unsafe fn is_at(&self, p: *const u8, kind: i32) -> bool {
        match (self, read_key(p, kind)) {
            (Key::Int(a), Key::Int(b)) => *a == b,
            (Key::Text(a), Key::Text(b)) => *a == b,
            _ => false,
        }
    }
}

impl Map {
    /// # Safety
    /// `i` must be below `cap`, with `size` the entry size.
    unsafe fn entry(&self, i: i64, size: i64) -> *mut u8 {
        self.entries.add((i * size) as usize)
    }

    /// Looks `key` up: the slot holding its entry number (`true`), or the
    /// slot a new entry for it should go in (`false`).
    ///
    /// # Safety
    /// The map must have an index table, and entries of `size` bytes.
    unsafe fn probe(&self, key: &Key, kind: i32, h: u32, size: i64) -> (usize, bool) {
        let mask = self.slot_count as usize - 1;
        let mut s = h as usize & mask;
        let mut free = None;
        // Ends: at most `cap` slots are taken, out of at least `2 * cap`.
        loop {
            match *self.slots.add(s) {
                EMPTY => return (free.unwrap_or(s), false),
                REMOVED => {
                    free.get_or_insert(s);
                }
                n => {
                    let e = self.entry(n as i64, size);
                    if *(e.add(HASH) as *const u32) == h && key.is_at(e.add(KEY), kind) {
                        return (s, true);
                    }
                }
            }
            s = (s + 1) & mask;
        }
    }

    /// Makes room for one more entry: compacts in place when at least half
    /// the entries are removed (so churn never grows a map), else moves the
    /// live entries to arrays twice as big in the map's arena.
    ///
    /// # Safety
    /// Entries of `size` bytes and `align` alignment.
    unsafe fn make_room(&mut self, size: i64, align: i64) {
        let removed = self.used - self.count;
        if removed > 0 && removed * 2 >= self.used {
            let mut j = 0;
            for i in 0..self.used {
                let e = self.entry(i, size);
                if *e.add(LIVE) != 0 {
                    if i != j {
                        // Moving down, so the ranges may overlap.
                        std::ptr::copy(e, self.entry(j, size), size as usize);
                    }
                    j += 1;
                }
            }
            self.used = j;
        } else {
            if self.arena.is_null() {
                self.arena = program_arena();
            }
            self.reallocate((self.cap * 2).max(8), size, align, self.arena);
        }
        self.rebuild(size);
    }

    /// New arrays for `cap` entries in `arena`, holding the live entries in order.
    ///
    /// # Safety
    /// `arena` must be live; entries of `size` bytes and `align` alignment.
    unsafe fn reallocate(&mut self, cap: i64, size: i64, align: i64, arena: *mut Arena) {
        if cap > i32::MAX as i64 {
            fail("a map can't hold more than 2147483647 keys");
        }
        let slot_count = (cap as usize * 2).next_power_of_two();
        let bytes = cap.checked_mul(size).and_then(|b| usize::try_from(b).ok());
        let bytes = bytes.unwrap_or_else(|| fail("out of memory: a map is too large"));
        // Entries are written before they're read; the table is filled by
        // `rebuild`. Inside a `par for`, other threads may share `arena`.
        let slot_bytes = slot_count * 4;
        let (entries, slots) = crate::par::shared_growth(arena, || {
            ((*arena).alloc_uninit(bytes, align as usize), (*arena).alloc_uninit(slot_bytes, 4) as *mut i32)
        });
        let mut j = 0;
        for i in 0..self.used {
            let e = self.entry(i, size);
            if *e.add(LIVE) != 0 {
                std::ptr::copy_nonoverlapping(e, entries.add((j * size) as usize), size as usize);
                j += 1;
            }
        }
        self.entries = entries;
        self.used = j;
        self.cap = cap;
        self.slots = slots;
        self.slot_count = slot_count as i64;
    }

    /// Refills the index table from the entries, which are all live.
    ///
    /// # Safety
    /// The table and entries must be allocated, entries of `size` bytes.
    unsafe fn rebuild(&mut self, size: i64) {
        let n = self.slot_count as usize;
        std::slice::from_raw_parts_mut(self.slots, n).fill(EMPTY);
        for i in 0..self.used {
            let h = *(self.entry(i, size).add(HASH) as *const u32);
            let mut s = h as usize & (n - 1);
            while *self.slots.add(s) != EMPTY {
                s = (s + 1) & (n - 1);
            }
            *self.slots.add(s) = i as i32;
        }
    }
}

/// The entry for `key`, or null when it's missing.
///
/// # Safety
/// `map` must be a valid map whose entries are `entry_size` bytes, and
/// `key` a key of this kind.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_map_find(map: *const Map, key: *const u8, kind: i32, entry_size: i64) -> *mut u8 {
    let m = &*map;
    if m.count == 0 {
        return std::ptr::null_mut();
    }
    let k = read_key(key, kind);
    match m.probe(&k, kind, k.hash(), entry_size) {
        (s, true) => m.entry(*m.slots.add(s) as i64, entry_size),
        _ => std::ptr::null_mut(),
    }
}

/// The entry for `key`, adding a zeroed one when it's missing. Adding a
/// key while a loop runs over the map panics.
///
/// # Safety
/// As for `kestrel_rt_map_find`; entries are `entry_align`-aligned.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_map_insert(
    map: *mut Map,
    key: *const u8,
    kind: i32,
    entry_size: i64,
    entry_align: i64,
    line: i32,
) -> *mut u8 {
    let m = &mut *map;
    let k = read_key(key, kind);
    let h = k.hash();
    let mut free = None;
    if m.slot_count > 0 {
        match m.probe(&k, kind, h, entry_size) {
            (s, true) => return m.entry(*m.slots.add(s) as i64, entry_size),
            (s, false) => free = Some(s),
        }
    }
    if m.locks > 0 {
        fail(&format!(
            "can't add a key to a map while looping over it (line {line}); collect the keys in a list and change the map after the loop"
        ));
    }
    if m.used == m.cap {
        m.make_room(entry_size, entry_align);
        free = None;
    }
    let i = m.used;
    let e = m.entry(i, entry_size);
    std::ptr::write_bytes(e, 0, entry_size as usize);
    *(e.add(HASH) as *mut u32) = h;
    *e.add(LIVE) = 1;
    std::ptr::copy_nonoverlapping(key, e.add(KEY), key_size(kind));
    m.used += 1;
    m.count += 1;
    let s = match free {
        Some(s) => s,
        // The table was rebuilt: find the new key's place again.
        None => m.probe(&k, kind, h, entry_size).0,
    };
    *m.slots.add(s) = i as i32;
    e
}

/// Removes `key`: 1 when it was there. Entries never move here, so a loop
/// over the map may remove keys; it skips the removed entries.
///
/// # Safety
/// As for `kestrel_rt_map_find`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_map_remove(map: *mut Map, key: *const u8, kind: i32, entry_size: i64) -> i8 {
    let m = &mut *map;
    if m.count == 0 {
        return 0;
    }
    let k = read_key(key, kind);
    let (s, true) = m.probe(&k, kind, k.hash(), entry_size) else { return 0 };
    let i = *m.slots.add(s);
    *m.slots.add(s) = REMOVED;
    *m.entry(i as i64, entry_size).add(LIVE) = 0;
    m.count -= 1;
    1
}

/// Gives the map at `map` its own compact copy of its entries, in the
/// current arena. The copy isn't being looped over.
///
/// # Safety
/// As for `kestrel_rt_map_insert`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_map_dup(map: *mut Map, entry_size: i64, entry_align: i64) {
    let m = &mut *map;
    let arena = kestrel_rt_current_arena();
    m.locks = 0;
    m.arena = arena;
    if m.count == 0 {
        m.entries = std::ptr::null_mut();
        m.slots = std::ptr::null_mut();
        (m.used, m.cap, m.slot_count) = (0, 0, 0);
        return;
    }
    m.reallocate(m.count, entry_size, entry_align, arena);
    m.rebuild(entry_size);
}

/// Whether two strings have the same text.
///
/// # Safety
/// Each pointer must be valid for its length (or the length is 0).
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_str_eq(a: *const u8, a_len: i64, b: *const u8, b_len: i64) -> i8 {
    if a_len != b_len {
        return 0;
    }
    if a_len <= 0 {
        return 1;
    }
    (std::slice::from_raw_parts(a, a_len as usize) == std::slice::from_raw_parts(b, b_len as usize)) as i8
}

#[cfg(test)]
mod tests {
    use super::*;

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
            let e = kestrel_rt_map_insert(m, &k as *const i32 as *const u8, 1, SIZE, 8, 1);
            *(e.add(VAL) as *mut i32) = v;
        }
    }

    fn get(m: &Map, k: i32) -> Option<i32> {
        unsafe {
            let e = kestrel_rt_map_find(m, &k as *const i32 as *const u8, 1, SIZE);
            (!e.is_null()).then(|| *(e.add(VAL) as *const i32))
        }
    }

    fn remove(m: &mut Map, k: i32) -> i8 {
        unsafe { kestrel_rt_map_remove(m, &k as *const i32 as *const u8, 1, SIZE) }
    }

    /// Live keys in entry order.
    fn keys(m: &Map) -> Vec<i32> {
        (0..m.used)
            .filter_map(|i| unsafe {
                let e = m.entry(i, SIZE);
                (*e.add(LIVE) != 0).then(|| *(e.add(KEY) as *const i32))
            })
            .collect()
    }

    #[test]
    fn inserted_keys_are_found() {
        let mut m = empty();
        for k in 1..=1000 {
            set(&mut m, k, k * 3);
        }
        assert_eq!(m.count, 1000);
        assert!((1..=1000).all(|k| get(&m, k) == Some(k * 3)));
        assert_eq!(get(&m, 0), None);
        assert_eq!(get(&m, -5), None);
    }

    #[test]
    fn inserting_a_key_again_replaces_its_value() {
        let mut m = empty();
        set(&mut m, 7, 1);
        set(&mut m, 7, 2);
        assert_eq!((m.count, get(&m, 7)), (1, Some(2)));
    }

    #[test]
    fn remove_reports_whether_the_key_was_there() {
        let mut m = empty();
        assert_eq!(remove(&mut m, 1), 0);
        set(&mut m, 1, 10);
        assert_eq!(remove(&mut m, 1), 1);
        assert_eq!(remove(&mut m, 1), 0);
        assert_eq!((m.count, get(&m, 1)), (0, None));
    }

    #[test]
    fn entries_keep_insertion_order() {
        let mut m = empty();
        for k in [1, 2, 3] {
            set(&mut m, k, 0);
        }
        remove(&mut m, 2);
        set(&mut m, 4, 0);
        set(&mut m, 2, 0);
        assert_eq!(keys(&m), [1, 3, 4, 2]);
    }

    #[test]
    fn churn_compacts_in_place_instead_of_growing() {
        let mut m = empty();
        for n in 0..10_000 {
            set(&mut m, n, n);
            if n >= 8 {
                assert_eq!(remove(&mut m, n - 8), 1);
            }
        }
        assert_eq!(m.count, 8);
        assert!(m.cap <= 32, "cap grew to {}", m.cap);
        assert_eq!(keys(&m), (9992..10_000).collect::<Vec<_>>());
        assert!((9992..10_000).all(|k| get(&m, k) == Some(k)));
    }

    #[test]
    fn string_keys_compare_by_text() {
        // map[string, int]: key at 8 (16 bytes), value at 24, entries 32 bytes.
        let mut m = empty();
        let key = |s: &str| [s.as_ptr() as i64, s.len() as i64];
        let a = String::from("sword");
        let b = String::from("sword");
        unsafe {
            let e = kestrel_rt_map_insert(&mut m, key(&a).as_ptr() as *const u8, STRING, 32, 8, 1);
            *(e.add(24) as *mut i32) = 30;
            let found = kestrel_rt_map_find(&m, key(&b).as_ptr() as *const u8, STRING, 32);
            assert_eq!(*(found.add(24) as *const i32), 30);
            assert!(kestrel_rt_map_find(&m, key("bow").as_ptr() as *const u8, STRING, 32).is_null());
            assert!(kestrel_rt_map_find(&m, key("").as_ptr() as *const u8, STRING, 32).is_null());
        }
    }

    #[test]
    fn dup_makes_a_compact_independent_copy() {
        let mut m = empty();
        for k in 0..20 {
            set(&mut m, k, k);
        }
        for k in 0..10 {
            remove(&mut m, k);
        }
        m.locks = 2;
        let mut c = Map { ..m };
        unsafe { kestrel_rt_map_dup(&mut c, SIZE, 8) };
        assert_eq!((c.count, c.used, c.locks), (10, 10, 0));
        set(&mut c, 15, 99);
        assert_eq!((get(&c, 15), get(&m, 15)), (Some(99), Some(15)));
        assert_eq!(keys(&c), (10..20).collect::<Vec<_>>());
    }

    #[test]
    fn strings_compare_by_text() {
        let (a, b) = ("abc", String::from("abc"));
        unsafe {
            assert_eq!(kestrel_rt_str_eq(a.as_ptr(), 3, b.as_ptr(), 3), 1);
            assert_eq!(kestrel_rt_str_eq(a.as_ptr(), 3, b.as_ptr(), 2), 0);
            assert_eq!(kestrel_rt_str_eq(a.as_ptr(), 3, "abd".as_ptr(), 3), 0);
            assert_eq!(kestrel_rt_str_eq(std::ptr::null(), 0, std::ptr::null(), 0), 1);
        }
    }
}
