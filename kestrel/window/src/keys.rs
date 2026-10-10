//! The `input.Key` enum's tags and the raylib codes they stand for.
//!
//! A `Key` arrives as a pointer to its tag byte: the variant's index in
//! `driver/std/input.kes`. `KEYS` lists the variants in that order (a test
//! compares the two files), so the tag is the index into it.

use std::ffi::c_int;

/// What one `Key` variant reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binding {
    /// Any of these raylib key codes: `Shift`, `Ctrl` and `Alt` match both sides.
    Keys(&'static [c_int]),
    /// A raylib mouse button (0 left, 1 right, 2 middle).
    Mouse(c_int),
}

use Binding::{Keys, Mouse};

/// Every `Key` variant, in `input.kes` order. The codes are raylib's
/// `KeyboardKey` and `MouseButton` values.
pub const KEYS: [(&str, Binding); 63] = [
    ("A", Keys(&[65])),
    ("B", Keys(&[66])),
    ("C", Keys(&[67])),
    ("D", Keys(&[68])),
    ("E", Keys(&[69])),
    ("F", Keys(&[70])),
    ("G", Keys(&[71])),
    ("H", Keys(&[72])),
    ("I", Keys(&[73])),
    ("J", Keys(&[74])),
    ("K", Keys(&[75])),
    ("L", Keys(&[76])),
    ("M", Keys(&[77])),
    ("N", Keys(&[78])),
    ("O", Keys(&[79])),
    ("P", Keys(&[80])),
    ("Q", Keys(&[81])),
    ("R", Keys(&[82])),
    ("S", Keys(&[83])),
    ("T", Keys(&[84])),
    ("U", Keys(&[85])),
    ("V", Keys(&[86])),
    ("W", Keys(&[87])),
    ("X", Keys(&[88])),
    ("Y", Keys(&[89])),
    ("Z", Keys(&[90])),
    ("Digit0", Keys(&[48])),
    ("Digit1", Keys(&[49])),
    ("Digit2", Keys(&[50])),
    ("Digit3", Keys(&[51])),
    ("Digit4", Keys(&[52])),
    ("Digit5", Keys(&[53])),
    ("Digit6", Keys(&[54])),
    ("Digit7", Keys(&[55])),
    ("Digit8", Keys(&[56])),
    ("Digit9", Keys(&[57])),
    ("Space", Keys(&[32])),
    ("Enter", Keys(&[257])),
    ("Escape", Keys(&[256])),
    ("Tab", Keys(&[258])),
    ("Backspace", Keys(&[259])),
    ("Shift", Keys(&[340, 344])),
    ("Ctrl", Keys(&[341, 345])),
    ("Alt", Keys(&[342, 346])),
    ("Up", Keys(&[265])),
    ("Down", Keys(&[264])),
    ("Left", Keys(&[263])),
    ("Right", Keys(&[262])),
    ("F1", Keys(&[290])),
    ("F2", Keys(&[291])),
    ("F3", Keys(&[292])),
    ("F4", Keys(&[293])),
    ("F5", Keys(&[294])),
    ("F6", Keys(&[295])),
    ("F7", Keys(&[296])),
    ("F8", Keys(&[297])),
    ("F9", Keys(&[298])),
    ("F10", Keys(&[299])),
    ("F11", Keys(&[300])),
    ("F12", Keys(&[301])),
    ("MouseLeft", Mouse(0)),
    ("MouseRight", Mouse(1)),
    ("MouseMiddle", Mouse(2)),
];

/// The binding for a `Key` tag, or `None` for a tag no variant has.
pub fn binding(tag: u8) -> Option<Binding> {
    KEYS.get(tag as usize).map(|&(_, binding)| binding)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The variant names inside `pub enum Key { ... }`, in order.
    fn input_kes_variants() -> Vec<&'static str> {
        let source = include_str!("../../driver/std/input.kes");
        let body = source.split("pub enum Key {").nth(1).expect("input.kes declares `pub enum Key`");
        let body = body.split('}').next().expect("the enum body ends");
        body.lines().map(str::trim).filter(|line| !line.is_empty()).collect()
    }

    #[test]
    fn keys_follow_input_kes() {
        let in_kes = input_kes_variants();
        let in_table: Vec<&str> = KEYS.iter().map(|&(name, _)| name).collect();
        assert_eq!(in_kes.len(), 63, "input.kes has 63 keys");
        assert_eq!(in_table, in_kes, "keys.rs lists the variants in input.kes order");
    }

    #[test]
    fn codes_match_raylib() {
        let code = |name: &str| KEYS.iter().find(|&&(n, _)| n == name).map(|&(_, b)| b).unwrap();
        assert_eq!(code("A"), Keys(&[65]));
        assert_eq!(code("Z"), Keys(&[90]));
        assert_eq!(code("Digit0"), Keys(&[48]));
        assert_eq!(code("Space"), Keys(&[32]));
        assert_eq!(code("Escape"), Keys(&[256]));
        assert_eq!(code("Left"), Keys(&[263]));
        assert_eq!(code("F1"), Keys(&[290]));
        assert_eq!(code("F12"), Keys(&[301]));
        assert_eq!(code("Shift"), Keys(&[340, 344]), "left and right shift");
        assert_eq!(code("Ctrl"), Keys(&[341, 345]), "left and right control");
        assert_eq!(code("Alt"), Keys(&[342, 346]), "left and right alt");
        assert_eq!(code("MouseLeft"), Mouse(0));
        assert_eq!(code("MouseRight"), Mouse(1));
        assert_eq!(code("MouseMiddle"), Mouse(2));
    }

    #[test]
    fn no_two_keys_share_a_code() {
        let mut seen = std::collections::HashSet::new();
        for (name, binding) in KEYS {
            let codes = match binding {
                Keys(codes) => codes.iter().map(|&c| (false, c)).collect::<Vec<_>>(),
                Mouse(button) => vec![(true, button)],
            };
            for code in codes {
                assert!(seen.insert(code), "{name} repeats a code");
            }
        }
    }

    #[test]
    fn a_tag_with_no_variant_has_no_binding() {
        assert_eq!(binding(0), Some(Keys(&[65])));
        assert_eq!(binding(62), Some(Mouse(2)));
        assert_eq!(binding(63), None);
        assert_eq!(binding(255), None);
    }
}
