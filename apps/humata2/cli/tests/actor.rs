//! Who `humata` acts as, and which daemon it finds (CLI.md §1, §2).

mod support;

use std::path::{Path, PathBuf};

use humata2_cli::{DaemonError, Environment, Principal, exit, find_socket, principal, rpc};
use humata2_proto::humata::v1 as pb;
use serde_json::json;
use support::{FakeConsole, FakeDaemon, run, run_as};

fn agent_environment() -> Environment {
    Environment {
        session_token: Some("session-secret".into()),
        ..Environment::default()
    }
}

#[test]
fn a_session_token_makes_the_agent() {
    assert_eq!(
        principal(&agent_environment()),
        Principal::Agent {
            token: "session-secret".into()
        }
    );
}

#[test]
fn a_script_token_makes_a_script() {
    let environment = Environment {
        script_token: Some("script-secret".into()),
        ..Environment::default()
    };
    assert_eq!(
        principal(&environment),
        Principal::Script {
            token: "script-secret".into()
        }
    );
}

#[test]
fn the_agent_stays_the_agent_with_a_script_token_too() {
    let environment = Environment {
        session_token: Some("session-secret".into()),
        script_token: Some("script-secret".into()),
        ..Environment::default()
    };
    assert!(matches!(principal(&environment), Principal::Agent { .. }));
}

#[test]
fn no_token_is_the_person() {
    assert_eq!(principal(&Environment::default()), Principal::User);
}

#[tokio::test]
async fn whoami_says_user_for_the_person() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    assert_eq!(run(&daemon, &console, &["--json", "whoami"]).await, exit::OK);
    assert_eq!(
        console.json()["result"],
        json!({
            "actor": null,
            "channel": null,
            "presence_possible": false,
            "contained": false,
            "tier": "user",
        })
    );
}

#[tokio::test]
async fn whoami_says_script_for_a_script_token() {
    let environment = Environment {
        script_token: Some("script-secret".into()),
        ..Environment::default()
    };
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    run_as(&environment, &daemon, &console, &["--json", "whoami"]).await;
    assert_eq!(console.json()["result"]["tier"], "script");
}

#[tokio::test]
async fn whoami_believes_the_daemon_when_it_says_agent() {
    // A process in a session's tree is its agent with or without a token.
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::daemon::WHO_AM_I,
        &pb::WhoAmIResponse {
            actor: Some(pb::Actor {
                actor: Some(pb::actor::Actor::Agent(Default::default())),
            }),
            channel: support::channel("ch_1"),
            ..Default::default()
        },
    );
    let console = FakeConsole::pipe();
    run(&daemon, &console, &["--json", "whoami"]).await;
    let result = &console.json()["result"];
    assert_eq!(result["tier"], "agent");
    assert_eq!(result["channel"], "ch_1");
}

/// D748: an agent that asks who it is learns its session's folder and
/// where each of its clones is, on which branch.
#[tokio::test]
async fn whoami_lists_an_agents_clones_with_their_paths_and_branches() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::daemon::WHO_AM_I,
        &pb::WhoAmIResponse {
            actor: Some(pb::Actor {
                actor: Some(pb::actor::Actor::Agent(pb::AgentActor {
                    session: support::session("se_1"),
                    ..Default::default()
                })),
            }),
            ..Default::default()
        },
    );
    daemon.reply(
        rpc::session::GET_SESSION,
        &pb::Session {
            id: support::session("se_1"),
            folder: "/home/me/Humata/acme/sessions/s.1".into(),
            repos: vec![pb::SessionRepo {
                repo: Some(pb::RepoRef { owner: "acme".into(), name: "api".into() }),
                branch: "humata/fix".into(),
                path: "/home/me/Humata/acme/sessions/s.1/api".into(),
                main_clone: false,
            }],
            ..Default::default()
        },
    );
    let console = FakeConsole::pipe();
    let exit = run_as(&agent_environment(), &daemon, &console, &["--json", "whoami"]).await;
    assert_eq!(exit, exit::OK);
    let result = &console.json()["result"];
    assert_eq!(result["tier"], "agent");
    assert_eq!(result["folder"], "/home/me/Humata/acme/sessions/s.1");
    assert_eq!(
        result["clones"],
        json!([{ "repo": "acme/api", "path": "/home/me/Humata/acme/sessions/s.1/api", "branch": "humata/fix" }])
    );
}

/// An agent whose session can't be read (an agent added to a session asks
/// only who it is) is still told who it is.
#[tokio::test]
async fn whoami_answers_an_agent_whose_session_cant_be_read() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::daemon::WHO_AM_I,
        &pb::WhoAmIResponse {
            actor: Some(pb::Actor {
                actor: Some(pb::actor::Actor::Agent(pb::AgentActor {
                    session: support::session("se_1"),
                    ..Default::default()
                })),
            }),
            ..Default::default()
        },
    );
    daemon.refuse(rpc::session::GET_SESSION, pb::error::Kind::Denied(Default::default()));
    let console = FakeConsole::pipe();
    let exit = run_as(&agent_environment(), &daemon, &console, &["--json", "whoami"]).await;
    assert_eq!(exit, exit::OK);
    let result = &console.json()["result"];
    assert_eq!(result["tier"], "agent");
    assert!(result.get("clones").is_none(), "{result}");
}

