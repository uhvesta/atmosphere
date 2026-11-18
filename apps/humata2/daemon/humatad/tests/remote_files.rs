//! The file manager on another machine, through the host's own socket:
//! list, new folder, rename, delete (to that machine's trash), restore,
//! write and read, with `location.machine` naming the machine (FM-9, FM-12;
//! D139, D156, D165). A real host daemon and a machine behind a fake `ssh`
//! whose `humatad2` is the real binary, as in `remote_transfers.rs`.
//!
//! Enabling is a move to `tests/remote_files.rs`.
//!
//! Nothing here depends on speed: a link is ended through the process group
//! the rig recorded for it and kept away with the fake machine's own `hang`
//! flag, and the test goes on only once the host itself shows the machine
//! offline, or online again, on its machine events (see `mod rig` at the
//! end of the file).

mod common;

use std::collections::BTreeSet;
use std::path::Path;

use common::{error_of, invalid_field};
use humata2_chunk::{CHUNK_MAX, Codec, Decoder, Encoder, Header};
use humata2_grpc::humata::v1 as pb;
use rig::{Rig, shared};
use sha2::{Digest, Sha256};
use tonic::transport::Channel;
use tonic::{Code, Status};

/// The one machine of these tests.
const NAME: &str = "box-1";

/// `path` in the channel's shared folder on the machine.
fn there(path: &str) -> pb::FileLocation {
    shared(Some(NAME), path)
}

async fn files(rig: &Rig) -> pb::file_service_client::FileServiceClient<Channel> {
    pb::file_service_client::FileServiceClient::new(rig.channel().await)
}

async fn trash(rig: &Rig) -> pb::trash_service_client::TrashServiceClient<Channel> {
    pb::trash_service_client::TrashServiceClient::new(rig.channel().await)
}

async fn list(rig: &Rig, location: pb::FileLocation) -> Result<Vec<pb::FileEntry>, Status> {
    files(rig)
        .await
        .list_directory(pb::ListDirectoryRequest {
            location: Some(location),
            page: None,
        })
        .await
        .map(|reply| reply.into_inner().entries)
}

fn sha256(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// `len` bytes that don't compress, the same for the same seed.
fn noise(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut data = Vec::with_capacity(len + 8);
    while data.len() < len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        data.extend_from_slice(&state.to_le_bytes());
    }
    data.truncate(len);
    data
}

/// The paths of `entries` in their root, sorted.
fn paths(entries: &[pb::FileEntry]) -> Vec<String> {
    let mut paths: Vec<String> = entries
        .iter()
        .filter_map(|entry| entry.location.as_ref())
        .map(|location| location.path.clone())
        .collect();
    paths.sort();
    paths
}

/// The machine an entry says it is on.
fn machine_of(entry: &pb::FileEntry) -> Option<&str> {
    let machine = entry.location.as_ref()?.machine.as_ref()?;
    Some(machine.value.as_str())
}

/// The machine a failure says is offline, when that is what it says.
fn offline_machine(status: &Status) -> Option<String> {
    match error_of(status).kind? {
        pb::error::Kind::MachineOffline(offline) => Some(offline.machine_name),
        _ => None,
    }
}

/// Whether `status` is the refusal of a path that leaves its root.
fn leaves_its_root(status: &Status) -> bool {
    status.code() == Code::PermissionDenied
        && matches!(error_of(status).kind, Some(pb::error::Kind::OutsideHumataFolders(_)))
}

