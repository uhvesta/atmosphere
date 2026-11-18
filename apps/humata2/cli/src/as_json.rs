//! Any reply as JSON, read through the protocol's descriptors so every
//! message prints the same way without code per type.
//!
//! Field names are the proto names. Every field is printed, with its
//! default when unset, except: a oneof member only when it is the one set,
//! and an unset message or `optional` field as `null`. Enum values are
//! lower case without their type's prefix (`BOARD_COLUMN_TO_DO` is
//! `"to_do"`). A message whose one field is `value` (ids, revisions) is
//! that value. Timestamps are RFC 3339, durations like `"90s"`, bytes
//! base64.

use std::sync::OnceLock;

use prost::Message;
use prost_reflect::{
    DescriptorPool, DynamicMessage, FieldDescriptor, Kind, MapKey, MessageDescriptor,
    ReflectMessage, Value,
};
use serde_json::{Map, Number, Value as Json, json};

static DESCRIPTORS: &[u8] = include_bytes!(env!("HUMATA2_DESCRIPTORS"));

fn pool() -> Option<&'static DescriptorPool> {
    static POOL: OnceLock<Option<DescriptorPool>> = OnceLock::new();
    POOL.get_or_init(|| DescriptorPool::decode(DESCRIPTORS).ok())
        .as_ref()
}

/// The descriptor for a message by its full name, like `humata.v1.Session`.
pub fn descriptor(full_name: &str) -> Option<MessageDescriptor> {
    pool()?.get_message_by_name(full_name)
}

/// The full name of the message a call takes, from its method path
/// (`/humata.v1.Service/Method`), as the protocol declares it.
pub fn request_name(path: &str) -> Option<String> {
    let (service, method) = path.strip_prefix('/')?.split_once('/')?;
    let service = pool()?.get_service_by_name(service)?;
    let method = service.methods().find(|candidate| candidate.name() == method)?;
    Some(method.input().full_name().to_owned())
}

/// An encoded `full_name` message as JSON. Bytes that don't decode print
/// as `{"undecoded": "<base64>"}` rather than failing the command.
pub fn encoded(full_name: &str, bytes: &[u8]) -> Json {
    let decoded = descriptor(full_name)
        .and_then(|descriptor| DynamicMessage::decode(descriptor, bytes).ok());
    match decoded {
        Some(message) => from_message(&message),
        None => json!({ "undecoded": base64(bytes) }),
    }
}

/// A typed message as JSON, by its full proto name.
pub fn typed(full_name: &str, message: &impl Message) -> Json {
    encoded(full_name, &message.encode_to_vec())
}

fn from_message(message: &DynamicMessage) -> Json {
    let descriptor = message.descriptor();
    match descriptor.full_name() {
        "google.protobuf.Timestamp" => {
            let seconds = int_field(message, "seconds");
            let nanos = int_field(message, "nanos");
            return Json::String(rfc3339(seconds, i32::try_from(nanos).unwrap_or(0)));
        }
        "google.protobuf.Duration" => {
            let seconds = int_field(message, "seconds");
            let nanos = int_field(message, "nanos");
            return Json::String(duration(seconds, nanos));
        }
        _ => {}
    }
    let fields: Vec<FieldDescriptor> = descriptor.fields().collect();
    if let [only] = fields.as_slice()
        && only.name() == "value"
        && !only.is_list()
        && !only.is_map()
        && !matches!(only.kind(), Kind::Message(_))
    {
        return from_value(&message.get_field(only), &only.kind());
    }
    let mut object = Map::new();
    for field in fields {
        let set = message.has_field(&field);
        let in_real_oneof = field
            .containing_oneof()
            .is_some_and(|oneof| !oneof.is_synthetic());
        if in_real_oneof && !set {
            continue;
        }
        let tracks_presence = field.supports_presence() && !field.is_list() && !field.is_map();
        let value = if tracks_presence && !set {
            Json::Null
        } else if field.is_map() {
            map(&message.get_field(&field), &field)
        } else {
            from_value(&message.get_field(&field), &field.kind())
        };
        object.insert(field.name().to_owned(), value);
    }
    Json::Object(object)
}

fn int_field(message: &DynamicMessage, name: &str) -> i64 {
    match message.get_field_by_name(name).as_deref() {
        Some(Value::I64(value)) => *value,
        Some(Value::I32(value)) => i64::from(*value),
        _ => 0,
    }
}

fn map(value: &Value, field: &FieldDescriptor) -> Json {
    let value_kind = match field.kind() {
        Kind::Message(entry) => entry.map_entry_value_field().kind(),
        other => other,
    };
    let Value::Map(entries) = value else {
        return Json::Object(Map::new());
    };
    let mut pairs: Vec<(String, Json)> = entries
        .iter()
        .map(|(key, value)| (map_key(key), from_value(value, &value_kind)))
        .collect();
    pairs.sort_by(|left, right| left.0.cmp(&right.0));
    Json::Object(pairs.into_iter().collect())
}

fn map_key(key: &MapKey) -> String {
    match key {
        MapKey::Bool(value) => value.to_string(),
        MapKey::I32(value) => value.to_string(),
        MapKey::I64(value) => value.to_string(),
        MapKey::U32(value) => value.to_string(),
        MapKey::U64(value) => value.to_string(),
        MapKey::String(value) => value.clone(),
    }
}

