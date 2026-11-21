//! The tools' input schemas: written once, sent as JSON Schema in
//! `tools/list`, and checked against every call before it reaches the
//! daemon (MCP.md §3: a call the schema rejects is a JSON-RPC error).
//!
//! Only the parts of JSON Schema the tools use: objects that allow no
//! other properties, strings with a length limit or a fixed set of values,
//! bounded integers, booleans, arrays, `null` where a field may be cleared,
//! and `oneOf`.

use std::fmt;

use serde_json::{Map, Value, json};

#[derive(Debug, Clone)]
pub(crate) struct Schema {
    kind: Kind,
    description: Option<&'static str>,
}

#[derive(Debug, Clone)]
enum Kind {
    String {
        max_length: Option<usize>,
        values: Option<Vec<&'static str>>,
    },
    Integer {
        minimum: Option<i64>,
        maximum: Option<i64>,
        default: Option<i64>,
    },
    Boolean,
    Array {
        items: Box<Schema>,
        min_items: Option<usize>,
        max_items: Option<usize>,
    },
    Object(Object),
    /// Any object: fields a provider defines, which the daemon checks.
    AnyObject,
    OneOf(Vec<Schema>),
}

#[derive(Debug, Clone)]
struct Object {
    properties: Vec<Property>,
    /// Exactly one of these must be given (`text` or `base64`).
    exactly_one_of: Vec<&'static str>,
}

/// One property of an object.
#[derive(Debug, Clone)]
pub(crate) struct Property {
    pub(crate) name: &'static str,
    schema: Schema,
    required: bool,
}

pub(crate) fn req(name: &'static str, schema: Schema) -> Property {
    Property {
        name,
        schema,
        required: true,
    }
}

pub(crate) fn opt(name: &'static str, schema: Schema) -> Property {
    Property {
        name,
        schema,
        required: false,
    }
}

fn of(kind: Kind) -> Schema {
    Schema {
        kind,
        description: None,
    }
}

pub(crate) fn string() -> Schema {
    of(Kind::String {
        max_length: None,
        values: None,
    })
}

pub(crate) fn one_of_strings(values: impl IntoIterator<Item = &'static str>) -> Schema {
    of(Kind::String {
        max_length: None,
        values: Some(values.into_iter().collect()),
    })
}

pub(crate) fn integer() -> Schema {
    of(Kind::Integer {
        minimum: None,
        maximum: None,
        default: None,
    })
}

pub(crate) fn boolean() -> Schema {
    of(Kind::Boolean)
}

pub(crate) fn array(items: Schema) -> Schema {
    of(Kind::Array {
        items: Box::new(items),
        min_items: None,
        max_items: None,
    })
}

pub(crate) fn object(properties: Vec<Property>) -> Schema {
    of(Kind::Object(Object {
        properties,
        exactly_one_of: Vec::new(),
    }))
}

pub(crate) fn any_object() -> Schema {
    of(Kind::AnyObject)
}

pub(crate) fn one_of(choices: Vec<Schema>) -> Schema {
    of(Kind::OneOf(choices))
}

impl Schema {
    pub(crate) fn describe(mut self, description: &'static str) -> Self {
        self.description = Some(description);
        self
    }

    /// At most `limit` characters (Unicode scalar values, as JSON Schema
    /// counts).
    pub(crate) fn max_length(mut self, limit: usize) -> Self {
        if let Kind::String { max_length, .. } = &mut self.kind {
            *max_length = Some(limit);
        }
        self
    }

    pub(crate) fn range(mut self, low: i64, high: i64) -> Self {
        if let Kind::Integer {
            minimum, maximum, ..
        } = &mut self.kind
        {
            *minimum = Some(low);
            *maximum = Some(high);
        }
        self
    }

    pub(crate) fn at_least(mut self, low: i64) -> Self {
        if let Kind::Integer { minimum, .. } = &mut self.kind {
            *minimum = Some(low);
        }
        self
    }

