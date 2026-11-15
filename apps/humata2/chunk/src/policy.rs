//! Which codec a stream uses: the user's preference, the link, and what
//! the other end supports.

use serde::Deserialize;

use crate::codec::Codec;

/// Settings › Advanced › Compression, globally or for one machine (TR-11).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Preference {
    /// lz4 locally and on fast links, zstd on slow ones.
    #[default]
    Auto,
    Lz4,
    Zstd,
    /// Send everything as it is.
    Off,
}

/// The link a stream runs over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    /// A unix socket on this machine.
    Local,
    /// Another machine. `bytes_per_second` is the link's measured
    /// throughput, unknown until enough has crossed it.
    Remote { bytes_per_second: Option<u64> },
}

/// Under this measured throughput Auto picks zstd: its smaller chunks are
/// worth its slower compression (D56, D65).
pub const SLOW_LINK_BYTES_PER_SECOND: u64 = 50_000_000;

const LZ4_FIRST: &[Codec] = &[Codec::Lz4, Codec::Zstd, Codec::None];
const ZSTD_FIRST: &[Codec] = &[Codec::Zstd, Codec::Lz4, Codec::None];
const NONE_ONLY: &[Codec] = &[Codec::None];

/// The codecs to use with `preference` on `link`, best first. It is also
/// the list a daemon offers its peers: they send with the first one they
/// support. Off offers only None, so peers send as is too. A preference for
/// one compressor falls back to the other before none, for a peer that
/// lacks it.
pub fn ranked(preference: Preference, link: Link) -> &'static [Codec] {
    match preference {
        Preference::Off => NONE_ONLY,
        Preference::Lz4 => LZ4_FIRST,
        Preference::Zstd => ZSTD_FIRST,
        Preference::Auto => match link {
            Link::Remote {
                bytes_per_second: Some(measured),
            } if measured < SLOW_LINK_BYTES_PER_SECOND => ZSTD_FIRST,
            // Until a remote link is measured, assume it's fast: lz4 costs
            // little if it isn't.
            Link::Local | Link::Remote { .. } => LZ4_FIRST,
        },
    }
}

/// The codec to send a stream with: the first of `ranked` that the peer
/// supports. A peer that lists nothing predates compression and gets
/// chunks as they are.
pub fn choose(preference: Preference, link: Link, peer: &[Codec]) -> Codec {
    ranked(preference, link)
        .iter()
        .copied()
        .find(|codec| peer.contains(codec))
        .unwrap_or(Codec::None)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVERYTHING: &[Codec] = &[Codec::None, Codec::Lz4, Codec::Zstd];

    fn remote(bytes_per_second: u64) -> Link {
        Link::Remote {
            bytes_per_second: Some(bytes_per_second),
        }
    }

    #[test]
    fn auto_is_lz4_locally_and_on_fast_or_unmeasured_links() {
        assert_eq!(choose(Preference::Auto, Link::Local, EVERYTHING), Codec::Lz4);
        assert_eq!(choose(Preference::Auto, remote(200_000_000), EVERYTHING), Codec::Lz4);
        assert_eq!(choose(Preference::Auto, remote(SLOW_LINK_BYTES_PER_SECOND), EVERYTHING), Codec::Lz4);
        let unmeasured = Link::Remote { bytes_per_second: None };
        assert_eq!(choose(Preference::Auto, unmeasured, EVERYTHING), Codec::Lz4);
    }

    #[test]
    fn auto_is_zstd_on_a_slow_link() {
        assert_eq!(choose(Preference::Auto, remote(SLOW_LINK_BYTES_PER_SECOND - 1), EVERYTHING), Codec::Zstd);
        assert_eq!(choose(Preference::Auto, remote(2_000_000), EVERYTHING), Codec::Zstd);
    }

    #[test]
    fn a_picked_codec_holds_whatever_the_link() {
        assert_eq!(choose(Preference::Zstd, Link::Local, EVERYTHING), Codec::Zstd);
        assert_eq!(choose(Preference::Lz4, remote(1_000_000), EVERYTHING), Codec::Lz4);
        assert_eq!(choose(Preference::Off, remote(1_000_000), EVERYTHING), Codec::None);
    }

    #[test]
    fn a_peer_without_the_picked_codec_gets_the_other_one_then_none() {
        assert_eq!(choose(Preference::Zstd, Link::Local, &[Codec::Lz4, Codec::None]), Codec::Lz4);
        assert_eq!(choose(Preference::Auto, Link::Local, &[Codec::Zstd]), Codec::Zstd);
        assert_eq!(choose(Preference::Lz4, Link::Local, &[Codec::None]), Codec::None);
    }

    #[test]
    fn a_peer_that_lists_nothing_gets_chunks_as_they_are() {
        assert_eq!(choose(Preference::Auto, Link::Local, &[]), Codec::None);
        assert_eq!(choose(Preference::Zstd, remote(1), &[]), Codec::None);
    }

    #[test]
    fn off_offers_only_none() {
        assert_eq!(ranked(Preference::Off, Link::Local), &[Codec::None]);
        assert!(ranked(Preference::Auto, Link::Local).contains(&Codec::None));
    }
}
