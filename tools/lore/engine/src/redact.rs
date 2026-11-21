//! Pre-write secret scanning and redaction (`SEC-1`).
//!
//! Scans `statement`, `body`, `dead_ends`, `attrs`, and inline verifier/evidence
//! fields before any `LogEvent` is appended to `.pblog`.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use lore_proto::{Claim, LoreError, Result, verifier};
use regex::Regex;

struct SecretRule {
    kind: &'static str,
    regex: Regex,
}

fn secret_rules() -> &'static [SecretRule] {
    static RULES: OnceLock<Vec<SecretRule>> = OnceLock::new();
    RULES.get_or_init(|| {
        let specs: &[(&str, &str)] = &[
            (
                "pem_private_key",
                r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*?(?:-----END [A-Z0-9 ]*PRIVATE KEY-----|$)",
            ),
            ("github_pat", r"\bghp_[A-Za-z0-9_]{8,}\b"),
            ("github_oauth", r"\bgho_[A-Za-z0-9_]{8,}\b"),
            ("aws_access_key", r"\bAKIA[0-9A-Z]{16}\b"),
            ("api_key_sk", r"\bsk-[A-Za-z0-9_-]{12,}\b"),
            ("bearer_token", r"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]{8,}"),
            (
                "humata_session_token",
                r#"\bHUMATA_SESSION_TOKEN\s*[=:]\s*['"]?[A-Za-z0-9._~+/=-]{8,}['"]?"#,
            ),
        ];

        specs
            .iter()
            .map(|(kind, pat)| SecretRule {
                kind,
                regex: Regex::new(pat).expect("static secret regex must compile"),
            })
            .collect()
    })
}

/// Scans a single string field.
///
/// - When `redact_secrets == false`, returns `Err(LoreError::SecretDetected { pattern_kind, field })`
///   on the first secret match.
/// - When `redact_secrets == true`, replaces all matches with `[REDACTED:<kind>]`
///   and returns `Ok(true)` if any replacement occurred.
pub fn scan_or_redact_field(field: &str, value: &mut String, redact_secrets: bool) -> Result<bool> {
    let mut modified = false;
    for rule in secret_rules() {
        if rule.regex.is_match(value) {
            if !redact_secrets {
                return Err(LoreError::SecretDetected {
                    pattern_kind: rule.kind.to_string(),
                    field: field.to_string(),
                });
            }
            let replacement = format!("[REDACTED:{}]", rule.kind);
            let replaced = rule.regex.replace_all(value, replacement.as_str());
            if replaced.as_ref() != value.as_str() {
                *value = replaced.into_owned();
                modified = true;
            }
        }
    }
    Ok(modified)
}

/// Scans all user-supplied text fields of a [`Claim`] (`SEC-1`).
///
/// Returns the sorted list of field names that were redacted when
/// `redact_secrets == true` (empty when no secrets were found).
pub fn scan_or_redact_claim(claim: &mut Claim, redact_secrets: bool) -> Result<Vec<String>> {
    let mut redacted_fields = BTreeSet::new();

    if scan_or_redact_field("statement", &mut claim.statement, redact_secrets)? {
        redacted_fields.insert("statement".to_string());
    }
    if scan_or_redact_field("body", &mut claim.body, redact_secrets)? {
        redacted_fields.insert("body".to_string());
    }

    for (idx, dead_end) in claim.dead_ends.iter_mut().enumerate() {
        let field = format!("dead_ends[{idx}]");
        if scan_or_redact_field(&field, dead_end, redact_secrets)? {
            redacted_fields.insert("dead_ends".to_string());
        }
    }

    for (idx, kw) in claim.keywords.iter_mut().enumerate() {
        let field = format!("keywords[{idx}]");
        if scan_or_redact_field(&field, kw, redact_secrets)? {
            redacted_fields.insert("keywords".to_string());
        }
    }

    for (key, val) in &mut claim.attrs {
        let field = format!("attrs.{key}");
        if scan_or_redact_field(&field, val, redact_secrets)? {
            redacted_fields.insert(field);
        }
    }

    for (idx, v) in claim.verifiers.iter_mut().enumerate() {
        if let Some(verifier::Check::BlobRef(blob)) = &mut v.check {
            let field = format!("verifiers[{idx}].summary");
            if scan_or_redact_field(&field, &mut blob.summary, redact_secrets)? {
                redacted_fields.insert("verifiers".to_string());
            }
        }
    }

    Ok(redacted_fields.into_iter().collect())
}
