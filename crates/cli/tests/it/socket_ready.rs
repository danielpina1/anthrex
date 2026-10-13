//! Task 4's review, I2: a socket that is bound but not yet listening exists as a file
//! and refuses a connection. The harnesses' readiness check must say "not ready" then.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use crate::support::socket_ready::accepts;

/// A Unix stream socket bound at `path` and not listening: the daemon's state between
/// `bind(2)` and `listen(2)`. Its descriptor.
fn bound_only(path: &Path) -> libc::c_int {
    let bytes = CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: plain libc calls on a descriptor this function owns; `addr` is zeroed and
    // `sun_path` gets at most its length minus one bytes, so it stays NUL-terminated.
    unsafe {
        let fd = libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0);
        assert!(fd >= 0, "socket(2)");
        let mut addr: libc::sockaddr_un = std::mem::zeroed();
        addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
        let bytes = bytes.as_bytes();
        assert!(bytes.len() < addr.sun_path.len(), "a short path");
        for (to, from) in addr.sun_path.iter_mut().zip(bytes) {
            *to = *from as libc::c_char;
        }
        let len = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
        let bound = libc::bind(fd, &addr as *const _ as *const libc::sockaddr, len);
        assert_eq!(bound, 0, "bind(2): {}", std::io::Error::last_os_error());
        fd
    }
}

#[test]
fn a_bound_socket_that_does_not_listen_is_not_ready() {
    let dir = tempfile::Builder::new()
        .prefix("ax-sock")
        .tempdir_in("/tmp")
        .unwrap();
    let path = dir.path().join("d.sock");
    assert!(!accepts(&path), "no file yet");
    let fd = bound_only(&path);
    assert!(path.exists(), "bind(2) made the file");
    assert!(!accepts(&path), "bound but not listening");
    // SAFETY: `fd` is the bound socket above, still open.
    assert_eq!(unsafe { libc::listen(fd, 8) }, 0);
    assert!(accepts(&path), "listening");
    // SAFETY: closes the descriptor this test opened, once.
    unsafe { libc::close(fd) };
}
