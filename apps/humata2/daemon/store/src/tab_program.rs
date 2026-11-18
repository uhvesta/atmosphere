//! What the program in a terminal tab says about itself (D480): the title
//! it sets (OSC 0 and 2) and the folder it reports (OSC 7). They sit beside
//! the tab, never in its `title`, which is the user's own name for it, and
//! closing the tab removes them.

use std::path::PathBuf;

use rusqlite::{OptionalExtension, params};

use crate::Store;
use crate::StoreError;
use crate::store::check_id;

/// The title and folder a tab's program last reported. Empty means the
/// program never said.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TabProgram {
    pub title: String,
    pub pwd: String,
}

impl Store {
    /// Saves what the program of `tab` reported. A value left `None` stays
    /// as it was.
    pub fn put_tab_program(
        &self,
        tab: &str,
        title: Option<&str>,
        pwd: Option<&str>,
    ) -> Result<(), StoreError> {
        check_id(tab)?;
        self.inner().write("tab.program", "save what a tab's program reported", |transaction| {
            transaction
                .execute(
                    "INSERT INTO tab_program (tab, program_title, pwd)
                     VALUES (?1, COALESCE(?2, ''), COALESCE(?3, ''))
                     ON CONFLICT (tab) DO UPDATE SET
                         program_title = COALESCE(?2, program_title),
                         pwd = COALESCE(?3, pwd)",
                    params![tab, title, pwd],
                )
                .map(drop)
        })
    }

    /// What the program of `tab` last reported; empty when it never said.
    pub fn tab_program(&self, tab: &str) -> Result<TabProgram, StoreError> {
        check_id(tab)?;
        self.inner().read("read what a tab's program reported", |connection| {
            connection
                .query_row(
                    "SELECT program_title, pwd FROM tab_program WHERE tab = ?1",
                    [tab],
                    |row| {
                        Ok(TabProgram {
                            title: row.get(0)?,
                            pwd: row.get(1)?,
                        })
                    },
                )
                .optional()
                .map(Option::unwrap_or_default)
        })
    }
}

/// The folder an OSC 7 report names, from `file://host/path` (the host is
/// ignored, the path percent-decoded) or a plain absolute path. `None` for
/// anything else, so a malformed report never replaces a good folder.
pub fn pwd_path(reported: &str) -> Option<PathBuf> {
    let path = match reported.strip_prefix("file://") {
        Some(rest) => &rest[rest.find('/')?..],
        None => reported,
    };
    if !path.starts_with('/') {
        return None;
    }
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let pair = path.get(at + 1..at + 3)?;
            out.push(u8::from_str_radix(pair, 16).ok()?);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    let text = String::from_utf8(out).ok()?;
    if text.contains('\0') {
        return None;
    }
    Some(PathBuf::from(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_url_gives_its_path_without_the_host() {
        assert_eq!(
            pwd_path("file://my-mac.local/Users/me/work"),
            Some(PathBuf::from("/Users/me/work"))
        );
    }

    #[test]
    fn percent_escapes_are_decoded() {
        assert_eq!(
            pwd_path("file:///tmp/a%20b%C3%A9"),
            Some(PathBuf::from("/tmp/a b\u{e9}"))
        );
    }

    #[test]
    fn a_plain_absolute_path_is_taken_as_it_is() {
        assert_eq!(pwd_path("/var/log"), Some(PathBuf::from("/var/log")));
    }

    #[test]
    fn reports_that_are_not_a_folder_give_none() {
        assert_eq!(pwd_path("relative/dir"), None);
        assert_eq!(pwd_path("file://host-only"), None);
        assert_eq!(pwd_path("file:///bad%zz"), None);
        assert_eq!(pwd_path("file:///cut%2"), None);
        assert_eq!(pwd_path("file:///nul%00"), None);
    }
}
