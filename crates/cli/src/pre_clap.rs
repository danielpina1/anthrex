//! The subcommands dispatched before clap parses anything: `hook`, `filter-run` and
//! `filter-hook`. Each is started by an agent's CLI, not by hand, so none is in
//! `anthrex --help`, and none may print clap's errors or start a runtime it does not
//! need.

use std::ffi::OsStr;
use std::time::Instant;

/// Runs one of the pre-clap subcommands and exits, or returns when `argv[1]` names none.
pub fn dispatch(started: Instant) {
    let mut args = std::env::args_os().skip(1);
    let Some(first) = args.next() else {
        return;
    };
    let code = if first == OsStr::new("hook") {
        crate::hook::run(args.collect(), started);
        0
    } else if first == OsStr::new("filter-run") {
        crate::filter_run::main(args.collect())
    } else if first == OsStr::new("filter-hook") {
        crate::filter_hook::run(args.collect(), started);
        0
    } else {
        return;
    };
    std::process::exit(code);
}
