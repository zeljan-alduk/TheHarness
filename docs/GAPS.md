# TheHarness — gap analysis vs. the best agent harnesses (2026-08-16, updated overnight 2026-08-17)

Reference points: Claude Code (Anthropic), OpenCode, DeepSeek Harness (DSH), Codex CLI, Gemini CLI,
Aider, Cursor/Devin-style agents. "✅" = we have it, "◐" = partial, "❌" = missing.
Priority: P0 = decisive for daily use, P1 = strong differentiator, P2 = polish/breadth.

## 1. Core agent loop
| Capability | Us | Best-in-class | Gap / plan | Prio |
|---|---|---|---|---|
| Streaming, tool calls, reasoning channel | ✅ | all | — | |
| Multi-turn sessions, interrupt/resume | ✅ | Claude Code | — | |
| Precise LLM compaction + auto | ✅ | Claude Code `/compact` | add compaction *quality eval* (does the agent still finish after compaction?) | P1 |
| Context-length detection | ✅ | — | — | |
| **Sub-agents / task delegation** (`Agent` tool, forks, parallel workers, worktree isolation) | ✅ `spawn_agent` (parallel, depth-1; worktree isolation via `workdir`) | Claude Code, DSH subagents, Codex | `spawn_agent {task, tools, workdir, isolation}` returning a summary; parallel fan-out; worktree per agent | **P0** |
| **Plan mode / approve-before-edit** | ✅ `/plan` (permissions mode) | Claude Code plan mode, Aider `/ask` | read-only mode + plan file + "execute plan" | P1 |
| **Permission system** (allow/deny rules, approval prompts, sandbox levels) | ✅ bypass/auto/ask/plan, rules, TUI/desktop/web prompts, seatbelt | Claude Code, Codex sandbox | rules in config; TUI approval prompt; per-tool allowlist; "ask on network / write outside cwd" | **P0** |
| Hooks (pre/post tool, on stop, on prompt) | ✅ `[hooks]` | Claude Code hooks, DSH hooks pkg | shell hooks in config; JSON on stdin; block/allow/rewrite | P1 |
| Task list / TODO tracking visible in UI | ✅ `todo` tool + Tasks panel | Claude Code TodoWrite, OpenCode todowrite, dsh-web-ui task board | `todo` tool + panel section | P1 |
| Deterministic multi-agent workflows (scripts) | ✅ TOML workflows (`/workflow`, `harness workflow`) | Claude Code `Workflow` | after sub-agents: a small workflow runner (pipeline/parallel/judge) | P2 |
| Retry / recovery from server hiccups | ✅ | — | — | |
| Cost/latency accounting per turn | ✅ | Claude Code `/cost` | per-tool timing table | P2 |

## 2. Tools
| Capability | Us | Best | Gap | Prio |
|---|---|---|---|---|
| bash, read/write/edit, list_dir, grep, glob, apply_patch | ✅ | all | — | |
| Background processes (dev servers, watch) | ✅ `bash background` + `process` | Claude Code `run_in_background`, Monitor | `bash {background:true}` + `process` tool (list/tail/kill) | P1 |
| Web fetch/search, downloads | ✅ (+segmented downloads) | — | search provider choice (SearXNG, Brave, Exa) | P2 |
| Vision (view_image), image paste, video frames | ✅ | Claude Code image paste | screenshots tool (`screenshot {app/url}`) via `screencapture` / headless browser | P1 |
| **Browser automation** (Chrome DevTools MCP / Playwright) | ◐ via MCP | Claude Code (chrome-devtools MCP), Gemini CLI | ship a default `mcp.json` with chrome-devtools + playwright, one-command enable | P1 |
| LSP (diagnostics, go-to-def, rename) | ✅ `lsp` tool + `diagnostics` | Claude Code LSP, OpenCode lsp | rust-analyzer/pyright/tsserver via LSP client; `diagnostics` after edits | P1 |
| PDF, archives | ✅ (agent-built) | — | — | |
| Notebook editing | ✅ `notebook_edit` | Claude Code | `notebook_edit` | P2 |
| MCP client (stdio + streamable HTTP) | ✅ | all | OAuth, resources & prompts, sampling | P2 |
| Skills (SKILL.md) + slash commands from plugins | ✅ | Claude Code skills/plugins | skill *frontmatter triggers* & auto-suggest; commands with args schema | P2 |

