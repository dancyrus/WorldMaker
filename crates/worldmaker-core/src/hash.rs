//! Small, stable, dependency-free hashing utilities.
//!
//! These hashes are part of the project's determinism contract: they are used
//! for seeds derived from text, for content-hashing fields, and for stage cache
//! keys. They must never change behavior between platforms or releases without
//! a decision-log entry.

pub const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
pub const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a 64-bit hash of a byte slice.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = FNV_OFFSET;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// Continue an FNV-1a hash with more bytes (for streaming several parts).
pub fn fnv1a_continue(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// Hash an `f32` slice by its little-endian bit patterns.
///
/// Used for determinism checks on whole fields. NaNs would hash by bit pattern;
/// simulation fields must not contain NaNs in the first place.
pub fn hash_f32_slice(data: &[f32]) -> u64 {
    let mut h = FNV_OFFSET;
    for &v in data {
        for b in v.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(FNV_PRIME);
        }
    }
    h
}

/// Hash a `u32` slice by little-endian bytes (determinism checks on id and
/// bitmask fields, e.g. plate ids).
pub fn hash_u32_slice(data: &[u32]) -> u64 {
    let mut h = FNV_OFFSET;
    for &v in data {
        for b in v.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(FNV_PRIME);
        }
    }
    h
}

/// SplitMix64: the standard 64-bit mixer, used to derive sub-seeds.
pub fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Turn arbitrary seed text into a u64 master seed. Never fails, never panics.
///
/// If the trimmed text parses as a plain unsigned integer it is used directly,
/// so "42" always means seed 42; anything else (including the empty string) is
/// FNV-1a hashed.
pub fn seed_from_text(text: &str) -> u64 {
    let t = text.trim();
    if let Ok(n) = t.parse::<u64>() {
        return n;
    }
    fnv1a(t.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_known_vector() {
        // FNV-1a("a") from the reference implementation.
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a(b""), FNV_OFFSET);
    }

    #[test]
    fn seed_text_numeric_and_weird_input() {
        assert_eq!(seed_from_text("42"), 42);
        assert_eq!(seed_from_text("  42  "), 42);
        assert_ne!(seed_from_text("dragon"), seed_from_text("Dragon"));
        // Never panics on odd input.
        let _ = seed_from_text("");
        let _ = seed_from_text("🐉🔥 ünïcødé \0 null");
        let _ = seed_from_text(&"x".repeat(100_000));
    }
}
