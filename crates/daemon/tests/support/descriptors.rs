//! What this test process itself still holds open, read from its own descriptor table.
//!
//! Under libtest every test of a binary is a thread of one process, and sibling threads
//! fork (every PTY window, git with a stdin file, `pre_exec` spawns). A forked child holds
//! a copy of each of this process's descriptors until it execs, and on macOS a
//! `posix_spawn` that lands between `pipe`/`socket` and `FD_CLOEXEC` keeps its copy for
//! the child's whole life. So "a fresh `flock` succeeds" or "a `connect` is refused" asks
//! whether *any process on the machine* still holds the file description, which a test
//! does not control. These helpers ask what the code under test does control: whether a
//! descriptor in this process still refers to the object (flake diagnosis, 2026-10-18).
//!
//! `/dev/fd` is a per-process listing on macOS and a symlink to `/proc/self/fd` on Linux.
//! The directory handle `read_dir` opens is itself listed; it is closed once the names are
//! collected, so its `fstat` fails with `EBADF` and is skipped like any descriptor another
//! thread closes meanwhile.

use std::net::SocketAddr;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// Every descriptor number currently listed for this process.
fn open_descriptors() -> Vec<libc::c_int> {
    std::fs::read_dir("/dev/fd")
        .expect("listing /dev/fd")
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
        .collect()
}

/// The descriptors of this process that refer to the same file as `path`, compared by
/// `(st_dev, st_ino)`, so a rename or a second hard link cannot hide one.
pub fn descriptors_on(path: &Path) -> Vec<libc::c_int> {
    let meta = std::fs::metadata(path).unwrap_or_else(|e| panic!("stat {}: {e}", path.display()));
    let want = (meta.dev(), meta.ino());
    open_descriptors()
        .into_iter()
        .filter(|&fd| {
            let mut st = std::mem::MaybeUninit::<libc::stat>::uninit();
            // SAFETY: `fstat` writes a `stat` into the buffer it is given and reads
            // nothing else; a closed or reused number only yields `EBADF` or another file.
            if unsafe { libc::fstat(fd, st.as_mut_ptr()) } != 0 {
                return false;
            }
            // SAFETY: `fstat` returned 0, so it filled the buffer.
            let st = unsafe { st.assume_init() };
            // `dev_t` is `i32` on macOS and `u64` on Linux; `MetadataExt::dev` widens it
            // the same way.
            #[allow(clippy::unnecessary_cast)]
            let got = (st.st_dev as u64, st.st_ino as u64);
            got == want
        })
        .collect()
}

/// The descriptors of this process that are sockets bound to `addr` (`getsockname`). A
/// listener and every connection it accepted both answer with the listening address.
pub fn sockets_bound_to(addr: SocketAddr) -> Vec<libc::c_int> {
    open_descriptors()
        .into_iter()
        .filter(|&fd| local_addr(fd) == Some(addr))
        .collect()
}

/// `getsockname` of `fd` as a `SocketAddr`, or `None` for a non-socket, a non-IP socket,
/// or a descriptor that is no longer open.
fn local_addr(fd: libc::c_int) -> Option<SocketAddr> {
    let mut storage = std::mem::MaybeUninit::<libc::sockaddr_storage>::zeroed();
    let mut len = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
    // SAFETY: the buffer is a zeroed `sockaddr_storage` and `len` is its size, so the
    // kernel writes at most that many bytes; any failure (ENOTSOCK, EBADF) returns early.
    if unsafe { libc::getsockname(fd, storage.as_mut_ptr().cast(), &mut len) } != 0 {
        return None;
    }
    // SAFETY: zero-initialised and partly filled by the kernel; every bit pattern is valid.
    let storage = unsafe { storage.assume_init() };
    match storage.ss_family as libc::c_int {
        libc::AF_INET => {
            // SAFETY: the family says the storage holds a `sockaddr_in`.
            let sin: libc::sockaddr_in = unsafe { std::ptr::read((&raw const storage).cast()) };
            let ip = std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
            Some(SocketAddr::from((ip, u16::from_be(sin.sin_port))))
        }
        libc::AF_INET6 => {
            // SAFETY: the family says the storage holds a `sockaddr_in6`.
            let sin6: libc::sockaddr_in6 = unsafe { std::ptr::read((&raw const storage).cast()) };
            let ip = std::net::Ipv6Addr::from(sin6.sin6_addr.s6_addr);
            Some(SocketAddr::from((ip, u16::from_be(sin6.sin6_port))))
        }
        _ => None,
    }
}
