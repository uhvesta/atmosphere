//! The CLI and the host tools beside the daemon (D371): which of
//! `<bin>/humata`, `<bin>/humata2_holder` and `<bin>/stack` is not the one the
//! release names, and the cards of the items that say so. The holder and the
//! stack are one host tools item: it waits while either differs. Only a file
//! that is there and differs counts: a missing one is nothing to migrate.

use humata2_grpc::humata::v1 as pb;
use humata2_machines::InstalledFile;

pub use humata2_daemon_ports::release::installed_tools as installed;

/// The tools a release is checked for, each with the thing its Migrate item is.
pub(crate) const TOOLS: [(InstalledFile, pb::MigrateThing); 3] = [
    (InstalledFile::Cli, pb::MigrateThing::Cli),
    (InstalledFile::Holder, pb::MigrateThing::HostTools),
    (InstalledFile::Stack, pb::MigrateThing::HostTools),
];

/// The Migrate things the tools are, each once.
pub(crate) const THINGS: [pb::MigrateThing; 2] =
    [pb::MigrateThing::Cli, pb::MigrateThing::HostTools];

/// Whether one of the tools `thing` is for is in `differing`. A host tools
/// item stays while the holder or the stack differs, so it is withdrawn only
/// when neither does.
pub(crate) fn wants(differing: &[InstalledFile], thing: pb::MigrateThing) -> bool {
    TOOLS.iter().any(|(file, tool)| *tool == thing && differing.contains(file))
}

/// The tools that are in `installed` and are not the ones `files` names.
pub fn differing(files: &[pb::BuildFile], installed: &[pb::BuildFile]) -> Vec<InstalledFile> {
    TOOLS
        .iter()
        .map(|(file, _)| *file)
        .filter(|file| {
            let name = file.file_name();
            let named = files.iter().find(|entry| entry.name == name);
            let there = installed.iter().find(|entry| entry.name == name);
            matches!((named, there), (Some(named), Some(there)) if !named.sha256.eq_ignore_ascii_case(&there.sha256))
        })
        .collect()
}

/// What a tool's item shows as `from`: the release `own` is at, with `files`
/// set to what is installed.
pub fn from_installed(own: &pb::BuildInfo) -> pb::BuildInfo {
    pb::BuildInfo {
        files: own.installed.clone(),
        installed: Vec::new(),
        ..own.clone()
    }
}

/// What a tool's item shows as `to`, and what a daemon item's `to` is made
/// of: `own` without what it has installed, which changes by itself.
pub fn release_only(own: &pb::BuildInfo) -> pb::BuildInfo {
    pb::BuildInfo {
        installed: Vec::new(),
        ..own.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, sha256: &str) -> pb::BuildFile {
        pb::BuildFile {
            name: name.to_owned(),
            sha256: sha256.to_owned(),
        }
    }

    #[test]
    fn only_a_tool_that_is_there_and_differs_is_listed() {
        let named = [file("humata", "aa"), file("humata2_holder", "bb")];
        let there = [file("humata", "AA"), file("humata2_holder", "cc")];

        assert_eq!(differing(&named, &there), [InstalledFile::Holder]);
        assert!(differing(&named, &[]).is_empty(), "files that aren't there are nothing to migrate");
        assert!(differing(&[], &there).is_empty(), "a release that names none has nothing to differ from");
    }

    #[test]
    fn a_stack_that_differs_is_listed_and_wanted_as_host_tools() {
        let named = [file("humata", "aa"), file("humata2_holder", "bb"), file("stack", "cc")];
        let there = [file("humata", "aa"), file("humata2_holder", "bb"), file("stack", "DD")];

        assert_eq!(differing(&named, &there), [InstalledFile::Stack]);
        assert!(wants(&differing(&named, &there), pb::MigrateThing::HostTools));
        assert!(!wants(&differing(&named, &there), pb::MigrateThing::Cli));
    }

    #[test]
    fn host_tools_stay_wanted_while_either_the_holder_or_the_stack_differs() {
        let named = [file("humata", "aa"), file("humata2_holder", "bb"), file("stack", "cc")];
        let holder_differs = [file("humata", "aa"), file("humata2_holder", "xx"), file("stack", "cc")];
        let neither_differs = [file("humata", "aa"), file("humata2_holder", "bb"), file("stack", "cc")];

        assert!(wants(&differing(&named, &holder_differs), pb::MigrateThing::HostTools));
        assert!(!wants(&differing(&named, &neither_differs), pb::MigrateThing::HostTools));
    }
}
