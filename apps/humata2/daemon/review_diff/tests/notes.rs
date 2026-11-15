//! Review plans and notes for the user (RV-32–RV-34, PR-23–PR-25) against
//! real repositories: a note goes outdated when its lines change, follows
//! them when only other lines move, and the switches the user turned off
//! deny the agent.

mod support;

use humata2_review_diff::{
    AgentSwitches, ChatError, ChatTool, DiffSpec, FileRef, Group, Note, NoteState, NoteTarget,
    RepoDiff, RepoSpec, ReviewChat, ReviewPlan, Side, ViewState,
};
use support::{Repo, diff};

fn file(path: &str) -> FileRef {
    FileRef {
        repo: "api".to_owned(),
        path: path.to_owned(),
    }
}

fn lines_note(path: &str, first: u32, last: u32) -> Note {
    Note {
        file: file(path),
        target: NoteTarget::Lines {
            first,
            last,
            side: Side::New,
        },
        text: "this is where the retry starts".to_owned(),
        anchor: None,
    }
}

fn numbered(lines: &[&str]) -> String {
    lines.iter().map(|line| format!("{line}\n")).collect()
}

fn anchored(repo: &Repo, shown: &RepoDiff, mut notes: Vec<Note>) -> Vec<Note> {
    let checkout = repo.checkout();
    checkout
        .anchor_notes_blocking(shown, &mut notes, |_| unreachable!("no hunk notes here"))
        .expect("anchor the notes");
    notes
}

fn states(repo: &Repo, shown: &RepoDiff, notes: &[Note]) -> Vec<Option<NoteState>> {
    repo.checkout()
        .note_states_blocking(shown, notes)
        .expect("note states")
}

/// a.rs committed, then edited so it is in the diff.
fn edited() -> Repo {
    let repo = Repo::new();
    repo.write("a.rs", "fn main() {}\n");
    repo.write("b.rs", "fn b() {}\n");
    repo.commit_all("base");
    repo.write("a.rs", &numbered(&["fn main() {", "    retry();", "    run();", "}"]));
    repo.write("b.rs", "fn b() { todo() }\n");
    repo
}

#[test]
fn a_note_goes_outdated_when_its_lines_change() {
    let repo = edited();
    let spec = RepoSpec::uncommitted("api");
    let notes = anchored(&repo, &diff(&repo, &spec), vec![lines_note("a.rs", 2, 2)]);
    assert_eq!(
        states(&repo, &diff(&repo, &spec), &notes),
        [Some(NoteState::Current { first: 2, last: 2 })]
    );
    repo.write("a.rs", &numbered(&["fn main() {", "    retry_twice();", "    run();", "}"]));
    assert_eq!(
        states(&repo, &diff(&repo, &spec), &notes),
        [Some(NoteState::Outdated)]
    );
}

#[test]
fn a_note_follows_its_lines_when_lines_are_added_above() {
    let repo = edited();
    let spec = RepoSpec::uncommitted("api");
    let notes = anchored(&repo, &diff(&repo, &spec), vec![lines_note("a.rs", 2, 3)]);
    repo.write(
        "a.rs",
        &numbered(&["// new header", "", "fn main() {", "    retry();", "    run();", "}"]),
    );
    assert_eq!(
        states(&repo, &diff(&repo, &spec), &notes),
        [Some(NoteState::Current { first: 4, last: 5 })]
    );
}

#[test]
fn a_note_on_a_file_that_left_the_diff_is_gone_and_a_file_note_stays() {
    let repo = edited();
    let spec = RepoSpec::uncommitted("api");
    let file_note = Note {
        target: NoteTarget::File,
        ..lines_note("b.rs", 1, 1)
    };
    let notes = anchored(&repo, &diff(&repo, &spec), vec![file_note, lines_note("a.rs", 1, 1)]);
    repo.git(&["checkout", "--", "a.rs"]);
    assert_eq!(
        states(&repo, &diff(&repo, &spec), &notes),
        [Some(NoteState::OnFile), Some(NoteState::Gone)]
    );
}

#[test]
fn a_note_past_the_end_of_its_file_is_refused() {
    let repo = edited();
    let shown = diff(&repo, &RepoSpec::uncommitted("api"));
    let mut notes = vec![lines_note("a.rs", 40, 41)];
    let result = repo
        .checkout()
        .anchor_notes_blocking(&shown, &mut notes, |_| unreachable!("no hunk notes"));
    assert!(result.is_err());
}

