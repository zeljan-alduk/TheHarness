# TheHarness

A local-first, self-improving agentic coding harness. Rust core, any OpenAI-compatible
local model (built and tested against **Qwen3.8-27B via LM Studio**; also works with
`llama-server` and Ollama).

The thesis: give the agent a *real* toolchain (shell, git, filesystem, internet, build
tools) **and its own source**, but only let it improve itself through an
**eval-gated loop** — proposals land on git branches and are judged by a benchmark score,
never by the agent's own opinion.

## Backends
- **Local / OpenAI-compatible** (default): LM Studio, llama.cpp, Ollama, OpenAI, OpenRouter… (`base_url`, `api_key`).
- **Claude Code (subscription)**: `provider = "claude-code"` or `/backend claude [model]` (default `claude-fable-5`).
  The harness runs the official `claude` CLI headlessly (`--print`, stream-json in/out) and exposes its own tools
  to it over an MCP bridge, so permissions, hooks, memory, redaction and the UI all still apply while your
  Anthropic subscription is used through the official client. Requires Claude Code installed and logged in.
- **Anthropic API**: `provider = "anthropic"` + `ANTHROPIC_API_KEY`.

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
- **Permissions**: `[permissions] mode = bypass|auto|ask|plan` + allow/deny/ask glob rules; risky shell
  commands and writes outside the workdir prompt (y / a=always / n) in the TUI, desktop and web UIs;
  `/permissions`, `/plan`, shift+tab cycles; `harness run -y` approves non-interactively.
- **Sub-agents**: `spawn_agent {task, workdir?, read_only?}` — fresh context, same tools/policy; several in
  one turn run in parallel. Read-only tool calls in one turn also run in parallel.
- **Task hand-off**: messages typed while a task runs are queued; `/next` (⌃N) stops the current task and
  starts the next; loop detection nudges then stops repeated identical calls; reflection never blocks the
  next task.
- **Sessions**: every turn is saved under `~/.config/harness/sessions/`; `/sessions`, `/resume <n|id|last>`,
  `harness --resume <id>`, `harness -c` (continue latest for this directory).
- **Tools** (all): bash (+background), process, read/write/edit_file, apply_patch, list_dir, grep, glob,
  diagnostics, notebook_edit, view_image, read_pdf, extract_archive, memory, load_skill, todo, spawn_agent,
  web_fetch, web_search, download_file, + MCP tools. `[hooks]` run shell hooks around tools; `[security]`
  redacts secrets; `[sandbox] mode = "seatbelt"` confines shell writes (macOS).
- **Web UI**: `harness serve` (localhost:7878) — same UI as the desktop app, from any browser.

## Memory (MEMORY.md · WORKFLOWS.md · BRAIN.md)
`~/.config/harness/` holds three markdown files injected into every session: **MEMORY.md** (settings,
preferences, ideas), **WORKFLOWS.md** (named recipes), **BRAIN.md** (what the agent learned: user,
projects ledger, how-tos, lessons). The agent edits them with the `memory` tool; after substantive
runs a *reflection* call appends durable lessons; long files are *consolidated*. Evals use an isolated
store. A project can add `HARNESS.md` (like CLAUDE.md) with instructions.

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
  main.rs      CLI: run | eval | self | models | config
  config.rs    harness.toml + HARNESS_* env overrides
  llm.rs       OpenAI-compatible chat client (tools, reasoning channel, <think> stripping)
  agent.rs     the loop: model → tool calls → results → model; budgets; context compaction
  events.rs    structured Event stream + Sink trait (StderrSink, JsonlSink) — core never prints
  sandbox.rs   local process supervision: timeout, process-group kill, env scrub, output caps
  tools/       bash, read_file, write_file, edit_file, list_dir, view_image, memory, load_skill, read_pdf, extract_archive, web_fetch, web_search, download_file (+ MCP tools)
  memory.rs    MEMORY/WORKFLOWS/BRAIN store, reflection, consolidation, pastes dir
  mcp.rs       MCP stdio client · plugins.rs plugin manager · setup.rs external tools · tui.rs terminal UI
  eval.rs      the fitness function: runs evals/tasks/* in fresh git-initialised workdirs
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
