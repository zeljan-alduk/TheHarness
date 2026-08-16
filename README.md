# TheHarness

A local-first, self-improving agentic coding harness. Rust core, any OpenAI-compatible
local model (built and tested against **Qwen3.8-27B via LM Studio**; also works with
`llama-server` and Ollama).

The thesis: give the agent a *real* toolchain (shell, git, filesystem, internet, build
tools) **and its own source**, but only let it improve itself through an
**eval-gated loop** — proposals land on git branches and are judged by a benchmark score,
never by the agent's own opinion.

## Version
`harness --version` → `1.0.NNN (sha)`; the build number increments on every release build (`build.rs`,
`.build-number`). Set `HARNESS_NO_BUMP=1` to build without bumping.

## Backends
- **Local / OpenAI-compatible** (default): LM Studio, llama.cpp, Ollama, OpenAI, OpenRouter… (`base_url`, `api_key`).
- **Claude Code (subscription)**: `provider = "claude-code"` or `/backend claude [model]` (default `claude-fable-5`).
  The harness runs the official `claude` CLI headlessly (`--print`, stream-json in/out) and exposes its own tools
  to it over an MCP bridge, so permissions, hooks, memory, redaction and the UI all still apply while your
  Anthropic subscription is used through the official client. Requires Claude Code installed and logged in.
- **Anthropic API**: `provider = "anthropic"` + `ANTHROPIC_API_KEY`.

Models without a function-calling API are handled by a **tool shim**: the tool catalogue goes into the
prompt and `<tool_call>{…}</tool_call>` blocks in the reply are parsed back into tool calls
(`[llm] tool_shim = "auto" | "on" | "off"`; auto switches over by itself when a server rejects tools or a
model writes calls as text).

## Platforms
macOS (primary; Kitty for inline images, seatbelt sandbox, temps via macmon), Linux (notify-send,
wl-paste/xclip for clipboard), Windows (`harness.exe`; install **Git for Windows** so the `bash` tool has a
POSIX shell — falls back to `cmd /C`; WezTerm shows inline images). State lives in `~/.config/harness`
(`%USERPROFILE%\.config\harness` on Windows). CI builds and unit-tests all three.

## Install & use (interactive, Claude-Code-style)

```sh
cargo install --path .                       # → ~/.cargo/bin/harness
mkdir -p ~/.config/harness && cp harness.toml ~/.config/harness/   # user config (edit model/server here)
cd ~/some/project && harness                 # interactive TUI: streaming, tools, /commands, esc to interrupt
```
Config lookup: `--config`, `$HARNESS_CONFIG`, `./harness.toml`, `~/.config/harness/harness.toml`, next to the binary.
Settings then layer on top of it, later winning: **managed** (`/etc/harness/managed.toml` or
`$HARNESS_MANAGED_CONFIG`) → **user** (`~/.config/harness/settings.toml`, written by `/settings`) →
**project** (`.harness/settings.toml`) → **local** (`.harness/settings.local.toml`, personal, gitignore it)
→ **CLI** (`--set ui.theme=light`, repeatable). Each file takes flat dotted keys (`"ui.theme" = "light"`)
or nested tables; `--setting-sources user,project` restricts which layers are read, and a project file in
an untrusted directory may not set `permissions.mode = bypass`.

## Quick start

```sh
# 1. Have a model served (LM Studio on :1234, or llama-server / ollama — edit harness.toml)
cargo build --release
./target/release/harness models                       # sanity check the server
./target/release/harness run -C /path/to/project "Add a --json flag to the CLI and tests"
./target/release/harness eval                          # run the benchmark, JSON report → $TMPDIR/harness-eval-runs/report.json
./target/release/harness self "Make the bash tool return the cwd in its result"   # agent edits itself on a proposal/* branch
./target/release/harness --json run "..."              # JSONL event stream on stdout (for UIs)
./target/release/harness tool bash '{"cmd":"ls"}'      # call one tool directly, no model (debug tools)
./target/release/harness acp                           # run as an ACP agent on stdio (Zed, JetBrains, nvim, Emacs)
./target/release/harness run --output-format stream-json "..."   # Claude-Code-compatible event stream
./target/release/harness checkpoint list               # file checkpoints of this directory's last session
```

