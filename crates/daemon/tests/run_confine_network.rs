//! M8a final fix batch F1d round 2 (F1d re-review 1, S1): what the user's
//! `confined_network` opens for a confined check, proof or `setup`: outbound IP to
//! remote hosts (and DNS), never a Unix socket outside its own directories, never a
//! loopback or local-interface service. A Unix socket the user lists in
//! `confined_unix_sockets`, and a loopback port listed in `confined_localhost_ports`, are
//! the only ways in, and the anthrex daemon's socket is never one.
//!
//! Every listener is this test's own, under a short `/tmp` path or on a loopback port.

#![cfg(target_os = "macos")]

mod support;

use daemon::run::exec::run_confined;
use std::net::TcpListener;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::time::Duration;
use support::confine::{lookup_probe, world};

const LONG: Duration = Duration::from_secs(60);

/// A short directory of this test's own under `/tmp` (a Unix socket address must fit in
/// about 104 bytes), removed on drop.
struct Short(PathBuf);

impl Short {
    fn new(tag: &str) -> Self {
        let dir = PathBuf::from(format!("/tmp/ax{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Short(dir)
    }
}

impl Drop for Short {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A Python probe that tries each target and prints `<name> connected|denied`.
/// `unix` targets are paths; `tcp` targets are loopback ports.
fn connect_probe(unix: &[(&str, &PathBuf)], tcp: &[(&str, u16)]) -> String {
    let mut targets = Vec::new();
    for (name, path) in unix {
        targets.push(format!(
            "({name:?}, socket.AF_UNIX, {:?})",
            path.display().to_string()
        ));
    }
    for (name, port) in tcp {
        targets.push(format!("({name:?}, socket.AF_INET, ('127.0.0.1', {port}))"));
    }
    format!(
        "python3 - <<'PY'\n\
         import socket\n\
         for name, fam, addr in [{}]:\n\
         \x20 s=socket.socket(fam); s.settimeout(5)\n\
         \x20 try:\n\
         \x20  s.connect(addr); print(name, 'connected')\n\
         \x20 except Exception as e:\n\
         \x20  print(name, 'denied')\n\
         PY\n",
        targets.join(",")
    )
}

fn nonblocking_unix(path: &PathBuf) -> UnixListener {
    let listener = UnixListener::bind(path).unwrap();
    listener.set_nonblocking(true).unwrap();
    listener
}

fn nonblocking_tcp() -> TcpListener {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    listener
}

fn untouched_unix(listener: &UnixListener) -> bool {
    matches!(listener.accept(), Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock)
}

fn untouched_tcp(listener: &TcpListener) -> bool {
    matches!(listener.accept(), Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock)
}

/// S1: `confined_network` alone opens neither a Unix socket outside the check's
/// directories (tmux, Docker, an IDE, another anthrex daemon) nor a loopback service
/// (an unauthenticated redis), nor the daemon's socket, nor the keychain.
#[test]
fn confined_network_alone_reaches_no_local_service() {
    let w = world();
    let task = w.task();
    let mut spec = w.spec(&[]);
    spec.network = true;
    let confinement = spec.for_checkout(&task).unwrap();
    let short = Short::new("n1");
    let service = short.0.join("svc.sock");
    let service_listener = nonblocking_unix(&service);
    let daemon = nonblocking_unix(&spec.daemon_socket);
    let tcp = nonblocking_tcp();
    let command = format!(
        "{}{}",
        connect_probe(
            &[("service", &service), ("daemon", &spec.daemon_socket)],
            &[("loopback", tcp.local_addr().unwrap().port())],
        ),
        lookup_probe(&["com.apple.SecurityServer"])
    );

    let outcome = run_confined(&task, &command, &[], LONG, Some(&confinement));
    for name in ["service", "daemon", "loopback"] {
        assert!(
            outcome.tail.contains(&format!("{name} denied")),
            "{name}: {}",
            outcome.tail
        );
    }
    assert!(
        !outcome.tail.contains("com.apple.SecurityServer 0"),
        "{}",
        outcome.tail
    );
    assert!(
        untouched_unix(&service_listener),
        "the service saw the check"
    );
    assert!(untouched_unix(&daemon), "the daemon's socket saw the check");
    assert!(untouched_tcp(&tcp), "the loopback service saw the check");
}

/// S1: remote TCP stays open with `confined_network`. A remote host cannot be reached
/// from the test suite, and a listener on this machine's own non-loopback address is
/// treated as local by seatbelt (denied, which is the point), so the profile's rule is
/// asserted instead, and DNS resolution (through mDNSResponder) is exercised for real.
#[test]
fn confined_network_keeps_remote_ip_and_name_resolution() {
    let w = world();
    let task = w.task();
    let mut spec = w.spec(&[]);
    spec.network = true;
    let confinement = spec.for_checkout(&task).unwrap();
    let profile = confinement.profile().unwrap();
    assert!(
        profile.contains("(allow network-outbound (remote ip \"*:*\"))"),
        "{profile}"
    );
    let allow = profile.find("(remote ip \"*:*\")").unwrap();
    let deny = profile
        .find("(deny network-outbound (remote ip \"localhost:*\"))")
        .unwrap_or_else(|| panic!("{profile}"));
    assert!(
        deny > allow,
        "the loopback deny must follow the allow: {profile}"
    );
    assert!(!profile.contains("(allow network*)"), "{profile}");
    let command =
        "python3 -c \"import socket; socket.getaddrinfo('localhost', 80); print('resolved')\"";
    let outcome = run_confined(&task, command, &[], LONG, Some(&confinement));
    assert!(outcome.tail.contains("resolved"), "{}", outcome.tail);

    // Off, the profile has no IP rule at all.
    let off = w.spec(&[]).for_checkout(&task).unwrap().profile().unwrap();
    assert!(!off.contains("(remote ip"), "{off}");
}

/// S1: a Unix socket the user lists in `confined_unix_sockets` and a loopback port
/// listed in `confined_localhost_ports` are reachable, with or without
/// `confined_network`; one not listed stays denied, and so does the daemon.
#[test]
fn listed_unix_sockets_and_localhost_ports_are_the_only_local_services() {
    let w = world();
    let task = w.task();
    let short = Short::new("n2");
    let listed = short.0.join("db.sock");
    let other = short.0.join("other.sock");
    let _listed = nonblocking_unix(&listed);
    let other_listener = nonblocking_unix(&other);
    let daemon = nonblocking_unix(&w.spec(&[]).daemon_socket);
    let open_port = nonblocking_tcp();
    let closed_port = nonblocking_tcp();
    for network in [false, true] {
        let mut spec = w.spec(&[]);
        spec.network = network;
        spec.unix_sockets = vec![listed.display().to_string()];
        spec.localhost_ports = vec![open_port.local_addr().unwrap().port()];
        let confinement = spec.for_checkout(&task).unwrap();
        let command = connect_probe(
            &[
                ("listed", &listed),
                ("other", &other),
                ("daemon", &spec.daemon_socket),
            ],
            &[
                ("open-port", open_port.local_addr().unwrap().port()),
                ("closed-port", closed_port.local_addr().unwrap().port()),
            ],
        );
        let outcome = run_confined(&task, &command, &[], LONG, Some(&confinement));
        for (name, verdict) in [
            ("listed", "connected"),
            ("other", "denied"),
            ("daemon", "denied"),
            ("open-port", "connected"),
            ("closed-port", "denied"),
        ] {
            assert!(
                outcome.tail.contains(&format!("{name} {verdict}")),
                "network={network}, {name}: {}",
                outcome.tail
            );
        }
    }
    assert!(untouched_unix(&other_listener));
    assert!(untouched_unix(&daemon));
    assert!(untouched_tcp(&closed_port));

    // The daemon's socket, and its directory, cannot be listed.
    let mut spec = w.spec(&[]);
    spec.unix_sockets = vec![spec.daemon_socket.display().to_string()];
    let err = spec.for_checkout(&task).unwrap_err();
    assert!(err.contains("daemon"), "{err}");
}