/// The names in `folder`, which may not be there.
fn names_in(folder: &Path) -> BTreeSet<String> {
    std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

// FM-9, FM-10. Fails if listing a folder with a machine named shows the
// host's own files, misses the machine's, or gives entries that don't say
// they are on that machine.
#[tokio::test(flavor = "multi_thread")]
async fn listing_a_folder_on_a_machine_shows_that_machines_files_named_on_that_machine() {
    let rig = Rig::start(&[NAME], &[]).await;
    let on_machine = rig.the(NAME).shared();
    std::fs::write(on_machine.join("on-the-machine.txt"), b"hi").unwrap();
    std::fs::create_dir(on_machine.join("docs")).unwrap();
    std::fs::write(rig.host_shared().join("on-the-mac.txt"), b"hi").unwrap();

    let entries = list(&rig, there("")).await.expect("the machine's folder is listed");

    assert_eq!(paths(&entries), ["docs", "on-the-machine.txt"]);
    for entry in &entries {
        assert_eq!(machine_of(entry), Some(NAME), "{entry:?}");
    }
}

// FM-9, D165. Fails if a new folder or a rename asked for on a machine
// happens on the host's disk, doesn't happen on the machine's, or answers
// with an entry that isn't the new one on that machine.
#[tokio::test(flavor = "multi_thread")]
async fn a_new_folder_and_a_rename_happen_on_the_machines_disk_and_not_the_hosts() {
    let rig = Rig::start(&[NAME], &[]).await;
    let on_machine = rig.the(NAME).shared();
    let mut files = files(&rig).await;

    let made = files
        .create_folder(pb::CreateFolderRequest {
            idempotency_key: None,
            location: Some(there("made")),
        })
        .await
        .expect("the folder is made")
        .into_inner();

    assert!(on_machine.join("made").is_dir(), "the folder isn't on the machine's disk");
    assert_eq!(machine_of(&made), Some(NAME), "{made:?}");

    let renamed = files
        .rename(pb::RenameRequest {
            idempotency_key: None,
            location: Some(there("made")),
            new_name: "renamed".to_owned(),
        })
        .await
        .expect("the folder is renamed")
        .into_inner();

    assert!(on_machine.join("renamed").is_dir(), "the new name isn't on the machine's disk");
    assert!(!on_machine.join("made").exists(), "the old name is still on the machine's disk");
    assert_eq!(paths(std::slice::from_ref(&renamed)), ["renamed"]);
    assert_eq!(machine_of(&renamed), Some(NAME), "{renamed:?}");
    assert_eq!(names_in(&rig.host_shared()), BTreeSet::new(), "the host's own folder changed");
}

// FM-3 on another machine, D165. Fails if a file deleted on a machine isn't
// taken out of its folder into that machine's trash (and not the host's),
// if that machine's trash listed through the host doesn't show it under the
// id the delete gave and on that machine, or if restoring by that id doesn't
// put the same bytes back on the machine.
#[tokio::test(flavor = "multi_thread")]
async fn a_deleted_file_is_in_the_machines_trash_and_restoring_it_brings_it_back_there() {
    let rig = Rig::start(&[NAME], &[]).await;
    let remote = rig.the(NAME);
    std::fs::write(remote.shared().join("doomed.txt"), b"keep me").unwrap();
    let (its_trash, host_trash) = (names_in(&remote.trash()), names_in(&rig.host_trash()));

    let moved = files(&rig)
        .await
        .move_to_trash(pb::MoveToTrashRequest {
            idempotency_key: None,
            locations: vec![there("doomed.txt")],
        })
        .await
        .expect("the file is deleted")
        .into_inner();

    assert!(!remote.shared().join("doomed.txt").exists(), "the file is still in its folder");
    assert_ne!(names_in(&remote.trash()), its_trash, "nothing went to the machine's trash");
    assert_eq!(names_in(&rig.host_trash()), host_trash, "the host's trash changed");
    assert_eq!(moved.items.len(), 1, "the delete names the one item it made");

    let listed = trash(&rig)
        .await
        .list_trash(pb::ListTrashRequest {
            machine: Some(pb::MachineId {
                value: NAME.to_owned(),
            }),
            page: None,
        })
        .await
        .expect("the machine's trash is listed")
        .into_inner();

    assert_eq!(listed.items.len(), 1, "{listed:?}");
    let item = &listed.items[0];
    assert_eq!(item.machine.as_ref().map(|machine| machine.value.as_str()), Some(NAME));
    assert_eq!(item.id.as_ref(), moved.items.first(), "the listing and the delete give one id");

    trash(&rig)
        .await
        .restore_from_trash(pb::RestoreFromTrashRequest {
            idempotency_key: None,
            id: item.id.clone(),
        })
        .await
        .expect("the item is restored");

    assert_eq!(std::fs::read(remote.shared().join("doomed.txt")).unwrap(), b"keep me");
    assert_eq!(names_in(&remote.trash()), its_trash, "the item is still in the machine's trash");
}

// FM-9, TR-11. Fails if a file written to a machine in several chunks isn't
// on that machine's disk byte for byte, or reads back through the host as
// other bytes, out of order, or without the whole file's checksum at its
// end.
#[tokio::test(flavor = "multi_thread")]
async fn a_file_written_to_a_machine_reads_back_with_the_same_bytes() {
    use pb::write_file_message::Message as Part;

    let rig = Rig::start(&[NAME], &[]).await;
    let data = noise(7, 2 * CHUNK_MAX + 12_345);
    let mut messages = vec![pb::WriteFileMessage {
        message: Some(Part::Header(pb::WriteFileHeader {
            idempotency_key: None,
            location: Some(there("big.bin")),
            size: data.len() as u64,
            sha256: sha256(&data),
            replace: false,
            expected_sha256: None,
        })),
    }];
    let mut encoder = Encoder::new(Codec::None);
    let pieces: Vec<&[u8]> = data.chunks(CHUNK_MAX).collect();
    for (index, piece) in pieces.iter().enumerate() {
        let chunk = encoder.encode(piece.to_vec()).unwrap();
        messages.push(pb::WriteFileMessage {
            message: Some(Part::Chunk(pb::FileChunk {
                offset: (index * CHUNK_MAX) as u64,
                data: chunk.data,
                last: index + 1 == pieces.len(),
                codec: chunk.header.codec.wire(),
                raw_size: chunk.header.raw_len,
                crc32: chunk.header.crc32,
                ..Default::default()
            })),
        });
    }

    let entry = files(&rig)
        .await
        .write_file(tokio_stream::iter(messages))
        .await
        .expect("the file is written")
        .into_inner();

    assert_eq!(machine_of(&entry), Some(NAME), "{entry:?}");
    assert!(
        std::fs::read(rig.the(NAME).shared().join("big.bin")).unwrap() == data,
        "the file on the machine's disk isn't what was written"
    );
    assert_eq!(names_in(&rig.host_shared()), BTreeSet::new(), "the host's own folder changed");

    let mut chunks = files(&rig)
        .await
        .read_file(pb::ReadFileRequest {
            location: Some(there("big.bin")),
            ..Default::default()
        })
        .await
        .expect("the file is read")
        .into_inner();
    let mut decoder = Decoder::new();
    let mut read = Vec::new();
    let mut count = 0;
    let mut checksum = None;
    while let Some(chunk) = chunks.message().await.expect("a chunk of the file") {
        assert_eq!(chunk.offset, read.len() as u64, "a chunk skipped or repeated bytes");
        let codec = Codec::from_wire(chunk.codec).expect("a codec this build knows");
        let raw_len = match (codec, chunk.raw_size) {
            (Codec::None, 0) => chunk.data.len() as u32,
            (_, raw_size) => raw_size,
        };
        let header = Header {
            codec,
            raw_len,
            crc32: chunk.crc32,
        };
        read.extend_from_slice(&decoder.decode(header, chunk.data).expect("a chunk that decodes"));
        count += 1;
        if chunk.last {
            checksum = Some(chunk.sha256);
            break;
        }
    }

    assert!(read == data, "the bytes read back aren't the bytes written");
    assert!(count > 2, "the file came back in {count} chunks, not several");
    assert_eq!(checksum, Some(sha256(&data)), "the last chunk carries the file's checksum");
}

// D165: the host judges, then forwards. Fails if a location that leaves its
// root, named on a machine, isn't refused by the host itself: with the
// machine's link down nothing can be sent to it, so any answer other than
// that refusal (machine-offline, say) means the host wanted the machine
// before it had judged the path.
#[tokio::test(flavor = "multi_thread")]
async fn a_location_that_leaves_its_root_is_refused_by_the_host_before_anything_is_sent() {
    let rig = Rig::start(&[NAME], &[]).await;
    let channel_folder = rig.the(NAME).shared().join("..");
    let before = names_in(&channel_folder);
    rig.link_lost(NAME).await;

    let listed = list(&rig, there("../..")).await.expect_err("a listing above the root");
    let made = files(&rig)
        .await
        .create_folder(pb::CreateFolderRequest {
            idempotency_key: None,
            location: Some(there("../escape")),
        })
        .await
        .expect_err("a folder made beside the root");

    assert!(leaves_its_root(&listed), "the listing was answered with {listed:?}");
    assert!(leaves_its_root(&made), "the new folder was answered with {made:?}");
    assert_eq!(names_in(&channel_folder), before, "the machine's disk changed");
}

// FM-12, HO-5, D165. Fails if a call for a machine whose link is down is
// answered with anything but machine-offline naming that machine (it hangs,
// fails some other way, or names another machine), or if the same call
// doesn't work again once the host shows the link back.
#[tokio::test(flavor = "multi_thread")]
async fn with_the_link_down_a_listing_names_the_machine_offline_and_works_again_when_it_is_back() {
    let rig = Rig::start(&[NAME], &[]).await;
    std::fs::write(rig.the(NAME).shared().join("a.txt"), b"a").unwrap();
    rig.link_lost(NAME).await;

    let failed = list(&rig, there("")).await.expect_err("a listing on a machine that is offline");

    assert_eq!(offline_machine(&failed).as_deref(), Some(NAME), "{failed:?}");

    rig.link_back(NAME).await;
    let entries = list(&rig, there("")).await.expect("the listing works once the link is back");

    assert_eq!(paths(&entries), ["a.txt"]);
}

// D165: one request moves files to one machine's trash. Fails if a delete
// naming files on two machines (here the machine and the host) isn't refused
// as a bad request naming the second location, or removes either file.
#[tokio::test(flavor = "multi_thread")]
async fn one_delete_naming_two_machines_is_refused() {
    let rig = Rig::start(&[NAME], &[]).await;
    let on_machine = rig.the(NAME).shared().join("a.txt");
    let on_host = rig.host_shared().join("b.txt");
    std::fs::write(&on_machine, b"a").unwrap();
    std::fs::write(&on_host, b"b").unwrap();

    let refused = files(&rig)
        .await
        .move_to_trash(pb::MoveToTrashRequest {
            idempotency_key: None,
            locations: vec![there("a.txt"), shared(None, "b.txt")],
        })
        .await
        .expect_err("a delete on two machines");

    let (field, problem) = invalid_field(&refused);
    assert_eq!(problem, pb::invalid_field::Problem::NotAllowedHere, "{refused:?}");
    assert!(field.starts_with("locations[1]"), "the refusal names {field}, not the second file");
    assert!(on_machine.exists(), "the machine's file was removed");
    assert!(on_host.exists(), "the host's file was removed");
}

/// The host daemon, the machines behind its `ssh`, and the test's hold on
/// their links. `remote_transfers.rs` and `remote_files.rs` carry the same
/// text, because each test file is a crate of its own; harness-needs.md
/// has the move to one file under `tests/common/`.
///
/// A machine here is three things the test owns: a fake machine (a home
/// folder and a fake `ssh` that runs commands in it), the machine's own
/// `humatad2` in the machine role (the release binary, started by the test,
/// since a link only relays to the machine's daemon and serves no files
/// itself), and a control folder for its link. The host is the admin
/// daemon; the only `ssh` on its PATH is this rig's, which hands each call
/// to the right machine.
mod rig {
    #![allow(dead_code)]

    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use humata2_fake_machine::FakeMachine;
    use humata2_grpc::humata::v1 as pb;
    use humata2_store::{ChannelRecord, Store};
    use rustix::process::{Pid, Signal, kill_process_group};
    use sha2::{Digest, Sha256};
    use tokio::sync::Mutex;
    use tonic::Streaming;
    use tonic::transport::Channel;

    use crate::common::{Build, Daemon, binary, folder, state_folder};

    /// The one channel, known to the host and to every machine.
    pub const CHANNEL: &str = "ch_remote";
    const CHANNEL_FOLDER: &str = "remote-channel";

    /// The most a test waits for something the host does on its own. It is a
    /// safety net that fails the test saying what it waited for, never a
    /// pace: every wait ends on a pushed change. The slowest thing waited
    /// for is a link coming back: the host dials again 5 s after a link
    /// ends, then 10, 20 and at most 30 s after each dial that fails
    /// (`peer_client.rs`), and a link's start is a daemon's start (15 s at
    /// most, `common::WAIT`). This is more than twice the two together.
    pub const LONG: Duration = Duration::from_secs(120);

    const FILES: [&str; 4] = ["humatad2", "humata2_holder", "humata", "stack"];

    /// The gate is a perl script until the harness has a helper of its own
    /// (harness-needs.md).
    const PERL: &str = "/usr/bin/perl";

    /// `humatad2` as a machine gets it from the tools bundle.
    const WRAPPER: &str = r##"#!/bin/sh
# humatad2 on one of this test's machines: the real one. A link is told the
# state folder and socket of the machine's own daemon, which the test
# started in the folder `~/.humata2` points at. The folder is named by its
# short, real path, because a socket's path must be short.
case " $* " in *" --stdio "*)
  at=$(cd -P "$HOME/.humata2" && pwd -P) || exit 70
  exec '__REAL__' "$@" --state "$at/state" --socket "$at/d.sock" ;;
esac
exec '__REAL__' "$@"
"##;

    /// The host's `ssh`.
    const SSH: &str = r##"#!/bin/sh
# The host's `ssh` in this test: each call goes to the fake machine whose
# host name it names. A link command (`serve --stdio`) runs in a process
# group of its own and leaves the group's id in the machine's control
# folder, so the test can end that one link and nothing else; a gated
# machine's link runs through the gate.
name=
for arg in "$@"; do
  case "$arg" in
__MACHINES__  esac
done
if [ -z "$name" ]; then
  echo "ssh: none of this test's machines is named in: $*" >&2
  exit 255
fi
control='__CONTROL__'/$name
for last in "$@"; do :; done
case "$last" in
  *'serve --stdio'*)
    echo $$ > "$control/link.new" && mv "$control/link.new" "$control/link"
    if [ -e "$control/gated" ]; then
      exec '__SETSID__' /usr/bin/perl '__GATE__' "$control" "$ssh" "$@"
    fi
    exec '__SETSID__' "$ssh" "$@" ;;
