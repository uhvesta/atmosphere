//! The step script a recording follows: one step per line, `#` comments,
//! blank lines ignored.

use crate::snapshot::Expect;

/// One step of a script and the line it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// The 1-based line in the script file.
    pub line: usize,
    pub kind: Kind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Wait(String),
    WaitGone(String),
    Quiet(u64),
    Type(String),
    Keys(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Snap { label: String, expect: Expect },
}

impl Kind {
    pub fn name(&self) -> &'static str {
        match self {
            Kind::Wait(_) => "wait",
            Kind::WaitGone(_) => "wait-gone",
            Kind::Quiet(_) => "quiet",
            Kind::Type(_) => "type",
            Kind::Keys(_) => "keys",
            Kind::Resize { .. } => "resize",
            Kind::Snap { .. } => "snap",
        }
    }
}

/// Why a script line couldn't be read.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ScriptError {
    #[error("script line {line}: unknown step `{word}`. Use wait, wait-gone, quiet, type, keys, resize or snap.")]
    UnknownStep { line: usize, word: String },
    #[error("script line {line}: `{step}` needs {needs}. Add it after the step name.")]
    MissingArgument {
        line: usize,
        step: &'static str,
        needs: &'static str,
    },
    #[error("script line {line}: `{value}` isn't a whole number. Write it in digits, like `800`.")]
    BadNumber { line: usize, value: String },
    #[error("script line {line}: unknown key `{key}`. Use enter, esc, up, down, left, right, tab, shift-tab, space, backspace, ctrl-c, ctrl-d, a single letter or digit, or \"literal text\" in quotes.")]
    UnknownKey { line: usize, key: String },
    #[error("script line {line}: a quote was opened and never closed. Close the \"literal text\" with a second quote.")]
    UnclosedQuote { line: usize },
    #[error("script line {line}: snapshot label `{label}` must be letters, digits, `_` or `-` only, so it can be part of a file name.")]
    BadLabel { line: usize, label: String },
    #[error("script line {line}: expectation `{value}` must be none, offer or prompt:<kind>.")]
    BadExpect { line: usize, value: String },
}

/// Reads a whole script.
pub fn parse_script(text: &str) -> Result<Vec<Step>, ScriptError> {
    let mut steps = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        steps.push(Step {
            line,
            kind: parse_line(line, trimmed)?,
        });
    }
    Ok(steps)
}

fn parse_line(line: usize, text: &str) -> Result<Kind, ScriptError> {
    let (word, rest) = match text.split_once(char::is_whitespace) {
        Some((word, rest)) => (word, rest.trim()),
        None => (text, ""),
    };
    let need = |step: &'static str, needs: &'static str| -> Result<(), ScriptError> {
        if rest.is_empty() {
            Err(ScriptError::MissingArgument { line, step, needs })
        } else {
            Ok(())
        }
    };
    match word {
        "wait" => {
            need("wait", "the text to wait for")?;
            Ok(Kind::Wait(rest.to_owned()))
        }
        "wait-gone" => {
            need("wait-gone", "the text to wait to disappear")?;
            Ok(Kind::WaitGone(rest.to_owned()))
        }
        "quiet" => {
            need("quiet", "a number of milliseconds")?;
            Ok(Kind::Quiet(number(line, rest)?))
        }
        "type" => {
            need("type", "the text to type")?;
            Ok(Kind::Type(rest.to_owned()))
        }
        "keys" => {
            need("keys", "at least one key")?;
            Ok(Kind::Keys(keys(line, rest)?))
        }
        "resize" => {
            let mut words = rest.split_whitespace();
            let (Some(cols), Some(rows)) = (words.next(), words.next()) else {
                return Err(ScriptError::MissingArgument {
                    line,
                    step: "resize",
                    needs: "columns and rows",
                });
            };
            Ok(Kind::Resize {
                cols: size(line, cols)?,
                rows: size(line, rows)?,
            })
        }
        "snap" => {
            let mut words = rest.split_whitespace();
            let (Some(label), Some(expect)) = (words.next(), words.next()) else {
                return Err(ScriptError::MissingArgument {
                    line,
                    step: "snap",
                    needs: "a label and an expectation",
                });
            };
            if label.is_empty()
                || !label
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err(ScriptError::BadLabel {
                    line,
                    label: label.to_owned(),
                });
            }
            // `unknown` is for exploring runs: the screen is labelled by
            // hand once it has been read.
            let expect = match Expect::parse(expect) {
                None => {
                    return Err(ScriptError::BadExpect {
                        line,
                        value: expect.to_owned(),
                    });
                }
                Some(expect) => expect,
            };
            Ok(Kind::Snap {
                label: label.to_owned(),
                expect,
            })
        }
        other => Err(ScriptError::UnknownStep {
            line,
            word: other.to_owned(),
        }),
    }
}