fn from_value(value: &Value, kind: &Kind) -> Json {
    match value {
        Value::Bool(value) => Json::Bool(*value),
        Value::I32(value) => json!(value),
        Value::I64(value) => json!(value),
        Value::U32(value) => json!(value),
        Value::U64(value) => json!(value),
        Value::F32(value) => Number::from_f64(f64::from(*value)).map_or(Json::Null, Json::Number),
        Value::F64(value) => Number::from_f64(*value).map_or(Json::Null, Json::Number),
        Value::String(value) => Json::String(value.clone()),
        Value::Bytes(value) => Json::String(base64(value)),
        Value::EnumNumber(number) => match kind {
            Kind::Enum(descriptor) => match descriptor.get_value(*number) {
                Some(found) => Json::String(enum_name(descriptor.name(), found.name())),
                None => json!(number),
            },
            _ => json!(number),
        },
        Value::Message(message) => from_message(message),
        Value::List(items) => Json::Array(items.iter().map(|item| from_value(item, kind)).collect()),
        Value::Map(_) => Json::Null,
    }
}

/// `BOARD_COLUMN_TO_DO` of `BoardColumn` is `to_do`.
pub fn enum_name(enum_name: &str, value: &str) -> String {
    let prefix = format!("{}_", screaming(enum_name));
    value
        .strip_prefix(&prefix)
        .unwrap_or(value)
        .to_ascii_lowercase()
}

/// `BoardColumn` is `BOARD_COLUMN`.
fn screaming(name: &str) -> String {
    let mut out = String::new();
    for (index, character) in name.chars().enumerate() {
        if character.is_ascii_uppercase() && index > 0 {
            out.push('_');
        }
        out.push(character.to_ascii_uppercase());
    }
    out
}

/// Seconds and nanoseconds since 1970 in UTC, as RFC 3339.
pub fn rfc3339(seconds: i64, nanos: i32) -> String {
    let days = seconds.div_euclid(86_400);
    let of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil(days);
    let mut text = format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}",
        of_day / 3600,
        of_day / 60 % 60,
        of_day % 60
    );
    if nanos > 0 {
        let fraction = format!("{nanos:09}");
        text.push('.');
        text.push_str(fraction.trim_end_matches('0'));
    }
    text.push('Z');
    text
}

/// Days since 1970-01-01 as a date (Howard Hinnant's `civil_from_days`).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month as u32, day as u32)
}

fn duration(seconds: i64, nanos: i64) -> String {
    if nanos == 0 {
        return format!("{seconds}s");
    }
    let fraction = format!("{:09}", nanos.unsigned_abs());
    let sign = if seconds < 0 || nanos < 0 { "-" } else { "" };
    format!(
        "{sign}{}.{}s",
        seconds.unsigned_abs(),
        fraction.trim_end_matches('0')
    )
}

/// Standard base64 with padding.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let triple = (u32::from(chunk[0]) << 16)
            | (u32::from(chunk.get(1).copied().unwrap_or(0)) << 8)
            | u32::from(chunk.get(2).copied().unwrap_or(0));
        let sextet = |shift: u32| char::from(ALPHABET[((triple >> shift) & 63) as usize]);
        out.push(sextet(18));
        out.push(sextet(12));
        out.push(if chunk.len() > 1 { sextet(6) } else { '=' });
        out.push(if chunk.len() > 2 { sextet(0) } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use humata2_proto::humata::v1 as pb;
    use serde_json::json;

    use super::*;
    use crate::daemon::rpc;

    #[test]
    fn every_reply_the_cli_reads_has_a_descriptor() {
        let missing: Vec<&str> = rpc::ALL
            .iter()
            .map(|rpc| rpc.response)
            .filter(|name| descriptor(name).is_none())
            .collect();
        assert_eq!(missing, Vec::<&str>::new());
    }

    #[test]
    fn ids_and_revisions_print_as_their_value() {
        let item = pb::BoardItem {
            id: Some(pb::BoardItemId { value: "bi_1".into() }),
            revision: Some(pb::Revision { value: 7 }),
            column: pb::BoardColumn::ToDo as i32,
            title: "Fix login".into(),
            ..Default::default()
        };
        let printed = typed("humata.v1.BoardItem", &item);
        assert_eq!(printed["id"], json!("bi_1"));
        assert_eq!(printed["revision"], json!(7));
        assert_eq!(printed["column"], json!("to_do"));
        assert_eq!(printed["title"], json!("Fix login"));
    }

    #[test]
    fn unset_messages_are_null_and_unset_oneof_members_are_left_out() {
        let item = pb::BoardItem::default();
        let printed = typed("humata.v1.BoardItem", &item);
        assert_eq!(printed["assigned_session"], Json::Null);
        assert_eq!(printed["description"], json!(""));

        let board = pb::BoardRef {
            board: Some(pb::board_ref::Board::Channel(pb::ChannelId { value: "ch_1".into() })),
        };
        let printed = typed("humata.v1.BoardRef", &board);
        assert_eq!(printed, json!({ "channel": "ch_1" }));
    }

    #[test]
    fn nested_enums_lose_their_own_prefix() {
        assert_eq!(enum_name("Method", "METHOD_CONFIRM_IN_APP"), "confirm_in_app");
        assert_eq!(enum_name("DaemonRole", "DAEMON_ROLE_HOST"), "host");
    }

    #[test]
    fn timestamps_are_utc_rfc3339() {
        assert_eq!(rfc3339(0, 0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_794_700_800, 0), "2026-11-15T00:00:00Z");
        assert_eq!(rfc3339(951_782_400, 500_000_000), "2000-02-29T00:00:00.5Z");
    }

    #[test]
    fn bytes_are_padded_base64() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
    }

    #[test]
    fn bytes_that_dont_decode_are_kept() {
        let printed = encoded("humata.v1.BoardItem", &[0xff, 0xff]);
        assert_eq!(printed, json!({ "undecoded": "//8=" }));
    }
}
