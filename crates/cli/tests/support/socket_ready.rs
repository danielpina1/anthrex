//! Task 4's review, I2: when a test daemon is ready. `bind(2)` creates the socket file
//! and `listen(2)` follows it; between the two a `connect(2)` is refused. So "the file
//! exists" is not "the daemon accepts", and a test that connects right after the former
//! can be refused under load. Every harness waits for [`accepts`] instead.

use std::path::Path;

/// Whether `socket` accepts a connection now (the probe connection is dropped at once;
/// the daemon reads it as a client that left before its `Hello`). As
/// `DaemonProcess::wait_up` checks.
pub fn accepts(socket: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(socket).is_ok()
}
