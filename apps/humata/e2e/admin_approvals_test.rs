//! A client other than the app, such as an agent's `humata`, asks for an
//! admin action: it waits for the person's approval in Humata and changes
//! nothing until then. Only a dev build's `humata mcp` has admin tools.

mod common;

#[test]
fn an_outside_clients_admin_action_waits_for_approval() {
    let root = common::scratch();
    let repository = root.path().join("repository");
    common::git_repository(&repository, &[("README.md", "hello\n")]);
    let (_daemon, app) = common::start(root.path(), &[]);
    let project = app
        .run(&[
            "project",
            "add",
            "--name",
            "check",
            "--repository",
            &repository.display().to_string(),
        ])
        .trim()
        .to_owned();

    let outside = app.other(common::binary("HUMATA_ADMIN"));
    outside.fails(&["--no-wait", "item", "add", "--project", &project, "Ship it"]);
    assert!(
        app.run(&["item", "list", "--project", &project])
            .trim()
            .is_empty()
    );

    let listed = outside.run(&["approval", "list"]);
    let pending: Vec<&str> = listed
        .lines()
        .filter(|line| line.split('\t').nth(1) == Some("pending"))
        .collect();
    assert_eq!(pending.len(), 1, "{listed}");
    let id = pending[0].split('\t').next().unwrap_or_default();
    let status = outside.run(&["approval", "status", id]);
    assert_eq!(status.split('\t').nth(1), Some("pending"), "{status}");

    let list = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
    let admin_tools = outside.with_input(&["mcp"], list);
    assert!(admin_tools.contains("\"create_item\""), "{admin_tools}");
    let release_tools = app.with_input(&["mcp"], list);
    assert!(
        release_tools.contains("\"propose_task\""),
        "{release_tools}"
    );
    assert!(
        !release_tools.contains("\"create_item\""),
        "{release_tools}"
    );
}
