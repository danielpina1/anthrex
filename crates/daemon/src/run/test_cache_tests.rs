//! Milestone 9.1 task M9.1.10: the result cache's file and index (decision 30), in a
//! temporary directory of its own. The executor's side (what it stores, and what a hit
//! skips) is `driver/tier_tests_cache.rs`.

use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use super::*;
use crate::run::model::Profile;
use crate::run::plan::resolve_profile;
use crate::run::tiers::cache_key::key;
use crate::run::tiers::{CacheCtx, Scope, profile_hash};

const DAYS: u32 = 14;

fn now() -> u64 {
    unix_now()
}

fn ctx() -> CacheCtx {
    CacheCtx {
        profile_hash: "00000000000000aa".to_string(),
        toolchain: "none".to_string(),
    }
}

fn gate(tree: &str, command: &str) -> CacheKey {
    key(tree, Scope::Gate, command, "a,b", &ctx())
}

fn green(at: u64) -> CacheEntry {
    CacheEntry {
        ok: true,
        secs: 3,
        flaky: Vec::new(),
        tail: "test result: ok. 2 passed".to_string(),
        at,
        run: "r1".to_string(),
        tier: 1,
    }
}

fn lines(repo: &Path) -> Vec<String> {
    std::fs::read_to_string(repo.join(CACHE_FILE))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// One cache line as the cache writes it.
fn line(key: &CacheKey, entry: &CacheEntry) -> String {
    serde_json::to_string(&CacheLine {
        key: key.clone(),
        entry: entry.clone(),
    })
    .unwrap()
}

fn temps(repo: &Path) -> Vec<String> {
    std::fs::read_dir(repo)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp"))
        .collect()
}

#[test]
fn green_step_is_stored_and_found_by_key() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo-data");
    let k = gate("tree1", "sh test.sh a");
    let mut entry = green(now());
    entry.flaky = vec!["tests::x".to_string()];
    let cache = TestCache::new(DAYS);
    assert_eq!(cache.lookup(&repo, &k), None, "an empty cache misses");
    cache.store(&repo, k.clone(), entry.clone()).unwrap();
    let found = cache.lookup(&repo, &k).expect("found in memory");
    assert!(found.ok);
    assert_eq!(
        (
            found.secs,
            &found.flaky,
            found.at,
            found.run.as_str(),
            found.tier
        ),
        (3, &entry.flaky, entry.at, "r1", 1)
    );
    // The file has one JSON line, the tail included.
    let written = lines(&repo);
    assert_eq!(written.len(), 1, "{written:?}");
    assert_eq!(written[0], line(&k, &entry));
    let mode = std::fs::metadata(repo.join(CACHE_FILE)).unwrap().mode() & 0o777;
    assert_eq!(mode, 0o600);
    drop(cache);

    // A new cache (a new daemon) reads the file.
    let cache = TestCache::new(DAYS);
    let found = cache.lookup(&repo, &k).expect("found in the file");
    assert_eq!(
        (found.ok, found.secs, &found.flaky, found.at, found.tier),
        (true, 3, &entry.flaky, entry.at, 1)
    );
    // Another repository's cache is its own.
    assert_eq!(cache.lookup(&tmp.path().join("other"), &k), None);
}

#[test]
fn a_red_entry_is_refused_and_a_red_line_is_never_a_hit() {
    // Decision 30: only green results are written. A caller that tries to store a red
    // one stores nothing, and a line that says `ok: false` (written by hand) is not a
    // hit.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().to_path_buf();
    let k = gate("tree1", "sh test.sh a");
    let red = CacheEntry {
        ok: false,
        ..green(now())
    };
    let cache = TestCache::new(DAYS);
    cache.store(&repo, k.clone(), red.clone()).unwrap();
    assert_eq!(cache.lookup(&repo, &k), None);
    assert!(lines(&repo).is_empty());
    std::fs::write(repo.join(CACHE_FILE), format!("{}\n", line(&k, &red))).unwrap();
    assert_eq!(TestCache::new(DAYS).lookup(&repo, &k), None);
}

