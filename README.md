# TheHarness

A local-first, self-improving agentic coding harness. Rust core, any OpenAI-compatible
local model (built and tested against **Qwen3.8-27B via LM Studio**; also works with
`llama-server` and Ollama).

The thesis: give the agent a *real* toolchain (shell, git, filesystem, internet, build
tools) **and its own source**, but only let it improve itself through an
**eval-gated loop** — proposals land on git branches and are judged by a benchmark score,
never by the agent's own opinion.

## Quick start

```sh
# 1. Have a model served (LM Studio on :1234, or llama-server / ollama — edit harness.toml)
cargo build --release
./target/release/harness models                       # sanity check the server
./target/release/harness run -C /path/to/project "Add a --json flag to the CLI and tests"
./target/release/harness eval                          # run the benchmark, JSON report → $TMPDIR/harness-eval-runs/report.json
./target/release/harness self "Make the bash tool return the cwd in its result"   # agent edits itself on a proposal/* branch
./target/release/harness --json run "..."              # JSONL event stream on stdout (for UIs)
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

## Architecture

```
src/
  main.rs      CLI: run | eval | self | models | config
  config.rs    harness.toml + HARNESS_* env overrides
  llm.rs       OpenAI-compatible chat client (tools, reasoning channel, <think> stripping)
  agent.rs     the loop: model → tool calls → results → model; budgets; context compaction
  events.rs    structured Event stream + Sink trait (StderrSink, JsonlSink) — core never prints
  sandbox.rs   local process supervision: timeout, process-group kill, env scrub, output caps
  tools/       bash, read_file, write_file, edit_file, list_dir, view_image, web_fetch, web_search
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
