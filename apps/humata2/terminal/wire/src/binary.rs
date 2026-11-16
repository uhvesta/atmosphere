//! Compact binary JSON carriers with bounded legacy-array compatibility.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{
    Deserializer, Serializer,
    de::{Error as _, SeqAccess, Visitor},
};
use std::fmt;

fn serialize<S: Serializer, const LIMIT: usize>(
    bytes: &[u8],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    if bytes.len() > LIMIT {
        return Err(serde::ser::Error::custom(format!(
            "Terminal byte payload exceeds its {LIMIT}-byte limit; split it before sending."
        )));
    }
    serializer.serialize_str(&STANDARD.encode(bytes))
}
fn deserialize<'de, D: Deserializer<'de>, const LIMIT: usize>(
    deserializer: D,
) -> Result<Vec<u8>, D::Error> {
    struct Bytes<const LIMIT: usize>;
    impl<'de, const LIMIT: usize> Visitor<'de> for Bytes<LIMIT> {
        type Value = Vec<u8>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(
                f,
                "base64 bytes or a legacy byte array of at most {LIMIT} bytes"
            )
        }
        fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
            if value.len() > LIMIT.div_ceil(3) * 4 {
                return Err(E::custom(
                    "The terminal base64 carrier exceeds its byte limit; split the payload.",
                ));
            }
            let bytes = STANDARD.decode(value).map_err(E::custom)?;
            if bytes.len() > LIMIT {
                return Err(E::custom(
                    "The terminal payload exceeds its decoded byte limit; split the payload.",
                ));
            }
            Ok(bytes)
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut bytes = Vec::new();
            while let Some(byte) = seq.next_element::<u8>()? {
                if bytes.len() == LIMIT {
                    return Err(A::Error::custom(
                        "The legacy terminal byte array exceeds its limit; split the payload.",
                    ));
                }
                if bytes.len() == bytes.capacity() {
                    bytes.try_reserve(1).map_err(A::Error::custom)?;
                }
                bytes.push(byte);
            }
            Ok(bytes)
        }
    }
    deserializer.deserialize_any(Bytes::<LIMIT>)
}

pub(super) mod input {
    use super::*;
    pub(crate) fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        super::serialize::<S, { crate::MAX_INPUT_BYTES }>(bytes, serializer)
    }
    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<u8>, D::Error> {
        super::deserialize::<D, { crate::MAX_INPUT_BYTES }>(deserializer)
    }
}
pub(super) mod output {
    use super::*;
    pub(crate) fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        super::serialize::<S, { crate::MAX_OUTPUT_BYTES }>(bytes, serializer)
    }
    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<u8>, D::Error> {
        super::deserialize::<D, { crate::MAX_OUTPUT_BYTES }>(deserializer)
    }
}

#[cfg(test)]
mod tests {
    use crate::{ClientFrame, MAX_INPUT_BYTES, ServerFrame};
    #[test]
    fn input_and_output_roundtrip_binary_as_compact_strings() {
        let bytes = vec![0, 255, 27, 10, 128];
        let input = ClientFrame::Input {
            id: 1,
            seq: 2,
            bytes: bytes.clone(),
        };
        let value = serde_json::to_value(input).unwrap();
        assert!(value["bytes"].is_string());
        assert!(
            matches!(serde_json::from_value::<ClientFrame>(value).unwrap(),ClientFrame::Input{bytes:actual,..}if actual==bytes)
        );
        let output = ServerFrame::Output {
            id: 1,
            offset: 12,
            bytes: bytes.clone(),
        };
        let value = serde_json::to_value(output).unwrap();
        assert!(value["bytes"].is_string());
        assert!(
            matches!(serde_json::from_value::<ServerFrame>(value).unwrap(),ServerFrame::Output{bytes:actual,..}if actual==bytes)
        );
    }
    #[test]
    fn bounded_legacy_arrays_remain_readable_and_bad_carriers_fail() {
        let legacy = serde_json::json!({"type":"input","id":1,"seq":2,"bytes":[0,255,27]});
        assert!(
            matches!(serde_json::from_value::<ClientFrame>(legacy).unwrap(),ClientFrame::Input{bytes,..}if bytes==[0,255,27])
        );
        for bytes in [
            serde_json::json!("!invalid"),
            serde_json::json!([256]),
            serde_json::json!(vec![0; MAX_INPUT_BYTES + 1]),
        ] {
            let value = serde_json::json!({"type":"input","id":1,"seq":2,"bytes":bytes});
            assert!(serde_json::from_value::<ClientFrame>(value).is_err());
        }
        assert!(
            serde_json::to_value(ClientFrame::Input {
                id: 1,
                seq: 1,
                bytes: vec![0; MAX_INPUT_BYTES + 1]
            })
            .is_err()
        );
    }
}
