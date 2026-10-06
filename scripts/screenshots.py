#!/usr/bin/env python3
"""Regenerates the README screenshots in docs/images/ from the real TUI.

Run it from the repository root:

    cargo build --workspace --examples
    python3 scripts/screenshots.py            # every scene
    python3 scripts/screenshots.py agents     # one scene: agents or design

It drives the real `target/debug/anthrex` client in a pseudo-terminal of `ROWS` x
`COLS`, the way `scripts/pty-smoke.py` does, against a daemon of its own: socket, data
directory, config and even `HOME` live in one fresh `/tmp/anthrex-shots-*` directory,
which the script removes at the end. Every agent is `target/debug/fake-agent`
(`ANTHREX_CLAUDE_BIN`, `ANTHREX_CODEX_BIN` and `ANTHREX_DECIDER_BIN`), and
`ANTHREX_GH_BIN` points at a path that does not exist, so no real agent, decider or
`gh` ever starts. Each scene ends with `anthrex daemon stop` under the same variables.

At each shot the script saves every byte the client has written so far and hands it to
`crates/tui/examples/render_svg.rs`, which replays it through the `vt100` crate and
writes the final screen as an SVG. The scenes reuse the smoke test's fixtures: the
scripted fake agents, the run engine's plan and script layout (`pty_smoke_run`), the
design flow's scripted orchestrator (`pty_smoke_design_fixtures`) and the stored
profile (`pty_smoke_adapt`). The demo content (the `acme-api` repository, its goal) is
made up; the agents' statuses come from real hooks and the runs from the real engine.

See docs/screenshots.md.
"""

import json
import os
import pty
import re
import select
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time
import fcntl

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from pty_smoke_adapt import STORED_PROFILE  # noqa: E402
from pty_smoke_design_fixtures import DRAFTS_IN, _call, _expect, _until  # noqa: E402
from pty_smoke_run import _write_script, git_env  # noqa: E402

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(REPO, "target/debug/anthrex")
FAKE = os.path.join(REPO, "target/debug/fake-agent")
RENDER = os.path.join(REPO, "target/debug/examples/render_svg")
OUT = os.path.join(REPO, "docs/images")
ROWS, COLS = 34, 140
# Generous hang guards, not costs: a screen settles in well under a second, and a gate of
# the design run in a few seconds (`ORCH_WAIT` of the design smoke is 120 s).
SCREEN_WAIT = 20.0
RUN_WAIT = 300.0


class Failure(Exception):
    pass


def fail(msg):
    raise Failure(msg)


# --- a minimal screen model, for waiting on text (the SVG itself is vt100's) ---------

CSI_RE = re.compile(r"\x1b\[([^a-zA-Z]*)([a-zA-Z])")
OSC_RE = re.compile(r"\x1b\].*?(\x07|\x1b\\)", re.S)


def screen_text(raw):
    """The text of the screen `raw` draws: cursor addressing and erases, nothing else
    (the same model as `pty-smoke.py`'s `Screen`)."""
    grid = [[" "] * COLS for _ in range(ROWS)]
    row = col = 0
    data = raw.decode("utf-8", errors="replace")
    i, n = 0, len(data)
    while i < n:
        ch = data[i]
        if ch == "\x1b":
            m = CSI_RE.match(data, i)
            if m:
                params, final = m.group(1), m.group(2)
                parts = [int(p) if p.isdigit() else None for p in params.split(";")] if params else []

                def num(k, default):
                    return parts[k] if k < len(parts) and parts[k] is not None else default

                if final in "Hf":
                    row = max(0, min(num(0, 1) - 1, ROWS - 1))
                    col = max(0, min(num(1, 1) - 1, COLS - 1))
                elif final == "J" and num(0, 0) in (2, 3):
                    grid = [[" "] * COLS for _ in range(ROWS)]
                elif final == "J" and num(0, 0) == 0:
                    grid[row][col:] = [" "] * (COLS - col)
                    for r in range(row + 1, ROWS):
                        grid[r] = [" "] * COLS
                elif final == "K" and num(0, 0) == 0:
                    grid[row][col:] = [" "] * (COLS - col)
                elif final == "K" and num(0, 0) == 2:
                    grid[row] = [" "] * COLS
                elif final == "C":
                    col = min(COLS - 1, col + num(0, 1))
                elif final == "D":
                    col = max(0, col - num(0, 1))
                i = m.end()
                continue
            m = OSC_RE.match(data, i) if data[i : i + 2] == "\x1b]" else None
            i = m.end() if m else i + 1
            continue
        if ch == "\r":
            col = 0
        elif ch == "\n":
            row = min(row + 1, ROWS - 1)
        elif ch >= " ":
            if col < COLS:
                grid[row][col] = ch
            col += 1
        i += 1
    return "\n".join("".join(r).rstrip() for r in grid)


