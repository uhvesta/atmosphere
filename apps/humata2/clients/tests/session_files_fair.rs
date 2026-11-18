//! A session's file list is capped, and the cap is shared among its repos:
//! one big repo never hides the others.
use humata2_clients::fair_share;
use humata2_clients::model::SessionFile;

fn files(repo: &str, count: usize) -> Vec<SessionFile> {
    (0..count)
        .map(|i| SessionFile {
            repo: repo.to_owned(),
            path: format!("src/f{i}.rs"),
            folder: false,
        })
        .collect()
}

fn count(list: &[SessionFile], repo: &str) -> usize {
    list.iter().filter(|file| file.repo == repo).count()
}

#[test]
fn a_list_under_the_cap_is_kept_whole() {
    let mut all = files("acme/api", 3);
    all.extend(files("acme/web", 2));
    assert_eq!(fair_share(all.clone(), 10), all);
}

#[test]
fn a_big_first_repo_leaves_room_for_the_others() {
    let mut all = files("acme/api", 600);
    all.extend(files("acme/web", 3));
    all.extend(files("acme/docs", 1));
    let kept = fair_share(all, 500);
    assert_eq!(kept.len(), 500);
    assert_eq!(count(&kept, "acme/web"), 3);
    assert_eq!(count(&kept, "acme/docs"), 1);
    assert_eq!(count(&kept, "acme/api"), 496);
}

#[test]
fn two_big_repos_split_the_cap() {
    let mut all = files("acme/api", 400);
    all.extend(files("acme/web", 400));
    let kept = fair_share(all, 500);
    assert_eq!(count(&kept, "acme/api"), 250);
    assert_eq!(count(&kept, "acme/web"), 250);
}

#[test]
fn each_repo_keeps_its_first_files_in_order() {
    let mut all = files("acme/api", 10);
    all.extend(files("acme/web", 10));
    let kept = fair_share(all, 4);
    let paths: Vec<(&str, &str)> = kept.iter().map(|f| (f.repo.as_str(), f.path.as_str())).collect();
    assert_eq!(
        paths,
        [("acme/api", "src/f0.rs"), ("acme/api", "src/f1.rs"), ("acme/web", "src/f0.rs"), ("acme/web", "src/f1.rs")]
    );
}
