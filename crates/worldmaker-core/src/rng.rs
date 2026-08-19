//! Seeded randomness: one u64 master seed, PCG sub-streams keyed by
//! (seed, stage id, purpose).
//!
//! Every consumer of randomness anywhere in the simulation must get its RNG
//! from [`sub_rng`] so that adding a new consumer never disturbs the stream of
//! an existing one, and so a world is fully reproducible from its master seed.

use crate::hash::{fnv1a, splitmix64};
use rand_pcg::Pcg64Mcg;

/// Derive a PCG sub-stream for (master seed, stage id, purpose).
///
/// The two strings are hashed independently and mixed into the 128-bit PCG
/// state through SplitMix64, so distinct (stage, purpose) pairs get
/// statistically independent streams.
pub fn sub_rng(master_seed: u64, stage_id: &str, purpose: &str) -> Pcg64Mcg {
    let hs = fnv1a(stage_id.as_bytes());
    let hp = fnv1a(purpose.as_bytes());
    let lo = splitmix64(master_seed ^ hs.rotate_left(17) ^ hp);
    let hi = splitmix64(
        master_seed
            .wrapping_add(splitmix64(hs))
            .wrapping_add(hp.rotate_left(29)),
    );
    // Pcg64Mcg requires an odd state internally; the constructor handles that.
    Pcg64Mcg::new(((hi as u128) << 64) | lo as u128)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::RngCore;

    #[test]
    fn streams_are_reproducible_and_distinct() {
        let mut a1 = sub_rng(7, "tectonics", "plate-seeds");
        let mut a2 = sub_rng(7, "tectonics", "plate-seeds");
        let mut b = sub_rng(7, "tectonics", "hotspots");
        let mut c = sub_rng(8, "tectonics", "plate-seeds");
        let x1 = a1.next_u64();
        assert_eq!(x1, a2.next_u64(), "same key must reproduce the same stream");
        assert_ne!(
            x1,
            b.next_u64(),
            "different purpose must give a different stream"
        );
        assert_ne!(
            x1,
            c.next_u64(),
            "different master seed must give a different stream"
        );
    }
}
