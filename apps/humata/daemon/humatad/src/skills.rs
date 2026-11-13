//! Skills in the agents' own folders, and which one wins where two that one
//! session would get share a name.
//!
//! A skill is a folder holding a `SKILL.md`, whose front matter may give
//! its `name` (else the folder's) and `description`. They're looked for in
//! `~/.claude/skills` and `~/.copilot/skills` on each host, and in each
//! repository's `.claude/skills` and `.github/skills`. A session is told of
//! each by name, description and path, not given its text, so it reads only
//! those its work needs.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use humata_wire::pb;
use sha2::{Digest, Sha256};

/// A SKILL.md larger than this is left out, as no agent would read it.
const LARGEST: u64 = 1 << 20;

/// Which skill wins, by name and candidates (their hashes, sorted and
/// joined by commas), as the store keeps them.
pub(crate) type Picks = HashMap<(String, String), String>;

/// The skills in the agents' folders under `home`, and in each repository,
/// found at the path given with it.
pub(crate) fn scan(
    home: Option<&Path>,
    repositories: &[(pb::SkillFolder, PathBuf)],
) -> Vec<pb::FoundSkill> {
    let mut found = Vec::new();
    if let Some(home) = home {
        look(
            &home.join(".claude/skills"),
            pb::SkillOrigin::ClaudeUser,
            None,
            &mut found,
        );
        look(
            &home.join(".copilot/skills"),
            pb::SkillOrigin::CopilotUser,
            None,
            &mut found,
        );
    }
    for (repository, path) in repositories {
        look(
            &path.join(".claude/skills"),
            pb::SkillOrigin::ClaudeRepository,
            Some(repository),
            &mut found,
        );
        look(
            &path.join(".github/skills"),
            pb::SkillOrigin::GithubRepository,
            Some(repository),
            &mut found,
        );
    }
    found
}

/// Each skill folder in `folder`; a folder that isn't there has none, and
/// a file that can't be read is left out, as the agent couldn't read it
/// either.
fn look(
    folder: &Path,
    origin: pb::SkillOrigin,
    repository: Option<&pb::SkillFolder>,
    found: &mut Vec<pb::FoundSkill>,
) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let mut skills: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    skills.sort();
    for skill in skills {
        let file = skill.join("SKILL.md");
        let Ok(metadata) = std::fs::metadata(&file) else {
            continue;
        };
        if !metadata.is_file() || metadata.len() > LARGEST {
            continue;
        }
        let Ok(bytes) = std::fs::read(&file) else {
            continue;
        };
        let (name, description) = front_matter(&String::from_utf8_lossy(&bytes));
        let Some(name) = name.or_else(|| {
            skill
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        }) else {
            continue;
        };
        found.push(pb::FoundSkill {
            name,
            description,
            path: file.display().to_string(),
            origin: origin.into(),
            project_id: repository.map(|r| r.project_id.clone()).unwrap_or_default(),
            repository: repository.map(|r| r.name.clone()).unwrap_or_default(),
            hash: hash(&bytes),
            host: String::new(),
        });
    }
}

/// A SHA-256 of `bytes`, hex.
pub(crate) fn hash(bytes: &[u8]) -> String {
    crate::hex(&Sha256::digest(bytes))
}

/// The `name` and `description` in the front matter, if it has them. A
/// folded or literal block (`>` or `|`) is read as one line.
fn front_matter(text: &str) -> (Option<String>, String) {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (None, String::new());
    }
    let mut fields: HashMap<&str, String> = HashMap::new();
    let mut block: Option<&str> = None;
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        if let Some(key) = block {
            if line.starts_with(char::is_whitespace) || line.trim().is_empty() {
                let value = fields.entry(key).or_default();
                if !line.trim().is_empty() {
                    if !value.is_empty() {
                        value.push(' ');
                    }
                    value.push_str(line.trim());
                }
                continue;
            }
            block = None;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.starts_with(char::is_whitespace) {
            continue;
        }
        let (key, value) = (key.trim(), value.trim());
        if value.starts_with('>') || value.starts_with('|') {
            block = Some(key);
            fields.insert(key, String::new());
        } else {
            fields.insert(key, unquoted(value).to_owned());
        }
    }
    let name = fields.remove("name").filter(|name| !name.is_empty());
    (name, fields.remove("description").unwrap_or_default())
}

