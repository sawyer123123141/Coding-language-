//! `xs.sort()` and `xs.sort_by(key)`: a stable sort over a list's or array's bytes, ordered by a
//! compare function that compiled code generates (see `kir/src/lower/sort.rs`).

use std::cmp::Ordering;

use crate::string::KStr;

/// The compare function: given the environment word and the addresses of two items, it says how
/// the first stands to the second: 0 for before, 1 for equal, anything else for after. It must be
/// a total order, which compiled code makes it (a float NaN sorts after every number).
pub type Compare = extern "C" fn(env: *const u8, a: *const u8, b: *const u8) -> i8;

/// Sorts the `len` items of `size` bytes at `items`, smallest first by `compare`, keeping the
/// order of equal items. The items move as bytes.
///
/// It sorts a vector of positions, comparing the items where they are, then permutes the items
/// through a temporary buffer. Moving by bytes is safe for every Kestrel value: a list or a map
/// keeps its arena pointer, and nothing points into the buffer of the list that holds it.
///
/// # Safety
/// `items` must be valid for reading and writing `len * size` bytes, and `compare` must be
/// callable with the items' addresses and `env`.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_sort(items: *mut u8, len: i64, size: i64, compare: Compare, env: *const u8) {
    if len < 2 || size <= 0 {
        return;
    }
    let (n, size) = (len as usize, size as usize);
    let Ok(count) = u32::try_from(n) else { crate::panic::fail("can't sort a list of more than 4294967295 items") };
    // SAFETY (all the pointer arithmetic below): the caller guarantees `len * size` bytes at `items`,
    // and every index is below `len`.
    let at = |i: u32| unsafe { items.add(i as usize * size) };
    let mut order: Vec<u32> = (0..count).collect();
    // `sort_by` is stable. The items don't move until the order is known.
    order.sort_by(|&i, &j| match compare(env, at(i), at(j)) {
        0 => Ordering::Less,
        1 => Ordering::Equal,
        _ => Ordering::Greater,
    });
    let mut sorted = vec![0u8; n * size];
    for (k, &i) in order.iter().enumerate() {
        // SAFETY: as above; `sorted` holds `n * size` bytes and doesn't overlap the items.
        unsafe { std::ptr::copy_nonoverlapping(at(i), sorted.as_mut_ptr().add(k * size), size) };
    }
    // SAFETY: as above.
    unsafe { std::ptr::copy_nonoverlapping(sorted.as_ptr(), items, n * size) };
}

/// How the first string stands to the second by code point: 0 for before, 1 for equal, 2 for
/// after. UTF-8 keeps the order of code points, so this compares bytes.
///
/// # Safety
/// Both strings must be valid for reading their bytes unless the pointer is null.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_str_cmp(a: *const u8, a_len: i64, b: *const u8, b_len: i64) -> i8 {
    // SAFETY: the caller guarantees both strings.
    let (a, b) = unsafe { (KStr::bytes(a, a_len), KStr::bytes(b, b_len)) };
    match a.cmp(b) {
        Ordering::Less => 0,
        Ordering::Equal => 1,
        Ordering::Greater => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmp(a: &str, b: &str) -> i8 {
        // SAFETY: valid strings.
        unsafe { kestrel_rt_str_cmp(a.as_ptr(), a.len() as i64, b.as_ptr(), b.len() as i64) }
    }

    #[test]
    fn strings_compare_by_code_point() {
        assert_eq!((cmp("a", "z"), cmp("z", "a"), cmp("ab", "ab")), (0, 2, 1));
        // A prefix comes first, uppercase before lowercase, 'é' (233) after 'z' (122), and a
        // character outside the BMP after one inside it.
        assert_eq!((cmp("ab", "abc"), cmp("B", "a"), cmp("z", "é"), cmp("\u{ffff}", "\u{10000}")), (0, 0, 0, 0));
        // The empty string, whether null or not, is first.
        // SAFETY: a null pointer with length 0 is the empty string.
        assert_eq!(unsafe { kestrel_rt_str_cmp(std::ptr::null(), 0, "a".as_ptr(), 1) }, 0);
        assert_eq!(cmp("", ""), 1);
    }

    /// Items of 16 bytes: a key and a tag that says where the item started.
    #[repr(C)]
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Item {
        key: i64,
        tag: i64,
    }

    extern "C" fn by_key(_env: *const u8, a: *const u8, b: *const u8) -> i8 {
        // SAFETY: the sort passes addresses of `Item`s.
        let (a, b) = unsafe { (*a.cast::<Item>(), *b.cast::<Item>()) };
        match a.key.cmp(&b.key) {
            Ordering::Less => 0,
            Ordering::Equal => 1,
            Ordering::Greater => 2,
        }
    }

    /// Sorts `v` with the runtime.
    fn sort(v: &mut [Item]) {
        // SAFETY: `v` is `v.len()` items of `size_of::<Item>()` bytes.
        unsafe { kestrel_rt_sort(v.as_mut_ptr().cast(), v.len() as i64, std::mem::size_of::<Item>() as i64, by_key, std::ptr::null()) };
    }

    #[test]
    fn equal_keys_keep_their_order() {
        let mut v: Vec<Item> = (0..40).map(|i| Item { key: i % 3, tag: i }).collect();
        sort(&mut v);
        assert!(v.windows(2).all(|w| w[0].key < w[1].key || (w[0].key == w[1].key && w[0].tag < w[1].tag)), "{v:?}");
    }

    #[test]
    fn no_items_and_one_item() {
        let mut v: Vec<Item> = Vec::new();
        sort(&mut v);
        assert!(v.is_empty());
        let mut v = vec![Item { key: 7, tag: 1 }];
        sort(&mut v);
        assert_eq!(v, [Item { key: 7, tag: 1 }]);
        // A null list (a list nothing was added to) has no items either.
        // SAFETY: with no items, nothing is read.
        unsafe { kestrel_rt_sort(std::ptr::null_mut(), 0, 16, by_key, std::ptr::null()) };
    }

    #[test]
    fn matches_the_standard_library_on_random_items() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut v: Vec<Item> = (0..10_000).map(|i| Item { key: (next() % 500) as i64 - 250, tag: i }).collect();
        let mut expected = v.clone();
        // (`sort_by_key` is stable too.)
        expected.sort_by_key(|i| i.key);
        sort(&mut v);
        assert_eq!(v, expected);
    }

    #[test]
    fn the_environment_reaches_the_compare_function() {
        extern "C" fn backwards(env: *const u8, a: *const u8, b: *const u8) -> i8 {
            // SAFETY: the test passes the address of a bool.
            assert!(unsafe { *env } == 1);
            by_key(std::ptr::null(), b, a)
        }
        let mut v: Vec<Item> = (0..5).map(|i| Item { key: i, tag: i }).collect();
        let flag = 1u8;
        // SAFETY: as in `sort`.
        unsafe { kestrel_rt_sort(v.as_mut_ptr().cast(), 5, 16, backwards, &flag) };
        assert_eq!(v.iter().map(|i| i.key).collect::<Vec<_>>(), [4, 3, 2, 1, 0]);
    }

    #[test]
    fn items_without_size_are_left_alone() {
        extern "C" fn never(_: *const u8, _: *const u8, _: *const u8) -> i8 {
            panic!("nothing to compare")
        }
        let mut bytes = [0u8; 4];
        // SAFETY: a size of 0 reads nothing.
        unsafe { kestrel_rt_sort(bytes.as_mut_ptr(), 3, 0, never, std::ptr::null()) };
    }
}
