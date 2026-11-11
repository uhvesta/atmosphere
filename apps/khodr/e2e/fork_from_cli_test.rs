//! A session on a remote host reached by running `khodrd host --stdio`
//! directly, as SSH would.

mod common;

#[test]
fn a_session_on_a_remote_host_forks_at_an_earlier_event() {
    let scratch = common::scratch();
    let khodrd = common::binary("KHODRD");
    let state = scratch.path().join("remote");
    let command = vec![
        khodrd.display().to_string(),
        "host".to_owned(),
        "--stdio".to_owned(),
        "--state".to_owned(),
        state.display().to_string(),
    ];
    common::fork_on_remote(scratch.path(), command);
}