#[test]
fn a_hunk_note_anchors_to_the_hunk_that_covers_its_line() {
    let repo = edited();
    let spec = RepoSpec::uncommitted("api");
    let shown = diff(&repo, &spec);
    let mut notes = vec![Note {
        target: NoteTarget::Hunk {
            line: 2,
            side: Side::New,
        },
        ..lines_note("a.rs", 0, 0)
    }];
    repo.checkout()
        .anchor_notes_blocking(&shown, &mut notes, |file| {
            Ok(support::hunks(&repo, &shown, &file.path, &spec))
        })
        .expect("anchor the hunk note");
    let anchor = notes[0].anchor.clone().expect("anchored");
    assert!(anchor.first <= 2 && 2 <= anchor.last);
}

#[test]
fn a_plan_orders_and_groups_files_and_the_rest_follow() {
    let repo = edited();
    repo.write("c.rs", "fn c() {}\n");
    let shown = diff(&repo, &RepoSpec::uncommitted("api"));
    let plan = ReviewPlan {
        spec: DiffSpec {
            repos: vec![RepoSpec::uncommitted("api")],
        },
        order: vec![file("b.rs"), file("a.rs")],
        groups: vec![Group {
            title: "The entry point".to_owned(),
            note: "start where main calls retry".to_owned(),
            first: file("a.rs"),
        }],
        notes: Vec::new(),
    };
    plan.check(std::slice::from_ref(&shown)).expect("a valid plan");
    let arrangement = plan.arrange(std::slice::from_ref(&shown));
    let sections: Vec<(Option<String>, Vec<String>)> = arrangement
        .sections
        .iter()
        .map(|section| {
            (
                section.title.clone(),
                section.files.iter().map(|f| f.path.clone()).collect(),
            )
        })
        .collect();
    assert_eq!(
        sections,
        [
            (None, vec!["b.rs".to_owned()]),
            (Some("The entry point".to_owned()), vec!["a.rs".to_owned()]),
            (None, vec!["c.rs".to_owned()]),
        ]
    );
}

#[test]
fn a_plan_naming_a_file_the_diff_does_not_show_is_refused() {
    let repo = edited();
    let shown = diff(&repo, &RepoSpec::uncommitted("api"));
    let plan = ReviewPlan {
        spec: DiffSpec {
            repos: vec![RepoSpec::uncommitted("api")],
        },
        order: vec![file("a.rs"), file("nope.rs")],
        groups: Vec::new(),
        notes: Vec::new(),
    };
    assert!(plan.check(std::slice::from_ref(&shown)).is_err());
}

#[test]
fn the_review_chat_marks_its_notes_outdated_after_an_edit() {
    let repo = edited();
    let spec = RepoSpec::uncommitted("api");
    let shown = diff(&repo, &spec);
    let in_diff = |f: &FileRef| shown.file(&f.path).is_some();
    let mut chat = ReviewChat::restore(ViewState::default(), AgentSwitches::default(), Vec::new());
    let note = anchored(&repo, &shown, vec![lines_note("a.rs", 2, 2)]).remove(0);
    let change = chat.note("claude", note, in_diff).expect("notes are on");
    assert!(!change.undo_id.is_empty());

    repo.write("a.rs", &numbered(&["fn main() {", "    give_up();", "    run();", "}"]));
    let notes: Vec<Note> = chat.notes().map(|note| note.note.clone()).collect();
    let newly = chat.refresh_notes(&states(&repo, &diff(&repo, &spec), &notes));
    assert_eq!(newly.len(), 1);
    assert!(chat.notes().all(|note| note.outdated));
    assert!(chat.refresh_notes(&states(&repo, &diff(&repo, &spec), &notes)).is_empty());
}

#[test]
fn a_switch_the_user_turned_off_denies_its_tools() {
    let repo = edited();
    let shown = diff(&repo, &RepoSpec::uncommitted("api"));
    let in_diff = |f: &FileRef| shown.file(&f.path).is_some();
    let switches = AgentSwitches {
        notes: false,
        ..AgentSwitches::default()
    };
    let mut chat = ReviewChat::restore(ViewState::default(), switches, Vec::new());
    let denied = chat.note("claude", lines_note("a.rs", 1, 1), in_diff);
    assert!(matches!(denied, Err(ChatError::SwitchOff { .. })));
    assert!(chat.notes().next().is_none());
    assert!(switches.allow(ChatTool::Context).is_ok());
    assert!(switches.allow(ChatTool::Focus).is_ok());
}