fn unquoted(value: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner;
        }
    }
    value
}

/// The project's own skills, as found skills; their hash is of their text.
pub(crate) fn own(project: &crate::store::Project) -> Vec<pb::FoundSkill> {
    project
        .skills
        .iter()
        .map(|skill| pb::FoundSkill {
            name: skill.name.clone(),
            description: String::new(),
            path: String::new(),
            origin: pb::SkillOrigin::Project.into(),
            project_id: project.id.clone(),
            repository: String::new(),
            hash: hash(skill.text.as_bytes()),
            host: String::new(),
        })
        .collect()
}

/// The candidates' hashes as a pick is kept under.
pub(crate) fn candidates_key<'a>(hashes: impl IntoIterator<Item = &'a str>) -> String {
    let sorted: BTreeSet<&str> = hashes.into_iter().collect();
    sorted.into_iter().collect::<Vec<_>>().join(",")
}

/// Each name `skills` give more than one file for, with one of each file.
fn shared_names<'a>(
    skills: impl IntoIterator<Item = &'a pb::FoundSkill>,
) -> Vec<(String, Vec<&'a pb::FoundSkill>)> {
    let mut by_name: BTreeMap<&str, Vec<&pb::FoundSkill>> = BTreeMap::new();
    for skill in skills {
        let files = by_name.entry(&skill.name).or_default();
        if !files.iter().any(|file| file.hash == skill.hash) {
            files.push(skill);
        }
    }
    by_name
        .into_iter()
        .filter(|(_, files)| files.len() > 1)
        .map(|(name, files)| (name.to_owned(), files))
        .collect()
}

/// Every clash among the skills one session would get: on each host, its
/// home folder's with, for each project, its repositories' there and its
/// own. A clash is listed once however many places have it, with the pick
/// made for it, if any.
pub(crate) fn clashes(skills: &[pb::FoundSkill], picks: &Picks) -> Vec<pb::SkillClash> {
    let project = pb::SkillOrigin::Project as i32;
    let mut hosts: BTreeSet<&str> = skills
        .iter()
        .filter(|skill| skill.origin != project)
        .map(|skill| skill.host.as_str())
        .collect();
    hosts.insert("");
    let projects: BTreeSet<&str> = skills
        .iter()
        .map(|skill| skill.project_id.as_str())
        .filter(|id| !id.is_empty())
        .collect();
    let mut seen = HashSet::new();
    let mut clashes = Vec::new();
    for host in hosts {
        let home = skills
            .iter()
            .filter(|skill| skill.host == host && skill.project_id.is_empty());
        let mut places: Vec<Vec<&pb::FoundSkill>> = vec![home.clone().collect()];
        for id in &projects {
            let theirs = skills.iter().filter(|skill| {
                skill.project_id == *id && (skill.origin == project || skill.host == host)
            });
            places.push(home.clone().chain(theirs).collect());
        }
        for place in places {
            for (name, files) in shared_names(place) {
                let key = candidates_key(files.iter().map(|file| file.hash.as_str()));
                if !seen.insert((name.clone(), key.clone())) {
                    continue;
                }
                clashes.push(pb::SkillClash {
                    winner: picks.get(&(name.clone(), key)).cloned().unwrap_or_default(),
                    name,
                    candidates: files.into_iter().cloned().collect(),
                });
            }
        }
    }
    clashes
}

