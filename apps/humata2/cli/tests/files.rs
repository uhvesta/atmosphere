//! `humata files …` (CLI.md §5.5): locations, reading and writing files.

mod support;

use humata2_chunk::{Codec, Decoder, Encoder, Header};
use humata2_cli::{exit, rpc};
use humata2_proto::humata::v1 as pb;
use pb::file_location::Root;
use pb::write_file_message::Message as Part;
use humata2_cli::Environment;
use support::{FakeConsole, FakeDaemon, run, run_as};

// SHA-256 of "hello".
const HELLO_SHA256: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

fn chunk(data: &[u8], offset: u64, last: bool) -> pb::FileChunk {
    pb::FileChunk {
        offset,
        data: data.to_vec(),
        sha256: String::new(),
        last,
        ..Default::default()
    }
}

#[tokio::test]
async fn a_session_location_names_machine_session_repo_and_path() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "files", "ls", "ma_1:se_1/acme/web/src/lib"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let sent: pb::ListDirectoryRequest = daemon.only(rpc::files::LIST_DIRECTORY);
    assert_eq!(
        sent.location,
        Some(pb::FileLocation {
            machine: support::machine("ma_1"),
            root: Some(Root::SessionClone(pb::SessionClone {
                session: support::session("se_1"),
                repo: Some(pb::RepoRef {
                    owner: "acme".into(),
                    name: "web".into(),
                }),
            })),
            path: "src/lib".into(),
        })
    );
}

#[tokio::test]
async fn a_folder_location_names_machine_session_and_path_with_no_repo() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "files", "ls", "ma_1:folder/se_1/notes"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let sent: pb::ListDirectoryRequest = daemon.only(rpc::files::LIST_DIRECTORY);
    assert_eq!(
        sent.location,
        Some(pb::FileLocation {
            machine: support::machine("ma_1"),
            root: Some(Root::SessionFolder(pb::SessionFolder {
                session: support::session("se_1"),
            })),
            path: "notes".into(),
        })
    );
}

#[tokio::test]
async fn a_folder_location_without_a_session_is_refused() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "files", "ls", "ma_1:folder/"]).await;
    assert_eq!(exit, exit::USAGE);
    assert!(!daemon.called(rpc::files::LIST_DIRECTORY));
}

#[tokio::test]
async fn a_received_location_names_the_machine_it_is_on_the_source_machine_and_the_path() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "files", "ls", "ma_1:received/ma_2/notes"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let sent: pb::ListDirectoryRequest = daemon.only(rpc::files::LIST_DIRECTORY);
    assert_eq!(
        sent.location,
        Some(pb::FileLocation {
            machine: support::machine("ma_1"),
            root: Some(Root::Received(pb::MachineReceived {
                from: support::machine("ma_2"),
            })),
            path: "notes".into(),
        })
    );
}

#[tokio::test]
async fn a_received_location_without_a_source_machine_is_refused() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "files", "ls", "ma_1:received/"]).await;
    assert_eq!(exit, exit::USAGE);
    assert!(!daemon.called(rpc::files::LIST_DIRECTORY));
}

#[tokio::test]
async fn a_location_without_a_repo_is_refused() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "files", "ls", "ma_1:se_1/acme"]).await;
    assert_eq!(exit, exit::USAGE);
    assert!(!daemon.called(rpc::files::LIST_DIRECTORY));
}

#[tokio::test]
async fn cat_writes_the_bytes_as_they_are() {
    let daemon = FakeDaemon::new();
    daemon.stream(
        rpc::files::READ_FILE,
        &[&chunk(b"hel", 0, false), &chunk(b"lo", 3, true)],
    );
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--channel", "ch_1", "files", "cat", "ma_1:shared/notes.txt"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    assert_eq!(console.written(), b"hello");
}

#[tokio::test]
async fn cat_with_json_gives_base64_size_and_digest() {
    let daemon = FakeDaemon::new();
    daemon.stream(
        rpc::files::READ_FILE,
        &[&chunk(b"hel", 0, false), &chunk(b"lo", 3, true)],
    );
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "--channel", "ch_1", "files", "cat", "ma_1:shared/notes.txt"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let result = &console.json()["result"];
    assert_eq!(result["data"], "aGVsbG8=");
    assert_eq!(result["bytes"], 5);
    assert_eq!(result["sha256"], HELLO_SHA256);
}

