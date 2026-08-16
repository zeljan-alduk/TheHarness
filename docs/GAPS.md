# TheHarness — gap analysis vs. the best agent harnesses (2026-08-16)

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
| **Sub-agents / task delegation** (`Agent` tool, forks, parallel workers, worktree isolation) | ❌ | Claude Code, DSH subagents, Codex | `spawn_agent {task, tools, workdir, isolation}` returning a summary; parallel fan-out; worktree per agent | **P0** |
| **Plan mode / approve-before-edit** | ❌ | Claude Code plan mode, Aider `/ask` | read-only mode + plan file + "execute plan" | P1 |
| **Permission system** (allow/deny rules, approval prompts, sandbox levels) | ❌ (bypass only) | Claude Code, Codex sandbox | rules in config; TUI approval prompt; per-tool allowlist; "ask on network / write outside cwd" | **P0** |
| Hooks (pre/post tool, on stop, on prompt) | ❌ | Claude Code hooks, DSH hooks pkg | shell hooks in config; JSON on stdin; block/allow/rewrite | P1 |
| Task list / TODO tracking visible in UI | ❌ | Claude Code TodoWrite, OpenCode todowrite, dsh-web-ui task board | `todo` tool + panel section | P1 |
| Deterministic multi-agent workflows (scripts) | ❌ | Claude Code `Workflow` | after sub-agents: a small workflow runner (pipeline/parallel/judge) | P2 |
| Retry / recovery from server hiccups | ✅ | — | — | |
| Cost/latency accounting per turn | ✅ | Claude Code `/cost` | per-tool timing table | P2 |

## 2. Tools
| Capability | Us | Best | Gap | Prio |
|---|---|---|---|---|
| bash, read/write/edit, list_dir | ✅ | all | **grep/glob tools** (ripgrep-backed, structured output) instead of shelling out; **apply_patch** (unified diff) | **P0** |
| Background processes (dev servers, watch) | ❌ | Claude Code `run_in_background`, Monitor | `bash {background:true}` + `process` tool (list/tail/kill) | P1 |
| Web fetch/search, downloads | ✅ (+segmented downloads) | — | search provider choice (SearXNG, Brave, Exa) | P2 |
| Vision (view_image), image paste, video frames | ✅ | Claude Code image paste | screenshots tool (`screenshot {app/url}`) via `screencapture` / headless browser | P1 |
| **Browser automation** (Chrome DevTools MCP / Playwright) | ◐ via MCP | Claude Code (chrome-devtools MCP), Gemini CLI | ship a default `mcp.json` with chrome-devtools + playwright, one-command enable | P1 |
| LSP (diagnostics, go-to-def, rename) | ❌ | Claude Code LSP, OpenCode lsp | rust-analyzer/pyright/tsserver via LSP client; `diagnostics` after edits | P1 |
| PDF, archives | ✅ (agent-built) | — | — | |
| Notebook editing | ❌ | Claude Code | `notebook_edit` | P2 |
| MCP client (stdio) | ✅ | all | **HTTP/SSE MCP transport**, OAuth, resources & prompts (not only tools), server sampling | P1 |
| Skills (SKILL.md) + slash commands from plugins | ✅ | Claude Code skills/plugins | skill *frontmatter triggers* & auto-suggest; commands with args schema | P2 |

## 3. Self-improvement & evaluation (our differentiator)
| Capability | Us | Best | Gap | Prio |
|---|---|---|---|---|
| Eval-gated `self` mode on branches/worktrees | ✅ | (unique) | **arbiter**: auto-run eval on `main..proposal/*` N times, regression gate, auto-merge on green | **P0** |
| Eval corpus | ◐ (3 tasks) | SWE-bench-style suites | 30–50 tasks across langs; repo-level tasks; timing/token cost in the score; flaky-run averaging | **P0** |
| Reflection → BRAIN.md; consolidation | ✅ | (unique-ish; DSH goal/memory pkgs) | evaluate memory usefulness (A/B with memory off) | P1 |
| Prompt/policy tuning by the agent | ◐ | — | make system prompt + tool descriptions data files the agent can propose changes to, measured by eval | P1 |

## 4. UI / UX
| Capability | Us | Best | Gap | Prio |
|---|---|---|---|---|
| Claude-Code-style TUI, dashboard, mouse, folding, images/video | ✅ | Claude Code / dsh-TUI | syntax highlighting in code blocks & diffs; **diff viewer** for edits (before/after) | P1 |
| Session persistence / resume (`--resume`, `/sessions`) | ❌ | Claude Code, OpenCode, Aider | save transcript JSONL per session; `/resume` picker; auto-title | **P0** |
| Desktop app (Tauri) | ✅ basic | DSH desktop, OpenCode desktop | eval view, self-mode view, diff viewer, run history | P2 |
| Web UI / remote control (phone) | ❌ | DSH web UI, agentrq | `harness serve` (HTTP+WS) reusing Event stream | P2 |
| Notifications when a long run finishes | ❌ | Claude Code push | macOS notification via `osascript` | P2 |
| Themes / config UI | ❌ | many | light theme, `/theme` | P2 |

## 5. Model & provider
| Capability | Us | Best | Gap | Prio |
|---|---|---|---|---|
| OpenAI-compatible local servers, model switch | ✅ | all | Anthropic/OpenAI/Gemini cloud providers as optional backends; per-task routing (small model for reflection/compaction) | P1 |
| Prompt caching awareness | ❌ | Claude Code | keep system prompt stable & prefix-cache friendly; measure TTFT | P2 |
| Structured output / JSON mode for reflection & compaction | ◐ | — | use server JSON schema mode when available | P2 |

## 6. Safety & ops
| Capability | Us | Best | Gap | Prio |
|---|---|---|---|---|
| Path jail, env scrub, timeouts, process-group kill | ✅ | Codex sandbox (seatbelt), Claude Code sandbox | macOS `sandbox-exec` / container backend option; network allowlist | P1 |
| Secrets handling | ◐ | — | redact secrets in tool outputs/logs; `.env` never sent to the model | P1 |
| Telemetry/logging of runs (JSONL) | ◐ (`--json`) | — | always-on run log under `~/.config/harness/logs/` for post-mortems and eval mining | P1 |
| Tests / CI for the harness itself | ◐ (20 unit + e2e script) | — | GitHub Actions: build, tests, headless e2e, `harness eval` nightly with a small model | P1 |

## Suggested order
1. Permissions + sub-agents + grep/glob/apply_patch + session persistence (P0 core)
2. Arbiter + bigger eval corpus (P0 differentiator)
3. Background processes, LSP diagnostics, hooks, todo panel, diff viewer, HTTP MCP, browser MCP defaults (P1)
4. Cloud providers & routing, sandbox backends, secrets, CI, logs (P1)
5. Web/remote UI, notebooks, themes, notifications, workflows (P2)