esac
exec "$ssh" "$@"
"##;

    /// The gate a gated machine's link runs through.
    const GATE: &str = r##"# The transport of a gated machine's link. It runs the real link command
# and carries its bytes, one process for each way so neither waits on the
# other. While `limit.in` (host to machine) or `limit.out` (machine to
# host) is in the control folder, that way lets through as many more bytes
# as the file says, counted from when the file appeared, then holds the
# rest until the file is removed. The count is exact to one read (64 KiB):
# a read already waiting when the file appears isn't counted.
use strict;
use warnings;
use IPC::Open2;

my ($ctl, @command) = @ARGV;
$SIG{PIPE} = 'IGNORE';

sub limit_of {
    open(my $file, '<', $_[0]) or return undef;
    my $line = <$file>;
    close($file);
    return defined $line && $line =~ /^(\d+)/ ? $1 : undef;
}

# Copies `$src` to `$dst` until `$src` ends, `$dst` is gone, or, while
# held, `$alive` says to stop.
sub pump {
    my ($src, $dst, $limit_file, $alive) = @_;
    binmode($src);
    binmode($dst);
    my $passed = 0;
    while (1) {
        my $limit = limit_of($limit_file);
        $passed = 0 unless defined $limit;
        my $room = defined $limit ? $limit - $passed : 65536;
        if ($room <= 0) {
            # Held. Looked at again shortly: removing the limit releases it.
            return unless $alive->();
            select(undef, undef, undef, 0.02);
            next;
        }
        $room = 65536 if $room > 65536;
        my $got = sysread($src, my $bytes, $room);
        last unless $got;
        my $sent = 0;
        while ($sent < $got) {
            my $wrote = syswrite($dst, $bytes, $got - $sent, $sent);
            return unless defined $wrote;
            $sent += $wrote;
        }
        $passed += $got if defined $limit;
    }
    close($dst);
}