#[tokio::test]
async fn put_sends_a_header_then_the_file() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let local = folder.path().join("hello.txt");
    std::fs::write(&local, b"hello").expect("the local file");
    let local = local.to_str().expect("a UTF-8 path");
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &[
            "--json", "--channel", "ch_1", "files", "put", local, "ma_1:channel/hello.txt",
            "--replace",
        ],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let parts: Vec<Option<Part>> = daemon
        .sent::<pb::WriteFileMessage>(rpc::files::WRITE_FILE)
        .into_iter()
        .map(|message| message.message)
        .collect();
    let Some(Some(Part::Header(header))) = parts.first() else {
        panic!("the first message is the header: {parts:?}");
    };
    assert_eq!(header.size, 5);
    assert_eq!(header.sha256, HELLO_SHA256);
    assert!(header.replace);
    assert_eq!(header.location.as_ref().map(|location| location.path.as_str()), Some("hello.txt"));
    let data: Vec<u8> = parts[1..]
        .iter()
        .flat_map(|part| match part {
            Some(Part::Chunk(chunk)) => chunk.data.clone(),
            other => panic!("only chunks follow the header: {other:?}"),
        })
        .collect();
    assert_eq!(data, b"hello");
    assert!(matches!(parts.last(), Some(Some(Part::Chunk(chunk))) if chunk.last));
}

#[tokio::test]
async fn put_of_a_big_file_sends_it_in_order_with_one_last_chunk() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let local = folder.path().join("big.bin");
    let bytes: Vec<u8> = (0..(5 << 19) + 7).map(|index: u32| (index % 251) as u8).collect();
    std::fs::write(&local, &bytes).expect("the local file");
    let local = local.to_str().expect("a UTF-8 path");
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "--channel", "ch_1", "files", "put", local, "ma_1:channel/big.bin"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let parts: Vec<Option<Part>> = daemon
        .sent::<pb::WriteFileMessage>(rpc::files::WRITE_FILE)
        .into_iter()
        .map(|message| message.message)
        .collect();
    let Some(Some(Part::Header(header))) = parts.first() else {
        panic!("the first message is the header: {parts:?}");
    };
    assert_eq!(header.size, bytes.len() as u64);
    let chunks: Vec<&pb::FileChunk> = parts[1..]
        .iter()
        .map(|part| match part {
            Some(Part::Chunk(chunk)) => chunk,
            other => panic!("only chunks follow the header: {other:?}"),
        })
        .collect();
    let mut sent = Vec::new();
    for chunk in &chunks {
        assert_eq!(chunk.offset, sent.len() as u64);
        sent.extend_from_slice(&chunk.data);
    }
    assert_eq!(sent, bytes);
    assert_eq!(chunks.iter().filter(|chunk| chunk.last).count(), 1);
    assert!(chunks.last().is_some_and(|chunk| chunk.last));
}

#[tokio::test]
async fn put_of_a_missing_local_file_sends_nothing() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let missing = folder.path().join("missing.txt");
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &[
            "--json",
            "--channel",
            "ch_1",
            "files",
            "put",
            missing.to_str().expect("a UTF-8 path"),
            "ma_1:channel/missing.txt",
        ],
    )
    .await;
    assert_ne!(exit, exit::OK);
    assert!(!daemon.called(rpc::files::WRITE_FILE));
}

#[tokio::test]
async fn mv_renames_in_place_and_refuses_a_path() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let renamed = run(
        &daemon,
        &console,
        &["--json", "--channel", "ch_1", "files", "mv", "ma_1:shared/a.txt", "b.txt"],
    )
    .await;
    assert_eq!(renamed, exit::OK);
    let sent: pb::RenameRequest = daemon.only(rpc::files::RENAME);
    assert_eq!(sent.new_name, "b.txt");
    let console = FakeConsole::pipe();
    let moved = run(
        &daemon,
        &console,
        &["--json", "--channel", "ch_1", "files", "mv", "ma_1:shared/a.txt", "docs/b.txt"],
    )
    .await;
    assert_eq!(moved, exit::USAGE);
    assert_eq!(daemon.sent::<pb::RenameRequest>(rpc::files::RENAME).len(), 1);
}