#[test]
fn cache_key_changes_with_tree_command_profile_toolchain_and_env() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().to_path_buf();
    let cache = TestCache::new(DAYS);
    let profile = |env: &[(&str, &str)]| -> Profile {
        let mut profile = resolve_profile(&Default::default(), &Default::default());
        profile.env = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<BTreeMap<_, _>>();
        profile
    };
    let with = |profile_hash: String, toolchain: &str| CacheCtx {
        profile_hash,
        toolchain: toolchain.to_string(),
    };
    let base_hash = profile_hash(&profile(&[("A", "1")]));
    let stored = key(
        "tree1",
        Scope::Gate,
        "sh test.sh a",
        "a",
        &with(base_hash.clone(), "t1"),
    );
    cache.store(&repo, stored.clone(), green(now())).unwrap();
    assert!(cache.lookup(&repo, &stored).is_some());
    let env_hash = profile_hash(&profile(&[("A", "2")]));
    assert_ne!(env_hash, base_hash, "the profile's env is in its hash");
    let mut other = profile(&[("A", "1")]);
    other.check_timeout_secs = 7;
    let other_profile = profile_hash(&other);
    for (what, changed) in [
        (
            "tree",
            key(
                "tree2",
                Scope::Gate,
                "sh test.sh a",
                "a",
                &with(base_hash.clone(), "t1"),
            ),
        ),
        (
            "command",
            key(
                "tree1",
                Scope::Gate,
                "sh test.sh b",
                "a",
                &with(base_hash.clone(), "t1"),
            ),
        ),
        (
            "profile",
            key(
                "tree1",
                Scope::Gate,
                "sh test.sh a",
                "a",
                &with(other_profile, "t1"),
            ),
        ),
        (
            "toolchain",
            key(
                "tree1",
                Scope::Gate,
                "sh test.sh a",
                "a",
                &with(base_hash.clone(), "t2"),
            ),
        ),
        (
            "env",
            key(
                "tree1",
                Scope::Gate,
                "sh test.sh a",
                "a",
                &with(env_hash, "t1"),
            ),
        ),
        (
            "scope",
            key(
                "tree1",
                Scope::Full,
                "sh test.sh a",
                "a",
                &with(base_hash.clone(), "t1"),
            ),
        ),
        (
            "affected",
            key(
                "tree1",
                Scope::Gate,
                "sh test.sh a",
                "a,b",
                &with(base_hash.clone(), "t1"),
            ),
        ),
    ] {
        assert_eq!(cache.lookup(&repo, &changed), None, "a new {what} misses");
    }
}

#[test]
fn entries_older_than_test_cache_days_are_misses_and_are_pruned() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().to_path_buf();
    let (old, fresh) = (gate("old", "c"), gate("fresh", "c"));
    let day = 86_400;
    let old_entry = green(now() - (u64::from(DAYS) * day + 60));
    let fresh_entry = green(now() - (u64::from(DAYS) * day - 3_600));
    let cache = TestCache::new(DAYS);
    cache.store(&repo, old.clone(), old_entry.clone()).unwrap();
    cache
        .store(&repo, fresh.clone(), fresh_entry.clone())
        .unwrap();
    // In memory, an entry past its days is a miss at once.
    assert_eq!(cache.lookup(&repo, &old), None);
    assert!(cache.lookup(&repo, &fresh).is_some());
    assert_eq!(lines(&repo).len(), 2);
    let inode = std::fs::metadata(repo.join(CACHE_FILE)).unwrap().ino();
    drop(cache);

    // The next load drops it and rewrites the file (a temporary file and a rename).
    let cache = TestCache::new(DAYS);
    assert!(cache.lookup(&repo, &fresh).is_some());
    assert_eq!(cache.lookup(&repo, &old), None);
    assert_eq!(lines(&repo), [line(&fresh, &fresh_entry)]);
    assert_ne!(
        std::fs::metadata(repo.join(CACHE_FILE)).unwrap().ino(),
        inode,
        "rewritten by a rename"
    );
    assert!(temps(&repo).is_empty(), "{:?}", temps(&repo));

    // A load that drops nothing leaves the file alone.
    let inode = std::fs::metadata(repo.join(CACHE_FILE)).unwrap().ino();
    assert!(TestCache::new(DAYS).lookup(&repo, &fresh).is_some());
    assert_eq!(
        std::fs::metadata(repo.join(CACHE_FILE)).unwrap().ino(),
        inode
    );
}

#[test]
fn zero_days_disables_the_cache() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo-data");
    let k = gate("tree1", "c");
    // A file an earlier daemon wrote is not read either.
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(
        repo.join(CACHE_FILE),
        format!("{}\n", line(&k, &green(now()))),
    )
    .unwrap();
    let cache = TestCache::new(0);
    assert!(!cache.enabled());
    assert_eq!(cache.lookup(&repo, &k), None);
    cache
        .store(&repo, gate("tree2", "c"), green(now()))
        .unwrap();
    assert_eq!(cache.lookup(&repo, &gate("tree2", "c")), None);
    assert_eq!(lines(&repo).len(), 1, "nothing written");
    assert!(TestCache::new(DAYS).enabled());
}

