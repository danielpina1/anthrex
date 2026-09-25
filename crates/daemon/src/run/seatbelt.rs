//! The macOS seatbelt profile a confined check, proof or `setup` runs under (final fix
//! batch F1d, from F1c re-review 2's R1, R2 and R4). Pure: it only builds text.
//!
//! Three rounds of deny-listing on an `(allow default)` base each left a route to an
//! unconfined actor open (the daemon socket, then LaunchServices, cfprefsd, the
//! keychain and localhost services). So the profile now starts from `(deny default)`
//! and allows only what a git, cargo, node or python toolchain needs, the way Codex's
//! `seatbelt_base_policy.sbpl` (codex-cli 0.156, itself derived from Chrome's
//! `common.sb`) and Claude Code's sandbox runtime do:
//!
//! - processes: `exec` and `fork`; signals and process information only within the
//!   same sandbox instance (each `sandbox-exec` is its own), so a check can signal its
//!   own children but no process of the user's, nor another check's;
//! - files: reads everywhere (reads are not the threat here: with the network off
//!   nothing read can leave), except other terminals; writes only under the writable
//!   set the caller gives (the checkout, its object store, its per-task `TMPDIR`, the
//!   user's `cache_dirs`) and a few harmless `/dev` nodes;
//! - PTYs: `openpty` and the PTYs the command creates itself (the
//!   `com.apple.sandbox.pty` extension marks them), never another terminal's;
//! - `sysctl` reads, POSIX semaphores and shared memory (Python multiprocessing, not
//!   the system's `apple.*` segments), preference *reads*;
//! - the Mach services in [`MACH_SERVICES`], each named with why a toolchain needs it;
//! - Unix sockets bound and connected under the writable set only, never the anthrex
//!   daemon's socket.
//!
//! Everything else is denied by the base, in particular: `lsopen` (LaunchServices would
//! start an app outside the sandbox), `user-preference-write` (cfprefsd would write any
//! domain for the caller), `appleevent-send`, the keychain (`com.apple.SecurityServer`,
//! `com.apple.securityd.xpc`), launchd and every other Mach service, setuid programs,
//! and the network, localhost included.
//!
//! With the user's `confined_network` for the repository (F1d round 2, S1), outbound
//! TCP/UDP to remote hosts is allowed, with DNS and the Mach services name resolution
//! and TLS verification need (never the keychain); loopback and this machine's own
//! addresses stay denied, as does every Unix socket outside the command's own
//! directories. The user's `confined_unix_sockets` (exact socket paths) and
//! `confined_localhost_ports` are the only ways to a local service. The anthrex
//! daemon's socket and launchd's per-user sockets (ssh-agent's among them) are denied
//! last, whatever else is allowed.

use std::path::{Path, PathBuf};

/// The Mach services a confined command may look up, and why a toolchain needs each.
/// Not in the list, among others: `com.apple.SecurityServer` and
/// `com.apple.securityd.xpc` (the keychain), `com.apple.coreservices.launchservicesd`,
/// `com.apple.lsd.*` and `com.apple.CoreServices.coreservicesd` (LaunchServices),
/// `com.apple.coreservices.appleevents`, and launchd itself.
pub const MACH_SERVICES: &[(&str, &str)] = &[
    (
        "com.apple.system.opendirectoryd.libinfo",
        "getpwuid/getpwnam: git, ssh, python's pwd and cargo find the user's home and name",
    ),
    (
        "com.apple.system.opendirectoryd.membership",
        "group membership (getgrouplist, access checks) in libc",
    ),
    (
        "com.apple.system.DirectoryService.libinfo_v1",
        "the older libinfo endpoint some libc paths still use",
    ),
    (
        "com.apple.bsd.dirhelper",
        "confstr(_CS_DARWIN_USER_TEMP_DIR/CACHE_DIR), used by Foundation and xcrun",
    ),
    (
        "com.apple.system.logger",
        "asl logging from system libraries; without it they log sandbox noise",
    ),
    ("com.apple.logd", "os_log from system libraries"),
    ("com.apple.diagnosticd", "os_log activity streaming"),
    (
        "com.apple.system.notification_center",
        "notify(3), which libinfo's caches and the time zone code use",
    ),
    (
        "com.apple.cfprefsd.daemon",
        "preference reads (xcrun, clang, Foundation); writes stay denied",
    ),
    (
        "com.apple.cfprefsd.agent",
        "per-user preference reads; writes stay denied",
    ),
];