#[tokio::test]
async fn cp_needs_a_source_and_a_destination() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "cp", "ma_1:shared/a.txt"]).await;
    assert_eq!(exit, exit::USAGE);
}

/// NOPATH (PL-12): `humata cp|mv <location>… <machine>` places by PL-3.
fn channel_on(machines: &[&str]) -> pb::Channel {
    pb::Channel {
        id: support::channel("ch_1"),
        machines: machines.iter().map(|m| pb::MachineId { value: (*m).into() }).collect(),
        ..Default::default()
    }
}

fn shared_on(machine: &str, path: &str) -> Option<pb::FileLocation> {
    Some(pb::FileLocation {
        machine: support::machine(machine),
        root: Some(Root::ChannelShared(pb::ChannelShared {
            channel: support::channel("ch_1"),
        })),
        path: path.into(),
    })
}

fn received_on(machine: &str, from: &str) -> Option<pb::FileLocation> {
    Some(pb::FileLocation {
        machine: support::machine(machine),
        root: Some(Root::Received(pb::MachineReceived {
            from: support::machine(from),
        })),
        path: String::new(),
    })
}

#[tokio::test]
async fn a_shared_file_sent_to_a_machine_of_the_channel_lands_in_its_shared_folder() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::channel::GET_CHANNEL, &channel_on(&["ma_1", "ma_2"]));
    let console = FakeConsole::pipe();
    let words = ["--json", "--channel", "ch_1", "cp", "ma_1:shared/a.txt", "ma_2"];
    assert_eq!(run(&daemon, &console, &words).await, exit::OK);
    let sent: pb::StartTransferRequest = daemon.only(rpc::transfer::START_TRANSFER);
    assert_eq!(sent.destination, shared_on("ma_2", ""));
    assert_eq!(sent.on_existing, pb::OnExisting::KeepBoth as i32);
}

#[tokio::test]
async fn a_sessions_file_lands_under_a_folder_named_for_the_session() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::session::LIST_SESSIONS,
        &pb::ListSessionsResponse {
            sessions: vec![pb::Session {
                id: support::session("se_1"),
                channel: support::channel("ch_1"),
                machine: support::machine("ma_1"),
                title: "Fix login".into(),
                folder: "/home/me/Humata/payments/fix-login".into(),
                ..Default::default()
            }],
            page: None,
        },
    );
    daemon.reply(rpc::channel::GET_CHANNEL, &channel_on(&["ma_1", "ma_2"]));
    let console = FakeConsole::pipe();
    let words = ["--json", "mv", "ma_1:se_1/acme/web/notes.md", "ma_2"];
    assert_eq!(run(&daemon, &console, &words).await, exit::OK);
    let sent: pb::StartTransferRequest = daemon.only(rpc::transfer::START_TRANSFER);
    assert_eq!(sent.destination, shared_on("ma_2", "fix-login"));
    assert_eq!(sent.mode, pb::TransferMode::Move as i32);
}

#[tokio::test]
async fn a_machine_outside_the_channel_gets_it_in_its_received_place() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::channel::GET_CHANNEL, &channel_on(&["ma_1"]));
    let console = FakeConsole::pipe();
    let words = ["--json", "--channel", "ch_1", "cp", "ma_1:shared/a.txt", "ma_3"];
    assert_eq!(run(&daemon, &console, &words).await, exit::OK);
    let sent: pb::StartTransferRequest = daemon.only(rpc::transfer::START_TRANSFER);
    assert_eq!(sent.destination, received_on("ma_3", "ma_1"));
}

#[tokio::test]
async fn sources_that_land_in_two_places_start_one_transfer_for_each() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::channel::GET_CHANNEL, &channel_on(&["ma_2"]));
    let console = FakeConsole::pipe();
    let words = [
        "--json",
        "--channel",
        "ch_1",
        "cp",
        "ma_1:shared/a.txt",
        "ma_1:received/ma_4/b.txt",
        "ma_1:shared/c.txt",
        "ma_2",
    ];
    assert_eq!(run(&daemon, &console, &words).await, exit::OK);
    let sent: Vec<pb::StartTransferRequest> = daemon.sent(rpc::transfer::START_TRANSFER);
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].destination, shared_on("ma_2", ""));
    assert_eq!(sent[0].sources.len(), 2);
    assert_eq!(sent[1].destination, received_on("ma_2", "ma_1"));
    assert_eq!(sent[1].sources.len(), 1);
    assert_ne!(sent[0].idempotency_key, sent[1].idempotency_key);
}

