//! Milestone 9.14 task M9.14.4: the `[target_pool]` table (decisions 1 and 2, C26).

use super::super::*;

fn keys(problems: &[Problem]) -> Vec<String> {
    let mut keys: Vec<String> = problems.iter().map(|p| p.key.clone()).collect();
    keys.sort();
    keys
}

#[test]
fn defaults_without_the_table() {
    let (config, problems) = parse("");
    assert!(problems.is_empty(), "{problems:?}");
    let p = &config.target_pool;
    assert!(p.enabled);
    assert_eq!(p.task_dirs, None);
    assert_eq!(p.engine_dirs, 2);
    assert_eq!(p.max_gb, 20);
    assert_eq!(p.idle_days, 7);
    assert!(p.disabled_repos.is_empty());
    assert_eq!(*p, TargetPool::default());
}

#[test]
fn each_key_reads_and_out_of_range_values_keep_the_default() {
    let low = "[target_pool]\nenabled = false\ntask_dirs = 1\nengine_dirs = 1\nmax_gb = 1\n\
               idle_days = 1\ndisabled_repos = [\"/repos/a\", \"/repos/b\"]\n";
    let (config, problems) = parse(low);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        config.target_pool,
        TargetPool {
            enabled: false,
            task_dirs: Some(1),
            engine_dirs: 1,
            max_gb: 1,
            idle_days: 1,
            disabled_repos: vec!["/repos/a".to_string(), "/repos/b".to_string()],
        }
    );
    let high = "[target_pool]\ntask_dirs = 64\nengine_dirs = 16\nmax_gb = 1024\nidle_days = 365\n";
    let (config, problems) = parse(high);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        config.target_pool,
        TargetPool {
            task_dirs: Some(64),
            engine_dirs: 16,
            max_gb: 1024,
            idle_days: 365,
            ..TargetPool::default()
        }
    );

    // One past each end of each range: one problem each, every field at its default.
    for (key, below, above, range) in [
        ("task_dirs", 0, 65, "1 and 64"),
        ("engine_dirs", 0, 17, "1 and 16"),
        ("max_gb", 0, 1025, "1 and 1024"),
        ("idle_days", 0, 366, "1 and 365"),
    ] {
        for value in [below, above] {
            let (config, problems) = parse(&format!("[target_pool]\n{key} = {value}\n"));
            assert_eq!(config.target_pool, TargetPool::default(), "{key} = {value}");
            assert_eq!(problems.len(), 1, "{key} = {value}: {problems:?}");
            assert_eq!(problems[0].key, format!("target_pool.{key}"));
            assert_eq!(problems[0].message, format!("must be between {range}"));
        }
    }
    let (_, problems) = parse("[target_pool]\ntask_dirs = 0\n");
    assert_eq!(
        problems[0].to_string(),
        "target_pool.task_dirs: must be between 1 and 64 (using [orchestrator] max_writers + 2)"
    );
    let (_, problems) = parse("[target_pool]\nmax_gb = 0\n");
    assert_eq!(
        problems[0].to_string(),
        "target_pool.max_gb: must be between 1 and 1024 (using 20)"
    );

    let (config, problems) = parse("[target_pool]\nenabled = \"yes\"\n");
    assert!(config.target_pool.enabled);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert_eq!(
        problems[0].to_string(),
        "target_pool.enabled: must be true or false (using true)"
    );

    // A relative path is a problem naming it, and is left out; the rest of the list stays.
    let (config, problems) =
        parse("[target_pool]\ndisabled_repos = [\"/repos/a\", \"rel/repo\", \"/repos/b\"]\n");
    assert_eq!(
        config.target_pool.disabled_repos,
        vec!["/repos/a".to_string(), "/repos/b".to_string()]
    );
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert_eq!(problems[0].key, "target_pool.disabled_repos");
    assert!(problems[0].message.contains("rel/repo"), "{problems:?}");
    assert!(
        problems[0]
            .message
            .contains("must be a list of absolute paths"),
        "{problems:?}"
    );

    // Not a list at all: one problem, no disabled repositories.
    let (config, problems) = parse("[target_pool]\ndisabled_repos = \"/repos/a\"\n");
    assert!(config.target_pool.disabled_repos.is_empty());
    assert_eq!(keys(&problems), ["target_pool.disabled_repos"]);

    // Not a table: one problem, every default.
    let (config, problems) = parse("target_pool = 3\n");
    assert_eq!(config.target_pool, TargetPool::default());
    assert_eq!(keys(&problems), ["target_pool"]);
}

#[test]
fn an_unknown_key_is_ignored_with_a_problem() {
    let (config, problems) = parse("[target_pool]\nmax_gb = 30\ntask_dir = 4\n");
    assert_eq!(config.target_pool.max_gb, 30);
    assert_eq!(config.target_pool.task_dirs, None);
    assert_eq!(keys(&problems), ["target_pool.task_dir"]);
    assert_eq!(problems[0].message, "unknown key, ignored");
}

#[test]
fn the_default_task_dirs_is_clamped() {
    let pool = TargetPool::default();
    assert_eq!(pool.task_dirs_for(3), 5);
    assert_eq!(pool.task_dirs_for(70), 64);
    assert_eq!(pool.task_dirs_for(u8::MAX), 64);
    let set = TargetPool {
        task_dirs: Some(9),
        ..TargetPool::default()
    };
    assert_eq!(set.task_dirs_for(3), 9);
    assert_eq!(set.task_dirs_for(70), 9);
}
