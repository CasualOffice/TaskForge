//! Deterministic primitives: the RNG, the identifier factory, and the clock.
//!
//! Every random draw in this tool comes from here, because a benchmark corpus
//! that differs between runs makes the numbers measured against it
//! incomparable (`docs/30-PERFORMANCE-AND-CAPACITY-TARGETS.md` §Measurement:
//! "committed as a seed script so every measurement is comparable").
//!
//! Three rules make that hold:
//!
//! 1. **No ambient entropy.** No `thread_rng`, no `SystemTime`, no
//!    `Uuid::now_v7()` — the latter embeds the wall clock in the identifier, so
//!    two runs of the same seed would produce different primary keys and a
//!    different B-tree layout.
//! 2. **Named streams.** Each concern draws from its own RNG derived from
//!    `(seed, stream name, index)`. Adding a draw to the comment generator
//!    therefore cannot shift the task titles, which keeps diffs between
//!    generator versions readable and lets projects be generated independently.
//! 3. **Integer arithmetic only.** No `ln`, `exp`, or `cos`: those route
//!    through the platform math library, which is free to differ by an ULP
//!    between targets. Skew is produced by integer weight tables instead
//!    (see `Det::weighted`).

use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};
use uuid::Uuid;

/// Milliseconds in a day, the unit every offset in this tool is expressed in.
pub const DAY_MS: i64 = 86_400_000;

/// A named deterministic random stream.
///
/// `SmallRng` is `xoshiro256++` on 64-bit targets and `xoshiro128++` on 32-bit
/// ones, so byte-identical output is guaranteed for a given `rand` version
/// (pinned by `Cargo.lock`) and pointer width — not across those. That is the
/// honest limit of the determinism claim.
#[derive(Debug)]
pub struct Det {
    seed: u64,
    rng: SmallRng,
}

impl Det {
    /// Open the stream called `name` for this corpus seed.
    pub fn stream(seed: u64, name: &str) -> Self {
        Self {
            seed,
            rng: SmallRng::seed_from_u64(mix(seed ^ fnv1a(name), 0x9e37_79b9_7f4a_7c15)),
        }
    }

    /// Open an independent sub-stream, addressed by name and index.
    ///
    /// Derived from the corpus seed rather than from this stream's current
    /// state, so consuming more draws here never perturbs the sub-stream.
    pub fn substream(&self, name: &str, index: u64) -> Self {
        Self {
            seed: self.seed,
            rng: SmallRng::seed_from_u64(mix(self.seed ^ fnv1a(name), index.wrapping_add(1))),
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.rng.random()
    }

    /// Uniform in `[0, n)`. Lemire's multiply-shift rather than `random_range`,
    /// so the output depends only on the raw 64-bit stream and not on the
    /// sampling algorithm of whichever `rand` version is in the lock file.
    ///
    /// The residual modulo bias is below 2^-64 relative for every `n` this tool
    /// uses (all are < 2^32), which is far under any distributional effect a
    /// query plan could notice.
    pub fn below(&mut self, n: u64) -> u64 {
        debug_assert!(n > 0, "below(0) has no value to return");
        let r = u128::from(self.next_u64());
        ((r * u128::from(n)) >> 64) as u64
    }

    /// Uniform in `[lo, hi)`.
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        debug_assert!(hi > lo, "empty range {lo}..{hi}");
        lo + self.below((hi - lo) as u64) as i64
    }

    /// True with probability `per_mille / 1000`.
    pub fn chance(&mut self, per_mille: u64) -> bool {
        self.below(1000) < per_mille
    }