#[tokio::test]
async fn a_machine_only_send_keeps_an_on_existing_that_was_asked_for() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::channel::GET_CHANNEL, &channel_on(&["ma_2"]));
    let console = FakeConsole::pipe();
    let words = [
        "--json",
        "--channel",
        "ch_1",
        "cp",
        "--on-existing",
        "replace",
        "ma_1:shared/a.txt",
        "ma_2",
    ];
    assert_eq!(run(&daemon, &console, &words).await, exit::OK);
    let sent: pb::StartTransferRequest = daemon.only(rpc::transfer::START_TRANSFER);
    assert_eq!(sent.on_existing, pb::OnExisting::Replace as i32);
}

#[tokio::test]
async fn a_send_to_a_location_still_fails_on_a_taken_name_unless_asked() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let words = ["--json", "--channel", "ch_1", "cp", "ma_1:shared/a.txt", "ma_2:shared/"];
    assert_eq!(run(&daemon, &console, &words).await, exit::OK);
    let sent: pb::StartTransferRequest = daemon.only(rpc::transfer::START_TRANSFER);
    assert_eq!(sent.on_existing, pb::OnExisting::Fail as i32);
    assert_eq!(sent.destination, shared_on("ma_2", ""));
}

#[tokio::test]
async fn a_send_to_a_machine_nobody_has_starts_nothing() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let words = ["--json", "--channel", "ch_1", "cp", "ma_1:shared/a.txt", "nowhere"];
    assert_eq!(run(&daemon, &console, &words).await, exit::NOT_FOUND);
    assert!(!daemon.called(rpc::transfer::START_TRANSFER));
}

/// D756: an agent's last word of `cp|mv`, as session se_1 of channel ch_1
/// with one clone, acme/api.
fn agent_on_ch_1() -> FakeDaemon {
    let daemon = FakeDaemon::new();
    who_is_agent_on_ch_1(&daemon);
    daemon
}

/// As [`agent_on_ch_1`], answering who the caller is for every location
/// of a copy whose sources and destination are all in the agent's clone:
/// each reply answers one call.
fn agent_on_ch_1_for_clone_copies() -> FakeDaemon {
    let daemon = FakeDaemon::new();
    for _ in 0..4 {
        who_is_agent_on_ch_1(&daemon);
    }
    daemon
}

fn who_is_agent_on_ch_1(daemon: &FakeDaemon) {
    daemon.reply(
        rpc::daemon::WHO_AM_I,
        &pb::WhoAmIResponse {
            actor: Some(pb::Actor {
                actor: Some(pb::actor::Actor::Agent(pb::AgentActor {
                    session: support::session("se_1"),
                    ..Default::default()
                })),
            }),
            channel: support::channel("ch_1"),
            ..Default::default()
        },
    );
    daemon.reply(
        rpc::session::GET_SESSION,
        &pb::Session {
            id: support::session("se_1"),
            repos: vec![pb::SessionRepo {
                repo: Some(pb::RepoRef {
                    owner: "acme".into(),
                    name: "api".into(),
                }),
                ..Default::default()
            }],
            ..Default::default()
        },
    );
}

/// Runs `humata --json --channel ch_1 cp ma_1:shared/a.txt <last>` as the
/// agent, and gives its exit and what it printed.
async fn agent_sends_to(daemon: &FakeDaemon, last: &str) -> (i32, String) {
    let console = FakeConsole::pipe();
    let environment = Environment {
        session_token: Some("session-secret".into()),
        ..Environment::default()
    };
    let words = ["--json", "--channel", "ch_1", "cp", "ma_1:shared/a.txt", last];
    let exit = run_as(&environment, daemon, &console, &words).await;
    (exit, console.stdout())
}

fn own_clone(path: &str) -> Option<pb::FileLocation> {
    Some(pb::FileLocation {
        machine: None,
        root: Some(Root::SessionClone(pb::SessionClone {
            session: support::session("se_1"),
            repo: Some(pb::RepoRef {
                owner: "acme".into(),
                name: "api".into(),
            }),
        })),
        path: path.into(),
    })
}

