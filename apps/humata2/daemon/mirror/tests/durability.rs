//! What the mirror keeps on disk: the entries are user data and survive
//! anything, the index is derived and is rebuilt from them (LD-22, LD-23,
//! D62.5). The layout these tests rely on is in the crate's docs.

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use humata2_mirror::{FORMAT_VERSION, Hit, Mirror, MirrorEntry, Query};
use rusqlite::Connection;
use support::*;

fn index_db(root: &Path) -> PathBuf {
    root.join("search").join("index.db")
}

fn entries_log(root: &Path, session: &str) -> PathBuf {
    root.join("mirror").join(session).join("entries.log")
}

/// Removes the index's database with the files SQLite keeps beside it.
fn delete_index(root: &Path) {
    fs::remove_file(index_db(root)).unwrap();
    for beside in ["index.db-wal", "index.db-shm", "index.db-journal"] {
        let _ = fs::remove_file(root.join("search").join(beside));
    }
}

fn user_version(root: &Path) -> i64 {
    let db = Connection::open(index_db(root)).unwrap();
    db.pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap()
}

fn set_user_version(root: &Path, version: i64) {
    let db = Connection::open(index_db(root)).unwrap();
    db.pragma_update(None, "user_version", version).unwrap();
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|item| item.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Whether any file under `dir` holds `needle`.
fn any_file_holds(dir: &Path, needle: &[u8]) -> bool {
    fs::read_dir(dir).unwrap().any(|item| {
        let path = item.unwrap().path();
        if path.is_dir() {
            any_file_holds(&path, needle)
        } else {
            fs::read(&path)
                .unwrap()
                .windows(needle.len())
                .any(|window| window == needle)
        }
    })
}

/// Two sessions in two channels under `root`, with their conversation
/// files in `dir`: `se_cape` (two turns) and `se_port` (one turn), both
/// about a lighthouse.
fn coast(root: &Path, dir: &Path) -> (Mirror, PathBuf, PathBuf) {
    let mirror = Mirror::open(root).unwrap();
    let cape = mirror_session_as(
        &mirror,
        dir,
        &meta("se_cape", "ch_north", "ma_mac", "claude"),
        &[
            user("where is the lighthouse?", &at(0)),
            says("on the northern cape", &at(1)),
            user("who keeps it?", &at(2)),
            says("the lighthouse keeper logs the tide", &at(3)),
        ],
    );
    let port = mirror_session_as(
        &mirror,
        dir,
        &meta("se_port", "ch_south", "ma_box", "second-agent"),
        &[
            user("is there a lighthouse at the port?", &at(10)),
            says("a small one on the pier", &at(11)),
        ],
    );
    (mirror, cape, port)
}

/// Everything a client can see of [`coast`].
#[derive(Debug, PartialEq)]
struct Seen {
    lighthouse: Vec<Hit>,
    tide: Vec<Hit>,
    north_only: Vec<Hit>,
    cape: Vec<MirrorEntry>,
    port: Vec<MirrorEntry>,
}

fn seen(mirror: &Mirror) -> Seen {
    let north = Query {
        channels: vec!["ch_north".to_owned()],
        ..words("lighthouse")
    };
    Seen {
        lighthouse: find(mirror, "lighthouse"),
        tide: find(mirror, "tide"),
        north_only: all_hits(mirror, &north),
        cape: entries(mirror, "se_cape"),
        port: entries(mirror, "se_port"),
    }
}

fn place(session: &str, turn: u32) -> (String, u32) {
    (session.to_owned(), turn)
}

#[test]
fn entries_and_hits_are_the_same_after_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("state");
    let (mirror, _, _) = coast(&root, dir.path());
    let before = seen(&mirror);
    assert_eq!(
        places(&before.lighthouse),
        vec![
            place("se_cape", 1),
            place("se_cape", 2),
            place("se_port", 1)
        ]
    );
    assert_eq!(
        places(&before.north_only),
        vec![place("se_cape", 1), place("se_cape", 2)]
    );
    assert_eq!(before.cape.len(), 4);
    drop(mirror);

    let mirror = Mirror::open(&root).unwrap();

    assert_eq!(seen(&mirror), before);
}

