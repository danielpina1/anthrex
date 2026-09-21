//! Where the socket, data and config live. See spec section 2.3.

use std::path::PathBuf;

fn uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

/// Directory that holds the daemon socket (created with mode 0700 by the daemon).
pub fn socket_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        let tmp = std::env::var_os("TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        return tmp.join(format!("anthrex-{}", uid()));
    }
    match dirs::runtime_dir() {
        Some(rt) => rt.join("anthrex"),
        None => PathBuf::from(format!("/tmp/anthrex-{}", uid())),
    }
}

/// The daemon's Unix socket. `ANTHREX_SOCKET` overrides it.
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("ANTHREX_SOCKET") {
        return PathBuf::from(p);
    }
    socket_dir().join("daemon.sock")
}

/// State, log and pid files. `ANTHREX_DATA_DIR` overrides it.
pub fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("ANTHREX_DATA_DIR") {
        return PathBuf::from(p);
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("anthrex")
}

/// The config file. `ANTHREX_CONFIG` overrides it.
pub fn config_path() -> PathBuf {
    if let Some(p) = std::env::var_os("ANTHREX_CONFIG") {
        return PathBuf::from(p);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("anthrex")
        .join("config.toml")
}

pub fn log_path() -> PathBuf {
    data_dir().join("daemon.log")
}

pub fn pid_path() -> PathBuf {
    data_dir().join("daemon.pid")
}

pub fn state_path() -> PathBuf {
    data_dir().join("state.json")
}

/// The daemon's `flock`-based lifetime lock.
pub fn lock_path() -> PathBuf {
    data_dir().join("daemon.lock")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Environment variables are process-global; serialize the tests that touch them.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn default_socket_path_is_inside_a_private_dir() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: tests in this module are serialized by ENV_LOCK and no other thread reads these variables.
        unsafe { std::env::remove_var("ANTHREX_SOCKET") };
        let p = socket_path();
        assert_eq!(p.file_name().unwrap(), "daemon.sock");
        let dir = p.parent().unwrap().to_string_lossy().into_owned();
        assert!(dir.contains("anthrex"), "{dir}");
        assert!(
            p.to_string_lossy().len() < 100,
            "socket path too long for sun_path: {}",
            p.display()
        );
    }

    #[test]
    fn env_overrides_socket_and_data_dir() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: see above.
        unsafe {
            std::env::set_var("ANTHREX_SOCKET", "/tmp/x/custom.sock");
            std::env::set_var("ANTHREX_DATA_DIR", "/tmp/x/data");
        }
        assert_eq!(socket_path(), PathBuf::from("/tmp/x/custom.sock"));
        assert_eq!(log_path(), PathBuf::from("/tmp/x/data/daemon.log"));
        assert_eq!(pid_path(), PathBuf::from("/tmp/x/data/daemon.pid"));
        assert_eq!(state_path(), PathBuf::from("/tmp/x/data/state.json"));
        assert_eq!(lock_path(), PathBuf::from("/tmp/x/data/daemon.lock"));
        unsafe {
            std::env::remove_var("ANTHREX_SOCKET");
            std::env::remove_var("ANTHREX_DATA_DIR");
        }
    }

    #[test]
    fn default_data_dir_ends_with_anthrex() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("ANTHREX_DATA_DIR");
            std::env::remove_var("ANTHREX_CONFIG");
        }
        assert_eq!(data_dir().file_name().unwrap(), "anthrex");
        assert_eq!(config_path().file_name().unwrap(), "config.toml");
    }

    #[test]
    fn config_env_override() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: see above.
        unsafe { std::env::set_var("ANTHREX_CONFIG", "/tmp/x/c.toml") };
        assert_eq!(config_path(), PathBuf::from("/tmp/x/c.toml"));
        unsafe { std::env::remove_var("ANTHREX_CONFIG") };
    }
}