#[tokio::test]
async fn an_agents_bare_word_that_is_no_machine_it_sees_starts_nothing() {
    let daemon = agent_on_ch_1();
    let (exit, _) = agent_sends_to(&daemon, "vm-b").await;
    assert_eq!(exit, exit::USAGE);
    assert!(!daemon.called(rpc::transfer::START_TRANSFER));
}

#[tokio::test]
async fn an_agents_dot_slash_word_lands_in_its_clone() {
    let daemon = agent_on_ch_1();
    assert_eq!(agent_sends_to(&daemon, "./vm-b").await.0, exit::OK);
    let sent: pb::StartTransferRequest = daemon.only(rpc::transfer::START_TRANSFER);
    assert_eq!(sent.destination, own_clone("vm-b"));
}

#[tokio::test]
async fn an_agents_word_with_a_slash_lands_in_that_folder_of_its_clone() {
    let daemon = agent_on_ch_1();
    assert_eq!(agent_sends_to(&daemon, "vm-b/").await.0, exit::OK);
    let sent: pb::StartTransferRequest = daemon.only(rpc::transfer::START_TRANSFER);
    assert_eq!(sent.destination, own_clone("vm-b"));
}

#[tokio::test]
async fn an_agents_dotted_word_lands_in_its_clone() {
    let daemon = agent_on_ch_1();
    assert_eq!(agent_sends_to(&daemon, "notes.md").await.0, exit::OK);
    let sent: pb::StartTransferRequest = daemon.only(rpc::transfer::START_TRANSFER);
    assert_eq!(sent.destination, own_clone("notes.md"));
}

/// Runs `humata --json cp note.txt <last>` as the agent, from its clone.
async fn agent_copies_note_to(daemon: &FakeDaemon, last: &str) -> i32 {
    let console = FakeConsole::pipe();
    let environment = Environment {
        session_token: Some("session-secret".into()),
        ..Environment::default()
    };
    run_as(&environment, daemon, &console, &["--json", "cp", "note.txt", last]).await
}

#[tokio::test]
async fn an_agents_copy_to_a_name_nothing_holds_makes_that_file() {
    let daemon = agent_on_ch_1_for_clone_copies();
    daemon.refuse(rpc::files::STAT, pb::error::Kind::NotFound(Default::default()));
    assert_eq!(agent_copies_note_to(&daemon, "vm-b.txt").await, exit::OK);
    let sent: pb::StartTransferRequest = daemon.only(rpc::transfer::START_TRANSFER);
    assert_eq!(sent.destination, own_clone(""));
    assert_eq!(sent.name, "vm-b.txt");
    assert_eq!(sent.sources, vec![own_clone("note.txt").unwrap()]);
}

#[tokio::test]
async fn an_agents_copy_onto_a_file_names_that_file() {
    let daemon = agent_on_ch_1_for_clone_copies();
    daemon.reply(
        rpc::files::STAT,
        &pb::FileEntry {
            location: own_clone("docs/old.txt"),
            kind: pb::file_entry::Kind::File as i32,
            ..Default::default()
        },
    );
    assert_eq!(agent_copies_note_to(&daemon, "docs/old.txt").await, exit::OK);
    let sent: pb::StartTransferRequest = daemon.only(rpc::transfer::START_TRANSFER);
    assert_eq!(sent.destination, own_clone("docs"));
    assert_eq!(sent.name, "old.txt");
}

#[tokio::test]
async fn an_agents_copy_into_a_folder_keeps_the_sources_name() {
    let daemon = agent_on_ch_1_for_clone_copies();
    daemon.reply(
        rpc::files::STAT,
        &pb::FileEntry {
            location: own_clone("docs"),
            kind: pb::file_entry::Kind::Folder as i32,
            ..Default::default()
        },
    );
    assert_eq!(agent_copies_note_to(&daemon, "./docs").await, exit::OK);
    let sent: pb::StartTransferRequest = daemon.only(rpc::transfer::START_TRANSFER);
    assert_eq!(sent.destination, own_clone("docs"));
    assert!(sent.name.is_empty());
}

