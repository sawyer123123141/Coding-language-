//! `checksum(x)`: a value's contents folded into 64 bits in a fixed order (see the syntax spec's
//! "Built-in functions"). Compiled code does the folding itself, word by word, with these
//! constants; a string's bytes come here. Tests and tools compute the same fold in Rust with
//! `START`, `mix` and `finish`.

use crate::string::KStr;

/// The fold's starting value.
pub const START: u64 = 0x243F_6A88_85A3_08D3;

/// The multiplier of one fold step.
pub const MULTIPLIER: u64 = 0x9E37_79B9_7F4A_7C15;

/// One fold step: the word `w` mixed into `h`.
pub fn mix(h: u64, w: u64) -> u64 {
    let x = (h ^ w).wrapping_mul(MULTIPLIER);
    x ^ (x >> 32)
}

/// The last step: SplitMix64's finaliser of the folded value.
pub fn finish(h: u64) -> u64 {
    let mut z = h;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// `bytes` mixed into `h`: their length, then 8-byte little-endian words, the last padded with
/// zeros.
pub fn mix_bytes(h: u64, bytes: &[u8]) -> u64 {
    let mut h = mix(h, bytes.len() as u64);
    for chunk in bytes.chunks(8) {
        let mut word = [0u8; 8];
        word[..chunk.len()].copy_from_slice(chunk);
        h = mix(h, u64::from_le_bytes(word));
    }
    h
}

/// A string's bytes mixed into `h` (see `mix_bytes`). A null pointer is the empty string.
///
/// # Safety
/// `ptr` must be valid for reading `len` bytes unless it is null.
#[no_mangle]
pub unsafe extern "C" fn kestrel_rt_hash_bytes(h: i64, ptr: *const u8, len: i64) -> i64 {
    // SAFETY: the caller guarantees `ptr` for `len` bytes.
    let bytes = unsafe { KStr::bytes(ptr, len) };
    mix_bytes(h as u64, bytes) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_follows_the_definition() {
        let pairs = [(0u64, 0u64), (START, 1), (u64::MAX, 0x7FF8_0000_0000_0000), (12_345, u64::MAX), (0xDEAD_BEEF, 0xFFFF_FFFF_FFFF_FFF6)];
        for (h, w) in pairs {
            let x = (h ^ w).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            assert_eq!(mix(h, w), x ^ (x >> 32), "mix({h:#x}, {w:#x})");
        }
    }

    #[test]
    fn finish_is_splitmix64s_finaliser() {
        // SplitMix64 seeded with 0: its first output is the finaliser of the golden ratio.
        assert_eq!(finish(0x9E37_79B9_7F4A_7C15), 0xE220_A839_7B1D_CDAF);
        assert_eq!(finish(0), 0);
    }

    #[test]
    fn bytes_hash_as_padded_words() {
        let text = b"hello, world!";
        let want = mix(mix(mix(START, 13), u64::from_le_bytes(*b"hello, w")), u64::from_le_bytes(*b"orld!\0\0\0"));
        assert_eq!(mix_bytes(START, text), want);
        // SAFETY: `text` is a valid slice.
        let got = unsafe { kestrel_rt_hash_bytes(START as i64, text.as_ptr(), text.len() as i64) };
        assert_eq!(got as u64, want);
    }

    #[test]
    fn the_empty_string_is_its_length_alone() {
        // SAFETY: a null pointer is the empty string.
        let got = unsafe { kestrel_rt_hash_bytes(START as i64, std::ptr::null(), 0) };
        assert_eq!(got as u64, mix(START, 0));
        // Eight bytes are one word, with no padding word after it.
        assert_eq!(mix_bytes(START, b"abcdefgh"), mix(mix(START, 8), u64::from_le_bytes(*b"abcdefgh")));
    }
}
