//! Humata's own skills (D790): the folders under `apps/humata2/skills`,
//! carried inside this binary and written into an agent's skills folder
//! (`~/.claude/skills`, `~/.codex/skills`, `~/.copilot/skills`) whenever the
//! daemon starts, on the host and on every machine.
//!
//! Each skill becomes `<folder>/<name>/` holding its own files, a copy of
//! the shared reference files under `shared/`, and a mark file. The mark is
//! how Humata knows a folder is its own:
//! - a marked folder is replaced when it differs from what this binary
//!   ships, and removed when this binary no longer ships that skill;
//! - a folder, file or link of the same name without the mark is the
//!   person's own and is never touched;
//! - the skills sync leaves marked folders out ([`crate::scan_skills`]),
//!   because each machine's daemon installs its own.
//!
//! A person who wants to keep an edited copy deletes the mark file.
//!
//! Filesystem only. The caller decides the folder, and whether the agent it
//! belongs to is set up at all.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::skills::{SkillsError, io_err, unique};

/// The skills tree as `//apps/humata2/skills:bundle` packs it: for each
/// file a line `<path>\t<size>\n`, then that many bytes.
static BUNDLE: &[u8] = include_bytes!(env!("HUMATA2_OWN_SKILLS"));

/// The file that marks a skill folder as installed by Humata.
pub(crate) const MARK: &str = ".humata-skill";

const MARK_TEXT: &str = "Humata installed this skill and replaces this folder with its own copy each time its daemon starts.\nTo keep your own version, delete this file: Humata then leaves the folder alone.\n";

/// The folder of reference files every skill gets a copy of.
const SHARED: &str = "shared";

/// What every skill must hold.
const ENTRY: &str = "SKILL.md";

/// What a folder being written or replaced is named until it is in place.
const PASSING: &str = ".humata-skill-";

/// One skill: its name and its files by path under its folder.
struct OwnSkill<'a> {
    name: &'a str,
    files: BTreeMap<String, &'a [u8]>,
}

/// What an install did about one skill in one folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnSkillChange {
    /// It wasn't there and was written.
    Installed,
    /// Humata's copy differed from this binary's and was replaced.
    Updated,
    /// Humata's copy was already this binary's.
    Unchanged,
    /// The name is taken by something Humata didn't install; it was left.
    Kept,
    /// Humata installed it once and no longer ships it; it was removed.
    Retired,
}

/// One skill and what the install did about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnSkillOutcome {
    pub skill: String,
    pub change: OwnSkillChange,
}

/// Installs the skills this binary carries into `folder`, one agent's
/// skills folder, making it when it isn't there. Safe to run at every
/// start: a second run changes nothing.
pub fn install_own_skills(folder: &Path) -> Result<Vec<OwnSkillOutcome>, SkillsError> {
    let files = unpack(BUNDLE)?;
    install(&skills_of(&files)?, folder)
}

fn bad(what: &'static str) -> SkillsError {
    SkillsError::OwnBundle { what }
}

/// The files of a packed tree, in order.
fn unpack(mut rest: &[u8]) -> Result<Vec<(&str, &[u8])>, SkillsError> {
    let mut files = Vec::new();
    while !rest.is_empty() {
        let end = rest
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or_else(|| bad("a file's header line doesn't end"))?;
        let (header, after) = rest.split_at(end);
        let header = std::str::from_utf8(header).map_err(|_| bad("a file's header isn't UTF-8"))?;
        let (path, size) = header.split_once('\t').ok_or_else(|| bad("a file's header has no size"))?;
        let size: usize = size.parse().map_err(|_| bad("a file's size isn't a number"))?;
        let body = after.get(1..).unwrap_or_default();
        if body.len() < size {
            return Err(bad("a file is cut short"));
        }
        let (body, next) = body.split_at(size);
        files.push((path, body));
        rest = next;
    }
    Ok(files)
}

fn plain(path: &str) -> bool {
    path.split('/').all(|part| !part.is_empty() && part != "." && part != ".." && !part.contains('\0'))
}

/// Groups a tree's files into skills by their top folder. Files of the
/// `shared` folder go into every skill under `shared/`; a file outside any
/// folder belongs to no skill.
fn skills_of<'a>(files: &[(&'a str, &'a [u8])]) -> Result<Vec<OwnSkill<'a>>, SkillsError> {
    let mut shared: Vec<(&str, &[u8])> = Vec::new();
    let mut by_name: BTreeMap<&str, BTreeMap<String, &[u8]>> = BTreeMap::new();
    for &(path, body) in files {
        if !plain(path) {
            return Err(bad("a file's path isn't made of plain names"));
        }
        let Some((top, inner)) = path.split_once('/') else {
            continue;
        };
        if top == SHARED {
            shared.push((path, body));
        } else {
            by_name.entry(top).or_default().insert(inner.to_owned(), body);
        }
    }
    let mut skills = Vec::new();
    for (name, mut files) in by_name {
        if !files.contains_key(ENTRY) {
            return Err(bad("a skill folder has no SKILL.md"));
        }
        for &(path, body) in &shared {
            if files.insert(path.to_owned(), body).is_some() {
                return Err(bad("a skill has a file where a shared file goes"));
            }
        }
        if files.insert(MARK.to_owned(), MARK_TEXT.as_bytes()).is_some() {
            return Err(bad("a skill ships the mark file itself"));
        }
        skills.push(OwnSkill { name, files });
    }
    Ok(skills)
}

