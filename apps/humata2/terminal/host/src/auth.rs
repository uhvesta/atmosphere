use crate::{Error, error::disk};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
};
#[derive(Clone)]
pub(crate) struct Token(String);
impl Token {
    pub(crate) fn load_or_create(path: &Path) -> Result<Self, Error> {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
        {
            Ok(mut file) => {
                let mut random = [0u8; 32];
                File::open("/dev/urandom")
                    .and_then(|mut f| f.read_exact(&mut random))
                    .map_err(|e| disk("generate an authentication token", path, e))?;
                let value: String = random.iter().map(|b| format!("{b:02x}")).collect();
                file.write_all(value.as_bytes())
                    .and_then(|_| file.sync_all())
                    .map_err(|e| disk("save authentication", path, e))?;
                Ok(Self(value))
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata = fs::symlink_metadata(path)
                    .map_err(|e| disk("inspect authentication", path, e))?;
                if metadata.file_type().is_symlink()
                    || !metadata.is_file()
                    || metadata.permissions().mode() & 0o077 != 0
                    || metadata.len() != 64
                {
                    return Err(Error::Invalid {
                        reason: "the existing token must be a private 0600 regular file containing a 64-character hexadecimal token",
                    });
                }
                let file = OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                    .open(path)
                    .map_err(|e| disk("read authentication", path, e))?;
                let opened = file
                    .metadata()
                    .map_err(|e| disk("inspect authentication", path, e))?;
                if !opened.is_file()
                    || opened.permissions().mode() & 0o077 != 0
                    || opened.len() != 64
                {
                    return Err(Error::Invalid {
                        reason: "the authentication token changed while opening it; preserve it and retry with a private regular token file",
                    });
                }
                let mut value = String::with_capacity(65);
                file.take(65)
                    .read_to_string(&mut value)
                    .map_err(|e| disk("read authentication", path, e))?;
                if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(Error::Invalid {
                        reason: "the existing authentication token is malformed; preserve it and use a separate private host folder",
                    });
                }
                Ok(Self(value))
            }
            Err(e) => Err(disk("create authentication", path, e)),
        }
    }
    pub(crate) fn matches(&self, candidate: &str) -> bool {
        if candidate.len() != self.0.len() {
            return false;
        }
        self.0
            .bytes()
            .zip(candidate.bytes())
            .fold(0u8, |different, (a, b)| different | (a ^ b))
            == 0
    }
}

/// Loopback listeners also reject rebinding Host names. Forwarded loopback
/// ports are allowed, so compare Origin against the request Host, not bind port.
pub(crate) fn allowed_origin(host: Option<&str>, origin: Option<&str>) -> bool {
    let Some(host) = host else {
        return false;
    };
    let Ok(authority) = host.parse::<axum::http::uri::Authority>() else {
        return false;
    };
    if !matches!(
        authority.host(),
        "127.0.0.1" | "localhost" | "[::1]" | "::1"
    ) {
        return false;
    }
    let Some(origin) = origin else {
        return true;
    }; // Native clients still authenticate.
    let Ok(uri) = origin.parse::<axum::http::Uri>() else {
        return false;
    };
    matches!(uri.scheme_str(), Some("http" | "https"))
        && uri
            .authority()
            .is_some_and(|a| a.as_str().eq_ignore_ascii_case(host))
        && uri.path() == "/"
        && uri.query().is_none()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn origins_bind_to_loopback_host_but_allow_forwarded_ports() {
        assert!(allowed_origin(
            Some("127.0.0.1:8741"),
            Some("http://127.0.0.1:8741")
        ));
        assert!(allowed_origin(Some("localhost:8741"), None));
        assert!(!allowed_origin(
            Some("attacker.example:8741"),
            Some("http://attacker.example:8741")
        ));
        assert!(!allowed_origin(
            Some("127.0.0.1:8741"),
            Some("https://attacker.example")
        ));
        assert!(!allowed_origin(Some("localhost:8741"), Some("null")));
        assert!(!allowed_origin(
            Some("localhost:8741"),
            Some("http://localhost:8742")
        ));
    }
    #[test]
    fn authentication_is_private_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        let first = Token::load_or_create(&path).unwrap();
        let second = Token::load_or_create(&path).unwrap();
        assert!(first.matches(&second.0));
        assert!(!first.matches(&"0".repeat(64)));
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
