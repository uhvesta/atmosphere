//! The peer link's mutual proof over the enrollment key (GRPC.md §8). When
//! the Mac adds a machine it writes a random key on it, as hex in a 0600
//! file, and keeps a copy in its keychain. In Hello the machine proves it
//! holds the key over the host's nonce and its own; in ProveHost the host
//! proves it over both the other way round. Each proof starts with a label
//! naming its side, so one side's proof never passes as the other's.
//!
//! An uncontained agent (SPEC D1) runs as the user and can read the key
//! file, so this keeps out a swapped machine and a contained agent, not an
//! uncontained one.

use std::fs::File;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use hmac::{Hmac, KeyInit, Mac};
use rustix::fs::{Mode, OFlags};
use rustix::io::Errno;
use sha2::Sha256;

/// Where the key lives in the state folder.
pub const KEY_FILE: &str = "enrollment-key";

/// The fewest bytes a host nonce may have.
pub const HOST_NONCE_MIN: usize = 32;
/// The most bytes a host nonce may have.
pub const HOST_NONCE_MAX: usize = 64;
/// The machine's nonce is always this long, so the bytes a proof covers
/// split into the two nonces one way only.
pub const MACHINE_NONCE_BYTES: usize = 32;

/// The fewest bytes a key may have.
const KEY_MIN: usize = 32;
/// The most of the key file read: far more than the hex of any key.
const KEY_FILE_MAX: u64 = 4096;

/// Whose proof: each covers a label of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// Hello's answer: over the host's nonce, then the machine's.
    Machine,
    /// ProveHost: over the machine's nonce, then the host's.
    Host,
}

impl Side {
    fn label(self) -> &'static [u8] {
        match self {
            Side::Machine => b"humata-peer-machine-v1",
            Side::Host => b"humata-peer-host-v1",
        }
    }
}

/// The enrollment key, keyed into an HMAC once.
pub struct EnrollmentKey {
    mac: Hmac<Sha256>,
}

impl EnrollmentKey {
    /// Reads the key at `path`. Refuses one other users can open, one that
    /// isn't this user's, and a link.
    pub fn read(path: &Path) -> Result<Self, KeyError> {
        let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let fd = match rustix::fs::open(path, flags, Mode::empty()) {
            Ok(fd) => fd,
            Err(Errno::NOENT) => {
                return Err(KeyError::NotEnrolled {
                    path: path.to_owned(),
                });
            }
            Err(Errno::LOOP) => {
                return Err(KeyError::NotAFile {
                    path: path.to_owned(),
                });
            }
            Err(errno) => {
                return Err(KeyError::Read {
                    path: path.to_owned(),
                    source: errno.into(),
                });
            }
        };
        let read_failed = |source| KeyError::Read {
            path: path.to_owned(),
            source,
        };
        let file = File::from(fd);
        let metadata = file.metadata().map_err(read_failed)?;
        if !metadata.is_file() {
            return Err(KeyError::NotAFile {
                path: path.to_owned(),
            });
        }
        if metadata.uid() != rustix::process::geteuid().as_raw() {
            return Err(KeyError::NotYours {
                path: path.to_owned(),
                owner: metadata.uid(),
            });
        }
        let mode = metadata.mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(KeyError::OpenToOthers {
                path: path.to_owned(),
                mode,
            });
        }
        let mut text = String::new();
        file.take(KEY_FILE_MAX)
            .read_to_string(&mut text)
            .map_err(read_failed)?;
        Self::from_hex(&text).ok_or_else(|| KeyError::Damaged {
            path: path.to_owned(),
        })
    }

    /// A key written as hex, with any whitespace around it.
    fn from_hex(text: &str) -> Option<Self> {
        let key = decode_hex(text.trim())?;
        if key.len() < KEY_MIN {
            return None;
        }
        let mac = Hmac::<Sha256>::new_from_slice(&key).ok()?;
        Some(Self { mac })
    }

    /// `side`'s proof for one link's two nonces.
    pub fn proof(&self, side: Side, host_nonce: &[u8], machine_nonce: &[u8]) -> Vec<u8> {
        self.keyed(side, host_nonce, machine_nonce)
            .finalize()
            .into_bytes()
            .to_vec()
    }

    /// Whether `proof` is `side`'s proof for the two nonces, compared in
    /// constant time.
    pub fn verify(
        &self,
        side: Side,
        host_nonce: &[u8],
        machine_nonce: &[u8],
        proof: &[u8],
    ) -> bool {
        self.keyed(side, host_nonce, machine_nonce)
            .verify_slice(proof)
            .is_ok()
    }

    fn keyed(&self, side: Side, host_nonce: &[u8], machine_nonce: &[u8]) -> Hmac<Sha256> {
        let (first, second) = match side {
            Side::Machine => (host_nonce, machine_nonce),
            Side::Host => (machine_nonce, host_nonce),
        };
        let mut mac = self.mac.clone();
        mac.update(side.label());
        mac.update(first);
        mac.update(second);
        mac
    }
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    let digits = text.as_bytes();
    if digits.len() % 2 != 0 {
        return None;
    }
    digits
        .chunks_exact(2)
        .map(|pair| {
            let high = char::from(pair[0]).to_digit(16)?;
            let low = char::from(pair[1]).to_digit(16)?;
            u8::try_from(high * 16 + low).ok()
        })
        .collect()
}

