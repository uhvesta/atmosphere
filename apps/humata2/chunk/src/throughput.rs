//! A link's throughput, measured from the transfers that cross it: no
//! probe traffic and no timers (TR-11, D65). Auto reads it through
//! [`Throughput::link`] to pick zstd on slow links.

use std::time::Duration;

use crate::policy::{Link, SLOW_LINK_BYTES_PER_SECOND};

/// Transfers smaller than this say more about the link's latency than its
/// throughput, so they aren't counted.
const SAMPLE_MIN_BYTES: u64 = 1 << 20;

/// What one transfer showed about its link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    /// The bytes that crossed the link, as encoded.
    pub bytes: u64,
    /// From the transfer's first chunk to its last.
    pub elapsed: Duration,
    /// How long this end waited on the link in that time: a sender for room
    /// to hand it a chunk, a receiver for the next chunk to arrive.
    pub waited: Duration,
}

/// A remote link's estimated throughput, from the transfers on it.
#[derive(Clone, Debug, Default)]
pub struct Throughput {
    bytes_per_second: Option<f64>,
}

impl Throughput {
    pub fn new() -> Self {
        Self::default()
    }

    /// Counts a finished transfer. When this end spent at least half the
    /// transfer waiting, the link set its pace and the transfer's rate is
    /// the link's. Otherwise this end set the pace and the rate is only a
    /// floor for the link's: it counts only if that floor is already
    /// fast, as a slow floor says nothing about the link.
    pub fn record(&mut self, sample: Sample) {
        let seconds = sample.elapsed.as_secs_f64();
        if sample.bytes < SAMPLE_MIN_BYTES || seconds <= 0.0 {
            return;
        }
        let rate = sample.bytes as f64 / seconds;
        let link_bound = sample.waited.saturating_mul(2) >= sample.elapsed;
        if !link_bound && rate < SLOW_LINK_BYTES_PER_SECOND as f64 {
            return;
        }
        // Each transfer weighs as much as all before it, so a link that
        // changed (Wi-Fi to cable, a VPN) is noticed within a few.
        self.bytes_per_second = Some(match self.bytes_per_second {
            Some(before) => (before + rate) / 2.0,
            None => rate,
        });
    }

    /// The estimate, unknown until a transfer has counted.
    pub fn bytes_per_second(&self) -> Option<u64> {
        self.bytes_per_second.map(|rate| rate as u64)
    }

    /// The link as the codec policy sees it.
    pub fn link(&self) -> Link {
        Link::Remote {
            bytes_per_second: self.bytes_per_second(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::Codec;
    use crate::policy::{Preference, choose};

    const EVERYTHING: &[Codec] = &[Codec::None, Codec::Lz4, Codec::Zstd];
    const MIB: u64 = 1 << 20;

    fn link_bound(bytes: u64, millis: u64) -> Sample {
        Sample {
            bytes,
            elapsed: Duration::from_millis(millis),
            waited: Duration::from_millis(millis * 9 / 10),
        }
    }

    fn auto(throughput: &Throughput) -> Codec {
        choose(Preference::Auto, throughput.link(), EVERYTHING)
    }

    #[test]
    fn an_unmeasured_link_is_taken_as_fast() {
        let throughput = Throughput::new();
        assert_eq!(throughput.bytes_per_second(), None);
        assert_eq!(auto(&throughput), Codec::Lz4);
    }

    #[test]
    fn a_slow_link_gets_zstd_and_a_fast_one_lz4() {
        let mut slow = Throughput::new();
        // 8 MiB in a second: about 8 MB/s.
        slow.record(link_bound(8 * MIB, 1000));
        assert_eq!(auto(&slow), Codec::Zstd);

        let mut fast = Throughput::new();
        // 8 MiB in 20 ms: about 420 MB/s.
        fast.record(link_bound(8 * MIB, 20));
        assert_eq!(auto(&fast), Codec::Lz4);
    }

    #[test]
    fn small_transfers_do_not_count() {
        let mut throughput = Throughput::new();
        throughput.record(link_bound(MIB - 1, 1000));
        assert_eq!(throughput.bytes_per_second(), None);
    }

    #[test]
    fn a_slow_transfer_this_end_held_back_says_nothing_about_the_link() {
        let mut throughput = Throughput::new();
        throughput.record(Sample {
            bytes: 8 * MIB,
            elapsed: Duration::from_secs(1),
            waited: Duration::ZERO,
        });
        assert_eq!(throughput.bytes_per_second(), None);
        assert_eq!(auto(&throughput), Codec::Lz4);
    }

    #[test]
    fn a_fast_transfer_proves_the_link_fast_whoever_held_it_back() {
        let mut throughput = Throughput::new();
        throughput.record(link_bound(8 * MIB, 1000));
        for _ in 0..4 {
            throughput.record(Sample {
                bytes: 8 * MIB,
                elapsed: Duration::from_millis(20),
                waited: Duration::ZERO,
            });
        }
        assert_eq!(auto(&throughput), Codec::Lz4);
    }

    #[test]
    fn a_link_that_slows_down_moves_to_zstd_within_a_few_transfers() {
        let mut throughput = Throughput::new();
        throughput.record(link_bound(8 * MIB, 20));
        let mut transfers = 0;
        while auto(&throughput) == Codec::Lz4 {
            throughput.record(link_bound(8 * MIB, 1000));
            transfers += 1;
            assert!(transfers <= 4, "still lz4 after {transfers} slow transfers");
        }
    }
}
