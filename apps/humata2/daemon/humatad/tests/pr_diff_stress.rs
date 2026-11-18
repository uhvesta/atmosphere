//! DV-9: a pull request's diff holds up at size, through a real `humatad2`
//! and the fake `gh` of `common/pr_github.rs`, with no network. acme/stress#7
//! changes 300 files against main: every line of a 20,000-line file, a
//! 200-character path, a rename, a binary, a mode change and 295 small
//! files. Its review's Diff lists all of them against the pull request's
//! base, and the big file comes in pages to its last line.

#[allow(dead_code)]
mod common;
#[allow(dead_code)]
#[path = "common/mcp_client.rs"]
mod mcp_client;
#[allow(dead_code)]
#[path = "common/pr_github.rs"]
mod pr_github;

use std::fmt::Write as _;

use common::repo_ref;
use humata2_grpc::humata::v1 as pb;
use pr_github::{FileContents, Fixture, Pr, open_pr_from_files};

const OWNER: &str = "acme";
const NAME: &str = "stress";
const BIG: &str = "big.txt";
const BIG_LINES: u32 = 20_000;
const SMALL_FILES: u32 = 295;
const RENAMED_FROM: &str = "notes/old_name.md";
const RENAMED_TO: &str = "docs/new_name.md";
const BINARY: &str = "assets/logo.bin";
const MODE: &str = "scripts/run.sh";
/// Pages a file of this size may take before the test calls it stuck.
const MAX_PAGES: usize = 200;

/// A path of exactly 200 characters, in folders, ending in a file name.
fn long_path() -> String {
    let mut path = "a_folder_with_a_rather_long_name/".repeat(4);
    path.push_str("and_a_file_name_that_is_long_too_");
    while path.len() < 197 {
        path.push('x');
    }
    path.push_str(".rs");
    path
}

fn big_file(changed: bool) -> FileContents {
    let suffix = if changed { ", changed" } else { "" };
    let mut text = String::with_capacity(BIG_LINES as usize * 24);
    for line in 1..=BIG_LINES {
        writeln!(text, "line {line}{suffix}").unwrap();
    }
    FileContents::text(text)
}

fn small_path(at: u32) -> String {
    format!("src/file_{at:03}.rs")
}

fn binary() -> FileContents {
    let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    bytes.extend((0..64_u8).map(|byte| byte.wrapping_mul(37)));
    bytes.extend([0_u8; 16]);
    FileContents::bytes(bytes)
}

fn script() -> FileContents {
    FileContents::text("#!/bin/sh\necho run\n")
}

/// Opens acme/stress#7 on the fixture's world and tells `gh` of it beside
/// the fixture's own pull requests; returns its index in `fixture.prs`.
fn open_stress_pr(fixture: &mut Fixture) -> usize {
    let base = vec![
        (BIG.to_owned(), big_file(false)),
        (
            RENAMED_FROM.to_owned(),
            FileContents::text("These notes move.\nThey keep every word they had.\n"),
        ),
        (MODE.to_owned(), script()),
    ];
    let mut head = vec![
        (BIG.to_owned(), Some(big_file(true))),
        (long_path(), Some(FileContents::text("pub fn deep() {}\n"))),
        (RENAMED_FROM.to_owned(), None),
        (
            RENAMED_TO.to_owned(),
            Some(FileContents::text("These notes move.\nThey keep every word they had.\n")),
        ),
        (BINARY.to_owned(), Some(binary())),
        (MODE.to_owned(), Some(script().with_executable(true))),
    ];
    head.extend((1..=SMALL_FILES).map(|at| {
        (small_path(at), Some(FileContents::text(format!("pub fn f{at}() -> u32 {{ {at} }}\n"))))
    }));
    let template = Pr {
        owner: OWNER,
        name: NAME,
        number: 7,
        title: "Change everything at once",
        author: "mira",
        base: "main",
        head_branch: "mira/everything",
        fork: false,
        state: "OPEN",
        work: std::path::PathBuf::new(),
    };
    let pr = open_pr_from_files(&fixture.world, template, &base, &head);
    fixture.gh.visible(OWNER, NAME);
    fixture.prs.push(pr);
    fixture.gh.publish_with_binaries(&fixture.prs);
    fixture.prs.len() - 1
}