## Desktop UI (Tauri)

```sh
cargo run -p harness-ui            # dev; or: cd ui && npx @tauri-apps/cli@2 build   (bundles TheHarness.app)
```
`ui/src-tauri` links the core as a library and runs the agent **in-process**; every `Event`
is forwarded to the webview (`agent-event`). The frontend (`ui/dist`, vanilla HTML/JS — no
Node build step) shows the live timeline (reasoning folds, tool calls with args/results,
inline images from `view_image`), a file browser of the workdir with **image / audio / video /
PDF / text previews**, and a git log/status panel. Model picker is populated from the server.

## Interactive TUI (`harness`)
Claude-Code-style terminal UI: streaming answer + reasoning, `⏺`/`⎿` tool blocks (click or ⌃O to
unfold), thinking (⌃T / click), dashboard panel (⌃P: live thinking, context gauge, tokens, tok/s +
TTFT, CPU/GPU/RAM), mouse/trackpad scrolling, esc interrupts, messages typed while running are queued.
Images: ⌃V pastes from the clipboard (saved under `~/.config/harness/pastes/`), image paths attach,
previews are real images in Kitty/WezTerm/iTerm2/Ghostty (half-blocks elsewhere). Videos open a frame
scrubber (ffmpeg): ←/→, space to select, enter attaches frames with timestamps. Commands: `/help
/model /cd /tools /mcp /plugin /memory /brain /workflows /remember /reflect /compact /net /panel …`.

## Permissions, sub-agents, sessions
- **Permissions**: `[permissions] mode = bypass|auto|ask|plan` + allow/deny/ask rules — `bash:git *`,
  `Bash(git * main)`, `WebFetch(domain:example.com)`, `Agent(subagent_type:review*)`. Risky shell commands
  and writes outside the workdir prompt (y / a=always / n) in the TUI, desktop and web UIs; catastrophic
  commands are refused outright and credential files (`.env`, ssh keys, `*.pem`, …) are never read into the
  context. `[permissions.auto]` puts borderline calls to an LLM classifier (the aux model) instead of
  interrupting you — it fails closed. `/permissions`, `/plan`, shift+tab cycles; `harness run -y` approves
  non-interactively.
