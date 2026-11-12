//! A terminal on a host reached over SSH: a command typed and its output
//! read, the connection dropped while it prints, and the output still there
//! when it's read again.

use ssh_host::{DetachedHost, cut_ssh, daemon_with_host, eventually, sshd, succeeds};

mod common;
mod ssh_host;

#[test]
fn a_terminal_on_a_host_keeps_its_output_through_a_dropped_connection() {
    let scratch = common::scratch();
    let root = scratch.path();
    let sshd = sshd(root);
    let folder = root.join("far");
    let _host = DetachedHost(&folder);
    let (_daemon, humata) = daemon_with_host(root, &sshd, "far", &folder);

    let opened = humata.run(&["terminal", "open", "--host", "far"]);
    let terminal = opened.split('\t').next().unwrap_or_default().to_owned();
    assert!(!terminal.is_empty(), "{opened}");
    let on_far = |arguments: &[&str]| {
        let mut all = arguments.to_vec();
        all.extend(["--host", "far"]);
        humata.run(&all)
    };

    // Typed, and read back: the arithmetic is the shell's, not the echo's.
    on_far(&["terminal", "type", &terminal, "echo made-$((6*7))"]);
    let shown = on_far(&["terminal", "read", &terminal, "--until", "made-42"]);
    assert!(shown.contains("made-42"), "{shown}");

    // It prints while the connection is down, and that's kept.
    on_far(&[
        "terminal",
        "type",
        &terminal,
        "sleep 2; echo while-$((2+3))",
    ]);
    assert!(cut_ssh(&folder) > 0, "no ssh reached the host");
    eventually("the terminal read again after the drop", || {
        succeeds(
            &humata,
            &[
                "terminal", "read", &terminal, "--host", "far", "--until", "while-5",
            ],
        )
    });
    let shown = on_far(&["terminal", "read", &terminal]);
    assert!(
        shown.contains("made-42") && shown.contains("while-5"),
        "{shown}"
    );
    assert!(on_far(&["terminal", "list"]).contains(&format!("{terminal}\tfar")));

    // Exiting is shown, and closing forgets it.
    on_far(&["terminal", "type", &terminal, "exit 3"]);
    eventually("the shell exited", || {
        on_far(&["terminal", "list"]).contains("exited 3")
    });
    on_far(&["terminal", "close", &terminal]);
    assert_eq!(on_far(&["terminal", "list"]), "");
}
