//! Checking a copy of a session that was carried into a side folder rather
//! than unpacked (D224): what a machine still held when the session was
//! restored elsewhere in the meantime. The check is the one an unpack makes
//! before it changes anything, run on the side folder alone; nothing is
//! placed or imported.
//!
//! A side folder holds the machine's staged files as `stage/` (its
//! manifest, records, the agent's files and the shell tabs' saved output)
//! and, when the machine still had one, the session's own folder as
//! `folder/`.

use std::fs;
use std::path::Path;

use crate::carry::{
    AGENT, AgentFiles, Area, CarryError, Item, MANIFEST, Manifest, ManifestProblem, RECORDS, STAGE, TABS, io, sha_of_file,
    verify,
};

/// What a side folder holds, once checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Brought {
    /// Files and links of the manifest, the records included.
    pub files: u32,
    pub bytes: u64,
    /// The machine's session folder came along, as `folder/`.
    pub folder: bool,
    /// The agent's files, as paths under `stage/agent`.
    pub agent_files: Vec<String>,
    /// The shell tabs' saved output, as paths under `stage/tabs`.
    pub tabs: Vec<String>,
}

/// Checks that `side` holds everything its manifest names and nothing else,
/// each file as sent. Blocking.
pub fn check_brought(side: &Path) -> Result<Brought, CarryError> {
    let stage = side.join(STAGE);
    let path = stage.join(MANIFEST);
    let bytes = fs::read(&path)
        .map_err(|source| CarryError::Manifest { path: path.clone(), source: ManifestProblem::Read(source) })?;
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|source| CarryError::Manifest { path: path.clone(), source: ManifestProblem::Parse(source) })?;

    let of = |area: Area| -> Vec<&Item> { manifest.items.iter().filter(|item| item.area == area).collect() };

    let folder_items = of(Area::Folder);
    let folder = side.join("folder");
    if let Some(first) = folder_items.first()
        && !folder.is_dir()
    {
        return Err(CarryError::Missing { item: first.name() });
    }
    verify(Area::Folder, &folder, &folder_items)?;

    let agent_dir = stage.join(AGENT);
    if manifest.agent_files == AgentFiles::None {
        if agent_dir.exists() {
            return Err(CarryError::Unexpected { item: "the agent's files".to_owned() });
        }
    } else {
        verify(Area::Agent, &agent_dir, &of(Area::Agent))?;
    }
    let tab_items = of(Area::Tabs);
    verify(Area::Tabs, &stage.join(TABS), &tab_items)?;

    let records = stage.join(RECORDS);
    if fs::symlink_metadata(&records).is_err() {
        return Err(CarryError::Missing { item: "the session's records".to_owned() });
    }
    let (size, sha256) = sha_of_file(&records).map_err(io("read", &records))?;
    match of(Area::Records).first() {
        Some(item) if item.size == size && item.sha256 == sha256 => {}
        _ => return Err(CarryError::Changed { item: "the session's records".to_owned() }),
    }

    Ok(Brought {
        files: u32::try_from(manifest.items.len()).unwrap_or(u32::MAX),
        bytes: manifest.items.iter().map(|item| item.size).sum(),
        folder: folder.is_dir(),
        agent_files: of(Area::Agent).iter().map(|item| item.rel.clone()).collect(),
        tabs: tab_items.iter().map(|item| item.rel.clone()).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::carry::items_under;

    /// A side folder as a carry leaves it: a session folder with one file,
    /// a conversation, one tab's output and the records, with its manifest.
    fn side(root: &Path, with_folder: bool) -> std::path::PathBuf {
        let side = root.join("from-box-1-op_1");
        let stage = side.join("stage");
        fs::create_dir_all(stage.join("agent")).unwrap();
        fs::create_dir_all(stage.join("tabs")).unwrap();
        fs::write(stage.join("agent/conv.jsonl"), "{\"said\":1}\n").unwrap();
        fs::write(stage.join("tabs/tb_1.scrollback"), "output").unwrap();
        fs::write(stage.join("records.sqlite"), "records").unwrap();
        let mut items = Vec::new();
        if with_folder {
            fs::create_dir_all(side.join("folder/api")).unwrap();
            fs::write(side.join("folder/api/a.txt"), "work").unwrap();
            items.extend(items_under(Area::Folder, &side.join("folder")).unwrap());
        }
        items.extend(items_under(Area::Agent, &stage.join("agent")).unwrap());
        items.extend(items_under(Area::Tabs, &stage.join("tabs")).unwrap());
        let (size, sha256) = sha_of_file(&stage.join("records.sqlite")).unwrap();
        items.push(Item { area: Area::Records, rel: RECORDS.to_owned(), size, sha256, link: None });
        let manifest = Manifest {
            operation: "op_1".to_owned(),
            session: "se_1".to_owned(),
            channel: "ch_1".to_owned(),
            folder: "sessions/cache".to_owned(),
            old_folder: "/old/sessions/cache".to_owned(),
            agent: "claude".to_owned(),
            conversation_id: "c1".to_owned(),
            agent_files: AgentFiles::File { name: "conv.jsonl".to_owned() },
            items,
            tabs: Vec::new(),
        };
        fs::write(stage.join("manifest.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
        side
    }

    #[test]
    fn a_side_folder_as_sent_checks_out_and_says_what_it_holds() {
        let root = tempfile::tempdir().unwrap();
        let side = side(root.path(), true);
        let brought = check_brought(&side).unwrap();
        assert!(brought.folder);
        assert_eq!(brought.agent_files, vec!["conv.jsonl".to_owned()]);
        assert_eq!(brought.tabs, vec!["tb_1.scrollback".to_owned()]);
        assert_eq!(brought.files, 4);
    }

    #[test]
    fn a_copy_without_the_session_folder_checks_out_when_none_was_expected() {
        let root = tempfile::tempdir().unwrap();
        let side = side(root.path(), false);
        let brought = check_brought(&side).unwrap();
        assert!(!brought.folder);
    }

    #[test]
    fn a_byte_changed_on_the_way_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let side = side(root.path(), true);
        fs::write(side.join("folder/api/a.txt"), "w0rk").unwrap();
        assert!(matches!(check_brought(&side), Err(CarryError::Changed { .. })));
        let side2 = self::side(&root.path().join("again"), true);
        fs::write(side2.join("stage/agent/conv.jsonl"), "{\"said\":2}\n").unwrap();
        assert!(matches!(check_brought(&side2), Err(CarryError::Changed { .. })));
    }

    #[test]
    fn a_file_the_manifest_doesnt_name_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let side = side(root.path(), true);
        fs::write(side.join("folder/extra.txt"), "x").unwrap();
        assert!(matches!(check_brought(&side), Err(CarryError::Unexpected { .. })));
    }

    #[test]
    fn missing_records_are_refused() {
        let root = tempfile::tempdir().unwrap();
        let side = side(root.path(), true);
        fs::remove_file(side.join("stage/records.sqlite")).unwrap();
        assert!(matches!(check_brought(&side), Err(CarryError::Missing { .. })));
    }

    #[test]
    fn a_folder_that_never_arrived_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let side = side(root.path(), true);
        fs::remove_dir_all(side.join("folder")).unwrap();
        assert!(matches!(check_brought(&side), Err(CarryError::Missing { .. })));
    }

    #[test]
    fn a_side_folder_without_a_manifest_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let side = side(root.path(), false);
        fs::remove_file(side.join("stage/manifest.json")).unwrap();
        assert!(matches!(check_brought(&side), Err(CarryError::Manifest { .. })));
    }
}
