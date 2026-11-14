//! Saved repositories listed for GitHub owners, against a stand-in for
//! GitHub's API.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

use humata_wire::{Client, ClientError, pb};

use super::daemon;
use crate::hosts::Hosts;

/// Answers GitHub's API at the address it returns: the organization
/// `acme` has two repositories over two pages, the user `solo` one, and
/// the token's own user `me` one; nothing else is there.
fn stand_in() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|error| panic!("{error}"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("{error}"));
    let base = format!("http://{address}");
    let served = base.clone();
    std::thread::spawn(move || {
        let base = served;
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(&mut stream);
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let target = line
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_owned();
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
                    break;
                }
            }
            let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
            let mut status = "200 OK";
            let mut headers = String::new();
            let reply = match path {
                "/orgs/acme/repos" if query.contains("page=2") => {
                    r#"[{"name": "lib", "html_url": "https://github.com/acme/lib"}]"#
                }
                "/orgs/acme/repos" => {
                    headers = format!(
                        "Link: <{base}/orgs/acme/repos?type=all&per_page=100&page=2>; rel=\"next\"\r\n"
                    );
                    r#"[{"name": "app", "html_url": "https://github.com/acme/app"}]"#
                }
                "/user" => r#"{"login": "me"}"#,
                "/user/repos" => {
                    r#"[{"name": "secret", "html_url": "https://github.com/me/secret"}]"#
                }
                "/users/solo/repos" => {
                    r#"[{"name": "tool", "html_url": "https://github.com/solo/tool"}]"#
                }
                _ => {
                    status = "404 Not Found";
                    r#"{"message": "Not Found"}"#
                }
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\n{headers}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
        }
    });
    base
}

fn hosts() -> Hosts {
    Hosts {
        coordinator_env: BTreeMap::from([
            ("GH_TOKEN".to_owned(), "test-token".to_owned()),
            ("HUMATA_GITHUB_API".to_owned(), stand_in()),
        ]),
        ..Hosts::default()
    }
}

async fn owners(client: &Client, request: pb::request::Kind) -> Vec<pb::RepositoryOwner> {
    match client.call(request).await {
        Ok(pb::response::Kind::RepositoryOwners(owners)) => owners.owners,
        other => panic!("expected the owners, got {other:?}"),
    }
}

async fn add(client: &Client, login: &str) {
    owners(
        client,
        pb::request::Kind::AddRepositoryOwner(pb::AddRepositoryOwner {
            login: login.to_owned(),
        }),
    )
    .await;
}

async fn refresh(client: &Client, login: &str) -> Vec<pb::RepositoryOwner> {
    owners(
        client,
        pb::request::Kind::RefreshRepositoryOwners(pb::RefreshRepositoryOwners {
            login: login.to_owned(),
        }),
    )
    .await
}

/// The saved repositories, as `(owner, location)`, sorted.
async fn saved(client: &Client) -> Vec<(String, String)> {
    let listed = client
        .call(pb::request::Kind::ListSavedRepositories(
            pb::ListSavedRepositories {},
        ))
        .await;
    let Ok(pb::response::Kind::SavedRepositories(saved)) = listed else {
        panic!("expected the saved repositories, got {listed:?}");
    };
    let mut saved: Vec<_> = saved
        .repositories
        .into_iter()
        .inspect(|repository| {
            assert_eq!(repository.source(), pb::RepositorySource::Github);
            assert_eq!(repository.last_used_unix_ms, 0);
        })
        .map(|repository| (repository.owner, repository.location))
        .collect();
    saved.sort();
    saved
}

fn pair(owner: &str, location: &str) -> (String, String) {
    (owner.to_owned(), location.to_owned())
}

#[tokio::test]
async fn an_owners_repositories_are_listed_on_github_page_by_page() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let client = daemon(state.path(), hosts()).await;
    add(&client, "@acme").await;
    let listed = refresh(&client, "acme").await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].login, "acme");
    assert_eq!(listed[0].repositories, 2);
    assert!(listed[0].refreshed_unix_ms > 0);
    assert!(listed[0].error.is_empty());
    assert_eq!(
        saved(&client).await,
        [
            pair("acme", "https://github.com/acme/app"),
            pair("acme", "https://github.com/acme/lib")
        ]
    );
}

#[tokio::test]
async fn a_user_is_listed_when_no_organization_has_its_name() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let client = daemon(state.path(), hosts()).await;
    add(&client, "https://github.com/solo").await;
    add(&client, "me").await;
    add(&client, "ghost").await;
    let listed = refresh(&client, "").await;
    let ghost = listed
        .iter()
        .find(|owner| owner.login == "ghost")
        .unwrap_or_else(|| panic!("ghost is listed: {listed:?}"));
    // GitHub has nobody by that name, which the person is told.
    assert!(!ghost.error.is_empty());
    assert_eq!(ghost.repositories, 0);
    assert_eq!(
        saved(&client).await,
        [
            pair("me", "https://github.com/me/secret"),
            pair("solo", "https://github.com/solo/tool")
        ]
    );
}

#[tokio::test]
async fn removing_an_owner_takes_its_repositories_out_of_the_list() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let client = daemon(state.path(), hosts()).await;
    add(&client, "acme").await;
    add(&client, "solo").await;
    refresh(&client, "").await;
    let left = owners(
        &client,
        pb::request::Kind::RemoveRepositoryOwner(pb::RemoveRepositoryOwner {
            login: "acme".to_owned(),
        }),
    )
    .await;
    assert_eq!(
        left.iter()
            .map(|owner| owner.login.as_str())
            .collect::<Vec<_>>(),
        ["solo"]
    );
    assert_eq!(
        saved(&client).await,
        [pair("solo", "https://github.com/solo/tool")]
    );
}

#[tokio::test]
async fn an_owner_that_cant_be_a_github_login_is_refused() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let client = daemon(state.path(), hosts()).await;
    let added = client
        .call(pb::request::Kind::AddRepositoryOwner(
            pb::AddRepositoryOwner {
                login: "acme/app".to_owned(),
            },
        ))
        .await;
    assert!(
        matches!(
            added,
            Err(ClientError::Refused {
                code: pb::ErrorCode::InvalidRequest,
                ..
            })
        ),
        "{added:?}"
    );
}
