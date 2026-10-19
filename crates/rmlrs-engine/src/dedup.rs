// SPDX-License-Identifier: Apache-2.0
//! The set of quad fingerprints used for duplicate removal.
//!
//! A fingerprint is 128 random-looking bits, so it needs no further
//! hashing. The set is split into 256 shards by the top 8 bits of the
//! fingerprint; a shard stores the low 95 bits plus a marker bit (12 bytes
//! per slot, in two arrays) and finds slots by linear probing. Compared with
//! the `HashSet<u128>` it replaces:
//!
//! - 12 bytes per slot instead of 17 (16-byte key and a control byte);
//! - growing copies one shard, not the whole set, so there is no moment
//!   where the old and the new table of the whole set are both alive (with
//!   one table, that moment is the peak: 1.5 times the table);
//! - no second hash of the fingerprint on every insert.
//!
//! Shards double when they would be more than 3/4 full. Growing by a
//! quarter instead kept the tables fuller, but measured worse (1.4 GB
//! against 0.43 GB on 20 million quads): the allocator kept the freed
//! smaller tables of 4,096 shards as holes it could not reuse. With 256
//! shards the tables soon become large allocations that go back to the
//! operating system when freed.
//!
//! The price is 25 fingerprint bits: two different quads are merged when
//! their 103 kept bits agree, about n² / 2¹⁰⁴ for n distinct quads
//! (2·10⁻¹⁵ at 200 million).

const SHARD_BITS: u32 = 8;
const SHARDS: usize = 1 << SHARD_BITS;
/// Grow when a shard would be more than `LOAD_NUM / LOAD_DEN` full.
const LOAD_NUM: usize = 3;
const LOAD_DEN: usize = 4;
const MIN_SLOTS: usize = 8;

#[derive(Debug, Default)]
pub(crate) struct FingerprintSet {
    shards: Vec<Shard>,
}

/// One shard: `hi[i] == 0` marks an empty slot.
#[derive(Debug, Default)]
struct Shard {
    hi: Vec<u64>,
    lo: Vec<u32>,
    len: usize,
}

impl FingerprintSet {
    /// Adds a fingerprint; `true` if it was not in the set.
    pub(crate) fn insert(&mut self, fp: u128) -> bool {
        if self.shards.is_empty() {
            self.shards.resize_with(SHARDS, Shard::default);
        }
        let shard = (fp >> (128 - SHARD_BITS)) as usize;
        // The low 95 bits. The top bit of `hi` is always set, so that a
        // zero `hi` can mark an empty slot.
        let hi = ((fp >> 32) as u64) | (1 << 63);
        let lo = fp as u32;
        self.shards[shard].insert(hi, lo)
    }

    /// Bytes held by the slots (for tests and measurements).
    #[cfg(test)]
    fn bytes(&self) -> usize {
        self.shards.iter().map(|s| s.hi.len() * 12).sum()
    }
}

impl Shard {
    fn insert(&mut self, hi: u64, lo: u32) -> bool {
        if (self.len + 1) * LOAD_DEN > self.hi.len() * LOAD_NUM {
            self.grow();
        }
        let n = self.hi.len();
        // The top bit of `hi` is the marker; the slot comes from the rest.
        let mut i = slot(hi << 1, n);
        loop {
            match self.hi[i] {
                0 => {
                    self.hi[i] = hi;
                    self.lo[i] = lo;
                    self.len += 1;
                    return true;
                }
                h if h == hi && self.lo[i] == lo => return false,
                _ => {
                    i += 1;
                    if i == n {
                        i = 0;
                    }
                }
            }
        }
    }

    fn grow(&mut self) {
        let n = (self.hi.len() * 2).max(MIN_SLOTS);
        let hi = std::mem::replace(&mut self.hi, vec![0; n]);
        let lo = std::mem::replace(&mut self.lo, vec![0; n]);
        self.len = 0;
        for (h, l) in hi.into_iter().zip(lo) {
            if h != 0 {
                self.insert(h, l);
            }
        }
    }
}

/// Maps `hi` onto `0..n` (multiply-shift, so `n` need not be a power of two).
fn slot(hi: u64, n: usize) -> usize {
    ((u128::from(hi) * n as u128) >> 64) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cheap stand-in for real fingerprints: SplitMix64 of a counter.
    fn fp(i: u64) -> u128 {
        let mix = |mut z: u64| {
            z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        };
        (u128::from(mix(i)) << 64) | u128::from(mix(!i))
    }

    #[test]
    fn inserts_once() {
        let mut s = FingerprintSet::default();
        let n = 200_000;
        for i in 0..n {
            assert!(s.insert(fp(i)));
        }
        for i in 0..n {
            assert!(!s.insert(fp(i)), "{i} seen before");
        }
        assert_eq!(s.shards.iter().map(|s| s.len).sum::<usize>(), n as usize);
    }

    #[test]
    fn zero_and_neighbours_are_distinct() {
        let mut s = FingerprintSet::default();
        assert!(s.insert(0));
        assert!(!s.insert(0));
        assert!(s.insert(1));
        assert!(s.insert(1 << 32));
        assert!(s.insert(u128::MAX));
    }

    #[test]
    fn memory_per_fingerprint() {
        let mut s = FingerprintSet::default();
        let n = 2_000_000;
        for i in 0..n {
            s.insert(fp(i));
        }
        // 12 bytes per slot at 37.5-75% load: 16 to 32 bytes per
        // fingerprint. A `HashSet<u128>` needs 19 to 39, plus the old table
        // while it grows.
        let per = s.bytes() as f64 / n as f64;
        assert!((16.0..=32.0).contains(&per), "{per} bytes per fingerprint");
    }
}