#[test]
fn cache_file_is_capped_at_twenty_thousand_lines() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().to_path_buf();
    let at = now();
    let keys: Vec<CacheKey> = (0..CACHE_LINES_MAX)
        .map(|n| gate(&format!("tree{n}"), "c"))
        .collect();
    let mut text = String::new();
    for k in &keys {
        text.push_str(&line(k, &green(at)));
        text.push('\n');
    }
    std::fs::write(repo.join(CACHE_FILE), text).unwrap();
    let cache = TestCache::new(DAYS);
    // Exactly at the cap: every line is kept and the file is not rewritten.
    let inode = std::fs::metadata(repo.join(CACHE_FILE)).unwrap().ino();
    assert!(cache.lookup(&repo, &keys[0]).is_some());
    assert_eq!(lines(&repo).len(), CACHE_LINES_MAX);
    assert_eq!(
        std::fs::metadata(repo.join(CACHE_FILE)).unwrap().ino(),
        inode
    );

    // The next store passes the cap: the oldest lines go, the new one stays, and the
    // file is replaced by a rename.
    let newest = gate("newest", "c");
    cache.store(&repo, newest.clone(), green(at)).unwrap();
    let kept = lines(&repo);
    assert!(kept.len() <= CACHE_LINES_MAX, "{}", kept.len());
    assert_eq!(kept.len(), CACHE_LINES_KEPT);
    assert_eq!(kept.last().unwrap(), &line(&newest, &green(at)));
    let dropped = CACHE_LINES_MAX + 1 - CACHE_LINES_KEPT;
    assert_eq!(kept[0], line(&keys[dropped], &green(at)), "oldest first");
    assert_ne!(
        std::fs::metadata(repo.join(CACHE_FILE)).unwrap().ino(),
        inode
    );
    assert!(temps(&repo).is_empty());
    // The index follows the file.
    assert_eq!(cache.lookup(&repo, &keys[0]), None);
    assert_eq!(cache.lookup(&repo, &keys[dropped - 1]), None);
    assert!(cache.lookup(&repo, &keys[dropped]).is_some());
    assert!(cache.lookup(&repo, &newest).is_some());
}

#[test]
fn a_corrupt_line_is_skipped_not_fatal() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().to_path_buf();
    let (a, b) = (gate("a", "c"), gate("b", "c"));
    let at = now();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"not json at all\n");
    bytes.extend_from_slice(format!("{}\n", line(&a, &green(at))).as_bytes());
    bytes.extend_from_slice(b"\xff\xfe\x00 binary\n");
    bytes.extend_from_slice(b"{\"key\": {\"tree\": \"x\"}}\n");
    // A line far past any real one is skipped without being held.
    bytes.extend(std::iter::repeat_n(b'x', 2 * 1024 * 1024));
    bytes.push(b'\n');
    bytes.extend_from_slice(format!("{}\n", line(&b, &green(at))).as_bytes());
    // A torn last line (a crash mid-append) has no newline.
    let torn = line(&gate("torn", "c"), &green(at));
    bytes.extend_from_slice(&torn.as_bytes()[..torn.len() / 2]);
    std::fs::write(repo.join(CACHE_FILE), &bytes).unwrap();
    let cache = TestCache::new(DAYS);
    assert!(cache.lookup(&repo, &a).is_some());
    assert!(cache.lookup(&repo, &b).is_some());
    assert_eq!(cache.lookup(&repo, &gate("torn", "c")), None);
    // The load dropped the bad lines, so it rewrote the file with the good ones.
    assert_eq!(lines(&repo), [line(&a, &green(at)), line(&b, &green(at))]);
    // A store after it appends a whole line.
    cache.store(&repo, gate("c", "c"), green(at)).unwrap();
    assert_eq!(lines(&repo).len(), 3);

    // A cache file that cannot be read at all (a directory in its place) is a miss,
    // and a store fails without a panic.
    let other = tmp.path().join("other");
    std::fs::create_dir_all(other.join(CACHE_FILE)).unwrap();
    let cache = TestCache::new(DAYS);
    assert_eq!(cache.lookup(&other, &a), None);
    assert!(cache.store(&other, a.clone(), green(at)).is_err());
}

#[test]
fn a_long_tail_is_cut_to_its_last_lines() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().to_path_buf();
    let k = gate("tree1", "c");
    let tail: Vec<String> = (0..300)
        .map(|n| format!("line {n:04} {}", "y".repeat(60)))
        .collect();
    let entry = CacheEntry {
        tail: tail.join("\n"),
        ..green(now())
    };
    let cache = TestCache::new(DAYS);
    cache.store(&repo, k, entry).unwrap();
    let written: CacheLine = serde_json::from_str(&lines(&repo)[0]).unwrap();
    let kept = written.entry.tail;
    assert!(kept.len() <= CACHE_TAIL_BYTES, "{}", kept.len());
    assert!(kept.ends_with(&tail[299]), "the last line is kept");
    assert!(
        kept.starts_with("line "),
        "whole lines only: {}",
        &kept[..20]
    );
    assert!(kept.lines().count() <= crate::run::exec::CHECK_TAIL_LINES);
}