/// Why the machine has no key it can prove itself with. `Display` says
/// what failed; `fix` says what to do.
#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error(
        "this machine isn't enrolled: it has no enrollment key at {}",
        path.display()
    )]
    NotEnrolled { path: PathBuf },

    #[error(
        "the enrollment key {} can be opened by other users (mode {mode:o})",
        path.display()
    )]
    OpenToOthers { path: PathBuf, mode: u32 },

    #[error(
        "the enrollment key {} belongs to user {owner}, not to the user running humatad2",
        path.display()
    )]
    NotYours { path: PathBuf, owner: u32 },

    #[error(
        "the enrollment key {} isn't a plain file; it may be a link to somewhere else",
        path.display()
    )]
    NotAFile { path: PathBuf },

    #[error("couldn't read the enrollment key {}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "the enrollment key {} isn't one Humata wrote: it should be at least 32 bytes as hex",
        path.display()
    )]
    Damaged { path: PathBuf },
}

impl KeyError {
    /// What the person can do about it.
    pub fn fix(&self) -> &'static str {
        match self {
            KeyError::NotEnrolled { .. } => {
                "add this machine from Humata on the Mac, which writes the key. If it was added before, its state was wiped or replaced, so it is added as a new machine"
            }
            KeyError::OpenToOthers { .. } | KeyError::NotYours { .. } | KeyError::NotAFile { .. } => {
                "don't just fix its permissions: someone may already have read or replaced it and could pose as your Mac. Forget this machine in Humata on the Mac and add it again, which writes a new key"
            }
            KeyError::Read { .. } | KeyError::Damaged { .. } => {
                "check that the file belongs to you and is readable. If it is gone or damaged, forget this machine in Humata on the Mac and add it again, which writes a new key"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOST_NONCE: [u8; 32] = [1; 32];
    const MACHINE_NONCE: [u8; 32] = [2; 32];

    fn key(byte: u8) -> EnrollmentKey {
        let hex: String = std::iter::repeat_n(format!("{byte:02x}"), 32).collect();
        EnrollmentKey::from_hex(&hex).unwrap()
    }

    #[test]
    fn each_sides_proof_verifies() {
        let key = key(0xab);
        for side in [Side::Machine, Side::Host] {
            let proof = key.proof(side, &HOST_NONCE, &MACHINE_NONCE);
            assert!(key.verify(side, &HOST_NONCE, &MACHINE_NONCE, &proof));
        }
    }

    #[test]
    fn a_machine_proof_is_not_a_host_proof() {
        let key = key(0xab);
        let machine = key.proof(Side::Machine, &HOST_NONCE, &MACHINE_NONCE);
        assert!(!key.verify(Side::Host, &HOST_NONCE, &MACHINE_NONCE, &machine));
        assert!(!key.verify(Side::Host, &MACHINE_NONCE, &HOST_NONCE, &machine));
        let host = key.proof(Side::Host, &HOST_NONCE, &MACHINE_NONCE);
        assert!(!key.verify(Side::Machine, &HOST_NONCE, &MACHINE_NONCE, &host));
    }

    #[test]
    fn another_key_or_nonce_fails() {
        let proof = key(0xab).proof(Side::Host, &HOST_NONCE, &MACHINE_NONCE);
        assert!(!key(0xcd).verify(Side::Host, &HOST_NONCE, &MACHINE_NONCE, &proof));
        assert!(!key(0xab).verify(Side::Host, &[3; 32], &MACHINE_NONCE, &proof));
        assert!(!key(0xab).verify(Side::Host, &HOST_NONCE, &[3; 32], &proof));
        assert!(!key(0xab).verify(Side::Host, &HOST_NONCE, &MACHINE_NONCE, &proof[..16]));
        assert!(!key(0xab).verify(Side::Host, &HOST_NONCE, &MACHINE_NONCE, &[]));
    }

    #[test]
    fn a_key_must_be_long_enough_hex() {
        assert!(EnrollmentKey::from_hex(&"ab".repeat(32)).is_some());
        assert!(EnrollmentKey::from_hex(&format!("  {}\n", "AB".repeat(32))).is_some());
        assert!(EnrollmentKey::from_hex(&"ab".repeat(31)).is_none());
        assert!(EnrollmentKey::from_hex(&"zz".repeat(32)).is_none());
        assert!(EnrollmentKey::from_hex(&"a".repeat(65)).is_none());
        assert!(EnrollmentKey::from_hex("").is_none());
    }

    fn write_key(dir: &Path, mode: u32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(KEY_FILE);
        std::fs::write(&path, "ab".repeat(32)).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    #[test]
    fn a_private_key_file_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let read = EnrollmentKey::read(&write_key(dir.path(), 0o600)).unwrap();
        let proof = key(0xab).proof(Side::Host, &HOST_NONCE, &MACHINE_NONCE);
        assert!(read.verify(Side::Host, &HOST_NONCE, &MACHINE_NONCE, &proof));
    }

    #[test]
    fn a_key_file_others_can_open_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        for mode in [0o644, 0o640, 0o604] {
            let read = EnrollmentKey::read(&write_key(dir.path(), mode));
            assert!(matches!(read, Err(KeyError::OpenToOthers { .. })), "{mode:o}");
        }
    }

    #[test]
    fn a_missing_key_file_means_not_enrolled() {
        let dir = tempfile::tempdir().unwrap();
        let read = EnrollmentKey::read(&dir.path().join(KEY_FILE));
        assert!(matches!(read, Err(KeyError::NotEnrolled { .. })));
    }

    #[test]
    fn a_linked_key_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let real = write_key(dir.path(), 0o600);
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(matches!(EnrollmentKey::read(&link), Err(KeyError::NotAFile { .. })));
    }
}
