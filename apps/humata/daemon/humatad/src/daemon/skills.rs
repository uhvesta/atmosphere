//! Settings › Skills, and the skills each session is told of.

use std::path::Path;

use humata_wire::{Reply, pb};

use super::Daemon;
use crate::error::DaemonError;
use crate::skills;
use crate::store::Project;

/// How long the list waits for each host before leaving it out.
const SKILL_LIST_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

impl Daemon {
    /// Every skill on the Mac and on each host that answers in time, with
    /// each project's own, and the clashes among them.
    pub(super) async fn list_skills(&self) -> Result<pb::SkillList, DaemonError> {
        let projects = self.store.projects()?;
        // Each host's clones of the projects' repositories, which it finds
        // itself, asked all at once.
        let cloned: Vec<pb::SkillFolder> = projects
            .iter()
            .flat_map(|project| {
                project
                    .repositories
                    .iter()
                    .map(|repository| pb::SkillFolder {
                        project_id: project.id.clone(),
                        name: repository.name.clone(),
                        path: String::new(),
                    })
            })
            .collect();
        let mut asking = Vec::new();
        for host in self.remote_names() {
            let remote = self.remote(&host)?;
            let (hosts_file, build) = (self.hosts_file.clone(), self.build.clone());
            let request = pb::request::Kind::ScanSkills(pb::ScanSkills {
                repositories: cloned.clone(),
            });
            let answer = tokio::spawn(async move {
                let client = remote.client(&hosts_file, &build).await.ok()?;
                match remote.call(&client, request).await.ok()? {
                    pb::response::Kind::SkillsScanned(scanned) => Some(scanned.skills),
                    _ => None,
                }
            });
            asking.push((host, answer));
        }
        let deadline = tokio::time::Instant::now() + SKILL_LIST_WAIT;
        let here: Vec<pb::SkillFolder> = projects
            .iter()
            .flat_map(|project| {
                project
                    .repositories
                    .iter()
                    .map(|repository| pb::SkillFolder {
                        project_id: project.id.clone(),
                        name: repository.name.clone(),
                        path: repository.path.display().to_string(),
                    })
            })
            .collect();
        let mut found = self.scan_skills("", here).await?;
        let mut unreachable = Vec::new();
        for (host, answer) in asking {
            match tokio::time::timeout_at(deadline, answer).await {
                Ok(Ok(Some(theirs))) => {
                    found.extend(theirs.into_iter().map(|mut skill| {
                        skill.host.clone_from(&host);
                        skill
                    }));
                }
                _ => unreachable.push(host),
            }
        }
        found.extend(projects.iter().flat_map(skills::own));
        let clashes = skills::clashes(&found, &self.store.skill_picks()?);
        Ok(pb::SkillList {
            skills: found,
            clashes,
            unreachable,
        })
    }

    pub(super) fn pick_skill(&self, pick: &pb::PickSkill) -> Result<(), DaemonError> {
        if pick.name.is_empty() || !pick.candidates.contains(&pick.winner) {
            return Err(DaemonError::BadSkillPick);
        }
        let key = skills::candidates_key(pick.candidates.iter().map(String::as_str));
        self.store.pick_skill(&pick.name, &key, &pick.winner)?;
        Ok(())
    }

    /// The skills on `host` in `folders` and its home folder.
    async fn scan_skills(
        &self,
        host: &str,
        folders: Vec<pb::SkillFolder>,
    ) -> Result<Vec<pb::FoundSkill>, DaemonError> {
        let request = pb::request::Kind::ScanSkills(pb::ScanSkills {
            repositories: folders,
        });
        match self.forward(host, request).await? {
            Reply::Response(pb::response::Kind::SkillsScanned(scanned)) => Ok(scanned
                .skills
                .into_iter()
                .map(|mut skill| {
                    skill.host = host.to_owned();
                    skill
                })
                .collect()),
            _ => Err(DaemonError::Host {
                host: host.to_owned(),
                source: humata_wire::ClientError::Unexpected {
                    expected: "the skills found",
                    got: "another answer".to_owned(),
                },
            }),
        }
    }

    /// The skills a session of `project` on `host` gets, working in
    /// `repositories` there (or in `folder`, with none): one of each name,
    /// the one picked where two differ, each repository's with its path
    /// from the folder the session works in.
    pub(super) async fn session_skills(
        &self,
        project: &Project,
        host: &str,
        repositories: &[pb::Repository],
        folder: &str,
    ) -> Result<Vec<pb::FoundSkill>, DaemonError> {
        let folders: Vec<pb::SkillFolder> = if repositories.is_empty() {
            (!folder.is_empty())
                .then(|| pb::SkillFolder {
                    project_id: project.id.clone(),
                    name: String::new(),
                    path: folder.to_owned(),
                })
                .into_iter()
                .collect()
        } else {
            repositories
                .iter()
                .map(|repository| pb::SkillFolder {
                    project_id: project.id.clone(),
                    name: repository.name.clone(),
                    path: repository.path.clone(),
                })
                .collect()
        };
        let mut found = self.scan_skills(host, folders.clone()).await?;
        found.extend(skills::own(project));
        let mut offered = skills::offered(found, &self.store.skill_picks()?)
            .map_err(|names| DaemonError::SkillClash { names })?;
        // The session works in a copy of each repository, not the folder
        // looked in: one alone is its folder, else each is a folder in it.
        for skill in &mut offered {
            let Some(looked) = folders.iter().find(|folder| {
                !skill.path.is_empty()
                    && folder.name == skill.repository
                    && skill.project_id == folder.project_id
                    && skill.origin != pb::SkillOrigin::Project as i32
                    && Path::new(&skill.path).starts_with(&folder.path)
            }) else {
                continue;
            };
            if let Ok(inside) = Path::new(&skill.path).strip_prefix(&looked.path) {
                let inside = if folders.len() > 1 {
                    Path::new(&looked.name).join(inside)
                } else {
                    inside.to_path_buf()
                };
                skill.path = inside.display().to_string();
            }
        }
        Ok(offered)
    }
}
