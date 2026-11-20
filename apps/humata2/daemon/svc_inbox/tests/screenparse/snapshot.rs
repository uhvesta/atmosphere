//! The labelled screen-text file a recording writes and a replay reads.

use std::path::Path;

/// What the prompt parser should make of the screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expect {
    None,
    Offer,
    Prompt(String),
    /// Recorded when a step timed out: nobody has judged this screen.
    Unknown,
}

impl Expect {
    pub fn parse(text: &str) -> Option<Expect> {
        match text {
            "none" => Some(Expect::None),
            "offer" => Some(Expect::Offer),
            "unknown" => Some(Expect::Unknown),
            other => match other.strip_prefix("prompt:") {
                Some(kind) if !kind.is_empty() && !kind.contains(char::is_whitespace) => {
                    Some(Expect::Prompt(kind.to_owned()))
                }
                _ => None,
            },
        }
    }

    fn text(&self) -> String {
        match self {
            Expect::None => "none".into(),
            Expect::Offer => "offer".into(),
            Expect::Unknown => "unknown".into(),
            Expect::Prompt(kind) => format!("prompt:{kind}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub agent: String,
    pub cli: String,
    pub script: String,
    pub label: String,
    pub expect: Expect,
    pub cols: u16,
    pub rows: u16,
    pub screen: String,
}

/// Why a snapshot file couldn't be read.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SnapshotError {
    #[error("snapshot has no `---` line between its header and its screen. Record it again with screenparse_record.")]
    NoSeparator,
    #[error("snapshot header line {line} (`{text}`) isn't `key: value`. Record it again with screenparse_record.")]
    BadHeaderLine { line: usize, text: String },
    #[error("snapshot header has no `{key}:` line. Record it again with screenparse_record.")]
    MissingKey { key: &'static str },
    #[error("snapshot `{key}` is `{value}`, which isn't valid for it. Fix the header by hand or record it again.")]
    BadValue { key: &'static str, value: String },
}

impl Snapshot {
    /// The file's content.
    pub fn render(&self) -> String {
        format!(
            "agent: {}\ncli: {}\nscript: {}\nlabel: {}\nexpect: {}\ncols: {}\nrows: {}\n---\n{}",
            self.agent,
            self.cli,
            self.script,
            self.label,
            self.expect.text(),
            self.cols,
            self.rows,
            self.screen
        )
    }

    /// The file name under the output folder; `index` is the snap's place
    /// in its script.
    pub fn file_name(&self, index: usize) -> String {
        format!("{}.w{}.{:02}-{}.txt", self.script, self.cols, index, self.label)
    }

    pub fn write(&self, folder: &Path, index: usize) -> std::io::Result<()> {
        std::fs::create_dir_all(folder)?;
        std::fs::write(folder.join(self.file_name(index)), self.render())
    }
}

/// Reads the text of a snapshot file.
pub fn read_snapshot(text: &str) -> Result<Snapshot, SnapshotError> {
    let mut offset = 0;
    let mut header: Vec<(&str, &str)> = Vec::new();
    let mut found = false;
    for (index, raw) in text.split_inclusive('\n').enumerate() {
        offset += raw.len();
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        if line == "---" {
            found = true;
            break;
        }
        match line.split_once(": ") {
            Some((key, value)) => header.push((key, value)),
            None => {
                return Err(SnapshotError::BadHeaderLine {
                    line: index + 1,
                    text: line.to_owned(),
                });
            }
        }
    }
    if !found {
        return Err(SnapshotError::NoSeparator);
    }
    let get = |key: &'static str| -> Result<&str, SnapshotError> {
        header
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| *v)
            .ok_or(SnapshotError::MissingKey { key })
    };
    let number = |key: &'static str| -> Result<u16, SnapshotError> {
        let value = get(key)?;
        value.parse().map_err(|_| SnapshotError::BadValue {
            key,
            value: value.to_owned(),
        })
    };
    let expect = get("expect")?;
    Ok(Snapshot {
        agent: get("agent")?.to_owned(),
        cli: get("cli")?.to_owned(),
        script: get("script")?.to_owned(),
        label: get("label")?.to_owned(),
        expect: Expect::parse(expect).ok_or_else(|| SnapshotError::BadValue {
            key: "expect",
            value: expect.to_owned(),
        })?,
        cols: number("cols")?,
        rows: number("rows")?,
        screen: text[offset..].to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Snapshot {
        Snapshot {
            agent: "claude".into(),
            cli: "2.1.329".into(),
            script: "permission_bash".into(),
            label: "bash_permission".into(),
            expect: Expect::Prompt("permission".into()),
            cols: 80,
            rows: 40,
            screen: "line one\n---\n  Do you want?\n".into(),
        }
    }

    #[test]
    fn a_written_snapshot_reads_back_the_same() {
        let snap = sample();
        assert_eq!(read_snapshot(&snap.render()), Ok(snap));
    }

    #[test]
    fn every_expectation_round_trips() {
        for expect in [Expect::None, Expect::Offer, Expect::Unknown, Expect::Prompt("mcp".into())] {
            let snap = Snapshot { expect, ..sample() };
            assert_eq!(read_snapshot(&snap.render()), Ok(snap));
        }
    }

    #[test]
    fn the_file_name_carries_script_width_index_and_label() {
        assert_eq!(sample().file_name(3), "permission_bash.w80.03-bash_permission.txt");
    }

    #[test]
    fn bad_files_give_the_right_error() {
        let good = sample().render();
        assert_eq!(read_snapshot("agent: x\n"), Err(SnapshotError::NoSeparator));
        assert!(matches!(
            read_snapshot("agent x\n---\n"),
            Err(SnapshotError::BadHeaderLine { line: 1, .. })
        ));
        assert!(matches!(
            read_snapshot("agent: x\n---\n"),
            Err(SnapshotError::MissingKey { .. })
        ));
        assert!(matches!(
            read_snapshot(&good.replace("cols: 80", "cols: wide")),
            Err(SnapshotError::BadValue { key: "cols", .. })
        ));
        assert!(matches!(
            read_snapshot(&good.replace("prompt:permission", "sometimes")),
            Err(SnapshotError::BadValue { key: "expect", .. })
        ));
    }
}