fn number(line: usize, value: &str) -> Result<u64, ScriptError> {
    value.parse().map_err(|_| ScriptError::BadNumber {
        line,
        value: value.to_owned(),
    })
}

fn size(line: usize, value: &str) -> Result<u16, ScriptError> {
    match value.parse::<u16>() {
        Ok(size) if size > 0 => Ok(size),
        _ => Err(ScriptError::BadNumber {
            line,
            value: value.to_owned(),
        }),
    }
}

fn keys(line: usize, text: &str) -> Result<Vec<u8>, ScriptError> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let Some(&first) = chars.peek() else {
            return Ok(out);
        };
        if first == '"' {
            chars.next();
            let mut closed = false;
            for c in chars.by_ref() {
                if c == '"' {
                    closed = true;
                    break;
                }
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
            if !closed {
                return Err(ScriptError::UnclosedQuote { line });
            }
            continue;
        }
        let mut name = String::new();
        while let Some(c) = chars.next_if(|c| !c.is_whitespace()) {
            name.push(c);
        }
        match named_key(&name) {
            Some(bytes) => out.extend_from_slice(bytes),
            None if name.len() == 1 && name.chars().all(|c| c.is_ascii_alphanumeric()) => {
                out.extend_from_slice(name.as_bytes());
            }
            None => return Err(ScriptError::UnknownKey { line, key: name }),
        }
    }
}

fn named_key(name: &str) -> Option<&'static [u8]> {
    Some(match name {
        "enter" => b"\r",
        "esc" => b"\x1b",
        "up" => b"\x1b[A",
        "down" => b"\x1b[B",
        "right" => b"\x1b[C",
        "left" => b"\x1b[D",
        "tab" => b"\t",
        "shift-tab" => b"\x1b[Z",
        "space" => b" ",
        "backspace" => b"\x7f",
        "ctrl-c" => b"\x03",
        "ctrl-d" => b"\x04",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_step_and_skips_comments_and_blanks() {
        let script = "# start\n\nwait Do you trust\nkeys enter\nquiet 500\ntype hello there\nresize 100 30\nsnap trust prompt:trust\nwait-gone Working\n";
        let steps = parse_script(script).unwrap();
        let kinds: Vec<_> = steps.iter().map(|s| s.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                Kind::Wait("Do you trust".into()),
                Kind::Keys(b"\r".to_vec()),
                Kind::Quiet(500),
                Kind::Type("hello there".into()),
                Kind::Resize { cols: 100, rows: 30 },
                Kind::Snap {
                    label: "trust".into(),
                    expect: Expect::Prompt("trust".into())
                },
                Kind::WaitGone("Working".into()),
            ]
        );
        assert_eq!(steps[0].line, 3);
    }

    #[test]
    fn keys_mix_names_quotes_and_single_characters() {
        let steps = parse_script("keys down \"a b\" shift-tab 2 y").unwrap();
        assert_eq!(steps[0].kind, Kind::Keys(b"\x1b[Ba b\x1b[Z2y".to_vec()));
    }

    #[test]
    fn bad_lines_give_their_own_error() {
        assert!(matches!(parse_script("dance"), Err(ScriptError::UnknownStep { line: 1, .. })));
        assert!(matches!(parse_script("wait"), Err(ScriptError::MissingArgument { .. })));
        assert!(matches!(parse_script("quiet soon"), Err(ScriptError::BadNumber { .. })));
        assert!(matches!(parse_script("resize 80 0"), Err(ScriptError::BadNumber { .. })));
        assert!(matches!(parse_script("keys hyper"), Err(ScriptError::UnknownKey { .. })));
        assert!(matches!(parse_script("keys \"open"), Err(ScriptError::UnclosedQuote { .. })));
        assert!(matches!(parse_script("snap a/b none"), Err(ScriptError::BadLabel { .. })));
        assert!(matches!(parse_script("snap a maybe"), Err(ScriptError::BadExpect { .. })));
        assert!(matches!(parse_script("snap a prompt:"), Err(ScriptError::BadExpect { .. })));
        assert!(parse_script("snap a unknown").is_ok());
    }
}
