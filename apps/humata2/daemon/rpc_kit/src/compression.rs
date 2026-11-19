//! Which codec this daemon's chunks use (TR-11, D65): the compression
//! setting in `<state>/compression.toml`, read for each stream so a change
//! applies to the next transfer without a restart, and, on a remote link,
//! the link's throughput measured from the transfers on it.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use humata2_chunk::{
    Codec, Header, Link, Preference, SETTINGS_FILE, Sample, Settings, Throughput, choose, ranked,
};
use humata2_grpc::humata::v1 as pb;

use crate::helpers;

pub struct Compression {
    file: PathBuf,
    /// The remote link's speed. A daemon has one remote link: the peer
    /// link to the host.
    remote: Mutex<Throughput>,
}

impl Compression {
    pub fn new(state: &Path) -> Self {
        Self {
            file: state.join(SETTINGS_FILE),
            remote: Mutex::new(Throughput::new()),
        }
    }

    /// The global preference. A file that can't be used is logged and
    /// read as Auto: a bad setting mustn't stop transfers.
    fn preference(&self) -> Preference {
        match Settings::load(&self.file) {
            Ok(settings) => settings.global(),
            Err(error) => {
                crate::log::say!(
                    "humatad2: warning: {}; using Auto until it is fixed",
                    helpers::chain(&error)
                );
                Preference::Auto
            }
        }
    }

    /// `link` with what has been measured of it.
    fn measured(&self, link: Link) -> Link {
        match link {
            Link::Local => Link::Local,
            Link::Remote { .. } => self
                .remote
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .link(),
        }
    }

    /// Counts a finished transfer on `link` toward its measured speed.
    pub fn record(&self, link: Link, sample: Sample) {
        if let Link::Remote { .. } = link {
            self.remote
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .record(sample);
        }
    }

    /// The codecs this daemon offers its callers, best first, as
    /// `humata.v1.ChunkCodec` numbers.
    pub fn offered(&self, link: Link) -> Vec<i32> {
        Codec::wire_list(ranked(self.preference(), self.measured(link)))
    }

    /// The codec to send a stream with to a reader that decodes `accepted`.
    pub fn pick(&self, link: Link, accepted: &[i32]) -> Codec {
        choose(
            self.preference(),
            self.measured(link),
            &Codec::known(accepted),
        )
    }
}

/// A chunk header from a message's codec fields. A sender that predates
/// compression sets none of them: its bytes are the data.
pub fn header(codec: i32, raw_size: u32, crc32: Option<u32>, data: &[u8]) -> Option<Header> {
    let codec = Codec::from_wire(codec)?;
    let raw_len = match (codec, raw_size) {
        (Codec::None, 0) => u32::try_from(data.len()).unwrap_or(u32::MAX),
        (_, raw_size) => raw_size,
    };
    Some(Header {
        codec,
        raw_len,
        crc32,
    })
}

/// The chunk header a `FileChunk` carries.
pub fn chunk_header(chunk: &pb::FileChunk) -> Option<Header> {
    header(chunk.codec, chunk.raw_size, chunk.crc32, &chunk.data)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const EVERYTHING: &[i32] = &[0, 1, 2];

    fn sample(millis: u64) -> Sample {
        Sample {
            bytes: 8 << 20,
            elapsed: Duration::from_millis(millis),
            waited: Duration::from_millis(millis),
        }
    }

    #[test]
    fn auto_turns_to_zstd_once_the_remote_link_measures_slow() {
        let dir = tempfile::tempdir().unwrap();
        let compression = Compression::new(dir.path());
        let remote = Link::Remote {
            bytes_per_second: None,
        };
        assert_eq!(compression.pick(remote, EVERYTHING), Codec::Lz4);

        compression.record(remote, sample(1000));

        assert_eq!(compression.pick(remote, EVERYTHING), Codec::Zstd);
        assert_eq!(
            compression.offered(remote).first(),
            Some(&Codec::Zstd.wire())
        );
        assert_eq!(compression.pick(Link::Local, EVERYTHING), Codec::Lz4);
    }

    #[test]
    fn local_transfers_do_not_count_toward_the_remote_link() {
        let dir = tempfile::tempdir().unwrap();
        let compression = Compression::new(dir.path());
        compression.record(Link::Local, sample(1000));
        let remote = Link::Remote {
            bytes_per_second: None,
        };
        assert_eq!(compression.pick(remote, EVERYTHING), Codec::Lz4);
    }

    #[test]
    fn off_overrides_a_slow_link() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(SETTINGS_FILE), "compression = \"off\"\n").unwrap();
        let compression = Compression::new(dir.path());
        let remote = Link::Remote {
            bytes_per_second: None,
        };
        compression.record(remote, sample(1000));
        assert_eq!(compression.pick(remote, EVERYTHING), Codec::None);
        assert_eq!(compression.offered(remote), vec![Codec::None.wire()]);
    }
}
