//! Snapshot compression is separate from the opaque VT checkpoint format.
use crate::{MAX_RAW_CHECKPOINT_BYTES, MAX_SNAPSHOT_BYTES};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::sync::OnceLock;
use terminal_codec::{Envelope, LZ4, Limits, Registry, ZLIB};

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)]
    Codec(#[from] terminal_codec::Error),
    #[error(
        "The snapshot carrier is invalid ({reason}). Preserve the current display and request a fresh compatible snapshot."
    )]
    Carrier { reason: &'static str },
    #[error(
        "The snapshot is missing {field}. Update the host and viewer together, then request a new snapshot."
    )]
    Missing { field: &'static str },
    #[error(
        "Snapshot {field} needs {actual} bytes, above its {limit}-byte limit. Reduce retained history before reconnecting."
    )]
    Limit {
        field: &'static str,
        actual: usize,
        limit: usize,
    },
    #[error(
        "Snapshot base64 data is damaged. Preserve the current display and request a fresh snapshot."
    )]
    Base64 {
        #[source]
        source: base64::DecodeError,
    },
}

/// Protocol2's numeric-array Zlib envelope remains readable. Protocol3 emits
/// an explicit codec envelope with base64 instead of JSON integer arrays.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    #[serde(default)]
    encoding: Option<String>,
    #[serde(default)]
    version: Option<u8>,
    #[serde(default)]
    codec: Option<String>,
    decoded_bytes: usize,
    #[serde(default)]
    encoded_bytes: Option<usize>,
    #[serde(default)]
    raw_crc32: Option<u32>,
    #[serde(default)]
    encoded_crc32: Option<u32>,
    #[serde(default)]
    data: Option<Vec<u8>>,
    #[serde(default)]
    data_base64: Option<String>,
}
#[derive(Serialize)]
struct Encoded<'a> {
    version: u8,
    codec: &'a str,
    decoded_bytes: usize,
    encoded_bytes: usize,
    raw_crc32: u32,
    encoded_crc32: u32,
    data_base64: String,
}
fn limits() -> Limits {
    Limits {
        max_decoded_bytes: MAX_RAW_CHECKPOINT_BYTES,
        max_encoded_bytes: MAX_SNAPSHOT_BYTES,
    }
}
fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(Registry::default)
}
fn encode(raw: &[u8]) -> Result<Envelope, Error> {
    registry()
        .encode_preferred(&[LZ4, ZLIB], raw, limits())
        .map_err(Into::into)
}
fn required<T>(value: Option<T>, field: &'static str) -> Result<T, Error> {
    value.ok_or(Error::Missing { field })
}
fn check(field: &'static str, actual: usize, limit: usize) -> Result<(), Error> {
    if actual > limit {
        return Err(Error::Limit {
            field,
            actual,
            limit,
        });
    }
    Ok(())
}
fn decode(payload: Payload) -> Result<Vec<u8>, Error> {
    check(
        "decoded checkpoint",
        payload.decoded_bytes,
        MAX_RAW_CHECKPOINT_BYTES,
    )?;
    if let Some(encoding) = payload.encoding {
        if encoding != "zlib"
            || payload.version.is_some()
            || payload.codec.is_some()
            || payload.encoded_bytes.is_some()
            || payload.raw_crc32.is_some()
            || payload.encoded_crc32.is_some()
            || payload.data_base64.is_some()
        {
            return Err(Error::Carrier {
                reason: "unsupported or ambiguous legacy encoding",
            });
        }
        let data = required(payload.data, "legacy compressed bytes")?;
        return registry()
            .decode_legacy_zlib(&data, payload.decoded_bytes, limits())
            .map_err(Into::into);
    }
    if payload.data.is_some() {
        return Err(Error::Carrier {
            reason: "new snapshots require one base64 byte carrier",
        });
    }
    let encoded_bytes = required(payload.encoded_bytes, "encoded_bytes")?;
    check("compressed checkpoint", encoded_bytes, MAX_SNAPSHOT_BYTES)?;
    let encoded = required(payload.data_base64, "data_base64")?;
    check(
        "base64 carrier",
        encoded.len(),
        MAX_SNAPSHOT_BYTES.div_ceil(3) * 4,
    )?;
    let envelope = Envelope {
        version: required(payload.version, "version")?,
        codec: required(payload.codec, "codec")?,
        decoded_bytes: payload.decoded_bytes,
        encoded_bytes,
        raw_crc32: required(payload.raw_crc32, "raw_crc32")?,
        encoded_crc32: required(payload.encoded_crc32, "encoded_crc32")?,
        data: STANDARD
            .decode(encoded)
            .map_err(|source| Error::Base64 { source })?,
    };
    registry().decode(&envelope, limits()).map_err(Into::into)
}
pub(super) fn serialize<S: Serializer>(raw: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
    let envelope = encode(raw).map_err(serde::ser::Error::custom)?;
    Encoded {
        version: envelope.version,
        codec: &envelope.codec,
        decoded_bytes: envelope.decoded_bytes,
        encoded_bytes: envelope.encoded_bytes,
        raw_crc32: envelope.raw_crc32,
        encoded_crc32: envelope.encoded_crc32,
        data_base64: STANDARD.encode(&envelope.data),
    }
    .serialize(serializer)
}
pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
    decode(Payload::deserialize(deserializer)?).map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SNAPSHOT_ENGINE, ServerFrame};
    fn frame(bytes: Vec<u8>) -> ServerFrame {
        ServerFrame::Snapshot {
            id: 1,
            engine: SNAPSHOT_ENGINE.into(),
            bytes,
            offset: 42,
            cols: 100,
            rows: 30,
            live: true,
            exited: None,
        }
    }
    fn payload(raw: &[u8]) -> serde_json::Value {
        serde_json::to_value(frame(raw.to_vec())).unwrap()["bytes"].clone()
    }
    fn restore(value: serde_json::Value) -> Result<Vec<u8>, serde_json::Error> {
        let mut value_frame = serde_json::to_value(frame(Vec::new())).unwrap();
        value_frame["bytes"] = value;
        serde_json::from_value::<ServerFrame>(value_frame).map(|frame| match frame {
            ServerFrame::Snapshot { bytes, .. } => bytes,
            _ => unreachable!(),
        })
    }
    #[test]
    fn native_bytes_roundtrip_in_a_compact_carrier_with_recorded_codec() {
        for raw in [
            Vec::new(),
            vec![0, 255, 27, 0, 128, 10],
            (0..9 * 1024 * 1024).map(|i| (i % 251) as u8).collect(),
        ] {
            let value = payload(&raw);
            assert_eq!(value["codec"], LZ4);
            assert!(value["data_base64"].is_string());
            assert!(value.get("data").is_none());
            assert_eq!(restore(value).unwrap(), raw);
        }
    }
    #[test]
    fn historical_zlib_carriers_remain_readable_with_strict_end_checks() {
        let raw = b"a saved checkpoint";
        let envelope = registry().encode(ZLIB, raw, limits()).unwrap();
        let legacy =
            serde_json::json!({"encoding":"zlib","decoded_bytes":raw.len(),"data":envelope.data});
        assert_eq!(restore(legacy.clone()).unwrap(), raw);
        let mut truncated = legacy.clone();
        truncated["data"].as_array_mut().unwrap().pop();
        assert!(restore(truncated).is_err());
        let mut trailing = legacy;
        trailing["data"].as_array_mut().unwrap().push(123.into());
        assert!(restore(trailing).is_err());
    }
    #[test]
    fn excessive_claims_and_expansion_are_rejected() {
        let mut value = payload(b"tiny");
        value["decoded_bytes"] = (MAX_RAW_CHECKPOINT_BYTES + 1).into();
        assert!(restore(value).is_err());
        let mut value = payload(b"tiny");
        value["encoded_bytes"] = (MAX_SNAPSHOT_BYTES + 1).into();
        assert!(restore(value).is_err());
        let mut value = payload(&vec![b'x'; 1024 * 1024]);
        value["decoded_bytes"] = 1.into();
        assert!(restore(value).is_err());
        assert!(encode(&vec![0; MAX_RAW_CHECKPOINT_BYTES + 1]).is_err());
    }
    #[test]
    fn corrupt_bytes_checksum_and_ambiguous_carriers_are_rejected() {
        let mut value = payload(b"checkpoint");
        value["data_base64"] = "!invalid".into();
        assert!(restore(value).is_err());
        let mut value = payload(b"checkpoint");
        value["raw_crc32"] = (value["raw_crc32"].as_u64().unwrap() ^ 1).into();
        assert!(restore(value).is_err());
        let mut value = payload(b"checkpoint");
        value["encoded_crc32"] = (value["encoded_crc32"].as_u64().unwrap() ^ 1).into();
        assert!(restore(value).is_err());
        let mut value = payload(b"checkpoint");
        value["data"] = serde_json::json!([1, 2, 3]);
        assert!(restore(value).is_err());
        let mut value = payload(b"checkpoint");
        value["codec"] = "missing-codec-v1".into();
        assert!(restore(value).is_err());
    }
}