my $link = open2(my $from, my $to, @command);
my $parent = $$;
my $inward = fork();
die "the link gate couldn't fork: $!" unless defined $inward;
if ($inward == 0) {
    close($from);
    # Not closed: a closed descriptor would be handed to the next file opened.
    open(STDOUT, '>', '/dev/null');
    pump(\*STDIN, $to, "$ctl/limit.in", sub { getppid() == $parent });
    exit(0);
}
close($to);
open(STDIN, '<', '/dev/null');
pump($from, \*STDOUT, "$ctl/limit.out", sub { 1 });
# The link command's output ended: the link is over.
kill('KILL', $inward);
waitpid($link, 0);
exit($? >> 8);
"##;

    fn executable(path: &Path, text: &str) {
        std::fs::write(path, text).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A tools bundle for linux-x86_64, the fake machine's platform, whose
    /// humatad2 is [`WRAPPER`] around the real binary.
    fn tools(dir: &Path) -> PathBuf {
        let root = dir.join("tools");
        std::fs::create_dir_all(root.join("linux-x86_64")).unwrap();
        let mut entries = serde_json::Map::new();
        for file in FILES {
            let body = if file == "humatad2" {
                let real = binary("HUMATAD2_RELEASE").display().to_string();
                WRAPPER.replace("__REAL__", &real)
            } else {
                format!("#!/bin/sh\n# {file}\nexit 0\n")
            };
            std::fs::write(root.join("linux-x86_64").join(file), &body).unwrap();
            let sha256: String = Sha256::digest(body.as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            entries.insert(
                file.to_owned(),
                serde_json::json!({ "path": format!("linux-x86_64/{file}"), "sha256": sha256 }),
            );
        }
        let manifest = serde_json::json!({ "platforms": { "linux-x86_64": entries } });
        std::fs::write(root.join("manifest.json"), manifest.to_string()).unwrap();
        root
    }

    /// Writes the channel into the store under `state`, before the daemon
    /// that owns the store starts. The host judges a location by its own
    /// record of the channel and the machine resolves it by its own, so
    /// both get the same one.
    fn seed_channel(state: &Path) {
        let store = Store::open(state.join("store")).unwrap();
        store
            .put_channel(&ChannelRecord {
                id: CHANNEL.to_owned(),
                name: "remote files".to_owned(),
                folder: CHANNEL_FOLDER.to_owned(),
                branch_prefix: None,
                default_agent: "zsh".to_owned(),
                repos: Vec::new(),
                machines: Vec::new(),
                revision: 1,
                created_unix_ms: 0,
            })
            .unwrap();
    }

    /// `path` in the channel's shared folder on `machine` (none: the host).
    pub fn shared(machine: Option<&str>, path: &str) -> pb::FileLocation {
        pb::FileLocation {
            machine: machine.map(|name| pb::MachineId {
                value: name.to_owned(),
            }),
            root: Some(pb::file_location::Root::ChannelShared(pb::ChannelShared {
                channel: Some(pb::ChannelId {
                    value: CHANNEL.to_owned(),
                }),
            })),
            path: path.to_owned(),
        }
    }

    /// A direction of a link's bytes.
    #[derive(Clone, Copy, Debug)]
    pub enum Way {
        /// From the host to the machine: what a copy to the machine sends.
        ToMachine,
        /// From the machine to the host: what a copy from the machine sends.
        ToHost,
    }

    impl Way {
        /// The gate's file for this way, in the machine's control folder.
        fn limit(self) -> &'static str {
            match self {
                Way::ToMachine => "limit.in",
                Way::ToHost => "limit.out",
            }
        }
    }

    /// One machine. Fields drop in order: its daemon ends before its
    /// folders go.
    pub struct Remote {
        pub name: &'static str,
        /// Where this rig's `ssh` and gate look for what to do with this
        /// machine's link, and where they leave the link's process group.
        control: PathBuf,
        /// The machine's own humatad2: what its link relays to.
        daemon: Daemon,
        machine: FakeMachine,
    }

    impl Remote {
        async fn start(name: &'static str, links: &Path, gated: bool) -> Self {
            let machine = FakeMachine::new();
            // The machine's `~/.humata2` is its daemon's folder: the add
            // writes the enrollment key into the state folder the daemon
            // uses, and the link finds the daemon's socket there.
            let dir = folder();
            seed_channel(&state_folder(dir.path()));
            std::os::unix::fs::symlink(dir.path(), machine.home().join(".humata2")).unwrap();
            let channels = machine.home().join("Humata");
            std::fs::create_dir_all(channels.join(CHANNEL_FOLDER).join("shared")).unwrap();
            let control = links.join(name);
            std::fs::create_dir_all(&control).unwrap();
            if gated {
                std::fs::write(control.join("gated"), "").unwrap();
            }
            let channels = channels.display().to_string();
            let home = machine.home().display().to_string();
            let daemon = Daemon::start_with_env(
                Build::Release,
                dir,
                &["--role", "machine", "--channels", channels.as_str()],
                &[("HOME", home.as_str())],
            )
            .await;
            Self {
                name,
                control,
                daemon,
                machine,
            }
        }

        /// The channel's shared folder on this machine's disk.
        pub fn shared(&self) -> PathBuf {
            self.machine.home().join("Humata").join(CHANNEL_FOLDER).join("shared")
        }

        /// This machine's Humata trash, on its disk.
        pub fn trash(&self) -> PathBuf {
            self.machine.home().join("Humata").join(".trash")
        }

        /// `path` in the channel's shared folder on this machine.
        pub fn at(&self, path: &str) -> pb::FileLocation {
            shared(Some(self.name), path)
        }

        /// From now on this machine's link carries `bytes` more that `way`
        /// (to within one 64 KiB read), then holds the rest until
        /// [`Self::release`]. Only a gated machine's link can be held.
        pub fn hold_after(&self, way: Way, bytes: u64) {
            assert!(
                self.control.join("gated").exists(),
                "{}'s link has no gate; start the rig with it among the gated machines",
                self.name
            );
            // Renamed into place, so the gate reads all of it or none.
            let new = self.control.join(format!("{}.new", way.limit()));
            std::fs::write(&new, bytes.to_string()).unwrap();
            std::fs::rename(&new, self.control.join(way.limit())).unwrap();
        }

        /// Lets the bytes held by [`Self::hold_after`] go on.
        pub fn release(&self, way: Way) {
            let limit = self.control.join(way.limit());
            std::fs::remove_file(&limit).unwrap_or_else(|error| {
                panic!("{}'s link wasn't held {way:?} ({}): {error}", self.name, limit.display())
            });
        }

        /// Ends this machine's link at once, as a dropped connection does:
        /// every process of it, and nothing else. The link command runs in
        /// a process group of its own, whose id this rig's `ssh` left in
        /// the control folder when the host dialled.
        fn cut(&self) {
            let file = self.control.join("link");
            let group = std::fs::read_to_string(&file)
                .ok()
                .and_then(|text| text.trim().parse::<i32>().ok())
                .and_then(Pid::from_raw)
                .unwrap_or_else(|| {
                    panic!(
                        "{}'s link left no process group in {}; is its link up?",
                        self.name,
                        file.display()
                    )
                });
            kill_process_group(group, Signal::KILL).unwrap_or_else(|error| {
                panic!("couldn't end {}'s link (process group {group:?}): {error}", self.name)
            });
            // The group is gone: the next cut must find the next link's.
            let _ = std::fs::remove_file(&file);
            // A hold was on the link that just ended; the next one runs free.
            for way in [Way::ToMachine, Way::ToHost] {
                let _ = std::fs::remove_file(self.control.join(way.limit()));
            }
        }
    }

    /// The host and its machines, each added and linked. Fields drop in
    /// order: the host first, so its links end before the machines do.
    pub struct Rig {
        /// The admin host daemon.
        pub host: Daemon,
        /// The host's machine events from before any machine was added:
        /// what a wait on a machine's reachability wakes on.
        feed: Mutex<Streaming<pb::Event>>,
        boxes: Vec<Remote>,
    }

    impl Rig {
        /// A host with one machine per name, added and linked. The links of
        /// the `gated` ones run through the gate, so a test can hold them
        /// at a byte count ([`Remote::hold_after`]).
        pub async fn start(plain: &[&'static str], gated: &[&'static str]) -> Self {
            assert!(
                gated.is_empty() || Path::new(PERL).exists(),
                "a gated link needs {PERL}, which isn't on this machine (harness-needs.md has \
                 the helper that replaces it)"
            );
            let dir = folder();
            let links = dir.path().join("links");
            let mut boxes = Vec::new();
            for (names, with_gate) in [(plain, false), (gated, true)] {
                for &name in names {
                    boxes.push(Remote::start(name, &links, with_gate).await);
                }
            }
            let gate = dir.path().join("gate.pl");
            std::fs::write(&gate, GATE).unwrap();
            let machines: String = boxes
                .iter()
                .map(|remote| {
                    format!(
                        "    '{name}.internal') name='{name}'; ssh='{}/ssh' ;;\n",
                        remote.machine.ssh_dir().display(),
                        name = remote.name
                    )
                })
                .collect();
            let bin = dir.path().join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            executable(
                &bin.join("ssh"),
                &SSH.replace("__MACHINES__", &machines)
                    .replace("__CONTROL__", &links.display().to_string())
                    .replace("__SETSID__", &binary("HUMATA2_SETSID").display().to_string())
                    .replace("__GATE__", &gate.display().to_string()),
            );
            seed_channel(&state_folder(dir.path()));
            // The host's channels are in `Humata` in its root, the default.
            let shared = dir.path().join("Humata").join(CHANNEL_FOLDER).join("shared");
            std::fs::create_dir_all(shared).unwrap();
            // Nothing of the real home folder: no skills of the person
            // running the tests are read, or sent to a machine.
            let skills = dir.path().join("skills-home");
            std::fs::create_dir_all(&skills).unwrap();
            let extra = [
                "--tools".to_owned(),
                tools(dir.path()).display().to_string(),
                "--ssh-config".to_owned(),
                dir.path().join("ssh_config").display().to_string(),
                "--skills-home".to_owned(),
                skills.display().to_string(),
            ];
            let extra: Vec<&str> = extra.iter().map(String::as_str).collect();
            // The daemon's own PATH, given to it alone: this rig's `ssh`
            // first, and nothing of the test process's environment changes.
            let path = format!("{}:/usr/bin:/bin", bin.display());
            let host =
                Daemon::start_with_env(Build::Admin, dir, &extra, &[("PATH", path.as_str())]).await;
            let feed = pb::event_service_client::EventServiceClient::new(host.channel().await)
                .subscribe(pb::SubscribeRequest {
                    topics: vec![pb::Topic::Machines as i32],
                    ..Default::default()
                })
                .await
                .expect("the host's event feed opens")
                .into_inner();
            let rig = Self {
                host,
                feed: Mutex::new(feed),
                boxes,
            };
            for remote in &rig.boxes {
                rig.add(remote.name).await;
            }
            rig
        }

        pub fn the(&self, name: &str) -> &Remote {
            self.boxes
                .iter()
                .find(|remote| remote.name == name)
                .unwrap_or_else(|| panic!("this rig has no machine {name}"))
        }

        /// The channel's shared folder on the host's disk.
        pub fn host_shared(&self) -> PathBuf {
            self.host.dir.path().join("Humata").join(CHANNEL_FOLDER).join("shared")
        }

        /// The host's own Humata trash, on its disk.
        pub fn host_trash(&self) -> PathBuf {
            self.host.dir.path().join("Humata").join(".trash")
        }

        /// A connection to the host's socket.
        pub async fn channel(&self) -> Channel {
            self.host.channel().await
        }

        async fn machines(&self) -> pb::machine_service_client::MachineServiceClient<Channel> {
            pb::machine_service_client::MachineServiceClient::new(self.channel().await)
        }

        /// Adds `name`, confirming the host key it presents, and waits for
        /// its link.
        async fn add(&self, name: &str) {
            let mut machines = self.machines().await;
            let request = |step: &str, fingerprint: &str| pb::AddSshMachineRequest {
                idempotency_key: Some(pb::IdempotencyKey {
                    value: format!("add-{name}-{step}"),
                }),
                ssh_host: format!("me@{name}.internal"),
                name: name.to_owned(),
                root: String::new(),
                trust_host_key_fingerprint: fingerprint.to_owned(),
            };
            let first = machines
                .add_ssh_machine(request("ask", ""))
                .await
                .expect("the add starts")
                .into_inner();
            let asked = self.ended(&first).await;
            let Some(pb::error::Kind::HostKeyUnknown(key)) =
                asked.error.clone().and_then(|error| error.kind)
            else {
                panic!("the first add of {name} should stop at its host key; it ended {asked:?}");
            };
            let added = machines
                .add_ssh_machine(request("trust", &key.presented_fingerprint))
                .await
                .expect("the confirmed add starts")
                .into_inner();
            let ended = self.ended(&added).await;
            assert_eq!(
                ended.state,
                pb::OperationState::Succeeded as i32,
                "adding {name} failed: {:?}",
                ended.error
            );
            self.linked(name).await;
        }

        /// The operation as its watch last showed it, once the watch ends.
        async fn ended(&self, operation: &pb::Operation) -> pb::Operation {
            let mut watch =
                pb::operation_service_client::OperationServiceClient::new(self.channel().await)
                    .watch_operation(pb::WatchOperationRequest {
                        id: operation.id.clone(),
                        from_version: 0,
                    })
                    .await
                    .expect("the operation can be watched")
                    .into_inner();
            let deadline = tokio::time::Instant::now() + LONG;
            let mut last = operation.clone();
            loop {
                match tokio::time::timeout_at(deadline, watch.message()).await {
                    Ok(Ok(Some(next))) => last = next,
                    Ok(Ok(None)) => return last,
                    Ok(Err(status)) => {
                        panic!("the operation's watch failed ({status:?}); it last was {last:?}")
                    }
                    Err(_) => {
                        panic!("the operation never ended within {LONG:?}; it last was {last:?}")
                    }
                }
            }
        }

        /// The machine `name` once `reached` holds for it. Nothing sleeps:
        /// the machine is read, and read again each time the feed tells of a
        /// machine. The feed was opened before the first machine was added,
        /// so no change falls between a read and the next event.
        async fn machine_until(
            &self,
            name: &str,
            what: &str,
            reached: impl Fn(&pb::Machine) -> bool,
        ) -> pb::Machine {
            let mut feed = self.feed.lock().await;
            let deadline = tokio::time::Instant::now() + LONG;
            loop {
                let now = self
                    .machines()
                    .await
                    .list_machines(pb::ListMachinesRequest::default())
                    .await
                    .expect("the host lists its machines")
                    .into_inner()
                    .machines
                    .into_iter()
                    .find(|machine| machine.name == name);
                if let Some(machine) = now.as_ref().filter(|machine| reached(machine)) {
                    return machine.clone();
                }
                match tokio::time::timeout_at(deadline, feed.message()).await {
                    Ok(Ok(Some(_))) => {}
                    Ok(ended) => panic!(
                        "the host's event feed ended ({ended:?}) while {name} wasn't {what} yet; \
                         the host last showed {now:?}"
                    ),
                    Err(_) => panic!(
                        "{name} was never {what} within {LONG:?}; the host last showed {now:?}"
                    ),
                }
            }
        }

        /// Waits until the host shows `name` online with its link up. ONLINE
        /// alone isn't the link: an add shows it once ssh worked, before the
        /// link is dialled. The daemon version is what the link's own
        /// handshake said, and the host takes the link into use before it
        /// shows either.
        pub async fn linked(&self, name: &str) -> pb::Machine {
            self.machine_until(name, "online with its link up", |machine| {
                machine.reachability == pb::Reachability::Online as i32
                    && !machine.daemon_version.is_empty()
            })
            .await
        }

        /// Ends `name`'s link as a dropped connection does and keeps it
        /// away; returns once the host shows the machine offline. The host
        /// dials again by itself after its backoff. Each dial then waits in
        /// the fake `ssh` (`hang`) until [`Self::link_back`], so the machine
        /// stays away for as long as the test needs, however slow the test
        /// is.
        pub async fn link_lost(&self, name: &str) -> pb::Machine {
            let remote = self.the(name);
            remote.machine.set("hang");
            remote.cut();
            self.machine_until(name, "offline", |machine| {
                machine.reachability == pb::Reachability::Offline as i32
            })
            .await
        }

        /// Lets the host's dials reach `name` again, and returns once the
        /// host shows its link up. No reconnect is asked for: a reconnect
        /// shows ONLINE before its link is up, and the dial the host makes
        /// by itself is what the product promises.
        pub async fn link_back(&self, name: &str) -> pb::Machine {
            self.the(name).machine.clear("hang");
            self.linked(name).await
        }
    }
}