class Client:
    """The real `anthrex` client in a pty of `ROWS` x `COLS`, recording every byte."""

    def __init__(self, argv, env, cwd):
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            try:
                os.chdir(cwd)
                os.execvpe(argv[0], argv, env)
            except Exception as error:  # pragma: no cover
                os.write(2, f"exec failed: {error}\n".encode())
            os._exit(127)
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
        self.raw = b""

    def pump(self, timeout=0.1):
        r, _, _ = select.select([self.fd], [], [], timeout)
        if self.fd in r:
            try:
                chunk = os.read(self.fd, 65536)
            except OSError:
                return
            self.raw += chunk

    def text(self):
        return screen_text(self.raw)

    def send(self, data):
        os.write(self.fd, data)
        self.settle(0.3)

    def settle(self, seconds):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            self.pump(0.05)

    def wait_for(self, what, timeout=SCREEN_WAIT):
        """Until `what` (a string, or a predicate on the screen's text) holds."""
        test = what if callable(what) else (lambda screen: what in screen)
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if test(self.text()):
                return
            self.pump(0.2)
        fail(f"timed out waiting for {what!r}\n--- screen ---\n{self.text()}")

    def shot(self, name, title, settle=1.0):
        """Saves the screen as docs/images/<name>.svg (and its text beside the capture)."""
        self.settle(settle)
        capture = os.path.join(tempfile.gettempdir(), f"anthrex-shot-{os.getpid()}-{name}.bin")
        with open(capture, "wb") as f:
            f.write(self.raw)
        out = os.path.join(OUT, f"{name}.svg")
        result = subprocess.run(
            [RENDER, "--rows", str(ROWS), "--cols", str(COLS), "--title", title, "--text", capture, out],
            capture_output=True,
            text=True,
            timeout=60,
        )
        os.remove(capture)
        if result.returncode != 0:
            fail(f"render_svg failed for {name}: {result.stderr}")
        print(f"ok: docs/images/{name}.svg")
        if os.environ.get("ANTHREX_SHOTS_VERBOSE"):
            print(result.stdout)

    def close(self):
        """Hangs up the pty and reaps this client's own pid only."""
        try:
            os.close(self.fd)
        except OSError:
            pass
        deadline = time.monotonic() + 5.0
        while time.monotonic() < deadline:
            try:
                pid, _ = os.waitpid(self.pid, os.WNOHANG)
            except ChildProcessError:
                return
            if pid == self.pid:
                return
            time.sleep(0.1)
        print(f"warning: client pid {self.pid} did not exit", file=sys.stderr)