fn pull_request_base() -> Option<pb::DiffBase> {
    Some(pb::DiffBase {
        base: Some(pb::diff_base::Base::PullRequestBase(pb::PullRequestBase {})),
    })
}

async fn diffs(fixture: &Fixture) -> pb::diff_service_client::DiffServiceClient<tonic::transport::Channel> {
    pb::diff_service_client::DiffServiceClient::new(fixture.daemon.channel().await)
}

fn file<'a>(repo: &'a pb::RepoDiff, path: &str) -> &'a pb::FileSummary {
    repo.files
        .iter()
        .find(|file| file.path == path)
        .unwrap_or_else(|| panic!("the Diff lists {path}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_huge_pull_request_lists_every_file_and_pages_its_big_file_to_the_end() {
    let mut fixture = Fixture::new().await;
    let stress = open_stress_pr(&mut fixture);
    let session = fixture.start_review(&[stress]).await;

    let listing = diffs(&fixture)
        .await
        .get_diff(pb::GetDiffRequest {
            session: session.id.clone(),
            base: pull_request_base(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    let repo = listing
        .repos
        .iter()
        .find(|repo| repo.repo == Some(repo_ref(OWNER, NAME)))
        .expect("the review's Diff has the pull request's repository");
    assert_eq!(repo.files.len(), 300, "every changed file is listed, and nothing else");

    let big = file(repo, BIG);
    assert_eq!(
        (big.change, big.added, big.removed),
        (pb::ChangeKind::Modified as i32, BIG_LINES, BIG_LINES),
        "{big:?}"
    );
    let long = file(repo, &long_path());
    assert_eq!(long.path.len(), 200);
    assert_eq!(long.change, pb::ChangeKind::Added as i32, "{long:?}");
    let renamed = file(repo, RENAMED_TO);
    assert_eq!(
        (renamed.change, renamed.old_path.as_str()),
        (pb::ChangeKind::Renamed as i32, RENAMED_FROM),
        "{renamed:?}"
    );
    assert!(
        repo.files.iter().all(|file| file.path != RENAMED_FROM),
        "a renamed file is listed once, under its new path"
    );
    let logo = file(repo, BINARY);
    assert!(logo.binary, "{logo:?}");
    let mode = file(repo, MODE);
    assert_eq!(
        (mode.change, mode.added, mode.removed),
        (pb::ChangeKind::Modified as i32, 0, 0),
        "a mode change is listed, with no lines changed: {mode:?}"
    );
    for at in 1..=SMALL_FILES {
        let small = file(repo, &small_path(at));
        assert_eq!((small.change, small.added), (pb::ChangeKind::Added as i32, 1), "{small:?}");
    }

    // The big file, page by page, with the trees the listing showed.
    let (mut removed, mut added) = (Vec::new(), Vec::new());
    let mut last_added = String::new();
    let mut from = None;
    let mut pages = 0;
    loop {
        pages += 1;
        assert!(pages <= MAX_PAGES, "the big file never reached its end in {MAX_PAGES} pages");
        let page = diffs(&fixture)
            .await
            .get_file_diff(pb::GetFileDiffRequest {
                session: session.id.clone(),
                base: pull_request_base(),
                file: Some(pb::RepoPath {
                    repo: Some(repo_ref(OWNER, NAME)),
                    path: BIG.to_owned(),
                }),
                trees: repo.trees.clone(),
                from,
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner();
        for line in page.hunks.iter().flat_map(|hunk| hunk.lines.iter()) {
            if line.kind == pb::diff_line::Kind::Removed as i32 {
                removed.push(line.old_line);
            } else if line.kind == pb::diff_line::Kind::Added as i32 {
                added.push(line.new_line);
                last_added.clone_from(&line.text);
            }
        }
        match page.next {
            Some(next) => from = Some(next),
            None => break,
        }
    }
    assert!(pages > 1, "a 20,000-line change comes in more than one page");
    let every: Vec<u32> = (1..=BIG_LINES).collect();
    assert!(removed == every, "each old line comes once, in order: {} of them", removed.len());
    assert!(added == every, "each new line comes once, in order: {} of them", added.len());
    assert_eq!(last_added, format!("line {BIG_LINES}, changed"));
}
