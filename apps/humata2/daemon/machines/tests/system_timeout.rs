//! A command that runs past its timeout is killed with everything it
//! started. Tagged `slow`: it waits on a real clock.

use std::time::{Duration, Instant};

use humata2_machines::{Exit, Invocation, Purpose, Runner, SystemRunner};

#[test]
fn a_command_past_its_timeout_is_killed_with_its_children() {
    let invocation = Invocation {
        program: "/bin/sh".to_owned(),
        // The background sleep holds stdout open; killing only the shell
        // would leave the runner waiting for it.
        args: vec!["-c".to_owned(), "sleep 30 & sleep 30".to_owned()],
        env: Vec::new(),
        stdin: None,
        timeout: Some(Duration::from_millis(300)),
        purpose: Purpose::Probe,
    };
    let started = Instant::now();

    let output = SystemRunner::new(&()).run(&invocation).unwrap();

    assert_eq!(output.exit, Exit::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
}
