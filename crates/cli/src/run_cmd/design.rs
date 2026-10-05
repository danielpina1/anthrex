//! Milestone 9.6's `anthrex run` commands (DF §7, task M9.6.16): `run show`, `run
//! approve --gate`, `run changes`, `run edit-doc`, `run rethink` and `run back`, the
//! `--design` flags of `run start` and `run iterate`, and `run status`'s design lines.
//! Every command sends one request and prints the reply; every refusal is the daemon's
//! own text. Only the flags and `edit-doc`'s file are checked here.

use std::path::{Path, PathBuf};

use clap::{Subcommand, ValueEnum};
use proto::{
    DesignMode, DocGateAction, DocGateKind, DocKind, RoundDesign, RunInfo, RunReply, RunRequest,
    RunState, safe_text,
};

use super::{Runs, print_outcome};

/// `--doc` and `--gate`: a design gate, and the document it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(super) enum GateArg {
    Brainstorm,
    Spec,
    Plan,
}

impl From<GateArg> for DocGateKind {
    fn from(gate: GateArg) -> Self {
        match gate {
            GateArg::Brainstorm => DocGateKind::Brainstorm,
            GateArg::Spec => DocGateKind::Spec,
            GateArg::Plan => DocGateKind::Plan,
        }
    }
}

impl From<GateArg> for DocKind {
    fn from(gate: GateArg) -> Self {
        match gate {
            GateArg::Brainstorm => DocKind::Brainstorm,
            GateArg::Spec => DocKind::Spec,
            GateArg::Plan => DocKind::Plan,
        }
    }
}

/// `run start --design`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(super) enum DesignArg {
    Full,
    Off,
}

impl From<DesignArg> for DesignMode {
    fn from(design: DesignArg) -> Self {
        match design {
            DesignArg::Full => DesignMode::Full,
            DesignArg::Off => DesignMode::Off,
        }
    }
}

/// `run iterate --design`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(super) enum RoundArg {
    Amend,
    Full,
    Off,
}

impl From<RoundArg> for RoundDesign {
    fn from(design: RoundArg) -> Self {
        match design {
            RoundArg::Amend => RoundDesign::Amend,
            RoundArg::Full => RoundDesign::Full,
            RoundArg::Off => RoundDesign::Off,
        }
    }
}

/// The design flow's own subcommands, flattened into `anthrex run`.
#[derive(Subcommand, Debug)]
pub(super) enum DesignCommand {
    /// Show a design document's version: its text, and with --diff and --findings its
    /// changes and its review
    Show {
        run: String,
        #[arg(long, value_enum)]
        doc: GateArg,
        /// The version (the latest by default)
        #[arg(long)]
        version: Option<u32>,
        /// Also print the line diff against the previous version
        #[arg(long)]
        diff: bool,
        /// Also print the review's findings, each with the orchestrator's answer
        #[arg(long)]
        findings: bool,
    },
    /// Ask the orchestrator to revise the document waiting at a design gate
    Changes {
        run: String,
        #[arg(long, value_enum)]
        gate: GateArg,
        #[arg(long)]
        note: String,
        /// Send the revision to the document reviewer again (default: --no-review)
        #[arg(long, overrides_with = "no_review")]
        review: bool,
        /// Do not send the revision for review (the default)
        #[arg(long = "no-review", overrides_with = "review")]
        no_review: bool,
    },
    /// Make a file's text the next version of the brainstorm report or the spec
    EditDoc {
        run: String,
        #[arg(long, value_enum)]
        gate: GateArg,
        /// The document's full text (at most 64 KiB)
        #[arg(long)]
        file: PathBuf,
    },
    /// Run both brainstormers again with a note, then merge their drafts again
    Rethink {
        run: String,
        #[arg(long)]
        note: String,
    },
    /// Go back from the spec or plan gate to the gate before it, with a note
    Back {
        run: String,
        #[arg(long, value_enum)]
        gate: GateArg,
        #[arg(long)]
        note: String,
    },
}

/// The most `edit-doc` reads and sends: 64 KiB, the spec's cap
/// (`template::cap_bytes`) and the most `run show` prints of any version, so a version
/// shown and edited fits when it is sent back. The daemon checks the document's own cap
/// (the brainstorm report's 32 KiB, once it has cut the drafts' appendix a shown report
/// carries) and refuses in its own words.
pub(super) const EDIT_CAP: usize = 64 * 1024;

/// `edit-doc`'s file, read before anything is sent: at most [`EDIT_CAP`] bytes, else a
/// usage error (clap's, exit 2). The CLI's own file I/O, so it blocks this process only.
pub(super) fn read_capped(file: &Path) -> anyhow::Result<String> {
    use std::io::Read;
    let shown = file.display();
    let mut bytes = Vec::new();
    std::fs::File::open(file)
        .and_then(|f| f.take(EDIT_CAP as u64 + 1).read_to_end(&mut bytes))
        .map_err(|e| anyhow::anyhow!("cannot read {shown}: {e}"))?;
    if bytes.len() > EDIT_CAP {
        let text = format!(
            "--file {shown}: the file is over the {} KiB a document may be; nothing was sent\n",
            EDIT_CAP / 1024
        );
        let usage = clap::Error::raw(clap::error::ErrorKind::ValueValidation, text);
        return Err(usage.into());
    }
    String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("{shown} is not UTF-8 text"))
}

