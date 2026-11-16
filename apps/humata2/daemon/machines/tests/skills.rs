//! Skills sync engine: scan, plan, and the atomic apply (SK-1 to SK-11).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};

use humata2_machines::{
    plan_skills_sync, scan_skills, ChangeKind, Choice, ClashUse, Managed, SkillName, SkillSet, SkillsError,
    SkillsSource,
};
use tempfile::TempDir;

fn put(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn source(root: &Path) -> Vec<SkillsSource> {
    vec![SkillsSource { agent: "claude".into(), root: root.to_path_buf() }]
}

fn scan(root: &Path) -> SkillSet {
    scan_skills(&source(root), &[]).unwrap()
}

fn paths(set: &SkillSet) -> Vec<&str> {
    set.entries().iter().map(|e| e.path.as_str()).collect()
}

fn name(skill: &str) -> SkillName {
    SkillName { agent: "claude".into(), skill: skill.into() }
}

fn no_local() -> BTreeMap<String, BTreeSet<String>> {
    BTreeMap::new()
}

/// Sends a whole set through begin, write and finish.
fn apply(managed: &Managed, set: &SkillSet) -> Result<(), SkillsError> {
    let mut incoming = managed.begin(set.manifest())?;
    for entry in set.entries() {
        let body = fs::read(&entry.source).unwrap();
        for (i, chunk) in body.chunks(3).enumerate() {
            incoming.write(&entry.path, (i * 3) as u64, chunk)?;
        }
    }
    incoming.finish()
}

fn link_target(root: &Path) -> PathBuf {
    fs::read_link(root.join(".skills")).unwrap()
}

#[test]
fn scan_lists_files_with_hash_size_and_mode() {
    let dir = TempDir::new().unwrap();
    put(dir.path(), "plan/SKILL.md", "hello");
    put(dir.path(), "plan/bin/run.sh", "#!/bin/sh\n");
    fs::set_permissions(dir.path().join("plan/bin/run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    let set = scan(dir.path());
    assert_eq!(paths(&set), ["claude/plan/SKILL.md", "claude/plan/bin/run.sh"]);
    let skill = &set.entries()[0];
    assert_eq!(skill.size, 5);
    assert_eq!(skill.sha256, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
    assert!(!skill.executable);
    assert!(set.entries()[1].executable);
}

#[test]
fn secrets_never_land_in_the_set_even_without_excludes() {
    let dir = TempDir::new().unwrap();
    put(dir.path(), "s/SKILL.md", "ok");
    for secret in [".env", ".env.local", "a.pem", "b.key", "c.p12", "d.pfx", "id_rsa", "id_rsa.pub", "id_ed25519", "id_ecdsa", ".netrc", ".npmrc", "credentials", "credentials.json", "x.token", "token", ".token", "KEY.PEM"] {
        put(dir.path(), &format!("s/{secret}"), "secret");
    }
    let set = scan(dir.path());
    assert_eq!(paths(&set), ["claude/s/SKILL.md"]);
    assert_eq!(set.excluded_files(), 18);
}

#[test]
fn user_excludes_apply_and_are_counted() {
    let dir = TempDir::new().unwrap();
    put(dir.path(), "s/SKILL.md", "ok");
    put(dir.path(), "s/notes.tmp", "x");
    put(dir.path(), "s/deep/cache/a.bin", "x");
    put(dir.path(), "old/SKILL.md", "x");
    let excludes = ["*.tmp".to_string(), "s/**/cache/*".to_string(), "old/**".to_string()];
    let set = scan_skills(&source(dir.path()), &excludes).unwrap();
    assert_eq!(paths(&set), ["claude/s/SKILL.md"]);
    assert_eq!(set.excluded_files(), 3);
}

#[test]
fn symlinks_are_not_followed_and_count_as_excluded() {
    let dir = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    put(outside.path(), "x/SKILL.md", "outside");
    put(dir.path(), "s/SKILL.md", "ok");
    symlink(outside.path().join("x"), dir.path().join("linked")).unwrap();
    symlink(outside.path().join("x/SKILL.md"), dir.path().join("s/file")).unwrap();
    let set = scan(dir.path());
    assert_eq!(paths(&set), ["claude/s/SKILL.md"]);
    assert_eq!(set.excluded_files(), 2);
}

#[test]
fn a_missing_source_folder_is_an_empty_set() {
    let dir = TempDir::new().unwrap();
    let set = scan(&dir.path().join("not-installed"));
    assert!(set.entries().is_empty());
}

#[test]
fn equal_content_gives_equal_set_id_and_changes_change_it() {
    let one = TempDir::new().unwrap();
    let two = TempDir::new().unwrap();
    for dir in [&one, &two] {
        put(dir.path(), "a/SKILL.md", "A");
        put(dir.path(), "b/SKILL.md", "B");
    }
    assert_eq!(scan(one.path()).set_id(), scan(two.path()).set_id());
    put(two.path(), "b/SKILL.md", "B2");
    assert_ne!(scan(one.path()).set_id(), scan(two.path()).set_id());
}

#[test]
fn the_executable_bit_is_part_of_the_set_id() {
    let dir = TempDir::new().unwrap();
    put(dir.path(), "a/run.sh", "x");
    let before = scan(dir.path()).set_id().to_owned();
    fs::set_permissions(dir.path().join("a/run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    assert_ne!(scan(dir.path()).set_id(), before);
}

#[test]
fn manifest_round_trips_and_rejects_a_wrong_id() {
    let dir = TempDir::new().unwrap();
    put(dir.path(), "a/SKILL.md", "A");
    let manifest = scan(dir.path()).manifest();
    let json = manifest.to_json().unwrap();
    let back = humata2_machines::SkillsManifest::from_json(&json).unwrap();
    assert_eq!(back.set_id, manifest.set_id);
    assert_eq!(back.entries.len(), 1);
    let tampered = json.replace(&manifest.set_id, &"0".repeat(64));
    assert!(humata2_machines::SkillsManifest::from_json(&tampered).is_err());
}

#[test]
fn first_sync_adds_everything() {
    let dir = TempDir::new().unwrap();
    put(dir.path(), "a/SKILL.md", "A");
    put(dir.path(), "b/SKILL.md", "B");
    let plan = plan_skills_sync(&scan(dir.path()), None, &no_local(), &BTreeMap::new());
    assert!(plan.first_sync);
    assert!(plan.needs_asking());
    let kinds: Vec<_> = plan.changes.iter().map(|c| (c.name.skill.as_str(), c.kind)).collect();
    assert_eq!(kinds, [("a", ChangeKind::Add), ("b", ChangeKind::Add)]);
}

#[test]
fn later_plans_add_update_remove_and_unchanged_is_empty() {
    let dir = TempDir::new().unwrap();
    put(dir.path(), "keep/SKILL.md", "K");
    put(dir.path(), "edit/SKILL.md", "E");
    put(dir.path(), "grow/SKILL.md", "G");
    put(dir.path(), "gone/SKILL.md", "X");
    let old = scan(dir.path()).manifest();

    let same = plan_skills_sync(&scan(dir.path()), Some(&old), &no_local(), &BTreeMap::new());
    assert!(!same.first_sync);
    assert!(same.changes.is_empty());
    assert!(!same.needs_asking());

    put(dir.path(), "edit/SKILL.md", "E2");
    put(dir.path(), "grow/extra.md", "more");
    fs::remove_dir_all(dir.path().join("gone")).unwrap();
    put(dir.path(), "fresh/SKILL.md", "F");
    let plan = plan_skills_sync(&scan(dir.path()), Some(&old), &no_local(), &BTreeMap::new());
    let kinds: Vec<_> = plan.changes.iter().map(|c| (c.name.skill.as_str(), c.kind)).collect();
    assert_eq!(
        kinds,
        [
            ("edit", ChangeKind::Update),
            ("fresh", ChangeKind::Add),
            ("gone", ChangeKind::Remove),
            ("grow", ChangeKind::Update),
        ]
    );
}

#[test]
fn a_file_going_from_a_skill_makes_it_an_update() {
    let dir = TempDir::new().unwrap();
    put(dir.path(), "s/SKILL.md", "A");
    put(dir.path(), "s/extra.md", "B");
    let old = scan(dir.path()).manifest();
    fs::remove_file(dir.path().join("s/extra.md")).unwrap();
    let plan = plan_skills_sync(&scan(dir.path()), Some(&old), &no_local(), &BTreeMap::new());
    assert_eq!(plan.changes.len(), 1);
    assert_eq!(plan.changes[0].kind, ChangeKind::Update);
}

#[test]
fn clashes_are_new_until_a_choice_is_saved() {
    let dir = TempDir::new().unwrap();
    put(dir.path(), "a/SKILL.md", "A");
    put(dir.path(), "b/SKILL.md", "B");
    let set = scan(dir.path());
    let old = set.manifest();
    let local = BTreeMap::from([("claude".to_string(), BTreeSet::from(["a".to_string(), "mine".to_string()]))]);

    let plan = plan_skills_sync(&set, Some(&old), &local, &BTreeMap::new());
    assert_eq!(plan.clashes.len(), 1);
    assert_eq!(plan.clashes[0].name, name("a"));
    assert_eq!(plan.clashes[0].current, ClashUse::Unpicked);
    assert!(plan.clashes[0].is_new);
    assert!(plan.needs_asking());

    let choices = BTreeMap::from([(name("a"), Choice::Synced)]);
    let plan = plan_skills_sync(&set, Some(&old), &local, &choices);
    assert_eq!(plan.clashes[0].current, ClashUse::Synced);
    assert!(!plan.clashes[0].is_new);
    assert!(!plan.needs_asking());
}

#[test]
fn plan_carries_the_excluded_count() {
    let dir = TempDir::new().unwrap();
    put(dir.path(), "a/SKILL.md", "A");
    put(dir.path(), "a/.env", "S");
    let set = scan(dir.path());
    let plan = plan_skills_sync(&set, None, &no_local(), &BTreeMap::new());
    assert_eq!(plan.excluded_files, 1);
}

#[test]
fn apply_writes_the_set_and_the_link_switches_from_old_to_new() {
    let host = TempDir::new().unwrap();
    let machine = TempDir::new().unwrap();
    let managed = Managed::open(machine.path()).unwrap();
    assert!(managed.current_manifest().unwrap().is_none());

    put(host.path(), "a/SKILL.md", "version one");
    put(host.path(), "a/run.sh", "#!/bin/sh");
    fs::set_permissions(host.path().join("a/run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    let one = scan(host.path());
    apply(&managed, &one).unwrap();
    assert_eq!(fs::read_to_string(machine.path().join(".skills/claude/a/SKILL.md")).unwrap(), "version one");
    let mode = fs::metadata(machine.path().join(".skills/claude/a/run.sh")).unwrap().permissions().mode();
    assert_ne!(mode & 0o111, 0);
    assert_eq!(managed.current_manifest().unwrap().unwrap().set_id, one.set_id());
    let old_target = link_target(machine.path());

    put(host.path(), "a/SKILL.md", "version two");
    let two = scan(host.path());
    let mut incoming = managed.begin(two.manifest()).unwrap();
    for entry in two.entries() {
        incoming.write(&entry.path, 0, &fs::read(&entry.source).unwrap()).unwrap();
    }
    // Before finish a reader still sees the whole old set.
    assert_eq!(link_target(machine.path()), old_target);
    assert_eq!(fs::read_to_string(machine.path().join(".skills/claude/a/SKILL.md")).unwrap(), "version one");
    incoming.finish().unwrap();
    assert_ne!(link_target(machine.path()), old_target);
    assert_eq!(fs::read_to_string(machine.path().join(".skills/claude/a/SKILL.md")).unwrap(), "version two");
    // Only the current set is left.
    let sets: Vec<_> = fs::read_dir(machine.path().join(".skills-sets")).unwrap().collect();
    assert_eq!(sets.len(), 1);
}

#[test]
fn applying_the_same_set_again_succeeds_and_changes_nothing() {
    let host = TempDir::new().unwrap();
    let machine = TempDir::new().unwrap();
    put(host.path(), "a/SKILL.md", "x");
    put(host.path(), "empty/keep", "");
    let set = scan(host.path());
    let managed = Managed::open(machine.path()).unwrap();
    apply(&managed, &set).unwrap();
    let target = link_target(machine.path());
    apply(&managed, &set).unwrap();
    assert_eq!(link_target(machine.path()), target);
    assert!(machine.path().join(".skills/claude/empty/keep").is_file());
    assert_eq!(fs::read_dir(machine.path().join(".skills-sets")).unwrap().count(), 1);
}

#[test]
fn a_cut_short_apply_leaves_the_old_set_and_the_next_apply_cleans_up() {
    let host = TempDir::new().unwrap();
    let machine = TempDir::new().unwrap();
    let managed = Managed::open(machine.path()).unwrap();
    put(host.path(), "a/SKILL.md", "one");
    apply(&managed, &scan(host.path())).unwrap();
    let old_target = link_target(machine.path());

    put(host.path(), "a/SKILL.md", "two two two");
    let two = scan(host.path());
    {
        let mut incoming = managed.begin(two.manifest()).unwrap();
        incoming.write("claude/a/SKILL.md", 0, b"two").unwrap();
    }
    assert_eq!(link_target(machine.path()), old_target);
    assert_eq!(fs::read_to_string(machine.path().join(".skills/claude/a/SKILL.md")).unwrap(), "one");

    // A kill leaves the temp folder behind; simulate it by planting one.
    fs::create_dir_all(machine.path().join(".skills-sets/.incoming-dead/claude")).unwrap();
    symlink("nowhere", machine.path().join(".skills.new-dead")).unwrap();
    apply(&managed, &two).unwrap();
    assert_eq!(fs::read_to_string(machine.path().join(".skills/claude/a/SKILL.md")).unwrap(), "two two two");
    assert_eq!(fs::read_dir(machine.path().join(".skills-sets")).unwrap().count(), 1);
    assert!(fs::symlink_metadata(machine.path().join(".skills.new-dead")).is_err());
}

#[test]
fn unsafe_or_unlisted_paths_are_refused_and_nothing_is_written() {
    let host = TempDir::new().unwrap();
    let parent = TempDir::new().unwrap();
    let root = parent.path().join("Humata");
    put(host.path(), "a/SKILL.md", "x");
    let set = scan(host.path());
    let managed = Managed::open(&root).unwrap();
    let mut incoming = managed.begin(set.manifest()).unwrap();
    for bad in ["../evil", "claude/../../evil", "/etc/evil", "claude//a", "", "claude/a/../../x", "claude/other/SKILL.md"] {
        assert!(incoming.write(bad, 0, b"x").is_err(), "{bad:?} should be refused");
    }
    drop(incoming);
    let names: Vec<_> = fs::read_dir(parent.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(names, ["Humata"]);
    assert!(!root.join("evil").exists());
    assert_eq!(fs::read_dir(root.join(".skills-sets")).unwrap().count(), 0);
}

#[test]
fn a_manifest_listing_an_unsafe_path_is_refused_at_begin() {
    let host = TempDir::new().unwrap();
    let machine = TempDir::new().unwrap();
    put(host.path(), "a/SKILL.md", "x");
    let mut manifest = scan(host.path()).manifest();
    manifest.entries[0].path = "claude/../../evil".into();
    let managed = Managed::open(machine.path()).unwrap();
    assert!(managed.begin(manifest).is_err());
}

#[test]
fn chunks_out_of_order_or_past_the_size_are_refused() {
    let host = TempDir::new().unwrap();
    let machine = TempDir::new().unwrap();
    put(host.path(), "a/SKILL.md", "four");
    let set = scan(host.path());
    let managed = Managed::open(machine.path()).unwrap();
    let mut incoming = managed.begin(set.manifest()).unwrap();
    assert!(incoming.write("claude/a/SKILL.md", 2, b"ur").is_err());
    assert!(incoming.write("claude/a/SKILL.md", 0, b"too long").is_err());
    incoming.write("claude/a/SKILL.md", 0, b"four").unwrap();
    incoming.finish().unwrap();
}

#[test]
fn a_wrong_hash_or_short_file_fails_finish_and_keeps_the_old_set() {
    let host = TempDir::new().unwrap();
    let machine = TempDir::new().unwrap();
    let managed = Managed::open(machine.path()).unwrap();
    put(host.path(), "a/SKILL.md", "good");
    apply(&managed, &scan(host.path())).unwrap();
    let old_target = link_target(machine.path());

    put(host.path(), "a/SKILL.md", "next");
    let next = scan(host.path());
    let mut incoming = managed.begin(next.manifest()).unwrap();
    incoming.write("claude/a/SKILL.md", 0, b"evil").unwrap();
    assert!(matches!(incoming.finish(), Err(SkillsError::HashMismatch { .. })));
    assert_eq!(link_target(machine.path()), old_target);
    assert_eq!(fs::read_to_string(machine.path().join(".skills/claude/a/SKILL.md")).unwrap(), "good");

    let mut short = managed.begin(next.manifest()).unwrap();
    short.write("claude/a/SKILL.md", 0, b"ne").unwrap();
    assert!(matches!(short.finish(), Err(SkillsError::Incomplete { .. })));
    assert_eq!(link_target(machine.path()), old_target);

    let missing = managed.begin(next.manifest()).unwrap();
    assert!(missing.finish().is_err());
    assert_eq!(link_target(machine.path()), old_target);
}

#[test]
fn a_real_skills_folder_is_refused_and_left_alone() {
    let machine = TempDir::new().unwrap();
    put(machine.path(), ".skills/mine/SKILL.md", "mine");
    assert!(matches!(Managed::open(machine.path()), Err(SkillsError::ForeignSkillsPath { .. })));
    assert_eq!(fs::read_to_string(machine.path().join(".skills/mine/SKILL.md")).unwrap(), "mine");
}

#[test]
fn a_link_to_somewhere_else_is_refused() {
    let machine = TempDir::new().unwrap();
    let elsewhere = TempDir::new().unwrap();
    symlink(elsewhere.path(), machine.path().join(".skills")).unwrap();
    assert!(matches!(Managed::open(machine.path()), Err(SkillsError::ForeignSkillsPath { .. })));
}

#[test]
fn machine_local_files_are_never_touched() {
    let host = TempDir::new().unwrap();
    let machine = TempDir::new().unwrap();
    put(machine.path(), "my-skills/local/SKILL.md", "local");
    put(machine.path(), "notes.txt", "notes");
    put(machine.path(), ".skills-other/x", "x");
    put(host.path(), "a/SKILL.md", "one");
    let managed = Managed::open(machine.path()).unwrap();
    apply(&managed, &scan(host.path())).unwrap();
    put(host.path(), "a/SKILL.md", "two");
    apply(&managed, &scan(host.path())).unwrap();
    assert_eq!(fs::read_to_string(machine.path().join("my-skills/local/SKILL.md")).unwrap(), "local");
    assert_eq!(fs::read_to_string(machine.path().join("notes.txt")).unwrap(), "notes");
    assert_eq!(fs::read_to_string(machine.path().join(".skills-other/x")).unwrap(), "x");
}

#[test]
fn choices_are_saved_and_survive_syncs() {
    let host = TempDir::new().unwrap();
    let machine = TempDir::new().unwrap();
    let managed = Managed::open(machine.path()).unwrap();
    assert!(managed.choices().unwrap().is_empty());
    managed.set_choice(&name("a"), Choice::Local).unwrap();
    managed.set_choice(&name("b"), Choice::Synced).unwrap();
    managed.set_choice(&name("a"), Choice::Synced).unwrap();
    put(host.path(), "a/SKILL.md", "x");
    apply(&managed, &scan(host.path())).unwrap();
    let choices = Managed::open(machine.path()).unwrap().choices().unwrap();
    assert_eq!(choices, BTreeMap::from([(name("a"), Choice::Synced), (name("b"), Choice::Synced)]));
}
