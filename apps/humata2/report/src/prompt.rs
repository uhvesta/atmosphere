//! The text "Copy report" puts on the clipboard: the report's summary and
//! where each of its files is, for pasting into an agent's chat.

use std::fmt::Write as _;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::error::PromptError;
use crate::write::FILES;

/// The prompt for the report saved in `dir`: what the report is, its
/// summary, and the absolute path of every file it holds now, each with
/// what it holds. A file deleted since the report was saved isn't listed.
///
/// # Errors
///
/// [`PromptError`] when the folder can't be read or has no `report.md`.
pub fn copy_prompt(dir: &Path) -> Result<String, PromptError> {
    let read_folder = |source| PromptError::ReadFolder {
        path: dir.to_owned(),
        source,
    };
    let dir = fs::canonicalize(dir).map_err(read_folder)?;
    let summary_path = dir.join("report.md");
    let report = fs::read_to_string(&summary_path).map_err(|source| {
        if source.kind() == ErrorKind::NotFound {
            PromptError::NotAReport { path: dir.clone() }
        } else {
            PromptError::ReadSummary {
                path: summary_path.clone(),
                source,
            }
        }
    })?;

    let mut present: Vec<String> = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|source| PromptError::ReadFolder {
        path: dir.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| PromptError::ReadFolder {
            path: dir.clone(),
            source,
        })?;
        let is_file = entry.file_type().is_ok_and(|kind| kind.is_file());
        if let (true, Some(name)) = (is_file, entry.file_name().to_str()) {
            present.push(name.to_owned());
        }
    }
    // The known files in their order, then anything else by name.
    let mut listed: Vec<(PathBuf, Option<&str>)> = FILES
        .iter()
        .filter(|(name, _)| present.iter().any(|file| file.as_str() == *name))
        .map(|(name, holds)| (dir.join(name), Some(*holds)))
        .collect();
    let mut others: Vec<&String> = present
        .iter()
        .filter(|file| !FILES.iter().any(|(name, _)| *name == file.as_str()))
        .collect();
    others.sort();
    listed.extend(others.into_iter().map(|name| (dir.join(name), None)));

    let mut prompt = String::new();
    let _ = writeln!(
        prompt,
        "Here is a Humata bug report. It is text, not pictures: the events in order, and the \
         app's state, view and layout as data. Read the files below to see what happened and \
         where, then find the cause.\n"
    );
    prompt.push_str(summary(&report).trim_end());
    let _ = writeln!(prompt, "\n\n## Files in {}\n", dir.display());
    for (path, holds) in listed {
        match holds {
            Some(holds) => {
                let _ = writeln!(prompt, "- {}: {holds}", path.display());
            }
            None => {
                let _ = writeln!(prompt, "- {}", path.display());
            }
        }
    }
    Ok(prompt)
}

/// The `## Summary` section of `report`, heading included, or the whole
/// report when it has none.
fn summary(report: &str) -> &str {
    let Some(start) = report.find("## Summary") else {
        return report;
    };
    let rest = &report[start..];
    let end = rest[2..].find("\n## ").map_or(rest.len(), |at| at + 2);
    &rest[..end]
}