/// With `confined_network` only: what name resolution and TLS certificate checks need.
pub const NETWORK_MACH_SERVICES: &[(&str, &str)] = &[
    (
        "com.apple.SystemConfiguration.DNSConfiguration",
        "the resolver configuration",
    ),
    (
        "com.apple.SystemConfiguration.configd",
        "network configuration (proxies, interfaces)",
    ),
    ("com.apple.networkd", "the network stack's helper"),
    (
        "com.apple.trustd",
        "TLS certificate evaluation (Security.framework, curl, git over https)",
    ),
    (
        "com.apple.trustd.agent",
        "per-user TLS certificate evaluation",
    ),
    ("com.apple.ocspd", "certificate revocation checks"),
];

/// The harmless `/dev` nodes a confined command may write (F1c round 3, N8).
const DEV_NODES: &[&str] = &[
    "/dev/null",
    "/dev/zero",
    "/dev/random",
    "/dev/urandom",
    "/dev/tty",
    "/dev/dtracehelper",
];

/// What one confined command may do beyond the base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grants<'a> {
    /// Where it may write, and bind and connect Unix sockets.
    pub writable: &'a [PathBuf],
    /// The user's `confined_network` for the repository: outbound IP to remote hosts
    /// and DNS, never loopback or a Unix socket (F1d round 2, S1).
    pub network: bool,
    /// The user's `confined_unix_sockets`: Unix sockets it may connect to, resolved and
    /// checked by `confine_cache` (never the daemon's, ssh-agent's or anthrex's).
    pub unix_sockets: &'a [PathBuf],
    /// The user's `confined_localhost_ports`: loopback ports it may connect to, bind
    /// and accept on.
    pub localhost_ports: &'a [u16],
    /// The anthrex daemon's socket, denied even with `network`.
    pub daemon_socket: &'a Path,
    /// The checkout whose protected agent-config paths no write may touch (final fix
    /// batch F2 round 2): [`protected_denial`].
    pub checkout: Option<&'a Path>,
}

