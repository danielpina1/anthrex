//! Milestone 9.5 task 10b: a decider call's route (decision 9a, rulings RL-2 and I6).
//! A `decider` list takes the place of the mode's runtime unless the deciders are off,
//! rotating per daemon; the call routes over what the probe finds installed at each
//! call. No decider is spawned: every binary is a path that does not exist or a
//! stand-in the probe only stats.

use std::path::Path;

use config::{Candidate, Pick, RouteList};
use proto::{DeciderMode, Effort, ModelEntry, Runtime, Strength};

use super::DeciderContext;
use super::call::{route_over, routed};
use crate::manager::ManagerConfig;
use crate::run::route_pick::Installed;

const NO_CLAUDE: &str = "/nonexistent/anthrex-test/claude";
const NO_CODEX: &str = "/nonexistent/anthrex-test/codex";
const NO_DECIDER: &str = "/nonexistent/anthrex-test/decider";

fn cand(runtime: Runtime, model: &str, effort: Option<Effort>) -> Candidate {
    Candidate {
        runtime,
        model: model.into(),
        effort,
    }
}

/// A context in `mode` with `list` as `[orchestrator.routes.decider]`, the roster with
/// `gpt-6-luna`, and the binaries `claude` and `codex`; `decider` is
/// `ANTHREX_DECIDER_BIN`.
fn ctx(
    mode: DeciderMode,
    list: RouteList,
    bins: (&str, &str),
    decider: Option<&str>,
) -> DeciderContext {
    let mut cfg = config::Orchestrator::default();
    cfg.models.push(ModelEntry {
        runtime: Runtime::Codex,
        model: "gpt-6-luna".into(),
        strength: Strength::Fast,
        note: String::new(),
    });
    cfg.deciders.mode = mode;
    cfg.tuning.routes.decider = list;
    let mut manager = ManagerConfig::for_tests("/tmp/unused.sock".into(), "/bin/sh".into());
    manager.claude_bin = bins.0.into();
    manager.codex_bin = bins.1.into();
    manager.decider_bin = decider.map(str::to_string);
    DeciderContext::new(&cfg, &manager, Path::new("/data"))
}

fn spread() -> RouteList {
    RouteList {
        candidates: vec![
            cand(Runtime::Codex, "gpt-6-luna", None),
            cand(Runtime::Claude, "claude-haiku-4-5", Some(Effort::Low)),
        ],
        pick: Pick::Spread,
    }
}

#[tokio::test]
async fn deciders_use_their_list_unless_off() {
    let none = Installed::new();
    let ctx = ctx(
        DeciderMode::Claude,
        spread(),
        (NO_CLAUDE, NO_CODEX),
        Some(NO_DECIDER),
    );
    // The list replaces the mode's runtime, rotating per daemon (clones share it).
    let picked: Vec<(Runtime, String)> = (0..3)
        .map(|_| route_over(&ctx.clone(), &none).ctx.route)
        .map(|r| (r.runtime, r.model))
        .collect();
    assert_eq!(
        picked,
        [
            (Runtime::Codex, "gpt-6-luna".to_string()),
            (Runtime::Claude, "claude-haiku-4-5".to_string()),
            (Runtime::Codex, "gpt-6-luna".to_string()),
        ]
    );
    let routed_once = route_over(&ctx, &none);
    let pick = routed_once.pick.expect("the list's pick");
    assert_eq!((pick.rotation, pick.route.is_some()), (3, true));
    assert_eq!(
        routed_once.ctx.mode,
        DeciderMode::Claude,
        "rotation 3 is haiku"
    );
    // A Codex pick runs as a Codex decider, on `ANTHREX_DECIDER_BIN` when it is set...
    let codex = route_over(&ctx, &none).ctx;
    assert_eq!(
        (codex.mode, codex.program.to_str()),
        (DeciderMode::Codex, Some(NO_DECIDER))
    );
    // ...else on Codex's command.
    let plain = self::ctx(DeciderMode::Claude, spread(), (NO_CLAUDE, NO_CODEX), None);
    let codex = route_over(&plain, &none).ctx;
    assert_eq!(codex.program.to_str(), Some(NO_CODEX));

    // `mode = "off"` still turns them off: no route, no probe.
    let off = self::ctx(
        DeciderMode::Off,
        spread(),
        (NO_CLAUDE, NO_CODEX),
        Some(NO_DECIDER),
    );
    let r = routed(&off).await;
    assert_eq!((r.ctx.mode, r.pick), (DeciderMode::Off, None));
    assert_eq!(r.ctx.route, off.route);
    // No list: the mode's route, as before.
    let today = self::ctx(
        DeciderMode::Claude,
        RouteList::default(),
        (NO_CLAUDE, NO_CODEX),
        None,
    );
    let r = route_over(&today, &none);
    assert_eq!((r.ctx.route, r.pick), (today.route.clone(), None));
}

#[tokio::test]
async fn deciders_route_over_what_is_installed_at_each_call() {
    let dir = tempfile::tempdir().unwrap();
    let codex = dir.path().join("codex");
    testexec::write_executable(&codex, "#!/bin/sh\nexit 0\n");
    let codex_bin = codex.display().to_string();
    let ctx = ctx(
        DeciderMode::Claude,
        RouteList::default(),
        (NO_CLAUDE, &codex_bin),
        Some(NO_DECIDER),
    );
    assert_eq!(
        ctx.route.runtime,
        Runtime::Claude,
        "the mode's, as configured"
    );
    // Only Codex is installed: a Codex decider, an explicit `mode` notwithstanding
    // (ruling T10b-1), moved from the mode's route, which the log line names.
    let moved = routed(&ctx).await;
    assert_eq!(moved.moved.as_ref(), Some(&ctx.route));
    assert_eq!(
        moved.moved_line().as_deref(),
        Some("decider: claude is not installed; using codex")
    );
    let first = moved.ctx;
    assert_eq!(
        (first.route.runtime, first.mode),
        (Runtime::Codex, DeciderMode::Codex)
    );
    // The binary removed between two calls: the second probes again and, with nothing
    // installed, resolves exactly as before.
    std::fs::remove_file(&codex).unwrap();
    let second = routed(&ctx).await;
    assert_eq!((second.moved_line(), &second.moved), (None, &None));
    let second = second.ctx;
    assert_eq!(
        (second.route, second.mode),
        (ctx.route.clone(), DeciderMode::Claude)
    );
    assert_eq!(second.program, ctx.program);
    // A list candidate not installed is skipped.
    testexec::write_executable(&codex, "#!/bin/sh\nexit 0\n");
    let listed = self::ctx(
        DeciderMode::Claude,
        RouteList {
            candidates: vec![
                cand(Runtime::Claude, "claude-haiku-4-5", None),
                cand(Runtime::Codex, "gpt-6-luna", None),
            ],
            pick: Pick::First,
        },
        (NO_CLAUDE, &codex_bin),
        Some(NO_DECIDER),
    );
    let r = routed(&listed).await;
    assert_eq!(r.ctx.route.model, "gpt-6-luna");
    assert_eq!(r.moved, None, "the list chose; nothing moved");
    let skipped = r.pick.expect("a pick").candidates[0].skipped_reason.clone();
    assert_eq!(skipped.as_deref(), Some("not installed"));
}