    pub fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len() as u64) as usize]
    }

    /// Index into `weights`, chosen proportionally. The only skew primitive in
    /// the tool; every non-uniform distribution is a weight table so that the
    /// shape is visible at its call site instead of hidden in a formula.
    pub fn weighted(&mut self, weights: &[u32]) -> usize {
        let total: u64 = weights.iter().map(|w| u64::from(*w)).sum();
        let mut t = self.below(total);
        for (i, w) in weights.iter().enumerate() {
            let w = u64::from(*w);
            if t < w {
                return i;
            }
            t -= w;
        }
        weights.len() - 1
    }

    /// Number of successes before the first failure, capped. Used wherever a
    /// long thin tail is wanted (comments per task, edits per task).
    pub fn geometric(&mut self, success_per_mille: u64, cap: u32) -> u32 {
        let mut n = 0;
        while n < cap && self.chance(success_per_mille) {
            n += 1;
        }
        n
    }

    /// A UUIDv7-shaped identifier whose timestamp is `ms`, not the wall clock.
    ///
    /// The layout is RFC 9562 §5.7: 48-bit big-endian milliseconds, version 7,
    /// variant 10. Keeping the real v7 shape matters because the corpus exists
    /// to measure index behaviour, and v7 keys are what production inserts —
    /// time-ordered, so they append to the right-hand edge of the B-tree
    /// instead of scattering (`docs/22-DATABASE-SCHEMA.md` §Conventions).
    pub fn uuid_at(&mut self, ms: i64) -> Uuid {
        let mut b = [0u8; 16];
        let ts = (ms.max(0) as u64) & 0x0000_FFFF_FFFF_FFFF;
        b[..6].copy_from_slice(&ts.to_be_bytes()[2..]);
        let hi = self.next_u64().to_be_bytes();
        b[6..14].copy_from_slice(&hi);
        b[14..].copy_from_slice(&self.next_u64().to_be_bytes()[..2]);
        b[6] = 0x70 | (b[6] & 0x0F); // version 7
        b[8] = 0x80 | (b[8] & 0x3F); // variant 10
        Uuid::from_bytes(b)
    }

    /// Lowercase hex of `bytes` random bytes — checksums, token hashes, and
    /// other opaque strings that must be stable across runs.
    pub fn hex(&mut self, bytes: usize) -> String {
        let mut s = String::with_capacity(bytes * 2);
        for _ in 0..bytes {
            let byte = (self.below(256)) as u8;
            s.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
            s.push(char::from_digit(u32::from(byte & 0x0F), 16).unwrap_or('0'));
        }
        s
    }
}

/// SplitMix64 finalizer. Cheap, well-distributed, and specified in integer
/// arithmetic, so it is identical on every target.
fn mix(a: u64, b: u64) -> u64 {
    let mut z = a.wrapping_add(b).wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in s.as_bytes() {
        h ^= u64::from(*byte);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Zero-padded base-36, which sorts lexicographically in the same order as it
/// sorts numerically — the property the board rank depends on (ADR-013).
pub fn base36(mut v: u64, width: usize) -> String {
    const DIGITS: &[u8; 36] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut buf = Vec::with_capacity(width);
    while v > 0 {
        buf.push(DIGITS[(v % 36) as usize]);
        v /= 36;
    }
    while buf.len() < width {
        buf.push(b'0');
    }
    buf.reverse();
    String::from_utf8(buf).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_draws() {
        let draws = |seed| {
            let mut d = Det::stream(seed, "task");
            (0..64).map(|_| d.next_u64()).collect::<Vec<_>>()
        };
        assert_eq!(draws(7), draws(7));
        assert_ne!(draws(7), draws(8));
    }

    #[test]
    fn substreams_do_not_perturb_each_other() {
        let root = Det::stream(1, "root");
        let first: Vec<u64> = {
            let mut s = root.substream("project", 3);
            (0..8).map(|_| s.next_u64()).collect()
        };
        // Draw heavily from a sibling; the sub-stream must be unaffected.
        let mut sibling = root.substream("project", 2);
        for _ in 0..1000 {
            sibling.next_u64();
        }
        let second: Vec<u64> = {
            let mut s = root.substream("project", 3);
            (0..8).map(|_| s.next_u64()).collect()
        };
        assert_eq!(first, second);
    }

    #[test]
    fn uuid_is_v7_shaped_and_time_ordered() {
        let mut d = Det::stream(3, "ids");
        let early = d.uuid_at(1_700_000_000_000);
        let late = d.uuid_at(1_800_000_000_000);
        assert_eq!(early.get_version_num(), 7);
        assert_eq!(late.get_variant(), uuid::Variant::RFC4122);
        assert!(early < late, "v7 identifiers must sort by their timestamp");
    }

    #[test]
    fn uuid_timestamp_is_the_argument_not_the_clock() {
        let ms = 1_780_272_000_000_i64;
        let mut d = Det::stream(9, "ids");
        let id = d.uuid_at(ms);
        let mut top = [0u8; 8];
        top[2..].copy_from_slice(&id.as_bytes()[..6]);
        assert_eq!(u64::from_be_bytes(top) as i64, ms);
    }

    #[test]
    fn base36_sorts_lexicographically() {
        let mut prev = base36(0, 6);
        for v in (1..5000).map(|v| v * 37) {
            let next = base36(v, 6);
            assert!(next > prev, "{next} must sort after {prev}");
            prev = next;
        }
    }

    #[test]
    fn weighted_respects_the_table() {
        let mut d = Det::stream(11, "w");
        let mut hits = [0u32; 3];
        for _ in 0..10_000 {
            hits[d.weighted(&[1, 0, 9])] += 1;
        }
        assert_eq!(hits[1], 0, "a zero weight must never be selected");
        assert!(hits[2] > hits[0] * 5, "9:1 should show up as roughly 9:1");
    }
}