#[tokio::test]
async fn a_copy_of_two_sources_never_names_a_new_file() {
    let daemon = agent_on_ch_1_for_clone_copies();
    daemon.refuse(rpc::files::STAT, pb::error::Kind::NotFound(Default::default()));
    let console = FakeConsole::pipe();
    let environment = Environment {
        session_token: Some("session-secret".into()),
        ..Environment::default()
    };
    let words = ["--json", "cp", "a.txt", "b.txt", "out.d/"];
    assert_eq!(run_as(&environment, &daemon, &console, &words).await, exit::OK);
    let sent: pb::StartTransferRequest = daemon.only(rpc::transfer::START_TRANSFER);
    assert!(sent.name.is_empty());
    assert!(!daemon.called(rpc::files::STAT));
}

#[tokio::test]
async fn an_agents_word_naming_a_machine_it_sees_sends_there() {
    let daemon = agent_on_ch_1();
    daemon.reply(
        rpc::machine::LIST_MACHINES,
        &pb::ListMachinesResponse {
            machines: vec![pb::Machine {
                id: support::machine("ma_2"),
                name: "vm-a".into(),
                ..Default::default()
            }],
            page: None,
        },
    );
    daemon.reply(rpc::channel::GET_CHANNEL, &channel_on(&["ma_1", "ma_2"]));
    assert_eq!(agent_sends_to(&daemon, "vm-a").await.0, exit::OK);
    let sent: pb::StartTransferRequest = daemon.only(rpc::transfer::START_TRANSFER);
    assert_eq!(sent.destination, shared_on("ma_2", ""));
}

