//! What a run prints and its exit code (CLI.md §7).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value as Json, json};

use crate::args::Global;
use crate::as_json;
use crate::call::{Done, Outcome};
use crate::console::Console;
use crate::error::{CliError, Failure};
use crate::exit;

/// Prints a run's result or failure and gives its exit code. `stale_as_of`
/// is when the host answered a read the machine served from its copy: it is
/// shown with the result so a copied read is never mistaken for a live one.
pub fn finish(
    console: &impl Console,
    flags: &Global,
    operation: &str,
    result: Result<Done, CliError>,
    stale_as_of: Option<String>,
) -> i32 {
    match result {
        Ok(done) => success(console, flags, operation, done, stale_as_of),
        Err(error) => failure(console, flags, operation, &error.failure()),
    }
}

fn success(
    console: &impl Console,
    flags: &Global,
    operation: &str,
    done: Done,
    stale_as_of: Option<String>,
) -> i32 {
    let outcome = match done.outcome {
        Outcome::Exited(code) => return code,
        Outcome::Printed if !flags.json => return exit::OK,
        Outcome::Planned => "planned",
        Outcome::Printed | Outcome::Reply if journaled(&done.result) => "journaled",
        Outcome::Printed | Outcome::Reply => "applied",
    };
    let journal = done.result.get("journal").cloned().unwrap_or(Json::Null);
    let strict_failure = outcome == "journaled" && flags.strict;
    if flags.json {
        let mut envelope = Map::new();
        envelope.insert("ok".into(), Json::Bool(!strict_failure));
        envelope.insert("operation".into(), json!(operation));
        envelope.insert("outcome".into(), json!(outcome));
        if outcome == "journaled" {
            envelope.insert("journal".into(), journal);
        }
        if let Some(at) = &stale_as_of {
            envelope.insert("stale_as_of".into(), json!(at));
        }
        envelope.insert("result".into(), done.result);
        if strict_failure {
            envelope.insert(
                "error".into(),
                json!({
                    "kind": "journaled",
                    "message": STRICT_MESSAGE,
                    "fix": STRICT_FIX,
                    "details": {},
                }),
            );
        }
        console.print(&Json::Object(envelope).to_string());
    } else {
        let text = human(&done.result);
        if !text.is_empty() {
            console.print(&text);
        }
        if let Some(at) = &stale_as_of {
            console.eprint(&format!(
                "This is the copy this machine kept; the Mac answered it at {at} and isn't reachable now."
            ));
        }
        match outcome {
            "planned" if done.result.get("token").is_some() => {
                console.eprint("Nothing changed. Run it again with --plan <token> to do this.")
            }
            "planned" => console.eprint("Nothing changed. Run it without --dry-run to do this."),
            "journaled" if strict_failure => {
                console.eprint(STRICT_MESSAGE);
                console.eprint(STRICT_FIX);
            }
            "journaled" => {
                console.eprint("Saved on this machine; the Mac applies it when it's back.")
            }
            _ => {}
        }
    }
    if strict_failure {
        exit::JOURNALED_STRICT
    } else {
        exit::OK
    }
}

const STRICT_MESSAGE: &str =
    "The change is only journaled on this machine: the Mac hasn't applied it.";
const STRICT_FIX: &str = "It applies when the Mac is reachable; check with `humata ops` or the item later, or run without --strict.";

/// A reply delivered to the journal rather than applied (GRPC.md §9).
fn journaled(result: &Json) -> bool {
    result.get("delivery").and_then(Json::as_str) == Some("journaled")
        || result.get("journaled").and_then(Json::as_bool) == Some(true)
}

fn failure(console: &impl Console, flags: &Global, operation: &str, failure: &Failure) -> i32 {
    if flags.json {
        let mut error = Map::new();
        error.insert("kind".into(), json!(failure.kind));
        error.insert("message".into(), json!(failure.message));
        error.insert("fix".into(), json!(failure.fix));
        error.insert("details".into(), failure.details.clone());
        if let Some((after, same_key)) = failure.retry {
            error.insert(
                "retry".into(),
                json!({ "after": at(after), "same_key": same_key }),
            );
        }
        console.print(
            &json!({ "ok": false, "operation": operation, "error": Json::Object(error) })
                .to_string(),
        );
    } else {
        console.eprint(&format!("humata: {}", failure.message));
        if !failure.fix.is_empty() {
            console.eprint(&failure.fix);
        }
        if let Some((after, _)) = failure.retry {
            let time = at(after);
            let clock = time.get(11..16).unwrap_or(&time);
            console.eprint(&format!("Try again after {clock} UTC."));
        }
    }
    failure.code
}

/// Now plus `after`, as RFC 3339.
fn at(after: Duration) -> String {
    let when = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .saturating_add(after);
    as_json::rfc3339(i64::try_from(when.as_secs()).unwrap_or(i64::MAX), 0)
}

/// A result for a person: `key: value` lines, nested ones indented, with
/// empty values left out.
pub fn human(value: &Json) -> String {
    let mut out = String::new();
    write(&mut out, value, 0);
    out.trim_end().to_owned()
}

fn write(out: &mut String, value: &Json, depth: usize) {
    let pad = "  ".repeat(depth);
    match value {
        Json::Object(fields) => {
            for (key, value) in fields {
                if empty(value) {
                    continue;
                }
                if scalar(value) {
                    out.push_str(&format!("{pad}{key}: {}\n", inline(value)));
                } else {
                    out.push_str(&format!("{pad}{key}:\n"));
                    write(out, value, depth + 1);
                }
            }
        }
        Json::Array(items) => {
            for item in items {
                if scalar(item) {
                    out.push_str(&format!("{pad}- {}\n", inline(item)));
                } else {
                    out.push_str(&format!("{pad}-\n"));
                    write(out, item, depth + 1);
                }
            }
        }
        other => out.push_str(&format!("{pad}{}\n", inline(other))),
    }
}

fn scalar(value: &Json) -> bool {
    !matches!(value, Json::Object(_) | Json::Array(_))
}

fn empty(value: &Json) -> bool {
    match value {
        Json::Null => true,
        Json::Bool(value) => !value,
        Json::String(text) => text.is_empty() || text == "unspecified",
        Json::Number(number) => number.as_f64() == Some(0.0),
        Json::Array(items) => items.is_empty(),
        Json::Object(fields) => fields.values().all(empty),
    }
}

fn inline(value: &Json) -> String {
    match value {
        Json::String(text) => text.clone(),
        other => other.to_string(),
    }
}