#[test]
fn the_index_is_kept_under_the_format_version() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("state");
    let (mirror, _, _) = coast(&root, dir.path());
    drop(mirror);

    assert_eq!(user_version(&root), FORMAT_VERSION);
}

#[test]
fn a_deleted_index_is_rebuilt_from_the_mirror_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("state");
    let (mirror, cape, port) = coast(&root, dir.path());
    let before = seen(&mirror);
    drop(mirror);
    // The mirror is the host's own copy: the agents' files aren't needed.
    fs::remove_file(cape).unwrap();
    fs::remove_file(port).unwrap();

    delete_index(&root);
    let mirror = Mirror::open(&root).unwrap();

    // The same hits, with the names, the filters and the "as of" time that
    // were saved with the mirror.
    assert_eq!(seen(&mirror), before);
    drop(mirror);
    assert_eq!(user_version(&root), FORMAT_VERSION);
}

#[test]
fn a_deleted_index_folder_is_rebuilt_from_the_mirror_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("state");
    let (mirror, _, _) = coast(&root, dir.path());
    let before = seen(&mirror);
    drop(mirror);

    fs::remove_dir_all(root.join("search")).unwrap();
    let mirror = Mirror::open(&root).unwrap();

    assert_eq!(seen(&mirror), before);
}

#[test]
fn an_index_of_another_format_version_is_rebuilt() {
    for other in [FORMAT_VERSION + 1, FORMAT_VERSION - 1] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("state");
        let (mirror, _, _) = coast(&root, dir.path());
        let before = seen(&mirror);
        drop(mirror);

        set_user_version(&root, other);
        let mirror = Mirror::open(&root).unwrap();

        assert_eq!(seen(&mirror), before, "from version {other}");
        drop(mirror);
        assert_eq!(user_version(&root), FORMAT_VERSION, "from version {other}");
    }
}

#[test]
fn a_rebuild_takes_nothing_from_the_old_index() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("state");
    let (mirror, _, _) = coast(&root, dir.path());
    drop(mirror);

    // With one session's mirror folder taken away, an index built from the
    // mirror files can't know that session; the old index still does.
    fs::remove_dir_all(root.join("mirror").join("se_port")).unwrap();
    set_user_version(&root, FORMAT_VERSION + 1);
    let mirror = Mirror::open(&root).unwrap();

    assert_eq!(
        places(&find(&mirror, "lighthouse")),
        vec![place("se_cape", 1), place("se_cape", 2)]
    );
    assert_eq!(find(&mirror, "pier"), vec![]);
}

#[test]
fn a_damaged_index_is_rebuilt() {
    let damages: [&[u8]; 3] = [b"", b"this is not a database", &[0xA5; 8192]];
    for damage in damages {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("state");
        let (mirror, _, _) = coast(&root, dir.path());
        let before = seen(&mirror);
        drop(mirror);

        delete_index(&root);
        fs::write(index_db(&root), damage).unwrap();
        let mirror = Mirror::open(&root).unwrap();

        assert_eq!(
            seen(&mirror),
            before,
            "after {} bytes of damage",
            damage.len()
        );
        drop(mirror);
        assert_eq!(user_version(&root), FORMAT_VERSION);
    }
}

#[test]
fn a_rebuilt_index_goes_on_catching_up() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("state");
    let (mirror, cape, _) = coast(&root, dir.path());
    drop(mirror);
    delete_index(&root);
    let mirror = Mirror::open(&root).unwrap();

    assert_eq!(mirror.catch_up("se_cape", &cape, &no_vars()).unwrap(), 0);
    append_lines(
        &cape,
        &[
            user("and the foghorn?", &at(4)),
            says("beside the lighthouse", &at(5)),
        ],
    );
    assert_eq!(mirror.catch_up("se_cape", &cape, &no_vars()).unwrap(), 2);

    assert_eq!(
        places(&find(&mirror, "lighthouse")),
        vec![
            place("se_cape", 1),
            place("se_cape", 2),
            place("se_cape", 3),
            place("se_port", 1)
        ]
    );
    assert_eq!(places(&find(&mirror, "foghorn")), vec![place("se_cape", 3)]);
    let cape_entries = entries(&mirror, "se_cape");
    assert_eq!(
        cape_entries
            .iter()
            .map(|entry| entry.seq)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4, 5]
    );
    assert_eq!(cape_entries[5].turn, 3);
}

