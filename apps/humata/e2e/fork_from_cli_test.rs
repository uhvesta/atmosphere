//! A session on a remote host reached by running `humatad host --stdio`
//! directly, as SSH would.

mod common;
mod fork;

#[test]
fn a_session_on_a_remote_host_forks_at_an_earlier_event() {
    let scratch = common::scratch();
    let humatad = common::binary("HUMATAD");
    let state = scratch.path().join("remote");
    let command = vec![
        humatad.display().to_string(),
        "host".to_owned(),
        "--stdio".to_owned(),
        "--state".to_owned(),
        state.display().to_string(),
    ];
    fork::fork_on_remote(scratch.path(), command);
}