- **Sub-agents**: `spawn_agent {task, workdir?, read_only?, isolation?, subagent_type?}` — fresh context, same
  tools/policy (or a custom agent's), several in one turn run in parallel. Read-only tool calls in one turn
  also run in parallel.
- **Task hand-off**: messages typed while a task runs are queued; `/next` (⌃N) stops the current task and
  starts the next; loop detection nudges then stops repeated identical calls; reflection never blocks the
  next task.
- **Sessions**: every turn is saved under `~/.config/harness/sessions/`; `/sessions`, `/resume <n|id|last>`,
  `harness --resume <id>`, `harness -c` (continue latest for this directory).
- **Tools** (all): bash (+background), process, read/write/edit_file, apply_patch, list_dir, grep, glob,
  diagnostics, notebook_edit, view_image, read_pdf, pdf_edit (in-place PDF text edits via PyMuPDF), extract_archive, memory, load_skill, todo, spawn_agent,
  web_fetch, web_search, download_file, + MCP tools. `[hooks]` run shell hooks around tools; `[security]`
  redacts secrets; `[sandbox] mode = "seatbelt"` confines shell writes (macOS).
- **Web UI**: `harness serve` (localhost:7878) — same UI as the desktop app, from any browser.

## Memory (MEMORY.md · WORKFLOWS.md · BRAIN.md)
`~/.config/harness/` holds three markdown files injected into every session: **MEMORY.md** (settings,
preferences, ideas), **WORKFLOWS.md** (named recipes), **BRAIN.md** (what the agent learned: user,
projects ledger, how-tos, lessons). The agent edits them with the `memory` tool; after substantive
runs a *reflection* call appends durable lessons; long files are *consolidated*. Evals use an isolated
store. A project can add `HARNESS.md` (like CLAUDE.md) with instructions.

## Project instructions, skills and custom agents
- **Instructions**: per directory the first of `AGENTS.md` → `CLAUDE.md` → `HARNESS.md` → `GEMINI.md` →
  `.cursorrules` → `.github/copilot-instructions.md` is loaded, walking from the repo root down to the
  working directory (most specific last), plus `~/.agents/AGENTS.md`, `~/.claude/CLAUDE.md`,
  `~/.config/harness/HARNESS.md` and any `*.local.md` / `*.override.md`. A file may pull in others with
  `@path/to/file` lines. Instruction files in sub-directories arrive when a tool first touches a file there.
- **Rules**: `.harness/rules/*.md` (also `.claude/rules`, `.cursor/rules/*.mdc`) with frontmatter
  `paths:`/`globs:` — always-on rules go into the system prompt, path-scoped ones are appended to the tool
  result the first time a matching file is touched.
- **Skills**: `.harness/skills/<name>/SKILL.md` (also `.agents/skills`, `.claude/skills`, the same under `~`,
  and plugin skills). Frontmatter: `description`, `allowed-tools`, `model`, `effort`, `paths:` (only offered
  when the project has a matching file). The model calls `load_skill {name}`; `/skills` lists them.
- **Custom agents**: `.harness/agents/<name>.md` (also `.claude/agents`, `.cursor/agents`, `~`): frontmatter
  `tools`, `model`, `effort`, `permission-mode`, `isolation`, `max-turns`; the body is the agent's system
  prompt. Delegate with `spawn_agent {task, subagent_type: "<name>"}`.

## Undo: file checkpoints
Before every file-changing tool call — and at each turn boundary — the working tree is snapshotted into a
shadow git repo under `~/.config/harness/snapshots/<session>` (never your project's `.git`; ignored and
oversized files are skipped). `/undo` and `/redo` move through them, `/checkpoints` lists them, `/rewind <n>`
restores files *and* the conversation, `/rewind code <n>` only the files, `/rewind conv` only the conversation,
`/fork` continues as a separate session. Outside the TUI: `harness checkpoint list|undo|redo|restore|diff|prune`.
Turn it off with `[checkpoints] enabled = false`.

## Editors (ACP) and headless use
- `harness acp` speaks the Agent Client Protocol on stdio, so Zed, JetBrains, Neovim, Emacs and other ACP
  clients can run the whole harness — tools, permissions, checkpoints, MCP, sub-agents — as their agent.
  Tool calls, thinking and diffs stream as `session/update`; approvals become `session/request_permission`.
- `harness run --output-format text|json|stream-json` and `--input-format stream-json` (one user message per
  line on stdin, multi-turn) use the same JSON shapes as the Claude Code CLI. `--json-schema <json|file>`
  forces the final answer to match a schema (one corrective turn, then a non-zero exit).

## Plugins & MCP
- `harness plugin list|install|enable|disable|remove|update` or `/plugin …` in the TUI. Catalog from the
  GitHub topics `harness-plugin` and `dsh-plugin` (● enabled ◐ disabled ○ downloadable). A plugin repo may
  provide skills (`SKILL.md`, exposed via `load_skill`), slash commands (`commands/*.md` → `/name`), and MCP
  servers (`mcp.json` / `.mcp.json`, or DSH `*.cordis.yml` `dsh-mcp-client` entries). TypeScript-only DSH
  plugins are flagged `ts-only` (they need the DSH runtime).
- MCP servers (stdio) from `~/.config/harness/mcp.json`, `<project>/.mcp.json`, `<project>/.harness/mcp.json`
  and enabled plugins are started once per session; their tools appear as `mcp__<server>__<tool>`.
  `harness mcp` lists them.

## Context management
The context window is detected at start (LM Studio / llama.cpp / Ollama). When the prompt exceeds
`compact_at_fraction` × context (or `context_budget_tokens`), the agent **compacts**: an LLM-written
handoff note (goals verbatim, files/commands/results with exact paths, findings, decisions, next steps)
replaces older messages; recent ones stay verbatim. `/compact [focus]` forces it.

## External tools
`harness setup` audits git, python, ripgrep, fd, jq, ffmpeg, poppler, 7-zip, unzip, tar, curl, uv, node,
gh, imagemagick, kitty; symlinks all found binaries into `~/.config/harness/bin` (first on the agent's
PATH) and `--install` adds missing ones with Homebrew.

## Architecture

```
src/
  main.rs      CLI: run | acp | eval | self | checkpoint | models | config
  config.rs    harness.toml + HARNESS_* env overrides
  llm.rs       OpenAI-compatible chat client (tools, reasoning channel, <think> stripping)
  agent.rs     the loop: model → tool calls → results → model; budgets; context compaction
  events.rs    structured Event stream + Sink trait (StderrSink, JsonlSink) — core never prints
  sandbox.rs   local process supervision: timeout, process-group kill, env scrub, output caps
  tools/       bash, read_file, write_file, edit_file, list_dir, view_image, memory, load_skill, read_pdf, pdf_edit, extract_archive, web_fetch, web_search, download_file (+ MCP tools)
  memory.rs    MEMORY/WORKFLOWS/BRAIN store, reflection, consolidation, pastes dir
  instructions.rs AGENTS.md/CLAUDE.md/HARNESS.md chain, @imports, path-scoped rules
  skills.rs / agentdefs.rs  skills and named custom agents from the standard directories
  checkpoints.rs shadow-git snapshots of the working tree (/undo, /redo, /rewind)
  acp.rs       Agent Client Protocol server (editors) · headless.rs  stream-json + --json-schema runs
  mcp.rs       MCP stdio client · plugins.rs plugin manager · setup.rs external tools · tui.rs terminal UI
  eval.rs      the fitness function: runs evals/tasks/* in fresh git-initialised workdirs
  arbiter.rs   proposal-vs-main verdict · selfimprove.rs  smart self-improvement loop (propose → gates → implement → arbiter → install)
  lib.rs       exposes all of the above as the `harness` library
evals/tasks/<name>/task.toml  (+ fixture/)  — prompt + `check` shell command (exit 0 = pass)
ui/src-tauri   Tauri 2 desktop app (Rust) · ui/dist  vanilla web frontend
```

### Layers and who may change them

| Layer | Files | Changed by |
|---|---|---|
| **Kernel** | `main.rs`, `llm.rs`, `sandbox.rs`, `agent.rs`, `eval.rs` | humans (until the eval loop has earned trust) |
| **Surface** | `tools/*`, system prompt in `agent.rs::system_prompt`, `harness.toml` | agent via `harness self` |
| **Fitness** | `evals/tasks/*` | humans add; agent may add (never weaken) |

### Arbiter
`harness arbiter proposal/x [--runs N] [--merge]` builds and tests the branch in its worktree, runs the
eval suite N times with the branch's binary, compares against a cached baseline for `main`, prints a
per-task table and a verdict (tests pass ∧ mean score not lower ∧ no always-pass→always-fail task);
`--merge` merges on green.

### Self-improvement protocol (`harness self`)
1. Requires a clean tree; creates `proposal/<slug>` from the current branch.
2. Agent reads README + relevant source, edits, must pass `cargo build --release` + `cargo test`.
3. Agent runs `harness eval` and reports score before/after, commits on the branch.
4. A human (or, later, an arbiter process) diffs `main..proposal/x`, reruns eval, merges or discards.

### Smart self-improvement loop (`harness improve`, `/improve` in the TUI)
The automated version of the protocol above, with two human gates that adapt to how smart the backend is:
1. **Propose** — a read-only agent run on the harness source (README roadmap, `docs/GAPS.md`, `TODO.md`,
   BRAIN lessons, optional focus `/improve <hint>`) returns a ranked JSON plan (`[self] max_items`).
2. **Gate 1** — `[self] auto = "smart"` (default): with a frontier backend (`provider = claude-code|anthropic`
   or a model matching `smart_models`, e.g. `claude*`) the plan is approved automatically and you are
   *informed* of what will be done; with a small local model you are *asked* to confirm (all / pick numbers /
   cancel). `auto = "always" | "never"` overrides.
3. **Implement** — each item gets its own `proposal/<slug>` branch + worktree; the agent must build, test and commit.
4. **Arbiter** — `harness arbiter`-style verdict (`arbiter_runs` eval runs per side vs the cached `main` baseline);
   green → merged into `main`. `skip_arbiter = true` gates on build + tests only.
5. **Install** — release build in a separate target dir, atomically renamed over the installed binary.
6. **Gate 2** — the TUI shows *"improved harness installed — restarting in 60s"* (`restart_grace_secs`); `esc` or
   `/cancel` keeps the running version (`/restart` later). Otherwise it re-execs the new binary and **resumes the
   session with the previously picked backend, model and effort** (also true for a plain `/restart`).

`harness improve [hint] [-y] [--no-install] [--skip-arbiter]` runs the same loop headless (`-y` answers gate 1).

Nothing self-modifies in place: the running binary is never the one being edited, and
`git` is the undo button (`git log`, `git diff`, `git revert`, branches) — for the harness
and for every eval workdir.

### Design choices
- **Rust** for a single static binary and airtight process supervision, not for LLM speed —
  the local model is the bottleneck by orders of magnitude.
- **Local processes, not containers** (for now). `sandbox.rs` supervises (timeouts, kills the
  whole process group, scrubs `*KEY*`/`*TOKEN*`/`*SECRET*` env vars, caps output) but does not
  isolate. Run the harness inside a container/VM if the model is untrusted.
- **Path jail** for file tools (no escaping the workdir, symlink-aware). `bash` is not jailed —
  it can't be without a container; the system prompt + git history are the guardrail.
- **Vision**: Qwen3.8 is a VLM; `view_image` attaches a file as an `image_url` part in a follow-up
  user turn (tool results are text-only in the OpenAI protocol). Old image payloads are dropped on compaction.
- **Downloads**: `download_file` fetches big files with parallel HTTP-range segments, checkpoints
  progress in `<file>.harness-dl.json`, retries with backoff, and **auto-resumes** on the next call
  after a timeout/crash; optional sha256 verification. `harness tool download_file '{...}'` runs it standalone.
- **Internet**: `web_fetch` (HTML→text, size cap) and `web_search` (DuckDuckGo HTML, no API
  key). Toggle with `[net] enabled` or `HARNESS_NET=0` / `--no-net`.
- **Context**: when the prompt exceeds `context_budget_tokens`, old tool results are compacted
  to a stub (the model is told to re-run if needed). Set LM Studio's context length ≥ 32k.
- **UI-agnostic core**: the loop emits `Event`s; the CLI is one `Sink`. A web/Tauri UI with
  image/audio previews is a second sink over HTTP/WebSocket — see roadmap.

## Roadmap
- [ ] `harness serve`: HTTP + WebSocket/SSE server exposing runs and the event stream
- [x] Tauri desktop UI with rich previews (images, audio, video, text)
- [ ] UI: eval runner view, `self` mode, streaming tokens, diff viewer, run history
- [ ] Streaming responses (token-level events)
- [ ] Arbiter: automated `main..proposal/*` evaluation with N-run averaging + regression gate
- [ ] More tools: `grep`/`glob` (ripgrep-backed), `apply_patch`, LSP diagnostics, image input for VL models
- [ ] Larger, harder eval corpus (SWE-bench-lite subset, repo-level tasks); per-task token/time budgets in the score
- [ ] Optional container backend for `sandbox.rs`
