//! Milestone 9.14 task M9.14.2: `run::git::tree_entries`, the `ls-tree` read behind
//! decision 26's inputs id, against a real temporary repository.

use crate::support;

use std::os::unix::fs::{PermissionsExt, symlink};

use daemon::run::git::{TreeEntry, tree_entries, tree_of};
use support::run_git::{T, out, real_git, repo, write};

#[test]
fn tree_entries_lists_every_path_of_a_tree() {
    let repo = repo();
    let root = &repo.root;
    write(root, "z/deep/nested.txt", "nested\n");
    write(root, "run.sh", "#!/bin/sh\n");
    let script = root.join("run.sh");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    symlink("run.sh", root.join("link")).unwrap();
    out(root, &["add", "-A"]);
    out(root, &["commit", "-q", "-m", "entries"]);
    let tree = tree_of(real_git(), root, "HEAD", T).unwrap();
    let got = tree_entries(real_git(), root, &tree, T).unwrap();
    let object = |path: &str| out(root, &["rev-parse", &format!("HEAD:{path}")]);
    let ours: Vec<&TreeEntry> = got
        .iter()
        .filter(|e| ["link", "run.sh", "z/deep/nested.txt"].contains(&e.path.as_str()))
        .collect();
    let want = [
        ("120000", "link"),
        ("100755", "run.sh"),
        ("100644", "z/deep/nested.txt"),
    ];
    assert_eq!(ours.len(), want.len(), "{got:?}");
    for (entry, (mode, path)) in ours.iter().zip(want) {
        assert_eq!(
            (entry.mode.as_str(), entry.path.as_str()),
            (mode, path),
            "{got:?}"
        );
        assert_eq!(entry.object, object(path));
    }
    // Every path of the tree, in git's own order, from the root whatever the
    // directory it runs in.
    let listed = out(
        root,
        &["ls-tree", "-r", "--full-tree", "--name-only", &tree],
    );
    let paths: Vec<&str> = got.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, listed.lines().collect::<Vec<_>>());
    let below = tree_entries(real_git(), &root.join("z"), &tree, T).unwrap();
    assert_eq!(below, got);
    // A tree that could be read as an option is refused.
    assert!(tree_entries(real_git(), root, "--output=x", T).is_err());
}