#[tokio::test]
async fn an_agent_cant_tell_a_hidden_machine_from_no_machine() {
    let nothing = agent_on_ch_1();
    let hidden = agent_on_ch_1();
    // The daemon lists only what the agent may see, but knows vm-b.
    hidden.reply(
        rpc::machine::GET_MACHINE,
        &pb::MachineDetail {
            machine: Some(pb::Machine {
                id: support::machine("ma_9"),
                name: "vm-b".into(),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    let refused = agent_sends_to(&nothing, "vm-b").await;
    assert_eq!(refused.0, exit::USAGE);
    assert_eq!(agent_sends_to(&hidden, "vm-b").await, refused);
    assert!(!hidden.called(rpc::transfer::START_TRANSFER));
}

/// The bytes a `files put` sent, decoded, and the codecs its chunks used.
fn received(daemon: &FakeDaemon) -> (Vec<u8>, Vec<Codec>) {
    let mut decoder = Decoder::new();
    let mut bytes = Vec::new();
    let mut codecs = Vec::new();
    for message in daemon.sent::<pb::WriteFileMessage>(rpc::files::WRITE_FILE) {
        let Some(Part::Chunk(chunk)) = message.message else { continue };
        assert_eq!(chunk.offset, bytes.len() as u64, "chunks arrive in order");
        let codec = Codec::from_wire(chunk.codec).expect("a known codec");
        codecs.push(codec);
        let header = Header {
            codec,
            raw_len: chunk.raw_size,
            crc32: chunk.crc32,
        };
        bytes.extend(decoder.decode(header, chunk.data).expect("the chunk decodes"));
    }
    (bytes, codecs)
}

fn source_like(len: usize) -> Vec<u8> {
    let line = b"pub fn put(local: &Path, location: FileLocation) -> Result<Done, CliError> {}\n";
    line.iter().copied().cycle().take(len).collect()
}

async fn put(daemon: &FakeDaemon, bytes: &[u8]) {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let local = folder.path().join("src.rs");
    std::fs::write(&local, bytes).expect("the local file");
    let console = FakeConsole::pipe();
    let local = local.to_str().expect("a UTF-8 path");
    let exit = run(
        daemon,
        &console,
        &["--json", "--channel", "ch_1", "files", "put", local, "ma_1:channel/src.rs"],
    )
    .await;
    assert_eq!(exit, exit::OK);
}

#[tokio::test]
async fn put_compresses_with_the_first_codec_the_daemon_offers() {
    let bytes = source_like((5 << 19) + 7);
    for (offered, expected) in [
        (vec![Codec::Lz4, Codec::Zstd, Codec::None], Codec::Lz4),
        (vec![Codec::Zstd, Codec::Lz4, Codec::None], Codec::Zstd),
        (vec![Codec::None], Codec::None),
    ] {
        let daemon = FakeDaemon::new();
        daemon.reply(
            rpc::daemon::GET_SERVER_INFO,
            &pb::ServerInfo {
                chunk_codecs: Codec::wire_list(&offered),
                ..Default::default()
            },
        );
        put(&daemon, &bytes).await;
        let (sent, codecs) = received(&daemon);
        assert_eq!(sent, bytes, "offered {offered:?}");
        assert!(codecs.contains(&expected), "offered {offered:?}, sent {codecs:?}");
        assert!(codecs.iter().all(|codec| [expected, Codec::None].contains(codec)));
    }
}

#[tokio::test]
async fn put_to_a_daemon_that_names_no_codec_sends_raw_chunks() {
    let bytes = source_like(3 << 19);
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::daemon::GET_SERVER_INFO,
        &pb::ServerInfo {
            chunk_codecs: vec![9],
            ..Default::default()
        },
    );
    put(&daemon, &bytes).await;
    let (sent, codecs) = received(&daemon);
    assert_eq!(sent, bytes);
    assert!(codecs.iter().all(|codec| *codec == Codec::None));
}

fn encoded(codec: Codec, raw: &[u8], offset: u64, last: bool) -> pb::FileChunk {
    let encoded = Encoder::new(codec).encode(raw.to_vec()).expect("encodes");
    pb::FileChunk {
        offset,
        data: encoded.data,
        sha256: String::new(),
        last,
        codec: encoded.header.codec.wire(),
        raw_size: encoded.header.raw_len,
        crc32: encoded.header.crc32,
    }
}

#[tokio::test]
async fn cat_decodes_chunks_whatever_their_codec() {
    let first = source_like(1 << 20);
    let second = source_like(4000);
    let daemon = FakeDaemon::new();
    daemon.stream(
        rpc::files::READ_FILE,
        &[
            &encoded(Codec::Zstd, &first, 0, false),
            &encoded(Codec::Lz4, &second, 1 << 20, true),
        ],
    );
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--channel", "ch_1", "files", "cat", "ma_1:shared/src.rs"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    assert_eq!(console.written(), [first, second].concat());
    let sent: pb::ReadFileRequest = daemon.only(rpc::files::READ_FILE);
    assert!(Codec::known(&sent.accept_codecs).contains(&Codec::Lz4));
}

#[tokio::test]
async fn cat_of_a_damaged_chunk_fails_rather_than_printing_it() {
    let mut chunk = encoded(Codec::Lz4, &source_like(64 << 10), 0, true);
    let middle = chunk.data.len() / 2;
    chunk.data[middle] ^= 0x5a;
    let daemon = FakeDaemon::new();
    daemon.stream(rpc::files::READ_FILE, &[&chunk]);
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--channel", "ch_1", "files", "cat", "ma_1:shared/src.rs"],
    )
    .await;
    assert_ne!(exit, exit::OK);
    assert!(console.written().is_empty());
}

/// D777: runs `humata --json --channel ch_1 cp <from> <to>` as the agent.
async fn agent_cp(daemon: &FakeDaemon, from: &str, to: &str) -> (i32, String) {
    let console = FakeConsole::pipe();
    let environment = Environment {
        session_token: Some("session-secret".into()),
        ..Environment::default()
    };
    let words = ["--json", "--channel", "ch_1", "cp", from, to];
    let exit = run_as(&environment, daemon, &console, &words).await;
    (exit, format!("{}{}", console.stdout(), console.stderr()))
}

/// The channel's shared folder on the agent's own machine: no machine named.
fn own_shared(path: &str) -> Option<pb::FileLocation> {
    Some(pb::FileLocation {
        machine: None,
        root: Some(Root::ChannelShared(pb::ChannelShared {
            channel: support::channel("ch_1"),
        })),
        path: path.into(),
    })
}

#[tokio::test]
async fn an_agents_bare_shared_folder_is_its_channels_shared_folder() {
    let daemon = agent_on_ch_1();
    assert_eq!(agent_cp(&daemon, "ma_1:shared/a.txt", "shared/").await.0, exit::OK);
    let sent: pb::StartTransferRequest = daemon.only(rpc::transfer::START_TRANSFER);
    assert_eq!(sent.destination, own_shared(""));
}

#[tokio::test]
async fn an_agents_shared_path_is_a_path_in_its_channels_shared_folder() {
    let daemon = agent_on_ch_1();
    assert_eq!(agent_cp(&daemon, "ma_1:shared/a.txt", "shared/notes").await.0, exit::OK);
    let sent: pb::StartTransferRequest = daemon.only(rpc::transfer::START_TRANSFER);
    assert_eq!(sent.destination, own_shared("notes"));
}

#[tokio::test]
async fn an_agents_bare_shared_source_is_in_its_channels_shared_folder() {
    let daemon = agent_on_ch_1();
    assert_eq!(agent_cp(&daemon, "shared/x.txt", "ma_1:shared/").await.0, exit::OK);
    let sent: pb::StartTransferRequest = daemon.only(rpc::transfer::START_TRANSFER);
    assert_eq!(sent.sources, vec![own_shared("x.txt").unwrap()]);
}

#[tokio::test]
async fn an_agents_dot_slash_shared_is_its_clone() {
    let daemon = agent_on_ch_1_for_clone_copies();
    assert_eq!(agent_cp(&daemon, "./shared/x.txt", "./shared/").await.0, exit::OK);
    let sent: pb::StartTransferRequest = daemon.only(rpc::transfer::START_TRANSFER);
    assert_eq!(sent.sources, vec![own_clone("shared/x.txt").unwrap()]);
    assert_eq!(sent.destination, own_clone("shared"));
}

#[tokio::test]
async fn an_agents_last_word_shared_is_the_shared_folder_when_no_machine_has_that_name() {
    let daemon = agent_on_ch_1();
    let (code, _) = agent_sends_to(&daemon, "shared").await;
    assert_eq!(code, exit::OK);
    let sent: pb::StartTransferRequest = daemon.only(rpc::transfer::START_TRANSFER);
    assert_eq!(sent.destination, own_shared(""));
}

#[tokio::test]
async fn an_agents_last_word_shared_is_that_machine_when_one_it_sees_has_the_name() {
    let daemon = agent_on_ch_1();
    daemon.reply(
        rpc::machine::LIST_MACHINES,
        &pb::ListMachinesResponse {
            machines: vec![pb::Machine {
                id: support::machine("ma_2"),
                name: "shared".into(),
                ..Default::default()
            }],
            page: None,
        },
    );
    daemon.reply(rpc::channel::GET_CHANNEL, &channel_on(&["ma_1", "ma_2"]));
    assert_eq!(agent_sends_to(&daemon, "shared").await.0, exit::OK);
    let sent: pb::StartTransferRequest = daemon.only(rpc::transfer::START_TRANSFER);
    assert_eq!(sent.destination, shared_on("ma_2", ""));
}

#[tokio::test]
async fn names_that_only_contain_shared_stay_in_the_clone() {
    for (typed, path) in [("sharedstuff/", "sharedstuff"), ("my/shared/", "my/shared")] {
        let daemon = agent_on_ch_1();
        assert_eq!(agent_sends_to(&daemon, typed).await.0, exit::OK);
        let sent: pb::StartTransferRequest = daemon.only(rpc::transfer::START_TRANSFER);
        assert_eq!(sent.destination, own_clone(path), "{typed}");
    }
}

#[tokio::test]
async fn a_persons_bare_shared_is_still_refused() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let words = ["--json", "--channel", "ch_1", "cp", "ma_1:shared/a.txt", "shared/"];
    assert_eq!(run(&daemon, &console, &words).await, exit::USAGE);
    assert!(!daemon.called(rpc::transfer::START_TRANSFER));
}

#[tokio::test]
async fn a_bare_shared_not_found_says_where_it_looked_and_differs_from_the_clones() {
    let bare = agent_on_ch_1();
    bare.refuse(rpc::transfer::START_TRANSFER, pb::error::Kind::NotFound(Default::default()));
    let (bare_exit, bare_out) = agent_cp(&bare, "shared/x", "ma_1:shared/").await;
    let clone = agent_on_ch_1();
    clone.refuse(rpc::transfer::START_TRANSFER, pb::error::Kind::NotFound(Default::default()));
    let (clone_exit, clone_out) = agent_cp(&clone, "./shared/x", "ma_1:shared/").await;
    assert_eq!(bare_exit, exit::NOT_FOUND);
    assert_eq!(clone_exit, exit::NOT_FOUND);
    assert_ne!(bare_out, clone_out);
}
