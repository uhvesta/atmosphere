//! A port on a host reached over SSH, forwarded to 127.0.0.1 here: a web
//! server there answers here, through the host's own connection.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use ssh_host::{DetachedHost, daemon_with_host, sshd};

mod common;
mod ssh_host;

/// A web server on 127.0.0.1 that answers every request with `body`.
fn web_server(body: &'static str) -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap_or_else(|error| panic!("{error}"));
    let port = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("{error}"))
        .port();
    std::thread::spawn(move || {
        for mut socket in listener.incoming().flatten() {
            let mut asked = [0; 1024];
            let _ = socket.read(&mut asked);
            let reply = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(reply.as_bytes());
        }
    });
    port
}

fn fetched(port: &str) -> String {
    let address = format!("127.0.0.1:{port}");
    let mut socket = TcpStream::connect(&address).unwrap_or_else(|error| panic!("{error}"));
    socket
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap_or_else(|error| panic!("{error}"));
    socket
        .write_all(b"GET / HTTP/1.1\r\nhost: localhost\r\n\r\n")
        .unwrap_or_else(|error| panic!("{error}"));
    let mut reply = String::new();
    socket
        .read_to_string(&mut reply)
        .unwrap_or_else(|error| panic!("{error}"));
    reply
}

#[test]
fn a_port_on_a_host_answers_here_until_the_forward_stops() {
    let scratch = common::scratch();
    let root = scratch.path();
    let sshd = sshd(root);
    let folder = root.join("far");
    let _host = DetachedHost(&folder);
    let (_daemon, humata) = daemon_with_host(root, &sshd, "far", &folder);
    let port = web_server("served on far").to_string();

    let forwarded = humata.run(&["port", "forward", &port, "--host", "far"]);
    let fields: Vec<&str> = forwarded.trim().split('\t').collect();
    let (id, local) = match fields.as_slice() {
        [id, "far", there, here, _] if *there == port => (
            (*id).to_owned(),
            here.trim_start_matches("127.0.0.1:").to_owned(),
        ),
        _ => panic!("not forwarded: {forwarded}"),
    };
    for _ in 0..2 {
        let reply = fetched(&local);
        assert!(reply.ends_with("served on far"), "{reply}");
    }
    assert!(
        humata
            .run(&["port", "list"])
            .starts_with(&format!("{id}\tfar"))
    );

    humata.run(&["port", "stop", &id]);
    assert_eq!(humata.run(&["port", "list"]), "");
}
