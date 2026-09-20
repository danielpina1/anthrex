mod support;

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use support::{TestDaemon, tempdir};

fn process_exists(pid: libc::pid_t) -> bool {
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

struct OwnedGitWrapper {
    pid_file: PathBuf,
    exited: bool,
}

impl OwnedGitWrapper {
    fn pid(&self) -> Option<libc::pid_t> {
        fs::read_to_string(&self.pid_file).ok()?.trim().parse().ok()
    }

    fn wait_for_exit(&mut self, timeout: Duration) -> bool {
        let Some(pid) = self.pid() else {
            return false;
        };
        let deadline = Instant::now() + timeout;
        while process_exists(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        self.exited = !process_exists(pid);
        self.exited
    }
}

impl Drop for OwnedGitWrapper {
    fn drop(&mut self) {
        if self.exited {
            return;
        }
        let Some(pid) = self.pid() else {
            return;
        };
        if process_exists(pid) {
            // SAFETY: this PID came from the wrapper executable created only for this test.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
}

fn injected_path(bin: &Path) -> OsString {
    let mut paths = vec![bin.to_path_buf()];
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(paths).unwrap()
}

fn wait_for_one_window(daemon: &TestDaemon) -> proto::WindowInfo {
    let mut client = daemon.client();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let windows = client.windows();
        if windows.len() == 1 {
            return windows.into_iter().next().unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "expected exactly one created window, found {windows:?}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn new_outlives_project_detection_timeout_without_duplicate_or_leak() {
    let fixture = tempdir();
    let cwd = fixture.path().join("plain");
    let bin = fixture.path().join("bin");
    let pid_file = fixture.path().join("git.pid");
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir_all(&bin).unwrap();
    let cwd = cwd.canonicalize().unwrap();
    let git = bin.join("git");
    fs::write(
        &git,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$$\" > '{}'\nexec /bin/sleep 30\n",
            pid_file.display()
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&git).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&git, permissions).unwrap();
    let mut wrapper = OwnedGitWrapper {
        pid_file: pid_file.clone(),
        exited: false,
    };

    let path = injected_path(&bin);
    let daemon = TestDaemon::start_configured(&[], |command| {
        command.env("PATH", path);
    });
    let output = daemon.anthrex_with_timeout(
        &[
            "new",
            "--runtime",
            "shell",
            "--name",
            "slow-project",
            "--dir",
            cwd.to_str().unwrap(),
        ],
        Duration::from_secs(8),
    );
    let window = wait_for_one_window(&daemon);
    let wrapper_exited = wrapper.wait_for_exit(Duration::from_secs(1));

    let remove = daemon.anthrex(&["rm", "slow-project"]);
    let remaining = daemon.client().windows();

    assert!(
        output.status.success(),
        "anthrex new failed with {}; stdout: {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        output.stderr.is_empty(),
        "anthrex new wrote stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1, "unexpected stdout: {stdout:?}");
    assert_eq!(stdout.trim().parse::<u32>().unwrap(), window.id,);
    assert_eq!(window.name, "slow-project");
    assert_eq!(window.cwd, cwd);
    assert_eq!(window.project, cwd, "timed-out Git must use cwd fallback");
    assert!(wrapper_exited, "timed-out Git wrapper process survived");
    assert!(
        remove.status.success(),
        "cleanup failed: {}",
        String::from_utf8_lossy(&remove.stderr)
    );
    assert!(
        remaining.is_empty(),
        "duplicate or leaked windows: {remaining:?}"
    );

    drop(daemon);
}