/// Final fix batch F2 round 2: the unconditional deny, after every allow so it wins
/// (SBPL takes the last matching rule), of writes to `checkout`'s protected agent-config
/// paths (decision 56's built-ins): `.claude` and `.codex` (the directories themselves
/// too, so neither can be created, replaced or renamed), `.mcp.json`, and `CLAUDE.md`
/// and `AGENTS.md` at any depth. A check, proof or `setup` never needs to write them,
/// and a child that escapes the command's group is still inside this sandbox.
pub fn protected_denial(checkout: &Path) -> Result<String, String> {
    let quoted = |name: &str| sbpl_string(&checkout.join(name));
    let text = checkout
        .to_str()
        .ok_or_else(|| format!("{} is not UTF-8", checkout.display()))?;
    if text.contains('"') || text.chars().any(char::is_control) {
        return Err(format!("{text:?} cannot be spelled in an SBPL regex"));
    }
    let mut escaped = String::new();
    for c in text.chars() {
        if "\\.+*?()|[]{}^$".contains(c) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    Ok(format!(
        "\n; Never the checkout's protected agent-config paths (decision 56's built-ins).\n\
         (deny file-write*\n  (subpath {claude})\n  (subpath {codex})\n  (literal {mcp})\n  \
         (regex #\"^{escaped}/(.*/)?(CLAUDE|AGENTS)\\.md$\"))\n",
        claude = quoted(".claude")?,
        codex = quoted(".codex")?,
        mcp = quoted(".mcp.json")?,
    ))
}

/// The profile for `grants`. A path that cannot be spelled in SBPL is refused.
pub fn profile(grants: &Grants<'_>) -> Result<String, String> {
    let mut p = String::from(BASE);
    p.push_str(
        "\n; Writes: the command's own directories and harmless /dev nodes.\n(allow file-write*\n",
    );
    for node in DEV_NODES {
        p.push_str(&format!("  (literal \"{node}\")\n"));
    }
    p.push_str("  (subpath \"/dev/fd\")\n");
    let quoted = grants
        .writable
        .iter()
        .map(|path| sbpl_string(path))
        .collect::<Result<Vec<_>, _>>()?;
    for path in &quoted {
        p.push_str(&format!("  (subpath {path})\n"));
    }
    p.push_str(")\n");
    if let Some(checkout) = grants.checkout {
        p.push_str(&protected_denial(checkout)?);
    }
    p.push_str("\n; Unix sockets: only under the command's own directories.\n(allow system-socket (socket-domain AF_UNIX))\n");
    for path in &quoted {
        p.push_str(&format!(
            "(allow network-bind (local unix-socket (subpath {path})))\n\
             (allow network-outbound (remote unix-socket (subpath {path})))\n"
        ));
    }
    p.push_str("\n; Mach services a toolchain needs (see MACH_SERVICES).\n(allow mach-lookup\n");
    for (name, _) in MACH_SERVICES {
        p.push_str(&format!("  (global-name \"{name}\")\n"));
    }
    p.push_str("  (local-name \"com.apple.cfprefsd.agent\"))\n");
    if !grants.unix_sockets.is_empty() {
        p.push_str("\n; The user's confined_unix_sockets, each by its exact path.\n");
        for socket in grants.unix_sockets {
            let socket = sbpl_string(socket)?;
            p.push_str(&format!(
                "(allow network-outbound (remote unix-socket (literal {socket})))\n"
            ));
        }
    }
    if grants.network || !grants.localhost_ports.is_empty() {
        p.push_str(
            "\n; IP sockets, for confined_network or confined_localhost_ports.\n\
             (allow system-socket (socket-domain AF_INET))\n\
             (allow system-socket (socket-domain AF_INET6))\n",
        );
    }
    if grants.network {
        p.push_str(
            "\n; The user's confined_network: outbound IP to remote hosts, name resolution\n\
             ; and TLS verification; never the keychain. Seatbelt's \"localhost\" also\n\
             ; matches this machine's own interface addresses, so the deny below keeps\n\
             ; every local service (bound to loopback or to 0.0.0.0) out of reach.\n\
             (allow network-outbound (remote ip \"*:*\"))\n\
             (allow network-outbound (literal \"/private/var/run/mDNSResponder\"))\n\
             (allow system-socket (require-all (socket-domain AF_SYSTEM) (socket-protocol 2)))\n\
             (allow mach-lookup\n",
        );
        for (name, _) in NETWORK_MACH_SERVICES {
            p.push_str(&format!("  (global-name \"{name}\")\n"));
        }
        p.push_str(")\n(deny network-outbound (remote ip \"localhost:*\"))\n");
    }
    if !grants.localhost_ports.is_empty() {
        p.push_str("\n; The user's confined_localhost_ports.\n");
        for port in grants.localhost_ports {
            p.push_str(&format!(
                "(allow network-outbound (remote ip \"localhost:{port}\"))\n\
                 (allow network-bind (local ip \"localhost:{port}\"))\n\
                 (allow network-inbound (local ip \"localhost:{port}\"))\n"
            ));
        }
    }
    // Last, so it wins over every allow above: the daemon's socket, and launchd's
    // per-user sockets (ssh-agent), are never reachable.
    let socket = sbpl_string(grants.daemon_socket)?;
    p.push_str(&format!(
        "\n; Never the anthrex daemon, nor launchd's per-user sockets (ssh-agent).\n\
         (deny network-outbound (remote unix-socket (literal {socket})))\n\
         (deny network-bind (local unix-socket (literal {socket})))\n"
    ));
    if let Ok(resolved) = grants.daemon_socket.canonicalize()
        && resolved != grants.daemon_socket
    {
        let resolved = sbpl_string(&resolved)?;
        p.push_str(&format!(
            "(deny network-outbound (remote unix-socket (literal {resolved})))\n\
             (deny network-bind (local unix-socket (literal {resolved})))\n"
        ));
    }
    p.push_str(
        "(deny network-outbound (remote unix-socket (regex #\"^/private/tmp/com\\.apple\\.launchd\\.\")))\n",
    );
    Ok(p)
}

/// The deny-by-default base (see the module doc for what each block is for).
const BASE: &str = r#"(version 1)
(deny default)

; Processes: exec and fork; signals and process information only inside this sandbox
; instance (each sandbox-exec is its own).
(allow process-exec)
(allow process-fork)
(allow signal (target same-sandbox))
(allow process-info* (target same-sandbox))

; Reads everywhere...
(allow file-read*)
; ...but never another terminal. A PTY this command opened itself carries the
; com.apple.sandbox.pty extension and is allowed below.
(deny file-read* file-write* file-ioctl (regex #"^/dev/ttys[0-9]+$"))
(allow pseudo-tty)
(allow file-read* file-write* file-ioctl (literal "/dev/ptmx"))
(allow file-read* file-write* file-ioctl
  (require-all (regex #"^/dev/ttys[0-9]+$") (extension "com.apple.sandbox.pty")))
(allow file-ioctl
  (literal "/dev/null") (literal "/dev/zero") (literal "/dev/random")
  (literal "/dev/urandom") (literal "/dev/tty") (literal "/dev/dtracehelper"))

; sysctl reads; the two writes Java and V8 make to read CPU information.
(allow sysctl-read)
(allow sysctl-write (sysctl-name "kern.grade_cputype") (sysctl-name "kern.tcsm_enable"))

; POSIX semaphores and shared memory (Python multiprocessing), never the system's
; apple.* segments for writing.
(allow ipc-posix-sem)
(allow ipc-posix-shm-read* ipc-posix-shm-read-metadata)
(allow ipc-posix-shm-write-create ipc-posix-shm-write-data ipc-posix-shm-write-unlink
  (require-not (ipc-posix-name-prefix "apple.")))

; Power-state queries some runtimes make; IOKit property reads.
(allow iokit-open (iokit-registry-entry-class "RootDomainUserClient"))
(allow iokit-get-properties)

; Guarded vnodes, the sandbox's own container query, and chflags through fsctl.
(allow system-mac-syscall (mac-policy-name "vnguard"))
(allow system-mac-syscall (require-all (mac-policy-name "Sandbox") (mac-syscall-number 67)))
(allow system-fsctl (fsctl-command FSIOC_CAS_BSDFLAGS))

; Preference reads (writes are denied by the base: cfprefsd checks the caller).
(allow user-preference-read)

; The system log socket.
(allow network-outbound (literal "/private/var/run/syslog"))
"#;

/// `path` as an SBPL string literal. A path that is not UTF-8 or holds a control
/// character is refused rather than approximated.
pub fn sbpl_string(path: &Path) -> Result<String, String> {
    let Some(text) = path.to_str() else {
        return Err(format!("{} is not UTF-8", path.display()));
    };
    if text.chars().any(char::is_control) {
        return Err(format!("{text:?} holds a control character"));
    }
    Ok(format!(
        "\"{}\"",
        text.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grants(network: bool) -> String {
        let writable = [PathBuf::from("/w/checkout"), PathBuf::from("/t/x")];
        profile(&Grants {
            writable: &writable,
            network,
            unix_sockets: &[],
            localhost_ports: &[],
            daemon_socket: Path::new("/s/daemon.sock"),
            checkout: Some(Path::new("/w/check.out")),
        })
        .unwrap()
    }

    /// F2 round 2: the protected agent-config paths of the checkout are denied after
    /// the write allows, a regex-special character in its path escaped.
    #[test]
    fn protected_agent_config_is_denied_after_the_write_allows() {
        let p = grants(false);
        let allow = p.find("(subpath \"/w/checkout\")").unwrap();
        let deny = p.find("(deny file-write*\n").expect("the protected deny");
        assert!(deny > allow, "{p}");
        for rule in [
            "(subpath \"/w/check.out/.claude\")",
            "(subpath \"/w/check.out/.codex\")",
            "(literal \"/w/check.out/.mcp.json\")",
            "(regex #\"^/w/check\\.out/(.*/)?(CLAUDE|AGENTS)\\.md$\")",
        ] {
            let at = p
                .find(rule)
                .unwrap_or_else(|| panic!("{rule} missing: {p}"));
            assert!(at > deny, "{rule}");
        }
        assert!(protected_denial(Path::new("/w/a\"b")).is_err());
    }

    #[test]
    fn the_base_denies_by_default_and_names_no_escape_route() {
        let p = grants(false);
        assert!(p.contains("(deny default)"), "{p}");
        assert!(!p.contains("(allow default)"), "{p}");
        for never in [
            "lsopen",
            "user-preference-write",
            "appleevent-send",
            "SecurityServer",
            "securityd",
            "launchservicesd",
            "com.apple.lsd",
            "appleevents",
            "xpc.launchd",
            "(allow network*)",
            "(allow network-outbound)",
        ] {
            assert!(!p.contains(never), "{never} in {p}");
        }
        assert!(p.contains("(subpath \"/w/checkout\")"), "{p}");
        assert!(
            p.contains("(allow network-outbound (remote unix-socket (subpath \"/t/x\")))"),
            "{p}"
        );
        assert!(p.contains("(literal \"/s/daemon.sock\")"), "{p}");
        for (name, why) in MACH_SERVICES {
            assert!(p.contains(&format!("(global-name \"{name}\")")), "{name}");
            assert!(!why.is_empty());
        }
    }

    #[test]
    fn network_is_remote_ip_only_and_listed_local_services_are_exact() {
        let p = grants(true);
        assert!(
            p.contains("(allow network-outbound (remote ip \"*:*\"))"),
            "{p}"
        );
        assert!(p.contains("com.apple.trustd"), "{p}");
        assert!(!p.contains("SecurityServer"), "{p}");
        assert!(!p.contains("(allow network*)"), "{p}");
        assert!(!p.contains("(allow system-socket)\n"), "{p}");
        let allow = p.find("(remote ip \"*:*\")").unwrap();
        let loopback = p
            .find("(deny network-outbound (remote ip \"localhost:*\"))")
            .unwrap();
        let daemon = p.find("(literal \"/s/daemon.sock\")").unwrap();
        assert!(loopback > allow && daemon > loopback, "{p}");
        let writable = [PathBuf::from("/w")];
        let listed = profile(&Grants {
            writable: &writable,
            network: true,
            unix_sockets: &[PathBuf::from("/tmp/.s.PGSQL.5432")],
            localhost_ports: &[6379],
            daemon_socket: Path::new("/s/daemon.sock"),
            checkout: None,
        })
        .unwrap();
        assert!(
            listed.contains(
                "(allow network-outbound (remote unix-socket (literal \"/tmp/.s.PGSQL.5432\")))"
            ),
            "{listed}"
        );
        let port = listed.find("(remote ip \"localhost:6379\")").unwrap();
        let loopback = listed.find("(remote ip \"localhost:*\")").unwrap();
        assert!(
            port > loopback,
            "a listed port must follow the loopback deny: {listed}"
        );
        assert!(listed.rfind("/s/daemon.sock").unwrap() > port, "{listed}");
    }

    #[test]
    fn quotes_and_backslashes_are_escaped() {
        assert_eq!(
            sbpl_string(Path::new("/a \"b\"\\c")).unwrap(),
            r#""/a \"b\"\\c""#
        );
        assert!(sbpl_string(Path::new("/a\nb")).is_err());
    }
}