/// The skills a session gets out of all it could: one of each name, the
/// one picked where files differ. Err with the names nobody has picked for.
pub(crate) fn offered(
    skills: Vec<pb::FoundSkill>,
    picks: &Picks,
) -> Result<Vec<pb::FoundSkill>, Vec<String>> {
    let mut unpicked = Vec::new();
    let mut winners: HashMap<String, String> = HashMap::new();
    for (name, files) in shared_names(&skills) {
        let key = candidates_key(files.iter().map(|file| file.hash.as_str()));
        match picks.get(&(name.clone(), key)) {
            Some(winner) => {
                winners.insert(name, winner.clone());
            }
            None => unpicked.push(name),
        }
    }
    if !unpicked.is_empty() {
        return Err(unpicked);
    }
    let mut given: HashSet<String> = HashSet::new();
    let mut offered = Vec::new();
    for skill in skills {
        if winners
            .get(&skill.name)
            .is_some_and(|winner| *winner != skill.hash)
        {
            continue;
        }
        if given.insert(skill.name.clone()) {
            offered.push(skill);
        }
    }
    offered.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(offered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap_or_else(|e| panic!("{e}"));
        }
        std::fs::write(path, text).unwrap_or_else(|e| panic!("{e}"));
    }

    fn found(name: &str, hash: &str, host: &str, project: &str) -> pb::FoundSkill {
        pb::FoundSkill {
            name: name.into(),
            hash: hash.into(),
            host: host.into(),
            project_id: project.into(),
            origin: if project.is_empty() {
                pb::SkillOrigin::ClaudeUser
            } else {
                pb::SkillOrigin::ClaudeRepository
            }
            .into(),
            ..Default::default()
        }
    }

    #[test]
    fn skills_are_found_in_the_home_folder_and_each_repository_with_their_front_matter() {
        let folder = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let home = folder.path().join("home");
        let repository = folder.path().join("repo");
        write(
            &home.join(".claude/skills/review/SKILL.md"),
            "---\nname: code-review\ndescription: \"Reviews a diff\"\n---\nRead the diff.",
        );
        write(
            &home.join(".copilot/skills/notes/SKILL.md"),
            "---\ndescription: >\n  Takes notes\n  as it goes\n---\n",
        );
        write(
            &repository.join(".github/skills/plain/SKILL.md"),
            "No front matter.",
        );
        write(
            &repository.join(".claude/skills/empty/README.md"),
            "not a skill",
        );
        let folder = pb::SkillFolder {
            project_id: "p".into(),
            name: "repo".into(),
            path: String::new(),
        };
        let skills = scan(Some(&home), &[(folder, repository)]);
        let summary: Vec<(&str, &str, &str)> = skills
            .iter()
            .map(|s| {
                (
                    s.name.as_str(),
                    s.description.as_str(),
                    s.repository.as_str(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("code-review", "Reviews a diff", ""),
                ("notes", "Takes notes as it goes", ""),
                ("plain", "", "repo"),
            ]
        );
        assert!(skills.iter().all(|s| s.hash.len() == 64));
    }

    #[test]
    fn copies_of_one_file_are_no_clash_but_different_files_are_until_picked() {
        let skills = vec![
            found("review", "a", "", ""),
            found("review", "a", "", "p"),
            found("notes", "b", "", ""),
        ];
        assert_eq!(
            offered(skills.clone(), &Picks::new()).map(|s| s.len()),
            Ok(2)
        );

        let mut skills = skills;
        skills.push(found("review", "c", "", "p"));
        assert_eq!(
            offered(skills.clone(), &Picks::new()),
            Err(vec!["review".to_owned()])
        );
        let picks = Picks::from([(("review".into(), "a,c".into()), "c".into())]);
        let given = offered(skills, &picks).unwrap_or_else(|names| panic!("{names:?}"));
        let review: Vec<&str> = given
            .iter()
            .filter(|s| s.name == "review")
            .map(|s| s.hash.as_str())
            .collect();
        assert_eq!(review, ["c"]);
    }

    #[test]
    fn a_pick_no_longer_holds_once_a_file_changes() {
        let picks = Picks::from([(("review".into(), "a,c".into()), "c".into())]);
        let changed = vec![found("review", "a", "", ""), found("review", "d", "", "p")];
        assert!(offered(changed, &picks).is_err());
    }

    #[test]
    fn clashes_are_only_among_skills_one_session_would_get() {
        let skills = vec![
            // Two hosts' own: never in one session.
            found("review", "a", "", ""),
            found("review", "b", "box", ""),
            // Two projects' repositories on the Mac: never in one session.
            found("lint", "c", "", "p"),
            found("lint", "d", "", "q"),
            // A host's own and a project's there: one session gets both.
            found("notes", "e", "box", ""),
            found("notes", "f", "box", "p"),
        ];
        let picks = Picks::from([(("notes".into(), "e,f".into()), "f".into())]);
        let clashes = clashes(&skills, &picks);
        let names: Vec<(&str, &str)> = clashes
            .iter()
            .map(|c| (c.name.as_str(), c.winner.as_str()))
            .collect();
        assert_eq!(names, [("notes", "f")]);
    }
}