/// Whether `slot` is free, Humata's own, or the person's.
enum Slot {
    Free,
    Ours,
    Theirs,
}

fn slot_of(slot: &Path) -> Result<Slot, SkillsError> {
    match fs::symlink_metadata(slot) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Slot::Free),
        Err(source) => Err(SkillsError::Io { action: "inspect", path: slot.to_path_buf(), source }),
        Ok(found) if found.is_dir() => {
            let marked = fs::symlink_metadata(slot.join(MARK)).is_ok_and(|mark| mark.is_file());
            Ok(if marked { Slot::Ours } else { Slot::Theirs })
        }
        Ok(_) => Ok(Slot::Theirs),
    }
}

/// The paths under `dir` of everything that isn't a folder, links included.
fn listed(dir: &Path, relative: &str, into: &mut BTreeSet<String>) -> Result<(), SkillsError> {
    for child in fs::read_dir(dir).map_err(io_err("read the folder", dir))? {
        let child = child.map_err(io_err("read the folder", dir))?;
        let path = child.path();
        let name = child.file_name().to_string_lossy().into_owned();
        let rel = if relative.is_empty() { name } else { format!("{relative}/{name}") };
        if child.file_type().map_err(io_err("inspect", &path))?.is_dir() {
            listed(&path, &rel, into)?;
        } else {
            into.insert(rel);
        }
    }
    Ok(())
}