class Sandbox:
    """One isolated daemon: socket, data, config and HOME under one fresh /tmp dir."""

    def __init__(self, scene, config="", under_home=True):
        """`under_home`: the repositories and the data directory live under the fake
        HOME, so the client shows them as `~/...`. A scene that runs workers keeps them
        outside it: on macOS a worker's sandbox does not let it read under HOME."""
        self.root = os.path.realpath(tempfile.mkdtemp(prefix=f"anthrex-shots-{scene}-", dir="/tmp"))
        self.home = os.path.join(self.root, "home")
        base = self.home if under_home else self.root
        self.src = os.path.join(base, "src")
        os.makedirs(self.home)
        os.makedirs(self.src)
        self.socket = os.path.join(self.root, "d.sock")
        self.config = os.path.join(self.root, "config.toml")
        with open(self.config, "w") as f:
            f.write(config)
        self.agent_script = os.path.join(self.root, "agent.jsonl")
        with open(self.agent_script, "w") as f:
            f.write(json.dumps({"read_line": True}) + "\n")
        # `claude` and `codex` on PATH are the fake agent too, so runtime detection finds
        # both (a document review then runs on the other runtime, as it would for real).
        bindir = os.path.join(self.root, "bin")
        os.makedirs(bindir)
        for name in ("claude", "codex"):
            os.symlink(FAKE, os.path.join(bindir, name))
        self.deciders = os.path.join(self.root, "deciders")
        os.makedirs(self.deciders)
        self.env = {
            "PATH": f"{bindir}:/usr/bin:/bin:/usr/sbin:/sbin",
            "HOME": self.home,
            "USER": "demo",
            "LANG": "en_US.UTF-8",
            "TERM": "xterm-256color",
            "COLORTERM": "truecolor",
            "SHELL": "/bin/sh",
            "PS1": "$ ",
            "ANTHREX_SOCKET": self.socket,
            "ANTHREX_DATA_DIR": os.path.join(base, ".local/share/anthrex"),
            "ANTHREX_CONFIG": self.config,
            # Pinned, never inherited: no real agent, decider or `gh` can start.
            "ANTHREX_CLAUDE_BIN": FAKE,
            "ANTHREX_CODEX_BIN": FAKE,
            "ANTHREX_DECIDER_BIN": FAKE,
            "ANTHREX_GH_BIN": "/nonexistent/anthrex-shots/gh",
            "FAKE_AGENT_SCRIPT": self.agent_script,
            "FAKE_AGENT_DECIDER_DIR": self.deciders,
            "FAKE_AGENT_TRANSCRIPT": os.path.join(self.root, "unused-transcript.jsonl"),
            "GIT_CONFIG_GLOBAL": "/dev/null",
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_TERMINAL_PROMPT": "0",
        }
        self.daemon = None
        self.clients = []

    def start(self):
        self.daemon = subprocess.Popen(
            [BIN, "daemon", "start", "--foreground"],
            cwd=self.home,
            env=self.env,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        deadline = time.monotonic() + 60
        while not os.path.exists(self.socket):
            if self.daemon.poll() is not None:
                fail(f"the daemon exited at its start ({self.daemon.returncode})")
            if time.monotonic() > deadline:
                fail("the daemon did not bind its socket within 60 s")
            time.sleep(0.1)

    def cmd(self, args, cwd=None, timeout=600, check=True):
        result = subprocess.run(
            [BIN, *args],
            cwd=cwd or self.home,
            env=self.env,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        if check and result.returncode != 0:
            fail(f"`anthrex {' '.join(args)}` exited {result.returncode}: {result.stderr}")
        return result.stdout

    def json(self, args, cwd=None):
        return json.loads(self.cmd(args + ["--json"], cwd=cwd))

    def client(self, argv=(), cwd=None):
        client = Client([BIN, *argv], self.env, cwd or self.home)
        self.clients.append(client)
        client.wait_for("agents")
        return client

    def hook(self, window, source, payload):
        subprocess.run(
            [BIN, "hook", "--window", str(window), "--source", source, json.dumps(payload)],
            env=self.env,
            capture_output=True,
            timeout=10,
        )

    def git(self, args, cwd):
        env = git_env({"HOME": self.home})
        subprocess.run(
            ["git", "-c", "user.name=Demo", "-c", "user.email=demo@example.com",
             "-c", "commit.gpgsign=false", "-c", "init.defaultBranch=main", *args],
            cwd=cwd, env=env, check=True, capture_output=True, timeout=30,
        )

    def repo(self, name, commits):
        """A repository under ~/src with `commits`: a list of (message, {path: text})."""
        path = os.path.join(self.src, name)
        os.makedirs(path)
        self.git(["init", "-q"], path)
        for key, value in (("user.name", "Demo"), ("user.email", "demo@example.com"), ("commit.gpgsign", "false")):
            self.git(["config", key, value], path)
        for message, files in commits:
            for rel, text in files.items():
                full = os.path.join(path, rel)
                os.makedirs(os.path.dirname(full), exist_ok=True)
                with open(full, "w") as f:
                    f.write(text)
            self.git(["add", "-A"], path)
            self.git(["commit", "-q", "-m", message], path)
        return path

    def stop(self):
        for client in self.clients:
            client.close()
        if self.daemon is not None:
            subprocess.run(
                [BIN, "daemon", "stop"], env=self.env, capture_output=True, timeout=60
            )
            try:
                self.daemon.wait(timeout=30)
            except subprocess.TimeoutExpired:
                print(f"LEAKED: daemon pid {self.daemon.pid} on {self.socket}", file=sys.stderr)
        if not os.environ.get("ANTHREX_SHOTS_KEEP"):
            shutil.rmtree(self.root, ignore_errors=True)


# --- the demo repositories ----------------------------------------------------------

ACME_API = [
    (
        "initial commit",
        {
            "Cargo.toml": '[package]\nname = "acme-api"\nversion = "0.4.0"\nedition = "2024"\n',
            "README.md": "# acme-api\n\nThe Acme accounts service.\n",
            "src/main.rs": "mod routes;\nmod session;\n\nfn main() {\n    routes::serve();\n}\n",
            "src/routes/mod.rs": "pub mod login;\n\npub fn serve() {}\n",
            "src/session.rs": "pub struct Session;\n",
            "src/middleware/mod.rs": "// request middleware\n",
            "check.sh": "echo check ok\n",
            "tests/t_ok.sh": "echo PASS t_ok\n",
        },
    ),
    (
        "add the login endpoint",
        {"src/routes/login.rs": "pub fn login(user: &str, password: &str) -> bool {\n    !user.is_empty() && !password.is_empty()\n}\n"},
    ),
    ("store sessions in redis", {"src/session.rs": "pub struct Session {\n    pub id: String,\n}\n"}),
    ("return 401 with a JSON body", {"src/routes/login.rs": "pub fn login(user: &str, password: &str) -> Result<(), u16> {\n    if user.is_empty() || password.is_empty() { Err(401) } else { Ok(()) }\n}\n"}),
]

ACME_WEB = [
    ("initial commit", {"package.json": '{ "name": "acme-web", "private": true }\n', "src/App.tsx": "export default function App() { return null }\n"}),
    ("add the login form", {"src/Login.tsx": "export function Login() { return null }\n"}),
]

GOAL = "Add rate limiting to the login endpoint"


# --- scene 1: agents, the tree, the overview, the conversation -----------------------

def transcript_lines(session):
    """A short Claude transcript for the `api` agent: its prompt, its prose and tool calls."""
    def line(kind, content):
        return {"type": kind, "sessionId": session, "isSidechain": False,
                "message": {"role": kind, "content": content},
                **({"origin": {"kind": "human"}, "promptSource": "typed"} if kind == "user" and isinstance(content, str) else {})}

    return [
        line("user", f"{GOAL}: 5 attempts per minute per client IP, then 429 with Retry-After."),
        line("assistant", [{"type": "text", "text": "I'll map how requests reach the login handler first, then add a token-bucket limiter as middleware in front of it."}]),
        line("assistant", [{"type": "tool_use", "id": "tu-read", "name": "Read", "input": {"file_path": "src/routes/login.rs"}}]),
        line("user", [{"type": "tool_result", "tool_use_id": "tu-read", "content": "pub fn login(user: &str, password: &str) -> Result<(), u16> {", "is_error": False}]),
        line("assistant", [{"type": "tool_use", "id": "tu-explore", "name": "Agent", "input": {"subagent_type": "Explore", "description": "Map the auth middleware"}}]),
        line("assistant", [{"type": "tool_use", "id": "tu-tests", "name": "Agent", "input": {"subagent_type": "general-purpose", "description": "Draft the limiter tests"}}]),
        line("user", [{"type": "tool_result", "tool_use_id": "tu-explore", "content": "Requests pass through src/routes/mod.rs::serve; there is no middleware layer yet.", "is_error": False}]),
        line("assistant", [{"type": "text", "text": "There is no middleware layer yet, so the limiter gets its own module keyed by client IP, and serve() wraps the login route with it."}]),
        line("assistant", [{"type": "tool_use", "id": "tu-edit", "name": "Edit", "input": {"file_path": "src/middleware/rate_limit.rs", "old_string": "", "new_string": "pub struct RateLimiter {\n    per_minute: u32,\n}"}}]),
    ]


def agents_scene():
    print("== scene: agents, tree, overview, conversation ==")
    box = Sandbox("agents", "[ui]\nsidebar_width = 40\n")
    try:
        api_repo = box.repo("acme-api", ACME_API)
        web_repo = box.repo("acme-web", ACME_WEB)
        box.start()

        def new(name, runtime, cwd, *extra):
            out = box.cmd(["new", "--runtime", runtime, "--name", name, *extra], cwd=cwd)
            return int(out.strip())

        shell = new("shell", "shell", api_repo)
        api = new("api", "claude", api_repo, "--worktree", "rate-limit", "--model", "opus")
        docs = new("docs", "claude", api_repo)
        review = new("review", "codex", api_repo)
        web = new("web", "codex", web_repo)
        windows = {w["id"]: w for w in json.loads(box.cmd(["ls", "--json"]))}
        api_dir = windows[api]["cwd"] if "cwd" in windows[api] else api_repo

        # The api agent: a live turn with two sub-agents, enriched by its transcript.
        session = "6f1c2d3e-0000-4000-8000-00000000a001"
        transcript = os.path.join(box.root, "api-transcript.jsonl")
        with open(transcript, "w") as f:
            for entry in transcript_lines(session):
                f.write(json.dumps(entry) + "\n")
        base = {"session_id": session, "cwd": api_dir, "transcript_path": transcript}

        def claude(window, event, **fields):
            box.hook(window, "claude", {"hook_event_name": event, **base, **fields})

        claude(api, "SessionStart", source="startup")
        claude(api, "UserPromptSubmit", prompt=transcript_lines(session)[0]["message"]["content"])
        claude(api, "PreToolUse", tool_name="Read", tool_use_id="tu-read", tool_input={"file_path": "src/routes/login.rs"})
        claude(api, "PostToolUse", tool_name="Read", tool_use_id="tu-read", tool_response="23 lines")
        claude(api, "PreToolUse", tool_name="Agent", tool_use_id="tu-explore",
               tool_input={"subagent_type": "Explore", "description": "Map the auth middleware"})
        claude(api, "SubagentStart", agent_id="sa-explore", agent_type="Explore")
        claude(api, "PreToolUse", tool_name="Agent", tool_use_id="tu-tests",
               tool_input={"subagent_type": "general-purpose", "description": "Draft the limiter tests"})
        claude(api, "SubagentStart", agent_id="sa-tests", agent_type="general-purpose")
        claude(api, "SubagentStop", agent_id="sa-explore")
        claude(api, "PostToolUse", tool_name="Agent", tool_use_id="tu-explore", tool_response="no middleware layer yet")
        claude(api, "PreToolUse", tool_name="Edit", tool_use_id="tu-edit",
               tool_input={"file_path": "src/middleware/rate_limit.rs", "old_string": "", "new_string": "pub struct RateLimiter {\n    per_minute: u32,\n}"})

        # docs: a finished turn. review (codex): waiting on a permission. web: idle.
        other = {"session_id": "6f1c2d3e-0000-4000-8000-00000000a002", "cwd": api_repo}
        for event, fields in [
            ("SessionStart", {"source": "startup"}),
            ("UserPromptSubmit", {"prompt": "Document the session store in README.md"}),
            ("PreToolUse", {"tool_name": "Write", "tool_input": {"file_path": "README.md"}}),
            ("PostToolUse", {"tool_name": "Write"}),
            ("Stop", {}),
        ]:
            box.hook(docs, "claude", {"hook_event_name": event, **other, **fields})
        codex = {"session_id": "6f1c2d3e-0000-4000-8000-00000000a003", "cwd": api_repo}
        for event, fields in [
            ("SessionStart", {"source": "startup"}),
            ("UserPromptSubmit", {"prompt": "Review the session store change"}),
            ("PreToolUse", {"tool_name": "Bash", "tool_input": {"command": "cargo test session"}}),
            ("PermissionRequest", {"tool_name": "Bash", "tool_input": {"command": "cargo test session"}}),
        ]:
            box.hook(review, "codex-hook", {"hook_event_name": event, **codex, **fields})
        box.hook(web, "codex-hook", {"hook_event_name": "SessionStart", "session_id": "6f1c2d3e-0000-4000-8000-00000000a004", "cwd": web_repo, "source": "startup"})

        c = box.client(["attach", "shell"], cwd=api_repo)
        c.wait_for("⚑ 4 review")
        # macOS's /bin/sh sets its own prompt (host and user); replace it before the shot.
        c.send(b"PS1='acme-api $ '; clear\r")
        c.settle(0.5)
        c.send(b"git --no-pager log --oneline --decorate -4; ls\r")
        c.wait_for("return 401")
        c.settle(3.0)
        c.shot("agents", "anthrex — agents")

        # The new-agent form: runtime, name, directory, an optional worktree branch.
        c.send(b"\x02c")
        c.wait_for("new agent")
        c.send(b"1")
        c.settle(0.5)
        c.shot("new-agent", "anthrex — new agent")
        c.send(b"\x1b")
        c.wait_for(lambda s: "new agent" not in s)

        c.send(b"\x02T")
        c.wait_for(" tree overview ")
        c.send(b"j")
        c.wait_for(lambda s: "○ 1 shell" not in s.split("\n")[-8:][0] and "branch" in s)
        c.shot("overview", "anthrex — overview and inspector")
        c.send(b"\x1b")
        c.settle(0.5)
        c.send(b"\x1b")
        c.settle(0.5)

        c.send(b"\x022")
        c.settle(0.5)
        c.send(b"\x02m")
        c.wait_for("token-bucket")
        c.send(b"\r")  # unfold the newest row, the Edit, into its diff
        c.wait_for("RateLimiter")
        c.shot("conversation", "anthrex — conversation view")
        c.send(b"\x1b")
        _ = (shell, web)
    finally:
        box.stop()


# --- scene 2: a goal through the design flow, then its run ----------------------------

DESIGN_CONFIG = """[ui]
sidebar_width = 40

[orchestrator]
git_timeout_secs = 5
wake_quiet_secs = 1
planners.timeout_secs = 120

[orchestrator.design]
default = "full"

[orchestrator.profile]
check_timeout_secs = 10
"""

# Triage puts the goal on the planned path, where the design flow is on.
TRIAGE = {"answer": {"kinds": ["code"], "scale": "plan", "reason": "middleware, routing and errors", "task": None}}

DRAFT = """## Understanding
Limit login attempts per client so password guessing is slow ({label}'s reading). Success: the sixth attempt within a minute gets 429.

## Assumptions
- (assumed) One API instance; no shared store is needed yet.

## Constraints found
- src/routes/mod.rs:3 serves every route; there is no middleware layer.
- src/routes/login.rs:1 returns 401 on bad credentials.

## Approaches
### 1. In-process token bucket
A middleware keeps one bucket per client IP in memory. Files: src/middleware/rate_limit.rs. Trade-offs: resets on restart. Risks: memory per IP. Size M.

### 2. Redis counter
A fixed window counter in the session store's Redis. Files: src/session.rs. Trade-offs: shared across instances, one more round trip. Size M.

## Recommendation
The in-process token bucket: one instance today, and it keeps login latency flat.

## Questions for you
None.
"""

REPORT = """## Where they agree
Both put the limit in a middleware in front of POST /login, keyed by client IP, answering 429 with Retry-After.

## Where they disagree
claude keeps the buckets in memory; codex weighs a Redis counter so several instances share one limit. Judgment: in memory now, behind a trait a Redis store can implement later.

## Approaches
### 1. In-process token bucket [both]
One bucket per client IP in memory. Size M.

### 2. Redis fixed window [codex]
A counter in the session store's Redis, shared by every instance. Size M.

## Recommendation
The in-process token bucket behind a small store trait.

## Questions for you
None.
"""

SPEC = """# Rate limiting for the login endpoint

## Goal and success criteria
POST /login allows 5 attempts per minute per client; the sixth gets 429 with Retry-After.

## Non-goals
Limits on other routes; a shared store across instances.

## Approach
An in-process token bucket in a middleware, as the brainstorm recommends.

## Design
src/middleware/rate_limit.rs holds the bucket store behind a trait; src/middleware/client_ip.rs derives the key; serve() wraps the login route.

## Requirements
R1 A bucket allows 5 attempts per minute per key. Acceptance: `limiter_allows_five_then_blocks` passes.
R2 The key is the client IP; X-Forwarded-For counts only from a trusted proxy. Acceptance: `client_ip_ignores_untrusted_forwarded_for` passes.
R3 POST /login answers 429 once the bucket is empty. Acceptance: `login_returns_429_after_five_attempts` passes.
R4 A 429 carries Retry-After in whole seconds. Acceptance: `retry_after_is_whole_seconds` passes.

## Interfaces
`trait BucketStore { fn take(&self, key: &str) -> Result<(), Duration>; }`

## Errors and edge cases
A missing client IP falls back to the socket address.

## Testing
Unit tests for the bucket and the key; one integration test through serve().

## Risks
Memory grows with distinct IPs; idle buckets are dropped after ten minutes.

## Open questions
"""


def brief(goal, files, test, steps):
    return (
        f"{goal}\nFiles:\n" + "".join(f"- {f}\n" for f in files)
        + f"Tests first:\n- {test}\nSteps:\n" + "".join(f"- {s}\n" for s in steps)
        + f"Acceptance:\n- {test} passes\nVerify:\n- sh check.sh\n"
    )


def task(id, title, owns, covers, test, deps=(), size="S", runtime=None, mode="check"):
    t = {
        "id": id,
        "title": title,
        "brief": brief(title + ".", owns, test, ["write the test", "make it pass"]),
        "acceptance": [f"{test} passes"],
        "owns": owns,
        "deps": list(deps),
        "size": size,
        "test_mode": mode,
        "covers": covers,
    }
    if mode == "tdd":
        t["test_to_write"] = test
    else:
        t["test_mode_reason"] = "the check runs the unit tests"
    if runtime:
        t["route"] = {"runtime": runtime}
    return t


PLAN_TASKS = [
    task("t1", "Token-bucket store", ["src/middleware/rate_limit.rs"], ["R1"], "limiter_allows_five_then_blocks"),
    task("t2", "Client IP key", ["src/middleware/client_ip.rs"], ["R2"], "client_ip_ignores_untrusted_forwarded_for"),
    task("t3", "Limit POST /login", ["src/routes/mod.rs", "src/routes/login.rs"], ["R3"],
         "login_returns_429_after_five_attempts", deps=["t1", "t2"], size="M", mode="tdd"),
    task("t4", "Retry-After on 429", ["src/routes/errors.rs"], ["R4"], "retry_after_is_whole_seconds",
         deps=["t1"], runtime="codex"),
    task("t5", "Document the limit", ["README.md"], ["R3"], "readme_mentions_rate_limit", deps=["t3", "t4"]),
]

ORCH_WAIT = 120.0
USER_WAIT = 900.0

ORCHESTRATOR = [
    {"hook": "UserPromptSubmit", "payload": {"prompt": "plan"}},
    _call("start_brainstorm", {"answers": ""}),
    {"read_message": {"expect": DRAFTS_IN}},
    _call("get_doc", {"kind": "brainstorm_draft", "from": "claude"}),
    _call("get_doc", {"kind": "brainstorm_draft", "from": "codex"}),
    _call("submit_doc", {"kind": "brainstorm", "text": REPORT}),
    _until("/gate/state", "specifying", USER_WAIT),
    _call("submit_doc", {"kind": "spec", "text": SPEC, "ready": False}),
    _until("/gate/spec_review/running", False, ORCH_WAIT),
    _expect("/gate/spec_review/review", 1),
    _call("submit_doc", {"kind": "spec", "text": SPEC, "ready": True, "responses": [
        {"id": "F1", "answer": "fixed"},
        {"id": "F2", "answer": "kept: one instance today; the store trait leaves room for Redis"},
    ]}),
    _until("/gate/state", "planning", USER_WAIT),
    _call("edit_plan", {"edits": [{"op": "add_task", "task": t} for t in PLAN_TASKS], "submit": True}),
    _expect("/awaiting_review", True),
    _until("/gate/plan_review/running", False, ORCH_WAIT),
    _call("edit_plan", {"edits": [], "submit": True, "responses": []}),
    _expect("/awaiting_approval", True),
    _until("/gate/state", "approved", USER_WAIT),
    {"read_message": {}},
]

SPEC_REVIEWER = [
    _call("get_doc", {"kind": "spec", "draft": 1}),
    _call("submit_findings", {"findings": [
        {"id": "F1", "severity": "minor", "place": "R2", "text": "name the trusted proxy setting"},
        {"id": "F2", "severity": "minor", "place": "Approach", "text": "several instances would each allow 5"},
    ]}),
]
PLAN_REVIEWER = [
    _call("get_doc", {"kind": "plan"}),
    _call("get_doc", {"kind": "spec"}),
    _call("submit_findings", {"findings": []}),
]


def finished_worker(path, content, message):
    return [
        {"git_commit": {"file": path, "content": content, "message": message}},
        _call("task_done", {"summary": message}),
    ]


APPROVE = [_call("submit_review", {"verdict": "approve", "summary": "matches the brief", "findings": []})]
# A worker that stays in its turn, so its task shows as running while the shot is taken.
BUSY = [{"wait_ms": 600000}]


def design_scene():
    print("== scene: the design flow and the run view ==")
    box = Sandbox("design", DESIGN_CONFIG)
    try:
        repo = box.repo("acme-api", ACME_API)
        with open(os.path.join(box.deciders, "triage-1.json"), "w") as f:
            f.write(json.dumps(TRIAGE))
        _write_script(repo, "orchestrator-run-1", ORCHESTRATOR)
        for label in ("claude", "codex"):
            _write_script(repo, f"brainstormer-{label}-1",
                          [_call("submit_doc", {"kind": "brainstorm_draft", "text": DRAFT.format(label=label)})])
        _write_script(repo, "doc_reviewer-spec-r1-1", SPEC_REVIEWER)
        _write_script(repo, "doc_reviewer-plan-r1-1", PLAN_REVIEWER)
        _write_script(repo, "worker-t1-1", finished_worker("src/middleware/rate_limit.rs", "pub struct RateLimiter;\n", "add the token-bucket store"))
        _write_script(repo, "reviewer-t1-1", APPROVE)
        _write_script(repo, "worker-t2-1", finished_worker("src/middleware/client_ip.rs", "pub fn client_ip() {}\n", "derive the client IP key"))
        _write_script(repo, "reviewer-t2-1", APPROVE)
        _write_script(repo, "worker-t3-1", BUSY)
        _write_script(repo, "worker-t4-1", BUSY)
        box.start()

        status = box.json(["profile", "status", "--dir", repo])
        meta = {"confirmed_at": int(time.time()), "report": None, "verification": None,
                "fingerprint": {}, "edited_keys": [], "project": status["project"]}
        os.makedirs(status["repo_dir"], exist_ok=True)
        with open(os.path.join(status["repo_dir"], "profile.meta.json"), "w") as f:
            f.write(json.dumps(meta))
        with open(os.path.join(status["repo_dir"], "profile.toml"), "w") as f:
            f.write(STORED_PROFILE)
        box.cmd(["new", "--runtime", "shell", "--name", "shell"], cwd=repo)

        def run_of(run_id):
            return box.json(["run", "status", run_id])["runs"][0]

        def wait_run(c, run_id, what, pred, within):
            deadline = time.monotonic() + within
            while True:
                run = run_of(run_id)
                if pred(run):
                    return run
                if time.monotonic() > deadline:
                    fail(f"run {run_id}: {what} did not happen within {within:.0f}s\n{json.dumps(run, indent=1)[:3000]}\n{c.text()}")
                c.pump(0.5)

        def at_gate(c, run_id, kind, version):
            def open_(r):
                g = r.get("doc_gate") or {}
                return r["state"] == "awaiting_approval" and (g.get("kind"), g.get("version"), g.get("revising")) == (kind, version, None)
            return wait_run(c, run_id, f"the {kind} gate", open_, ORCH_WAIT)

        def from_alert(c, document, header):
            """`C-b a`, then Enter on the gate's alert (and on the action menu's preselected
            entry, if one opens): the gate's screen, whose title starts with `header`."""
            c.send(b"\x02a")
            c.wait_for(" ALERTS ")
            c.wait_for(document)
            c.send(b"\r")
            c.settle(1.0)
            if header not in c.text():
                c.send(b"\r")
            c.wait_for(header)

        def approve(c, page):
            c.wait_for(lambda s: "loading" not in s)
            c.send(b"a")
            c.wait_for(page)
            c.send(b"y")

        c = box.client(["attach", "shell"], cwd=repo)
        start = ["run", "start", "--goal", GOAL, "--design", "full", "--dir", repo]
        if sys.platform != "darwin":
            start.append("--unconfined-checks")
        run_id = box.cmd(start, cwd=repo).strip().splitlines()[-1]
        runs = [{"run_id": run_id}]
        run_id = runs[0]["run_id"]
        h4 = run_id[-4:]

        at_gate(c, run_id, "brainstorm", 1)
        from_alert(c, "brainstorm v1", f"Brainstorm · run {h4} · v1 of ")
        c.wait_for(lambda s: "loading" not in s)
        c.shot("brainstorm-gate", "anthrex — brainstorm gate")
        approve(c, "Approve brainstorm v1? Spec starts next.")

        at_gate(c, run_id, "spec", 1)
        from_alert(c, "spec v1", f"Spec · run {h4} · v1 of ")
        c.wait_for(lambda s: "loading" not in s)
        c.shot("spec-gate", "anthrex — spec gate")
        approve(c, "Approve spec v1? Planning starts next.")

        at_gate(c, run_id, "plan", 1)
        from_alert(c, "plan v1", f"plan · {GOAL}")
        c.wait_for(lambda s: "loading" not in s)
        c.shot("plan-review", "anthrex — plan review")
        approve(c, "Approve plan v1? The run starts next.")

        def mixed(r):
            states = {t["id"]: t["state"] for t in r["tasks"]}
            return states.get("t1") == "merged" and states.get("t2") == "merged" and states.get("t4") not in (None, "pending")
        wait_run(c, run_id, "t1 and t2 merged, t4 started", mixed, RUN_WAIT)
        # The run view: the overview, filtered to the run, then into its node.
        c.send(b"\x02T")
        c.wait_for(" tree overview ")
        c.send(b"/" + h4.encode())
        c.send(b"\r")
        for _ in range(4):
            if f" run · {GOAL} · {h4} " in c.text():
                break
            c.send(b"l")
            c.settle(1.0)
        c.wait_for(f" run · {GOAL} · {h4} ")
        # Select the running task t3: the canvas follows it, the inspector describes it.
        # Then `i` hides the inspector, so the canvas has the whole body.
        def inspecting_t3(screen):
            return "t3  Limit POST /login" in screen
        for _ in range(20):
            if inspecting_t3(c.text()):
                break
            c.send(b"j")
        c.wait_for(inspecting_t3)
        c.send(b"i")
        c.wait_for(lambda screen: "OUTCOME" not in screen)
        c.settle(3.0)
        c.shot("run-view", "anthrex — run view")
        c.send(b"i")
        c.wait_for("OUTCOME")
        c.settle(1.0)
        c.shot("task-inspector", "anthrex — run view, task inspector")
        if os.environ.get("ANTHREX_SHOTS_VERBOSE"):
            print(c.text())

        # The action menu on the run, which the daemon computes for its state.
        c.send(b".")
        c.wait_for("j/k move")
        c.shot("action-menu", "anthrex — action menu")
    finally:
        box.stop()


SCENES = {"agents": agents_scene, "design": design_scene}


def main():
    for path in (BIN, FAKE, RENDER):
        if not os.path.exists(path):
            fail(f"{path} is missing: run `cargo build --workspace --examples` first")
    os.makedirs(OUT, exist_ok=True)
    wanted = sys.argv[1:] or list(SCENES)
    for name in wanted:
        if name not in SCENES:
            fail(f"unknown scene {name!r}; scenes: {', '.join(SCENES)}")
        SCENES[name]()


if __name__ == "__main__":
    # A SIGTERM (a `timeout`, say) unwinds through each scene's `finally`, which stops
    # its daemon, instead of killing the script outright.
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    try:
        main()
    except Failure as error:
        print(f"FAIL: {error}", file=sys.stderr)
        sys.exit(1)
