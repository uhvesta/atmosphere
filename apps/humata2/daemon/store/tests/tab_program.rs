use std::path::PathBuf;

use humata2_store::{Store, TabKind, TabProgram, TabRecord};

fn tab(id: &str) -> TabRecord {
    TabRecord {
        id: id.to_owned(),
        session: None,
        kind: TabKind::Shell,
        title: "my name".to_owned(),
        holder_socket: PathBuf::from(format!("/tmp/{id}.sock")),
        holder_pid: 1,
        created_unix_ms: 1,
        holder_generation: 0,
    }
}

#[test]
fn a_tab_whose_program_never_spoke_reads_empty() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_tab(&tab("tb_a")).unwrap();
    assert_eq!(store.tab_program("tb_a").unwrap(), TabProgram::default());
}

#[test]
fn the_title_and_folder_survive_reopening_and_the_tab_name_is_untouched() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open(dir.path()).unwrap();
        store.put_tab(&tab("tb_a")).unwrap();
        store.put_tab_program("tb_a", Some("build"), Some("/tmp/work")).unwrap();
    }
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(
        store.tab_program("tb_a").unwrap(),
        TabProgram {
            title: "build".to_owned(),
            pwd: "/tmp/work".to_owned()
        }
    );
    assert_eq!(store.tab("tb_a").unwrap().unwrap().title, "my name");
}

#[test]
fn saving_one_value_keeps_the_other() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_tab(&tab("tb_a")).unwrap();
    store.put_tab_program("tb_a", Some("one"), Some("/a")).unwrap();
    store.put_tab_program("tb_a", Some("two"), None).unwrap();
    store.put_tab_program("tb_a", None, Some("/b")).unwrap();
    let now = store.tab_program("tb_a").unwrap();
    assert_eq!((now.title.as_str(), now.pwd.as_str()), ("two", "/b"));
}

#[test]
fn resaving_the_tab_keeps_what_its_program_said() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_tab(&tab("tb_a")).unwrap();
    store.put_tab_program("tb_a", Some("build"), None).unwrap();
    store.put_tab(&tab("tb_a")).unwrap();
    assert_eq!(store.tab_program("tb_a").unwrap().title, "build");
}
