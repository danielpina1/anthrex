//! Task 4's review, I2: when a test daemon is ready. `bind(2)` creates a socket file and
//! `listen(2)` follows it; between the two a `connect(2)` is refused. The daemon now puts
//! its socket in place only once it listens (`lifecycle::bind_socket`), except at a path
//! whose staging name does not fit `sun_path`; every harness still waits for [`accepts`],
//! the property itself, as a second guard.

use std::path::Path;

/// Whether `socket` accepts a connection now (the probe connection is dropped at once;
/// the daemon reads it as a client that left before its `Hello`). As
/// `DaemonProcess::wait_up` checks.
pub fn accepts(socket: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(socket).is_ok()
}