    pub(crate) fn default_value(mut self, value: i64) -> Self {
        if let Kind::Integer { default, .. } = &mut self.kind {
            *default = Some(value);
        }
        self
    }

    pub(crate) fn min_items(mut self, limit: usize) -> Self {
        if let Kind::Array { min_items, .. } = &mut self.kind {
            *min_items = Some(limit);
        }
        self
    }

    pub(crate) fn max_items(mut self, limit: usize) -> Self {
        if let Kind::Array { max_items, .. } = &mut self.kind {
            *max_items = Some(limit);
        }
        self
    }

    /// Exactly one of these properties must be given.
    pub(crate) fn exactly_one_of(mut self, names: &[&'static str]) -> Self {
        if let Kind::Object(object) = &mut self.kind {
            object.exactly_one_of = names.to_vec();
        }
        self
    }

    /// True when this is an object with that property.
    pub(crate) fn has_property(&self, name: &str) -> bool {
        match &self.kind {
            Kind::Object(object) => object.properties.iter().any(|property| property.name == name),
            _ => false,
        }
    }

    /// True when this is an object that requires that property.
    pub(crate) fn requires(&self, name: &str) -> bool {
        match &self.kind {
            Kind::Object(object) => object
                .properties
                .iter()
                .any(|property| property.name == name && property.required),
            _ => false,
        }
    }

    /// Adds a property to an object.
    pub(crate) fn push_property(&mut self, property: Property) {
        if let Kind::Object(object) = &mut self.kind {
            object.properties.push(property);
        }
    }

    pub(crate) fn to_json(&self) -> Value {
        let mut out = match &self.kind {
            Kind::String { max_length, values } => {
                let mut out = Map::new();
                out.insert("type".to_owned(), json!("string"));
                if let Some(limit) = max_length {
                    out.insert("maxLength".to_owned(), json!(limit));
                }
                if let Some(values) = values {
                    out.insert("enum".to_owned(), json!(values));
                }
                out
            }
            Kind::Integer {
                minimum,
                maximum,
                default,
            } => {
                let mut out = Map::new();
                out.insert("type".to_owned(), json!("integer"));
                if let Some(low) = minimum {
                    out.insert("minimum".to_owned(), json!(low));
                }
                if let Some(high) = maximum {
                    out.insert("maximum".to_owned(), json!(high));
                }
                if let Some(value) = default {
                    out.insert("default".to_owned(), json!(value));
                }
                out
            }
            Kind::Boolean => {
                let mut out = Map::new();
                out.insert("type".to_owned(), json!("boolean"));
                out
            }
            Kind::Array {
                items,
                min_items,
                max_items,
            } => {
                let mut out = Map::new();
                out.insert("type".to_owned(), json!("array"));
                out.insert("items".to_owned(), items.to_json());
                if let Some(limit) = min_items {
                    out.insert("minItems".to_owned(), json!(limit));
                }
                if let Some(limit) = max_items {
                    out.insert("maxItems".to_owned(), json!(limit));
                }
                out
            }
            Kind::Object(object) => {
                let mut out = Map::new();
                out.insert("type".to_owned(), json!("object"));
                let mut properties = Map::new();
                for property in &object.properties {
                    properties.insert(property.name.to_owned(), property.schema.to_json());
                }
                out.insert("properties".to_owned(), Value::Object(properties));
                let required: Vec<&str> = object
                    .properties
                    .iter()
                    .filter(|property| property.required)
                    .map(|property| property.name)
                    .collect();
                if !required.is_empty() {
                    out.insert("required".to_owned(), json!(required));
                }
                out.insert("additionalProperties".to_owned(), json!(false));
                if !object.exactly_one_of.is_empty() {
                    let choices: Vec<Value> = object
                        .exactly_one_of
                        .iter()
                        .map(|name| json!({ "required": [name] }))
                        .collect();
                    out.insert("oneOf".to_owned(), Value::Array(choices));
                }
                out
            }
            Kind::AnyObject => {
                let mut out = Map::new();
                out.insert("type".to_owned(), json!("object"));
                out
            }
            Kind::OneOf(choices) => {
                let mut out = Map::new();
                let choices: Vec<Value> = choices.iter().map(Schema::to_json).collect();
                out.insert("oneOf".to_owned(), Value::Array(choices));
                out
            }
        };
        if let Some(description) = self.description {
            out.insert("description".to_owned(), json!(description));
        }
        Value::Object(out)
    }

    /// Checks `value`; the error names the first field that doesn't fit.
    pub(crate) fn validate(&self, value: &Value) -> Result<(), Violation> {
        self.check(value, "")
    }

    fn check(&self, value: &Value, path: &str) -> Result<(), Violation> {
        let violation = |problem| Violation {
            path: if path.is_empty() {
                "/".to_owned()
            } else {
                path.to_owned()
            },
            problem,
        };
        match &self.kind {
            Kind::String { max_length, values } => {
                let Some(text) = value.as_str() else {
                    return Err(violation(Problem::WrongType { expected: "a string" }));
                };
                if let Some(limit) = max_length
                    && text.chars().count() > *limit
                {
                    return Err(violation(Problem::TooLong { limit: *limit }));
                }
                if let Some(values) = values
                    && !values.iter().any(|allowed| *allowed == text)
                {
                    return Err(violation(Problem::UnknownValue {
                        allowed: values.clone(),
                    }));
                }
                Ok(())
            }
            Kind::Integer {
                minimum, maximum, ..
            } => {
                let Some(number) = as_integer(value) else {
                    return Err(violation(Problem::WrongType {
                        expected: "an integer",
                    }));
                };
                if let Some(low) = minimum
                    && number < i128::from(*low)
                {
                    return Err(violation(Problem::BelowMinimum { minimum: *low }));
                }
                if let Some(high) = maximum
                    && number > i128::from(*high)
                {
                    return Err(violation(Problem::AboveMaximum { maximum: *high }));
                }
                Ok(())
            }
            Kind::Boolean => {
                if value.is_boolean() {
                    Ok(())
                } else {
                    Err(violation(Problem::WrongType {
                        expected: "true or false",
                    }))
                }
            }
            Kind::Array {
                items,
                min_items,
                max_items,
            } => {
                let Some(entries) = value.as_array() else {
                    return Err(violation(Problem::WrongType {
                        expected: "an array",
                    }));
                };
                if let Some(limit) = min_items
                    && entries.len() < *limit
                {
                    return Err(violation(Problem::TooFewItems { limit: *limit }));
                }
                if let Some(limit) = max_items
                    && entries.len() > *limit
                {
                    return Err(violation(Problem::TooManyItems { limit: *limit }));
                }
                for (index, entry) in entries.iter().enumerate() {
                    items.check(entry, &format!("{path}/{index}"))?;
                }
                Ok(())
            }
            Kind::Object(object) => {
                let Some(fields) = value.as_object() else {
                    return Err(violation(Problem::WrongType {
                        expected: "an object",
                    }));
                };
                for (name, field) in fields {
                    let field_path = format!("{path}/{name}");
                    let Some(property) = object
                        .properties
                        .iter()
                        .find(|property| property.name == name.as_str())
                    else {
                        return Err(Violation {
                            path: field_path,
                            problem: Problem::Unexpected,
                        });
                    };
                    property.schema.check(field, &field_path)?;
                }
                for property in &object.properties {
                    if property.required && !fields.contains_key(property.name) {
                        return Err(Violation {
                            path: format!("{path}/{}", property.name),
                            problem: Problem::Missing,
                        });
                    }
                }
                if !object.exactly_one_of.is_empty() {
                    let given = object
                        .exactly_one_of
                        .iter()
                        .filter(|name| fields.contains_key(**name))
                        .count();
                    if given != 1 {
                        return Err(violation(Problem::NotExactlyOne {
                            names: object.exactly_one_of.clone(),
                        }));
                    }
                }
                Ok(())
            }
            Kind::AnyObject => {
                if value.is_object() {
                    Ok(())
                } else {
                    Err(violation(Problem::WrongType {
                        expected: "an object",
                    }))
                }
            }
            Kind::OneOf(choices) => {
                let mut failures = Vec::new();
                let mut matched = 0;
                for choice in choices {
                    match choice.check(value, path) {
                        Ok(()) => matched += 1,
                        Err(failure) => failures.push((choice, failure)),
                    }
                }
                match matched {
                    1 => Ok(()),
                    0 => {
                        // Prefer the branch whose discriminator(s) matched (e.g.
                        // `root: "session_folder"` beats `root: "clone"` even on
                        // equal-depth failures), then known keys, then failure depth.
                        let best = failures.into_iter().max_by_key(|(choice, failure)| {
                            let (disc_matched, disc_mismatched, known_keys) =
                                shape_score(choice, value);
                            let segments = failure.path.bytes().filter(|&b| b == b'/').count();
                            (
                                disc_matched,
                                !disc_mismatched,
                                known_keys,
                                segments,
                                failure.path.len(),
                            )
                        });
                        match best {
                            Some((choice, failure))
                                if failure.path.len() > path.len().max(1) =>
                            {
                                let (disc_matched, disc_mismatched, _) =
                                    shape_score(choice, value);
                                if disc_matched > 0 || !disc_mismatched {
                                    Err(failure)
                                } else {
                                    Err(violation(Problem::NoChoiceFits))
                                }
                            }
                            _ => Err(violation(Problem::NoChoiceFits)),
                        }
                    }
                    _ => Err(violation(Problem::NoChoiceFits)),
                }
            }
        }
    }

    /// Generates a minimal valid example value matching this schema.
    pub(crate) fn minimal_example(&self) -> Value {
        match &self.kind {
            Kind::String { values: Some(values), .. } => {
                json!(values.first().copied().unwrap_or("example"))
            }
            Kind::String { max_length, values: None } => {
                let sample = "example";
                match max_length {
                    Some(limit) if *limit < sample.len() => json!(&sample[..*limit]),
                    _ => json!(sample),
                }
            }
            Kind::Integer { minimum, default, .. } => {
                json!(default.or(*minimum).unwrap_or(0))
            }
            Kind::Boolean => json!(false),
            Kind::Array { items, min_items, .. } => {
                let count = min_items.unwrap_or(0);
                Value::Array((0..count).map(|_| items.minimal_example()).collect())
            }
            Kind::AnyObject => json!({}),
            Kind::OneOf(choices) => choices
                .first()
                .map(Schema::minimal_example)
                .unwrap_or_else(|| json!({})),
            Kind::Object(object) => {
                let mut out = Map::new();
                for property in &object.properties {
                    if property.required {
                        out.insert(property.name.to_owned(), property.schema.minimal_example());
                    }
                }
                if !object.exactly_one_of.is_empty()
                    && !object
                        .exactly_one_of
                        .iter()
                        .any(|name| out.contains_key(*name))
                    && let Some(first_name) = object.exactly_one_of.first()
                    && let Some(property) = object
                        .properties
                        .iter()
                        .find(|property| property.name == *first_name)
                {
                    out.insert(property.name.to_owned(), property.schema.minimal_example());
                }
                Value::Object(out)
            }
        }
    }

    /// Returns `(required, optional)` parameter labels for compact 1-line tool
    /// summaries in `humata_list`.
    pub(crate) fn signature_params(&self) -> (Vec<String>, Vec<String>) {
        let Kind::Object(object) = &self.kind else {
            return (Vec::new(), Vec::new());
        };
        let mut req_names = Vec::new();
        let mut opt_names = Vec::new();
        for property in &object.properties {
            if property.required {
                req_names.push(format!("{}*", property.name));
            } else if !object.exactly_one_of.contains(&property.name) {
                opt_names.push(property.name.to_owned());
            }
        }
        if !object.exactly_one_of.is_empty() {
            req_names.push(format!("{}*", object.exactly_one_of.join("|")));
        }
        (req_names, opt_names)
    }

    /// Compact inline schema string for error self-correction, focused along
    /// `violation_path` (a JSON Pointer such as `"/location/repo"`).
    pub(crate) fn to_compact_focused(&self, violation_path: &str) -> String {
        let focus: Vec<&str> = violation_path
            .trim_start_matches('/')
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect();
        self.format_compact(&focus, 0)
    }

    fn format_compact(&self, focus: &[&str], depth: usize) -> String {
        match &self.kind {
            Kind::String { values: Some(values), .. } => {
                const MAX_ENUM: usize = 8;
                if values.len() <= MAX_ENUM {
                    values
                        .iter()
                        .map(|v| format!("\"{v}\""))
                        .collect::<Vec<_>>()
                        .join("|")
                } else {
                    let head = values[..5]
                        .iter()
                        .map(|v| format!("\"{v}\""))
                        .collect::<Vec<_>>()
                        .join("|");
                    format!("{head}|...({} values)", values.len())
                }
            }
            Kind::String { max_length: Some(limit), values: None } => format!("string(<={limit})"),
            Kind::String { max_length: None, values: None } => "string".to_owned(),
            Kind::Integer { minimum, maximum, .. } => match (minimum, maximum) {
                (Some(low), Some(high)) => format!("int({low}..{high})"),
                (Some(low), None) => format!("int(>={low})"),
                (None, Some(high)) => format!("int(<={high})"),
                (None, None) => "int".to_owned(),
            },
            Kind::Boolean => "bool".to_owned(),
            Kind::Array {
                items,
                min_items,
                max_items,
            } => {
                let next_focus = match focus.first() {
                    Some(seg) if seg.chars().all(|c| c.is_ascii_digit()) => &focus[1..],
                    _ => &[],
                };
                let inner = if depth >= 2 && focus.is_empty() {
                    match &items.kind {
                        Kind::Object(_) | Kind::OneOf(_) => "object".to_owned(),
                        _ => items.format_compact(next_focus, depth + 1),
                    }
                } else {
                    items.format_compact(next_focus, depth + 1)
                };
                match (min_items, max_items) {
                    (Some(lo), Some(hi)) => format!("[{inner}]({lo}..{hi})"),
                    (Some(lo), None) => format!("[{inner}](>={lo})"),
                    (None, Some(hi)) => format!("[{inner}](<={hi})"),
                    (None, None) => format!("[{inner}]"),
                }
            }
            Kind::AnyObject => "object".to_owned(),
            Kind::OneOf(choices) => choices
                .iter()
                .map(|choice| choice.format_compact(focus, depth))
                .collect::<Vec<_>>()
                .join(" | "),
            Kind::Object(object) => {
                if depth >= 2 && focus.is_empty() && object.properties.len() > 4 {
                    let keys: Vec<String> = object
                        .properties
                        .iter()
                        .map(|p| {
                            if p.required {
                                format!("{}*", p.name)
                            } else {
                                format!("{}?", p.name)
                            }
                        })
                        .collect();
                    return format!("{{{}}}", keys.join(", "));
                }
                let mut parts = Vec::with_capacity(object.properties.len() + 1);
                for property in &object.properties {
                    let marker = if property.required { "*" } else { "?" };
                    let child_focus = match focus.first() {
                        Some(seg) if *seg == property.name => &focus[1..],
                        _ => &[],
                    };
                    let ty = property.schema.format_compact(child_focus, depth + 1);
                    parts.push(format!("{}{marker}: {ty}", property.name));
                }
                if !object.exactly_one_of.is_empty() {
                    parts.push(format!("oneOf({})", object.exactly_one_of.join(", ")));
                }
                format!("{{{}}}", parts.join(", "))
            }
        }
    }
}

/// Scores how closely an object `value` matches an object `choice` in a `oneOf`:
/// `(discriminators_matched, discriminator_mismatched, known_keys)`.
fn shape_score(choice: &Schema, value: &Value) -> (usize, bool, usize) {
    let (Kind::Object(object), Some(fields)) = (&choice.kind, value.as_object()) else {
        return (0, false, 0);
    };
    let mut disc_matched = 0;
    let mut disc_mismatched = false;
    let mut known_keys = 0;
    for property in &object.properties {
        if let Some(given) = fields.get(property.name) {
            known_keys += 1;
            if let Kind::String {
                values: Some(values),
                ..
            } = &property.schema.kind
                && values.len() == 1
            {
                if given.as_str() == Some(values[0]) {
                    disc_matched += 1;
                } else {
                    disc_mismatched = true;
                }
            }
        }
    }
    (disc_matched, disc_mismatched, known_keys)
}

/// An integer, from a JSON number with no fraction.
fn as_integer(value: &Value) -> Option<i128> {
    let Value::Number(number) = value else {
        return None;
    };
    if let Some(signed) = number.as_i64() {
        return Some(i128::from(signed));
    }
    if let Some(unsigned) = number.as_u64() {
        return Some(i128::from(unsigned));
    }
    let float = number.as_f64()?;
    if float.fract() == 0.0 && float.abs() < 9.0e15 {
        // Exact: the value is a whole number below 2^53.
        return Some(float as i128);
    }
    None
}

/// Why a call doesn't fit its tool's schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Violation {
    /// A JSON Pointer to the field, `/` for the arguments as a whole.
    pub(crate) path: String,
    pub(crate) problem: Problem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Problem {
    WrongType { expected: &'static str },
    TooLong { limit: usize },
    UnknownValue { allowed: Vec<&'static str> },
    BelowMinimum { minimum: i64 },
    AboveMaximum { maximum: i64 },
    TooFewItems { limit: usize },
    TooManyItems { limit: usize },
    Missing,
    Unexpected,
    NotExactlyOne { names: Vec<&'static str> },
    NoChoiceFits,
}

impl Problem {
    /// The problem's name in the error's `data`, matching
    /// `InvalidField.Problem` where one fits.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Problem::WrongType { .. } | Problem::NotExactlyOne { .. } | Problem::NoChoiceFits => {
                "bad_format"
            }
            Problem::TooLong { .. } | Problem::TooManyItems { .. } => "too_long",
            Problem::UnknownValue { .. } => "unknown_value",
            Problem::BelowMinimum { .. }
            | Problem::AboveMaximum { .. }
            | Problem::TooFewItems { .. } => "out_of_range",
            Problem::Missing => "missing",
            Problem::Unexpected => "not_allowed_here",
        }
    }
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = &self.path;
        match &self.problem {
            Problem::WrongType { expected } => write!(f, "{path} must be {expected}"),
            Problem::TooLong { limit } => {
                write!(f, "{path} is longer than {limit} characters; shorten it")
            }
            Problem::UnknownValue { allowed } => {
                write!(f, "{path} must be one of {}", allowed.join(", "))
            }
            Problem::BelowMinimum { minimum } => write!(f, "{path} must be at least {minimum}"),
            Problem::AboveMaximum { maximum } => write!(f, "{path} must be at most {maximum}"),
            Problem::TooFewItems { limit } => write!(f, "{path} needs at least {limit} item(s)"),
            Problem::TooManyItems { limit } => write!(f, "{path} takes at most {limit} items"),
            Problem::Missing => write!(f, "{path} is required"),
            Problem::Unexpected => write!(
                f,
                "{path} isn't a field of this tool; humata_describe gives each tool's fields"
            ),
            Problem::NotExactlyOne { names } => {
                write!(f, "{path} takes exactly one of {}", names.join(", "))
            }
            Problem::NoChoiceFits => write!(
                f,
                "{path} fits none of the shapes this field takes; humata_describe gives them"
            ),
        }
    }
}