#[test]
fn a_cut_off_append_is_never_read_back() {
    // What a crash can leave after the last whole append: bytes that mean
    // nothing, the start of a real record, a run of zeros the file system
    // added, a single byte.
    let tails: [fn(&[u8]) -> Vec<u8>; 4] = [
        |_| b"\x00\xff\xfe{\"seq\":4,\"turn\":2,\"text\":\"half an ent".to_vec(),
        |log| log[..log.len().min(12)].to_vec(),
        |_| vec![0; 4096],
        |_| b"\n".to_vec(),
    ];
    for (n, tail) in tails.into_iter().enumerate() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("state");
        let (mirror, cape, _) = coast(&root, dir.path());
        let before = seen(&mirror);
        drop(mirror);

        let log = entries_log(&root, "se_cape");
        let whole = fs::read(&log).unwrap();
        assert!(
            !whole.is_empty(),
            "the session's entries are in {}",
            log.display()
        );
        append_bytes(&log, &tail(&whole));

        // The entries and hits are the ones from before the cut-off append.
        let mirror = Mirror::open(&root).unwrap();
        assert_eq!(seen(&mirror), before, "tail {n}");

        // The next catch-up adds what is new in the agent's file, once.
        assert_eq!(
            mirror.catch_up("se_cape", &cape, &no_vars()).unwrap(),
            0,
            "tail {n}"
        );
        append_lines(
            &cape,
            &[
                user("and the foghorn?", &at(4)),
                says("beside the lighthouse", &at(5)),
            ],
        );
        assert_eq!(
            mirror.catch_up("se_cape", &cape, &no_vars()).unwrap(),
            2,
            "tail {n}"
        );
        let after = entries(&mirror, "se_cape");
        assert_eq!(after[..4], before.cape[..], "tail {n}");
        assert_eq!(
            after[4..]
                .iter()
                .map(|entry| (entry.seq, entry.turn, entry.text.as_str()))
                .collect::<Vec<_>>(),
            vec![(4, 3, "and the foghorn?"), (5, 3, "beside the lighthouse")],
            "tail {n}"
        );
        drop(mirror);

        // The cut-off bytes are gone for good: the entries read back the
        // same after a restart, and an index built from the file again
        // finds them all.
        let mirror = Mirror::open(&root).unwrap();
        assert_eq!(entries(&mirror, "se_cape"), after, "tail {n}");
        drop(mirror);
        delete_index(&root);
        let mirror = Mirror::open(&root).unwrap();
        assert_eq!(entries(&mirror, "se_cape"), after, "tail {n}");
        assert_eq!(
            places(&find(&mirror, "lighthouse")),
            vec![
                place("se_cape", 1),
                place("se_cape", 2),
                place("se_cape", 3),
                place("se_port", 1)
            ],
            "tail {n}"
        );
        assert_eq!(entries(&mirror, "se_port"), before.port, "tail {n}");
    }
}

#[test]
fn a_forgotten_session_has_no_entries_and_no_hits() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("state");
    let (mirror, _, _) = coast(&root, dir.path());
    let port_before = entries(&mirror, "se_port");

    mirror.forget("se_cape").unwrap();

    assert_eq!(entries(&mirror, "se_cape"), vec![]);
    assert_eq!(
        places(&find(&mirror, "lighthouse")),
        vec![place("se_port", 1)]
    );
    assert_eq!(find(&mirror, "tide"), vec![]);
    assert_eq!(entries(&mirror, "se_port"), port_before);
    drop(mirror);
    // Its mirror is gone from the disk, not only from the answers.
    assert!(!any_file_holds(&root.join("mirror"), b"northern cape"));
    assert!(any_file_holds(&root.join("mirror"), b"on the pier"));
}