/// Whether the folder at `slot` holds exactly `skill`'s files.
fn same(slot: &Path, skill: &OwnSkill<'_>) -> Result<bool, SkillsError> {
    let mut found = BTreeSet::new();
    listed(slot, "", &mut found)?;
    if !found.iter().eq(skill.files.keys()) {
        return Ok(false);
    }
    for (path, body) in &skill.files {
        let file = slot.join(path);
        // A link where a file goes isn't Humata's copy.
        if !fs::symlink_metadata(&file).is_ok_and(|kind| kind.is_file()) {
            return Ok(false);
        }
        if fs::read(&file).map_err(io_err("read", &file))? != *body {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Writes `skill`'s files into `stage`, the mark first, so a folder cut
/// short is already known as Humata's.
fn fill(stage: &Path, skill: &OwnSkill<'_>) -> Result<(), SkillsError> {
    let mark = stage.join(MARK);
    fs::write(&mark, MARK_TEXT).map_err(io_err("write", &mark))?;
    for (path, body) in &skill.files {
        let file = stage.join(path);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent).map_err(io_err("create", parent))?;
        }
        fs::write(&file, body).map_err(io_err("write", &file))?;
    }
    Ok(())
}

/// Writes `skill` beside its slot and returns where.
fn staged(folder: &Path, skill: &OwnSkill<'_>) -> Result<PathBuf, SkillsError> {
    let stage = folder.join(format!("{PASSING}new-{}", unique()));
    fs::create_dir(&stage).map_err(io_err("create", &stage))?;
    match fill(&stage, skill) {
        Ok(()) => Ok(stage),
        Err(error) => {
            let _ = fs::remove_dir_all(&stage);
            Err(error)
        }
    }
}

/// Puts `skill` at `slot`, in place of Humata's older copy when `replace`.
fn put(folder: &Path, slot: &Path, skill: &OwnSkill<'_>, replace: bool) -> Result<(), SkillsError> {
    let stage = staged(folder, skill)?;
    let old = folder.join(format!("{PASSING}old-{}", unique()));
    if replace && let Err(source) = fs::rename(slot, &old) {
        let _ = fs::remove_dir_all(&stage);
        return Err(SkillsError::Io { action: "move aside", path: slot.to_path_buf(), source });
    }
    if let Err(source) = fs::rename(&stage, slot) {
        if replace {
            let _ = fs::rename(&old, slot);
        }
        let _ = fs::remove_dir_all(&stage);
        return Err(SkillsError::Io { action: "put in place", path: slot.to_path_buf(), source });
    }
    if replace {
        // The new copy is in place; a leftover is swept at the next install.
        let _ = fs::remove_dir_all(&old);
    }
    Ok(())
}

fn install(skills: &[OwnSkill<'_>], folder: &Path) -> Result<Vec<OwnSkillOutcome>, SkillsError> {
    fs::create_dir_all(folder).map_err(io_err("create", folder))?;
    let mut outcomes = Vec::new();
    let mut said = |skill: &str, change| outcomes.push(OwnSkillOutcome { skill: skill.to_owned(), change });
    for skill in skills {
        let slot = folder.join(skill.name);
        let change = match slot_of(&slot)? {
            Slot::Theirs => OwnSkillChange::Kept,
            Slot::Free => {
                put(folder, &slot, skill, false)?;
                OwnSkillChange::Installed
            }
            Slot::Ours if same(&slot, skill)? => OwnSkillChange::Unchanged,
            Slot::Ours => {
                put(folder, &slot, skill, true)?;
                OwnSkillChange::Updated
            }
        };
        said(skill.name, change);
    }
    // What an install cut short left behind, and skills no longer shipped.
    let shipped: BTreeSet<&str> = skills.iter().map(|skill| skill.name).collect();
    for child in fs::read_dir(folder).map_err(io_err("read the folder", folder))? {
        let child = child.map_err(io_err("read the folder", folder))?;
        let path = child.path();
        let name = child.file_name().to_string_lossy().into_owned();
        if shipped.contains(name.as_str()) || !matches!(slot_of(&path)?, Slot::Ours) {
            continue;
        }
        fs::remove_dir_all(&path).map_err(io_err("remove", &path))?;
        if !name.starts_with(PASSING) {
            said(&name, OwnSkillChange::Retired);
        }
    }
    Ok(outcomes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill<'a>(name: &'a str, files: &[(&str, &'a str)]) -> OwnSkill<'a> {
        let mut all: BTreeMap<String, &[u8]> =
            files.iter().map(|(path, body)| ((*path).to_owned(), body.as_bytes())).collect();
        all.insert(MARK.to_owned(), MARK_TEXT.as_bytes());
        OwnSkill { name, files: all }
    }

    fn changes(outcomes: &[OwnSkillOutcome]) -> Vec<(&str, OwnSkillChange)> {
        outcomes.iter().map(|outcome| (outcome.skill.as_str(), outcome.change)).collect()
    }

    fn read(path: PathBuf) -> String {
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn a_skill_is_written_with_its_reference_files_and_a_second_install_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("agent/skills");
        let board = skill("board", &[("SKILL.md", "pointer"), ("board.md", "commands"), ("deep/more.md", "more")]);

        let first = install(std::slice::from_ref(&board), &folder).unwrap();
        assert_eq!(changes(&first), [("board", OwnSkillChange::Installed)]);
        assert_eq!(read(folder.join("board/SKILL.md")), "pointer");
        assert_eq!(read(folder.join("board/board.md")), "commands");
        assert_eq!(read(folder.join("board/deep/more.md")), "more");

        let second = install(&[board], &folder).unwrap();
        assert_eq!(changes(&second), [("board", OwnSkillChange::Unchanged)]);
    }

    #[test]
    fn humatas_copy_is_replaced_when_it_differs_and_loses_files_no_longer_shipped() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().to_path_buf();
        install(&[skill("board", &[("SKILL.md", "v1"), ("old.md", "old")])], &folder).unwrap();
        // Edited by hand, too.
        fs::write(folder.join("board/SKILL.md"), "edited").unwrap();

        let next = install(&[skill("board", &[("SKILL.md", "v2"), ("new.md", "new")])], &folder).unwrap();
        assert_eq!(changes(&next), [("board", OwnSkillChange::Updated)]);
        assert_eq!(read(folder.join("board/SKILL.md")), "v2");
        assert_eq!(read(folder.join("board/new.md")), "new");
        assert!(!folder.join("board/old.md").exists());
    }

    #[test]
    fn a_skill_of_the_same_name_that_humata_did_not_install_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().to_path_buf();
        fs::create_dir_all(folder.join("board")).unwrap();
        fs::write(folder.join("board/SKILL.md"), "mine").unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        fs::write(elsewhere.path().join(MARK), "").unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), folder.join("linked")).unwrap();

        let outcomes = install(
            &[skill("board", &[("SKILL.md", "ours")]), skill("linked", &[("SKILL.md", "ours")])],
            &folder,
        )
        .unwrap();
        assert_eq!(changes(&outcomes), [("board", OwnSkillChange::Kept), ("linked", OwnSkillChange::Kept)]);
        assert_eq!(read(folder.join("board/SKILL.md")), "mine");
        assert!(!elsewhere.path().join("SKILL.md").exists());
    }

    #[test]
    fn deleting_the_mark_makes_an_installed_skill_the_persons_own() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().to_path_buf();
        install(&[skill("board", &[("SKILL.md", "v1")])], &folder).unwrap();
        fs::remove_file(folder.join("board").join(MARK)).unwrap();
        fs::write(folder.join("board/SKILL.md"), "my edit").unwrap();

        let next = install(&[skill("board", &[("SKILL.md", "v2")])], &folder).unwrap();
        assert_eq!(changes(&next), [("board", OwnSkillChange::Kept)]);
        assert_eq!(read(folder.join("board/SKILL.md")), "my edit");
    }

    #[test]
    fn a_skill_humata_no_longer_ships_is_removed_and_the_persons_own_stay() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().to_path_buf();
        install(&[skill("old-name", &[("SKILL.md", "x")]), skill("board", &[("SKILL.md", "x")])], &folder).unwrap();
        fs::create_dir_all(folder.join("mine")).unwrap();
        fs::write(folder.join("mine/SKILL.md"), "mine").unwrap();

        let next = install(&[skill("board", &[("SKILL.md", "x")])], &folder).unwrap();
        assert_eq!(
            changes(&next),
            [("board", OwnSkillChange::Unchanged), ("old-name", OwnSkillChange::Retired)]
        );
        assert!(!folder.join("old-name").exists());
        assert_eq!(read(folder.join("mine/SKILL.md")), "mine");
    }

    #[test]
    fn shared_files_go_into_every_skill_and_a_file_outside_a_folder_into_none() {
        let files: Vec<(&str, &[u8])> = [
            ("README.md", "not a skill"),
            ("a/SKILL.md", "a"),
            ("b/SKILL.md", "b"),
            ("b/ref.md", "ref"),
            ("shared/basics.md", "basics"),
        ]
        .into_iter()
        .map(|(path, body)| (path, body.as_bytes()))
        .collect();
        let skills = skills_of(&files).unwrap();
        let dir = tempfile::tempdir().unwrap();
        install(&skills, dir.path()).unwrap();
        assert_eq!(read(dir.path().join("a/shared/basics.md")), "basics");
        assert_eq!(read(dir.path().join("b/shared/basics.md")), "basics");
        assert_eq!(read(dir.path().join("b/ref.md")), "ref");
        assert!(!dir.path().join("shared").exists());
        assert!(!dir.path().join("README.md").exists());
    }

    #[test]
    fn a_packed_tree_unpacks_to_its_files_and_a_cut_one_is_refused() {
        let packed = b"a/SKILL.md\t5\nhellob/x.md\t0\nc/y.md\t3\n\n\t\n";
        let files = unpack(packed).unwrap();
        assert_eq!(
            files,
            [("a/SKILL.md", &b"hello"[..]), ("b/x.md", &b""[..]), ("c/y.md", &b"\n\t\n"[..])]
        );
        assert!(unpack(b"a/SKILL.md\t9\nshort").is_err());
        assert!(unpack(b"no header end").is_err());
    }

    #[test]
    fn a_folder_without_a_skill_md_or_with_a_path_that_climbs_is_refused() {
        assert!(skills_of(&[("a/notes.md", &b"x"[..])]).is_err());
        assert!(skills_of(&[("a/SKILL.md", &b"x"[..]), ("a/../../evil", &b"x"[..])]).is_err());
    }

    /// The `name:` of a SKILL.md's front matter.
    fn front_matter_name(text: &str) -> Option<&str> {
        let rest = text.strip_prefix("---\n")?;
        let (front, _) = rest.split_once("\n---\n")?;
        front.lines().find_map(|line| line.strip_prefix("name:")).map(str::trim)
    }

    #[test]
    fn every_skill_this_binary_carries_is_named_as_its_folder_and_has_a_description() {
        let files = unpack(BUNDLE).unwrap();
        let skills = skills_of(&files).unwrap();
        assert!(!skills.is_empty());
        for skill in &skills {
            let text = std::str::from_utf8(skill.files[ENTRY]).unwrap();
            assert_eq!(front_matter_name(text), Some(skill.name), "{}", skill.name);
            let front = text.split("\n---\n").next().unwrap();
            let described = front.lines().filter_map(|line| line.strip_prefix("description:")).any(|said| !said.trim().is_empty());
            assert!(described, "{}", skill.name);
        }
    }

    #[test]
    fn every_file_a_carried_skill_points_to_is_installed_with_it() {
        let files = unpack(BUNDLE).unwrap();
        for skill in skills_of(&files).unwrap() {
            let text = std::str::from_utf8(skill.files[ENTRY]).unwrap();
            let (_, body) = text.split_once("\n---\n").unwrap();
            for word in body.split('`').skip(1).step_by(2) {
                if word.ends_with(".md") && !word.contains(' ') {
                    assert!(skill.files.contains_key(word), "{} points to {word}, which it doesn't hold", skill.name);
                }
            }
        }
    }
}
