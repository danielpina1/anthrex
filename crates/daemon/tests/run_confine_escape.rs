//! M8a final fix batch F1d (F1c re-review 2, R1, R2, R4, M2): a confined check, proof or
//! `setup` runs under a deny-by-default profile. It cannot hand work to an unconfined
//! actor (LaunchServices, cfprefsd, the keychain, AppleEvents, launchd), cannot reach
//! the network (localhost included) unless the user's config enables it for the
//! repository, may still allocate and use its own PTYs but never another terminal's,
//! and its `TMPDIR` is short enough for a Unix socket.
//!
//! Every payload is harmless and aimed inside this test's own temporary directories.
//! Anything a payload could register outside them (an app with LaunchServices, a
//! preference domain) is removed by its exact name, whether or not it got through.

#![cfg(target_os = "macos")]

mod support;

use daemon::run::exec::run_confined;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use support::confine::world;

const LONG: Duration = Duration::from_secs(60);
/// How long a test watches for a marker that only an escaped process would write,
/// after the confined command itself has returned (docs/timing-budgets.md, F1d row):
/// LaunchServices starts an app asynchronously after `open` returns.
const ESCAPE_WINDOW: Duration = Duration::from_secs(3);

const LSREGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

/// Whether `marker` appears within [`ESCAPE_WINDOW`] (a deadline loop: it returns as
/// soon as it does).
fn appears(marker: &Path) -> bool {
    let deadline = Instant::now() + ESCAPE_WINDOW;
    loop {
        if marker.exists() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A two-file application bundle at `app` whose executable touches `marker`.
fn build_app(app: &Path, id: &str, marker: &Path) {
    let macos = app.join("Contents/MacOS");
    std::fs::create_dir_all(&macos).unwrap();
    let exe = macos.join("probe");
    std::fs::write(
        &exe,
        format!("#!/bin/sh\n/usr/bin/touch '{}'\n", marker.display()),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        app.join("Contents/Info.plist"),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>CFBundleExecutable</key><string>probe</string><key>CFBundleIdentifier</key><string>{id}</string><key>CFBundlePackageType</key><string>APPL</string></dict></plist>\n"
        ),
    )
    .unwrap();
}

/// R1: `open` of an app the check built must not start it. LaunchServices would start
/// it as a child of launchd, outside the sandbox, with the user's full rights.
#[test]
fn a_confined_check_cannot_open_an_app_it_built() {
    let w = world();
    let task = w.task();
    let confinement = w.spec(&[]).for_checkout(&task).unwrap();
    let app = task.join("Probe.app");
    let marker = w.outside.join("launched");
    let id = format!("com.anthrex.f1d-test.open.{}", std::process::id());
    build_app(&app, &id, &marker);
    let command = format!("open -g -n '{}'; echo open=$?", app.display());

    let outcome = run_confined(&task, &command, &[], LONG, Some(&confinement));
    let escaped = appears(&marker);
    // Whatever happened, forget the bundle by its exact path.
    let _ = std::process::Command::new(LSREGISTER)
        .arg("-u")
        .arg(&app)
        .output();
    assert!(
        !escaped,
        "an app opened from a confined check ran outside it: {outcome:?}"
    );
    assert!(!outcome.tail.contains("open=0"), "{}", outcome.tail);
}

/// R1: `defaults write` goes through cfprefsd, which writes the plist for the caller.
/// A confined check must not write any preference domain (a terminal's startup
/// command is one).
#[test]
fn a_confined_check_cannot_write_preferences() {
    let w = world();
    let task = w.task();
    let confinement = w.spec(&[]).for_checkout(&task).unwrap();
    let domain = format!("com.anthrex.f1d-test.prefs.{}", std::process::id());
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    let plist = home.join(format!("Library/Preferences/{domain}.plist"));
    let command = format!("defaults write {domain} probe yes; echo write=$?");

    let outcome = run_confined(&task, &command, &[], LONG, Some(&confinement));
    let read = std::process::Command::new("defaults")
        .args(["read", &domain, "probe"])
        .output()
        .unwrap();
    // Remove the test domain by its exact name, whether or not it was written.
    let _ = std::process::Command::new("defaults")
        .args(["delete", &domain])
        .output();
    let _ = std::fs::remove_file(&plist);
    assert!(
        !read.status.success(),
        "a confined check wrote the preference domain {domain}: {outcome:?}"
    );
    assert!(outcome.tail.contains("write=1"), "{}", outcome.tail);
}

/// A Python probe that looks up each Mach service by name and prints `<name> <kr>`
/// (0: found).
fn lookup_probe(names: &[&str]) -> String {
    let list: Vec<String> = names.iter().map(|n| format!("{n:?}")).collect();
    format!(
        "python3 - <<'PY'\n\
         import ctypes\n\
         lib=ctypes.CDLL('/usr/lib/libSystem.B.dylib')\n\
         bp=ctypes.c_uint.in_dll(lib,'bootstrap_port')\n\
         for n in [{}]:\n\
         \x20 p=ctypes.c_uint(0)\n\
         \x20 print(n, lib.bootstrap_look_up(bp, n.encode(), ctypes.byref(p)))\n\
         PY",
        list.join(",")
    )
}

/// R1, R4: the keychain (securityd), LaunchServices and AppleEvents are out of reach;
/// Directory Services, which every toolchain needs to look up the user, is not.
#[test]
fn a_confined_check_cannot_reach_the_keychain_launchservices_or_appleevents() {
    let w = world();
    let task = w.task();
    let confinement = w.spec(&[]).for_checkout(&task).unwrap();
    let denied = [
        "com.apple.SecurityServer",
        "com.apple.securityd.xpc",
        "com.apple.coreservices.launchservicesd",
        "com.apple.lsd.mapdb",
        "com.apple.coreservices.appleevents",
        "com.apple.CoreServices.coreservicesd",
    ];
    let allowed = "com.apple.system.opendirectoryd.libinfo";
    let mut names = denied.to_vec();
    names.push(allowed);
    let probe = lookup_probe(&names);

    // Unconfined, the probe finds them all: it is live.
    let open = run_confined(&task, &probe, &[], LONG, None);
    for name in &names {
        assert!(open.tail.contains(&format!("{name} 0")), "{}", open.tail);
    }
    let outcome = run_confined(&task, &probe, &[], LONG, Some(&confinement));
    assert!(outcome.ok, "{outcome:?}");
    for name in denied {
        assert!(
            !outcome.tail.contains(&format!("{name} 0")),
            "a confined check looked up {name}: {}",
            outcome.tail
        );
    }
    assert!(
        outcome.tail.contains(&format!("{allowed} 0")),
        "{}",
        outcome.tail
    );
}

/// R4: without the user's `confined_network`, a confined check reaches no network,
/// localhost included: a TCP listener and a UDP socket this test owns on 127.0.0.1 see
/// nothing.
#[test]
fn a_confined_check_cannot_reach_localhost() {
    let w = world();
    let task = w.task();
    let confinement = w.spec(&[]).for_checkout(&task).unwrap();
    let tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    tcp.set_nonblocking(true).unwrap();
    let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    udp.set_nonblocking(true).unwrap();
    let command = format!(
        "python3 - <<'PY'\n\
         import socket\n\
         s=socket.socket(); s.settimeout(5)\n\
         try:\n\
         \x20 s.connect(('127.0.0.1',{tcp})); print('tcp connected')\n\
         except Exception as e:\n\
         \x20 print('tcp denied', e)\n\
         u=socket.socket(socket.AF_INET, socket.SOCK_DGRAM)\n\
         try:\n\
         \x20 u.sendto(b'x',('127.0.0.1',{udp})); print('udp sent')\n\
         except Exception as e:\n\
         \x20 print('udp denied', e)\n\
         PY",
        tcp = tcp.local_addr().unwrap().port(),
        udp = udp.local_addr().unwrap().port(),
    );
    let outcome = run_confined(&task, &command, &[], LONG, Some(&confinement));
    assert!(outcome.tail.contains("tcp denied"), "{}", outcome.tail);
    assert!(outcome.tail.contains("udp denied"), "{}", outcome.tail);
    assert!(
        matches!(tcp.accept(), Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock),
        "the test's TCP listener saw a connection from the confined check"
    );
    let mut buf = [0u8; 4];
    assert!(
        matches!(udp.recv(&mut buf), Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock),
        "the test's UDP socket got a datagram from the confined check"
    );
}

/// R2: a test suite that allocates a PTY (pexpect, `portable-pty`, `script(1)`, anthrex's
/// own) works under confinement.
#[test]
fn a_confined_check_can_open_a_pty() {
    let w = world();
    let task = w.task();
    let confinement = w.spec(&[]).for_checkout(&task).unwrap();
    let command = "python3 -c 'import os,pty; m,s=pty.openpty(); os.write(s,b\"hi\"); print(\"pty\", os.read(m,2).decode())' && script -q /dev/null echo script-ok";
    let outcome = run_confined(&task, command, &[], LONG, Some(&confinement));
    assert!(outcome.ok, "{outcome:?}");
    assert!(outcome.tail.contains("pty hi"), "{}", outcome.tail);
    assert!(outcome.tail.contains("script-ok"), "{}", outcome.tail);
}

/// R2's residual, closed: a PTY the check did not create itself (another terminal's)
/// can be neither read nor written.
#[test]
fn a_confined_check_cannot_open_another_terminal() {
    let Some(tty) = std::fs::read_dir("/dev").unwrap().flatten().find_map(|e| {
        let name = e.file_name().into_string().ok()?;
        let digits = name.strip_prefix("ttys")?;
        (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
            .then(|| e.path())
            .filter(|p| std::fs::File::open(p).is_ok())
    }) else {
        eprintln!("no terminal of this user's to probe; skipped");
        return;
    };
    let w = world();
    let task = w.task();
    let confinement = w.spec(&[]).for_checkout(&task).unwrap();
    let command = format!(
        "python3 - <<'PY'\n\
         for mode in ('rb','ab'):\n\
         \x20 try:\n\
         \x20  open({tty:?}, mode, buffering=0); print('opened', mode)\n\
         \x20 except Exception as e:\n\
         \x20  print('denied', mode)\n\
         PY",
        tty = tty.display().to_string()
    );
    let outcome = run_confined(&task, &command, &[], LONG, Some(&confinement));
    assert!(outcome.tail.contains("denied rb"), "{}", outcome.tail);
    assert!(outcome.tail.contains("denied ab"), "{}", outcome.tail);
}

/// F1c re-review 2, M2: a check's `TMPDIR` is short, so a test that binds a Unix socket
/// in a temporary directory of its own (`tempfile`, `mkdtemp`) fits in `sun_path`
/// (104 bytes on macOS). It is never the daemon's own `$TMPDIR/anthrex-<uid>`.
#[test]
fn a_confined_checks_tmpdir_holds_a_unix_socket() {
    let w = world();
    let task = w.task();
    let confinement = w.spec(&[]).for_checkout(&task).unwrap();
    let tmp = confinement.tmp().display().to_string();
    assert!(tmp.len() <= 48, "TMPDIR is {} bytes: {tmp}", tmp.len());
    assert!(
        !confinement.tmp().starts_with(
            proto::paths::socket_dir()
                .canonicalize()
                .unwrap_or_default()
        ),
        "{tmp}"
    );
    let command = "python3 - <<'PY'\n\
         import socket, tempfile, os\n\
         d=tempfile.mkdtemp()\n\
         p=os.path.join(d, 'a-socket-name-of-thirty-bytes.sock')\n\
         s=socket.socket(socket.AF_UNIX)\n\
         s.bind(p); s.listen(); c=socket.socket(socket.AF_UNIX); c.connect(p)\n\
         print('bound', len(p))\n\
         PY";
    let outcome = run_confined(&task, command, &[], LONG, Some(&confinement));
    assert!(outcome.ok, "{outcome:?}");
    assert!(outcome.tail.contains("bound"), "{}", outcome.tail);
}

/// R4: with the user's `confined_network` for the repository, a confined check reaches
/// localhost and a Unix socket outside its directories (a local database, Docker), but
/// never the anthrex daemon's socket, and never the keychain.
#[test]
fn confined_network_opens_the_network_but_never_the_daemon_or_the_keychain() {
    use std::os::unix::net::UnixListener;
    let w = world();
    let task = w.task();
    let mut spec = w.spec(&[]);
    spec.network = true;
    let confinement = spec.for_checkout(&task).unwrap();
    let tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    // A short base path: a Unix socket address must fit in ~104 bytes.
    let base = PathBuf::from(format!("/tmp/axnet-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let service = base.join("db.sock");
    let _service = UnixListener::bind(&service).unwrap();
    let daemon = UnixListener::bind(&spec.daemon_socket).unwrap();
    daemon.set_nonblocking(true).unwrap();
    let command = format!(
        "python3 - <<'PY'\n\
         import socket\n\
         s=socket.socket(); s.settimeout(5)\n\
         try:\n\
         \x20 s.connect(('127.0.0.1',{tcp})); print('tcp connected')\n\
         except Exception as e:\n\
         \x20 print('tcp denied', e)\n\
         for name, p in [('service', {service:?}), ('daemon', {daemon:?})]:\n\
         \x20 u=socket.socket(socket.AF_UNIX)\n\
         \x20 try:\n\
         \x20  u.connect(p); print(name, 'connected')\n\
         \x20 except Exception as e:\n\
         \x20  print(name, 'denied')\n\
         PY\n{keychain}",
        tcp = tcp.local_addr().unwrap().port(),
        service = service.display().to_string(),
        daemon = spec.daemon_socket.display().to_string(),
        keychain = lookup_probe(&["com.apple.SecurityServer"]),
    );
    let outcome = run_confined(&task, &command, &[], LONG, Some(&confinement));
    let _ = std::fs::remove_dir_all(&base);
    assert!(outcome.tail.contains("tcp connected"), "{}", outcome.tail);
    assert!(
        outcome.tail.contains("service connected"),
        "{}",
        outcome.tail
    );
    assert!(outcome.tail.contains("daemon denied"), "{}", outcome.tail);
    assert!(
        !outcome.tail.contains("com.apple.SecurityServer 0"),
        "{}",
        outcome.tail
    );
    assert!(
        matches!(daemon.accept(), Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock),
        "the daemon's socket saw a connection from the confined check"
    );
}

/// R3: a `cache_dirs` entry whose path runs through a link a worker or an earlier check
/// planted in its checkout is refused, and nothing is granted through it.
#[test]
fn a_cache_dir_through_a_planted_link_is_refused() {
    let w = world();
    let task = w.task();
    let target = w.outside.join("launchagents");
    std::fs::create_dir_all(&target).unwrap();
    std::os::unix::fs::symlink(&target, task.join(".cache")).unwrap();
    let err = w
        .spec(&[&task.join(".cache/pip")])
        .for_checkout(&task)
        .unwrap_err();
    assert!(err.contains("runs through the link"), "{err}");
    // The entry itself swapped for a link, by a check it was granted to.
    let cache = w.outside.join("cache");
    std::os::unix::fs::symlink(&target, &cache).unwrap();
    let err = w.spec(&[&cache]).for_checkout(&task).unwrap_err();
    assert!(err.contains("itself a link"), "{err}");
}