#[test]
fn a_forgotten_session_stays_gone_through_a_restart_and_a_rebuild() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("state");
    let (mirror, _, _) = coast(&root, dir.path());
    mirror.forget("se_cape").unwrap();
    drop(mirror);

    let mirror = Mirror::open(&root).unwrap();
    assert_eq!(entries(&mirror, "se_cape"), vec![]);
    assert_eq!(
        places(&find(&mirror, "lighthouse")),
        vec![place("se_port", 1)]
    );
    drop(mirror);

    delete_index(&root);
    let mirror = Mirror::open(&root).unwrap();
    assert_eq!(entries(&mirror, "se_cape"), vec![]);
    assert_eq!(
        places(&find(&mirror, "lighthouse")),
        vec![place("se_port", 1)]
    );
}

#[test]
fn forgetting_twice_or_a_session_never_mirrored_is_fine() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("state");
    let (mirror, _, _) = coast(&root, dir.path());

    mirror.forget("se_never_seen").unwrap();
    mirror.forget("se_cape").unwrap();
    mirror.forget("se_cape").unwrap();

    assert_eq!(
        places(&find(&mirror, "lighthouse")),
        vec![place("se_port", 1)]
    );
}

#[test]
fn a_forgotten_session_mirrors_again_from_the_start() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("state");
    let (mirror, cape, _) = coast(&root, dir.path());
    let before = entries(&mirror, "se_cape");
    mirror.forget("se_cape").unwrap();

    mirror
        .describe(&meta("se_cape", "ch_north", "ma_mac", "claude"))
        .unwrap();
    assert_eq!(mirror.catch_up("se_cape", &cape, &no_vars()).unwrap(), 4);

    assert_eq!(entries(&mirror, "se_cape"), before);
    assert_eq!(
        places(&find(&mirror, "lighthouse")),
        vec![
            place("se_cape", 1),
            place("se_cape", 2),
            place("se_port", 1)
        ]
    );
}

#[test]
fn a_session_id_never_names_a_path_outside_the_mirror_folder() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("state");
    let talks = dir.path().join("talks");
    fs::create_dir(&talks).unwrap();
    let ids = [
        "../../escaped",
        "a/b",
        "..",
        ".",
        "se 1/../x",
        "/abs/olute",
        "C:\\x",
        "se_ü",
        "a%2Fb",
    ];
    let mirror = Mirror::open(&root).unwrap();
    for (n, id) in ids.into_iter().enumerate() {
        let file = talks.join(format!("{n}.jsonl"));
        write_lines(&file, &[user(&format!("pelican item{n}"), &at(0))]);
        mirror
            .describe(&meta(id, "ch_main", "ma_mac", "claude"))
            .unwrap();
        assert_eq!(mirror.catch_up(id, &file, &no_vars()).unwrap(), 1, "{id:?}");
    }
    drop(mirror);

    // Each id is its own session, before and after a rebuild.
    for rebuild in [false, true] {
        if rebuild {
            delete_index(&root);
        }
        let mirror = Mirror::open(&root).unwrap();
        for (n, id) in ids.into_iter().enumerate() {
            assert_eq!(
                texts(&entries(&mirror, id)),
                vec![format!("pelican item{n}")],
                "{id:?}"
            );
            let hits = find(&mirror, &format!("item{n}"));
            assert_eq!(places(&hits), vec![place(id, 1)], "{id:?}");
        }
    }

    // And each is one folder right under mirror/.
    assert_eq!(names(dir.path()), vec!["state", "talks"]);
    assert_eq!(names(&root), vec!["mirror", "search"]);
    let folders = names(&root.join("mirror"));
    assert_eq!(folders.len(), ids.len(), "{folders:?}");
    assert!(
        folders
            .iter()
            .all(|name| root.join("mirror").join(name).is_dir()),
        "{folders:?}"
    );

    let mirror = Mirror::open(&root).unwrap();
    for id in ids {
        mirror.forget(id).unwrap();
    }
    assert_eq!(names(&root.join("mirror")), Vec::<String>::new());
    assert_eq!(names(dir.path()), vec!["state", "talks"]);
}
