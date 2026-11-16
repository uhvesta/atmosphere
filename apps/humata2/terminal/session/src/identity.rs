use crate::{Error, SavedSession, error::disk};
use std::{collections::BTreeMap, fs::File, io::Read, path::Path};

pub(super) fn new_uuid() -> Result<String, Error> {
    let path = Path::new("/dev/urandom");
    let mut bytes = [0; 16];
    File::open(path)
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|source| disk("allocate a durable terminal UUID using", path, source))?;
    Ok(uuid::Builder::from_random_bytes(bytes)
        .into_uuid()
        .to_string())
}
pub(super) fn validate_uuid(value: &str) -> Result<(), Error> {
    if value.len() != 36 {
        return Err(Error::UuidFormat {
            reason: "expected a 36-byte canonical hyphenated UUID",
        });
    }
    let uuid = uuid::Uuid::parse_str(value).map_err(|source| Error::UuidParse { source })?;
    if uuid.is_nil() || uuid.is_max() || uuid.hyphenated().to_string() != value {
        return Err(Error::UuidFormat {
            reason: "use a non-sentinel lowercase canonical UUID",
        });
    }
    Ok(())
}
pub(super) fn validate_title(id: u64, title: &str) -> Result<(), Error> {
    text(id, "title", title, 512, false)
}
pub(super) fn validate_attributes(
    id: u64,
    attributes: &BTreeMap<String, String>,
) -> Result<(), Error> {
    if attributes.len() > 32 {
        return Err(Error::SessionMetadata {
            id,
            field: "attributes",
            actual: attributes.len(),
            limit: 32,
            reason: "too many entries",
        });
    }
    let mut total = 0usize;
    for (key, value) in attributes {
        text(id, "attribute key", key, 64, false)?;
        text(id, "attribute value", value, 4096, true)?;
        total = total
            .checked_add(key.len())
            .and_then(|n| n.checked_add(value.len()))
            .ok_or(Error::SessionMetadata {
                id,
                field: "attributes",
                actual: usize::MAX,
                limit: 32768,
                reason: "size overflow",
            })?;
    }
    if total > 32768 {
        return Err(Error::SessionMetadata {
            id,
            field: "attributes",
            actual: total,
            limit: 32768,
            reason: "total metadata is too large",
        });
    }
    Ok(())
}
fn text(id: u64, field: &'static str, value: &str, limit: usize, empty: bool) -> Result<(), Error> {
    let reason = if value.len() > limit {
        "text exceeds this field's byte limit"
    } else if !empty && value.trim().is_empty() {
        "nonempty text is required"
    } else if value.chars().any(char::is_control) {
        "control characters are not allowed"
    } else {
        return Ok(());
    };
    Err(Error::SessionMetadata {
        id,
        field,
        actual: value.len(),
        limit,
        reason,
    })
}
pub(super) fn validate_session(session: &SavedSession, legacy: bool) -> Result<(), Error> {
    if !legacy || !session.uuid.is_empty() {
        validate_uuid(&session.uuid)?;
    }
    if !legacy || !session.automatic_title.is_empty() {
        validate_title(session.id, &session.automatic_title)?;
    }
    if let Some(name) = &session.name_override {
        validate_title(session.id, &name.title)?;
    }
    validate_attributes(session.id, &session.attributes)
}
pub(super) fn legacy_title(title: &str, id: u64) -> String {
    let mut bounded = String::new();
    for c in title.chars().filter(|c| !c.is_control()) {
        if bounded.len() + c.len_utf8() > 512 {
            break;
        }
        bounded.push(c);
    }
    if bounded.trim().is_empty() {
        format!("Terminal {id}")
    } else {
        bounded
    }
}