## 3. Self-improvement & evaluation (our differentiator)
| Capability | Us | Best | Gap | Prio |
|---|---|---|---|---|
| Eval-gated `self` mode + **arbiter** (`harness arbiter`) | ✅ | (unique) | schedule arbiter runs nightly | P2 |
| Eval corpus | ◐ (20 tasks, checks validated) | SWE-bench-style suites | 30–50 tasks across langs; repo-level tasks; timing/token cost in the score; flaky-run averaging | **P0** |
| Reflection → BRAIN.md; consolidation | ✅ | (unique-ish; DSH goal/memory pkgs) | evaluate memory usefulness (A/B with memory off) | P1 |
| Prompt/policy tuning by the agent | ◐ (system prompt still in code) | — | make system prompt + tool descriptions data files the agent can propose changes to, measured by eval | P1 |

## 4. UI / UX
| Capability | Us | Best | Gap | Prio |
|---|---|---|---|---|
| Claude-Code-style TUI, dashboard, mouse, folding, images/video | ✅ | Claude Code / dsh-TUI | syntax highlighting in code blocks & diffs; **diff viewer** for edits (before/after) | P1 |
| Session persistence / resume (`--resume`, `/sessions`) | ✅ | Claude Code, OpenCode, Aider | save transcript JSONL per session; `/resume` picker; auto-title | **P0** |
| Desktop app (Tauri) | ✅ basic | DSH desktop, OpenCode desktop | eval view, self-mode view, diff viewer, run history | P2 |
| Web UI / remote control (phone) | ✅ `harness serve` (localhost by default) | DSH web UI, agentrq | `harness serve` (HTTP+WS) reusing Event stream | P2 |
| Notifications when a long run finishes | ✅ macOS | Claude Code push | macOS notification via `osascript` | P2 |
| Themes / config UI | ◐ `/theme light|dark` | many | light theme, `/theme` | P2 |

## 5. Model & provider
| Capability | Us | Best | Gap | Prio |
|---|---|---|---|---|
| OpenAI-compatible servers, Anthropic Messages API (provider=anthropic), model switch, aux routing | ✅ | all | — | |
| Prompt caching awareness | ❌ | Claude Code | keep system prompt stable & prefix-cache friendly; measure TTFT | P2 |
| Structured output / JSON mode for reflection & compaction | ◐ | — | use server JSON schema mode when available | P2 |

## 6. Safety & ops
| Capability | Us | Best | Gap | Prio |
|---|---|---|---|---|
| Path jail, env scrub, timeouts, process-group kill, optional seatbelt | ✅ | Codex sandbox | container backend | P2 |
| Secrets handling | ✅ redaction of known token formats | — | redact secrets in tool outputs/logs; `.env` never sent to the model | P1 |
| Telemetry/logging of runs (JSONL) | ✅ sessions + event logs | — | always-on run log under `~/.config/harness/logs/` for post-mortems and eval mining | P1 |
| Tests / CI for the harness itself | ✅ 24 unit + 36-step headless e2e + GitHub Actions | — | GitHub Actions: build, tests, headless e2e, `harness eval` nightly with a small model | P1 |

## Status after the overnight pass (2026-08-17)
Done: permissions, sub-agents, grep/glob/apply_patch, sessions, arbiter, 14-task corpus, background
processes, hooks, todo panel, diffs, HTTP MCP, diagnostics, notebook edit, seatbelt, secrets redaction,
notifications, event logs, theme, web UI, CI, task hand-off (queue/next/loop detection).
Closed since: LSP client, workflows, Anthropic adapter, syntax highlighting, prompt-as-data, browser-MCP
defaults, /context map, compaction progress + map, temps/power in dashboard, Windows build (harness.exe
links; CI matrix macOS/Linux/Windows). Remaining: container sandbox, MCP resources/prompts/OAuth,
larger eval corpus, runtime validation on Windows/Linux (CI does unit tests + tool smoke there).
