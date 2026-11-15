//! POSIX shell quoting, and the placeholders in provider templates.

use crate::error::QuoteError;

/// `{name}`, replaced by the machine's name (MP-3).
pub(crate) const NAME: &str = "{name}";
/// `{cmd}`, replaced by the command to run on the machine (MP-7).
pub(crate) const CMD: &str = "{cmd}";

/// `value` as one word for any POSIX shell: bare when every byte is safe,
/// otherwise in single quotes with each `'` written as `'\''`. `what` names
/// the value in the error.
pub(crate) fn quote(what: &str, value: &str) -> Result<String, QuoteError> {
    if value.contains('\0') {
        return Err(QuoteError {
            what: what.to_owned(),
        });
    }
    if value.is_empty() {
        return Ok("''".to_owned());
    }
    if value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"_@%+:,./-".contains(&b))
    {
        return Ok(value.to_owned());
    }
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');
    for c in value.chars() {
        if c == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(c);
        }
    }
    quoted.push('\'');
    Ok(quoted)
}

/// A command line for the machine's own shell that runs `script` with
/// `sh`, whatever the login shell is: `sh -c '<script>'`.
pub(crate) fn sh_command(script: &str) -> Result<String, QuoteError> {
    Ok(format!("sh -c {}", quote("the script for the machine", script)?))
}

/// `template` with each exact `{name}` and (when given) `{cmd}` replaced,
/// in one pass, so nothing a value contains is substituted again. Every
/// other brace stays as written: `${HOME}`, jq's `{state: .status}` and Go
/// templates' `{{.Status}}` reach the shell unchanged. The values must
/// already be quoted.
pub(crate) fn render(template: &str, name: &str, cmd: Option<&str>) -> String {
    let mut out = String::with_capacity(template.len() + cmd.map_or(0, str::len) + 16);
    let mut rest = template;
    while let Some(index) = rest.find('{') {
        out.push_str(&rest[..index]);
        let tail = &rest[index..];
        if let Some(after) = tail.strip_prefix(NAME) {
            out.push_str(name);
            rest = after;
        } else if let (Some(cmd), Some(after)) = (cmd, tail.strip_prefix(CMD)) {
            out.push_str(cmd);
            rest = after;
        } else {
            out.push('{');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    const HOSTILE: [&str; 12] = [
        "",
        "plain",
        "two words",
        "it's",
        "'",
        "''",
        "\"$HOME\" `id` $(id) ${HOME}",
        "a\nb\n",
        "back\\slash\\",
        "-n",
        "*?[a]~ #;&|<>(){}!",
        "ünïcödé ✓",
    ];

    /// What `sh` makes of `line`, printed with `printf %s`.
    fn sh_prints(line: &str) -> Vec<u8> {
        let output = Command::new("/bin/sh")
            .arg("-c")
            .arg(line)
            .env_clear()
            .output()
            .expect("run /bin/sh");
        assert!(output.status.success(), "{line}: {output:?}");
        output.stdout
    }

    #[test]
    fn quoted_values_reach_the_shell_as_one_unchanged_word() {
        for value in HOSTILE {
            let line = format!("printf '%s|' {}", quote("value", value).unwrap());
            assert_eq!(sh_prints(&line), format!("{value}|").into_bytes(), "{value:?}");
        }
    }

    #[test]
    fn a_script_survives_two_shells() {
        for value in HOSTILE {
            let script = format!("printf '%s|' {}", quote("value", value).unwrap());
            let outer = sh_command(&sh_command(&script).unwrap()).unwrap();
            assert_eq!(sh_prints(&outer), format!("{value}|").into_bytes(), "{value:?}");
        }
    }

    #[test]
    fn nul_cannot_be_quoted() {
        assert!(quote("value", "a\0b").is_err());
    }

    #[test]
    fn only_exact_placeholders_are_replaced_once() {
        let rendered = render(
            "x {name} ${HOME} {{.Status}} {state: .s} {cmd} {nam}",
            "box",
            Some("'{name}'"),
        );
        assert_eq!(rendered, "x box ${HOME} {{.Status}} {state: .s} '{name}' {nam}");
    }

    #[test]
    fn cmd_stays_literal_where_it_is_not_given() {
        assert_eq!(render("{cmd} {name}", "box", None), "{cmd} box");
    }
}