/// A fake daemon that says the caller is session `se_1`'s agent, with
/// `repos` in that session.
fn agent_daemon(repos: &[(&str, &str)]) -> FakeDaemon {
    let daemon = FakeDaemon::new();
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
            repos: repos
                .iter()
                .map(|(owner, name)| pb::SessionRepo {
                    repo: Some(pb::RepoRef {
                        owner: (*owner).into(),
                        name: (*name).into(),
                    }),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        },
    );
    daemon
}

#[tokio::test]
async fn an_agent_names_its_own_clone_with_a_bare_path() {
    let daemon = agent_daemon(&[("acme", "api")]);
    let console = FakeConsole::pipe();
    let exit = run_as(
        &agent_environment(),
        &daemon,
        &console,
        &["--json", "files", "ls", "src/lib"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let sent: pb::ListDirectoryRequest = daemon.only(rpc::files::LIST_DIRECTORY);
    let location = sent.location.expect("a location");
    assert_eq!(location.path, "src/lib");
    assert_eq!(location.machine, None);
    let Some(pb::file_location::Root::SessionClone(clone)) = location.root else {
        panic!("not a clone: {:?}", location.root);
    };
    // The daemon serves only a clone that names its session and repo.
    assert_eq!(clone.session, support::session("se_1"));
    let repo = clone.repo.expect("a repo");
    assert_eq!((repo.owner.as_str(), repo.name.as_str()), ("acme", "api"));
}

#[tokio::test]
async fn an_agent_with_no_repository_names_its_own_folder_with_a_bare_path() {
    let daemon = agent_daemon(&[]);
    let console = FakeConsole::pipe();
    let exit = run_as(
        &agent_environment(),
        &daemon,
        &console,
        &["--json", "files", "ls", "notes"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let sent: pb::ListDirectoryRequest = daemon.only(rpc::files::LIST_DIRECTORY);
    let location = sent.location.expect("a location");
    assert_eq!(
        location.root,
        Some(pb::file_location::Root::SessionFolder(pb::SessionFolder {
            session: support::session("se_1"),
        }))
    );
}

#[tokio::test]
async fn an_agent_with_two_repositories_must_say_which_clone_a_bare_path_is_in() {
    let daemon = agent_daemon(&[("acme", "api"), ("acme", "web")]);
    let console = FakeConsole::pipe();
    let exit = run_as(
        &agent_environment(),
        &daemon,
        &console,
        &["--json", "files", "ls", "src"],
    )
    .await;
    assert_eq!(exit, exit::USAGE);
    assert!(!daemon.called(rpc::files::LIST_DIRECTORY));
}

#[tokio::test]
async fn the_person_must_say_which_machine_a_path_is_on() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "files", "ls", "src/lib"]).await;
    assert_eq!(exit, exit::USAGE);
    assert!(!daemon.called(rpc::files::LIST_DIRECTORY));
}

#[tokio::test]
async fn an_agent_without_channel_uses_its_own() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::daemon::WHO_AM_I,
        &pb::WhoAmIResponse {
            channel: support::channel("ch_mine"),
            ..Default::default()
        },
    );
    let console = FakeConsole::pipe();
    let exit = run_as(&agent_environment(), &daemon, &console, &["--json", "access"]).await;
    assert_eq!(exit, exit::OK);
    let sent: pb::GetChannelRulesRequest = daemon.only(rpc::access::GET_CHANNEL_RULES);
    assert_eq!(sent.channel, support::channel("ch_mine"));
}

#[tokio::test]
async fn without_a_channel_to_use_the_command_asks_for_one() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    assert_eq!(run(&daemon, &console, &["--json", "access"]).await, exit::USAGE);
    assert!(!daemon.called(rpc::access::GET_CHANNEL_RULES));
}

fn home_environment() -> Environment {
    Environment {
        home: Some(PathBuf::from("/nowhere/home")),
        ..Environment::default()
    }
}

#[test]
fn humata_socket_is_the_only_socket_tried() {
    let environment = Environment {
        socket: Some(PathBuf::from("/nowhere/mine.sock")),
        ..home_environment()
    };
    let found = find_socket(&environment, |_| true).expect("a socket");
    assert_eq!(found, PathBuf::from("/nowhere/mine.sock"));
    let missing = find_socket(&environment, |_| false);
    assert!(matches!(missing, Err(DaemonError::NotRunning { tried }) if tried.len() == 1));
}

#[test]
fn the_macs_socket_comes_before_this_machines() {
    let everything = find_socket(&home_environment(), |_| true).expect("a socket");
    let mac = find_socket(&home_environment(), |path| {
        path.starts_with("/nowhere/home/Library")
    })
    .expect("a socket");
    assert_eq!(everything, mac);
}

#[test]
fn humata_root_moves_this_machines_socket() {
    let environment = Environment {
        root: Some(PathBuf::from("/nowhere/root")),
        ..home_environment()
    };
    let found = find_socket(&environment, |path: &Path| path.starts_with("/nowhere/root"))
        .expect("a socket under the root");
    assert!(found.starts_with("/nowhere/root"));
}

#[test]
fn no_socket_lists_every_place_tried() {
    let missing = find_socket(&home_environment(), |_| false);
    assert!(matches!(missing, Err(DaemonError::NotRunning { tried }) if tried.len() > 1));
}

#[test]
fn no_home_and_no_humata_socket_is_an_error() {
    assert!(matches!(
        find_socket(&Environment::default(), |_| true),
        Err(DaemonError::NoHome)
    ));
}
