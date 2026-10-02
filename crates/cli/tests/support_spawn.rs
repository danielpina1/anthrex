//! The test harness's own spawn (the final fix wave, ETXTBSY): a spawn that fails with
//! `ExecutableFileBusy` is retried within its short deadline, any other failure is not,
//! and the deadline is kept.

mod support;

use std::io;
use std::time::{Duration, Instant};

use support::{SPAWN_BUSY_WAIT, retry_busy};

fn busy() -> io::Error {
    io::Error::from(io::ErrorKind::ExecutableFileBusy)
}

#[test]
fn a_busy_spawn_is_retried_until_it_starts() {
    let mut tries = 0;
    let started = retry_busy(Instant::now() + SPAWN_BUSY_WAIT, || {
        tries += 1;
        if tries < 3 { Err(busy()) } else { Ok(tries) }
    });
    assert_eq!(started.unwrap(), 3);
}

#[test]
fn another_failure_and_the_deadline_end_the_retries() {
    let mut tries = 0;
    let missing = retry_busy(Instant::now() + SPAWN_BUSY_WAIT, || -> io::Result<()> {
        tries += 1;
        Err(io::Error::from(io::ErrorKind::NotFound))
    });
    assert_eq!(missing.unwrap_err().kind(), io::ErrorKind::NotFound);
    assert_eq!(tries, 1);

    let at = Instant::now();
    let still = retry_busy(at + Duration::from_millis(100), || -> io::Result<()> {
        Err(busy())
    });
    assert_eq!(still.unwrap_err().kind(), io::ErrorKind::ExecutableFileBusy);
    assert!(at.elapsed() >= Duration::from_millis(100), "it kept trying");
}