/// One design command: `edit-doc`'s file is read first, then the daemon is asked.
pub(super) async fn run(socket: &Path, command: DesignCommand) -> anyhow::Result<()> {
    let text = match &command {
        DesignCommand::EditDoc { file, .. } => Some(read_capped(file)?),
        _ => None,
    };
    let mut runs = Runs::connect(socket).await?;
    let runs = &mut runs;
    match command {
        DesignCommand::Show {
            run,
            doc,
            version,
            diff,
            findings,
        } => show(runs, &run, (doc, version), (diff, findings)).await,
        DesignCommand::Changes {
            run,
            gate,
            note,
            review,
            ..
        } => act(runs, &run, gate, DocGateAction::Changes { note, review }).await,
        DesignCommand::EditDoc { run, gate, .. } => {
            let text = text.expect("read above");
            act(runs, &run, gate, DocGateAction::Edit { text }).await
        }
        DesignCommand::Rethink { run, note } => {
            act(
                runs,
                &run,
                GateArg::Brainstorm,
                DocGateAction::Rethink { note },
            )
            .await
        }
        DesignCommand::Back { run, gate, note } => {
            act(runs, &run, gate, DocGateAction::Back { note }).await
        }
    }
}

/// `run approve <run> --gate <kind>`: the gate's approve (decision 7). Without `--gate`
/// it is 9.5's `run approve` (`orch::approve`), which the daemon refuses at the
/// brainstorm and spec gates in its own words (ruling T1-O1).
pub(super) async fn approve(
    runs: &mut Runs,
    run: &str,
    hold: Option<String>,
    gate: Option<GateArg>,
) -> anyhow::Result<()> {
    match gate {
        Some(gate) => act(runs, run, gate, DocGateAction::Approve).await,
        None => super::orch::approve(runs, run, hold).await,
    }
}

/// `action` at the `gate` of the run `run` names: `Done`'s message, or the refusal.
async fn act(
    runs: &mut Runs,
    run: &str,
    gate: GateArg,
    action: DocGateAction,
) -> anyhow::Result<()> {
    let run = runs.resolve(run).await?;
    let kind = gate.into();
    runs.done(RunRequest::DocGate { run, kind, action }).await
}

/// `run show`: the version on stdout ([`show_text`]), its header on stderr.
async fn show(
    runs: &mut Runs,
    run: &str,
    (doc, version): (GateArg, Option<u32>),
    (diff, findings): (bool, bool),
) -> anyhow::Result<()> {
    let info = runs.resolve_info(run).await?;
    let request = RunRequest::ShowDoc {
        run: info.run_id.clone(),
        kind: doc.into(),
        version,
        diff,
        findings,
    };
    match runs.request(request).await? {
        RunReply::Doc { doc, .. } => {
            let (out, err) = show_text(&info, &doc, (diff, findings));
            eprint!("{err}");
            print!("{out}");
            Ok(())
        }
        other => print_outcome(other),
    }
}

/// `run status`'s design lines (DF §7) for a design run in a design phase: `design:
/// <phase>` (`brainstorming`, `specifying`, `planning`, or `at the <kind> gate`; a
/// paused run's, the phase it paused in), then while a gate waits for the user
/// `waiting for you: <kind> v<n> (anthrex run show <run> --doc <kind>)`. While the
/// orchestrator revises, the phase line says so with the user's note's head instead
/// (the gate screen's words). Nothing for a run without the flow, or past its plan gate.
pub(super) fn status_lines(run: &RunInfo) -> String {
    if run.design == DesignMode::Off {
        return String::new();
    }
    let at = match run.state {
        RunState::Paused => run.paused_from,
        state => Some(state),
    };
    let gate = run.doc_gate.as_ref();
    let phase = match (at, gate) {
        (Some(RunState::AwaitingApproval), Some(gate)) => {
            format!("at the {} gate", gate.kind.label())
        }
        (
            Some(state @ (RunState::Brainstorming | RunState::Specifying | RunState::Planning)),
            _,
        ) => state.label().to_string(),
        _ => return String::new(),
    };
    let gate = gate.filter(|_| at == Some(RunState::AwaitingApproval));
    let mut out = match gate.and_then(|g| g.revising.as_deref().map(|note| (g, note))) {
        Some((g, note)) => {
            let note: String = safe_text::one_line(note).chars().take(NOTE_HEAD).collect();
            let next = g.version + 1;
            format!("  design: {phase}, revising v{next}… (your note: \"{note}\")\n")
        }
        None => format!("  design: {phase}\n"),
    };
    if let Some(g) =
        gate.filter(|g| g.revising.is_none() && run.state == RunState::AwaitingApproval)
    {
        let kind = g.kind.label();
        out.push_str(&format!(
            "  waiting for you: {kind} v{} (anthrex run show {} --doc {kind})\n",
            g.version, run.run_id
        ));
    }
    out
}

/// The head of the user's note a revising gate shows (the gate screen's 60).
const NOTE_HEAD: usize = 60;

#[path = "design_show.rs"]
mod show;
use show::show_text;

#[cfg(test)]
#[path = "design_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "design_tests_daemon.rs"]
mod daemon_tests;
