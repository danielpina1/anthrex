# Milestone briefs

Each file here is the complete requirements for one milestone, written so a coding agent can implement it without further questions. `docs/ROADMAP.md` lists the milestones, their order and their status. `AGENTS.md` at the repository root lists the rules that apply to every milestone.

## How a brief is structured

Every brief has the same sections, in this order.

1. **Header.** Status, dependencies, the spec sections it implements, and the branch name to use.
2. **Goal.** What the user can do when the milestone is done, in a few sentences.
3. **Scope.** What is in, and what is explicitly out.
4. **Design decisions.** Numbered and final. They are not to be re-litigated during implementation. If one proves wrong, stop on it and record the evidence under "Implementation notes".
5. **Interfaces.** Exact names and shapes of new or changed types, protocol messages, CLI commands, keys and config options. Later milestones rely on these names.
6. **Tasks.** Numbered `M<N>.<k>`. Each task lists the files it touches, the tests to write first with what they must assert, the change itself, and its acceptance criteria. Tasks are ordered so that each one leaves the tree building and all tests passing.
7. **Verification.** The commands that must pass, and any milestone-specific checks.
8. **Manual check.** What a human should look at in a real terminal with real `claude` and `codex` windows. Automated tests cannot see everything.
9. **Risks and gotchas.** What is likely to go wrong, and how to recognise it.
10. **Follow-ups handled.** Items from `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md` that this milestone closes.
11. **Implementation notes.** Empty until implementation. The implementer records every deviation, surprise and decision here.

## Status values

| Status | Meaning |
|--------|---------|
| `done` | Merged into `main`. |
| `ready` | All dependencies are done. It can be picked up. |
| `blocked` | Waits for the milestones listed under "Depends on". |
| `in progress` | Someone is working on it, on the branch named in the header. |

## Writing style for briefs

- Tests are described by name and by what they assert, not as complete code. The implementer writes the code.
- Exact values are written once, in the brief: names, paths, limits, key bindings, timeouts, message fields.
- Every claim about an external tool (Claude Code, Codex, git, vt100, ratatui) says which version it was verified against.
