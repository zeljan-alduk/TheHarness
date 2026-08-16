# TheHarness — gap analysis vs. the 10 most popular agent harnesses (2026-08-16)

Baseline: TheHarness **1.0.017** (35 tools, 55 slash commands, 9 hook events, 4 permission modes,
local/Anthropic/Claude-Code backends). Research: four parallel web-research passes over official docs,
changelogs, GitHub/npm/PyPI stats and the Terminal-Bench 2.1 leaderboard on 2026-08-16.
Legend: ✅ have · ◐ partial · ❌ missing · **P0** do next · P1 strong differentiator · P2 polish · — out of scope.

## 1. The ten most popular agents (adoption-weighted) and what their harness is known for

| # | Agent | Signal (2026-08-16) | Harness in one line | Notable harness ideas |
|---|---|---|---|---|
| 1 | **Claude Code** (Anthropic) v2.1.233 | 141.6k★ · npm 19.7M/wk · TB2.1 #1 (83.8%) | Terminal-native agent + Desktop/IDE/web/mobile; sub-agents, hooks, MCP, SDK | ~31 hook events (command/http/prompt/agent/mcp), auto-mode classifier permissions (default since Aug 14), fork sub-agents sharing prompt cache, agent teams, JS dynamic workflows, `claude agents` supervisor, `/goal`, checkpoints (code+conversation), Remote Control + push, self-hosted cloud runners, credential-masking sandbox proxy, `/fast`, Routines/cron |
| 2 | **OpenAI Codex** (CLI + app + cloud) 0.147 | 106k★ · npm 13.7M/wk · 5M+ weekly users · TB2.1 83.1% | Rust TUI, sandboxed by default; app-server JSON-RPC protocol shared by TUI/IDE/desktop/web | Guardian auto-review of approvals, permission profiles + in-sandbox network proxy, native Windows sandbox, `/goal` GA, TOML sub-agents + proactive delegation, `unified_exec` PTY, `js_repl` code mode, hosted cached/indexed web search, Codex Remote (QR), `/import` from Claude Code/Cursor, plugins + marketplaces, `--output-schema` |
| 3 | **Cursor** (Agent / Cursor CLI) 3.12 | ~$3B ARR · 18% SO-survey · TB2.1 79.3% (CLI) | Agent-first IDE + headless `cursor-agent`; Agents Window (local/worktree/cloud/SSH) | `/best-of-n` multi-model race + judge, Auto-review run mode (allowlist → sandbox → LLM classifier), prompt-based hooks with `failClosed`, sub-agents from `.cursor/.claude/.codex/agents`, Cursor Router, Bugbot learned rules + Autofix, Automations (cron/webhooks), iOS/iPad apps, ACP agent mode |
| 4 | **GitHub Copilot** (CLI 1.0.80 + cloud agent + VS Code) | 4.7M paid seats · 20M users | Terminal + cloud agent + Agent HQ hosting Claude/Codex; multi-vendor models | `/fleet` parallel orchestrator + Critic/Rubber-duck agents, `/delegate` → draft PR + `/pr auto` drive-to-green, `/rewind` without git, Chronicle SQLite cross-session memory + Copilot Memory with citations, MXC sandbox on 3 OSes, `--cloud` sessions, reads `.claude/*`/CLAUDE.md/GEMINI.md, Agent Plugins 1.0 open spec, SDK in 6 languages, BYOK incl. Ollama |
| 5 | **OpenCode** (anomalyco) 1.18 | 198k★ (most-starred) · npm 2.35M/wk | Client/server TUI + desktop + web; provider-agnostic (75+); engine reused by Kilo | shadow-git snapshots (`/undo /redo /fork`, revert to message), `AGENTS.md` + CLAUDE.md fallback, LSP with 30+ auto-downloaded servers + 24 auto-formatters, markdown agents/commands, `doom_loop` as a permission, `.env` read denied by default, `opencode serve` + attach from TUI/phone, ACP, GitHub App w/ cron, session warming for prompt caches, `opencode-bench` |
| 6 | **Cline** 4.1 (+ CLI 3.0, SDK, Kanban) | 66k★ · ~5M installs | VS Code/JetBrains + hub-spoke daemon; sessions outlive clients | Plan/Act w/ per-mode model, agent teams (task board + mailbox), Kanban of worktree agents (also runs Claude Code/Codex/OpenCode), `--zen` fire-and-forget, chat connectors (Telegram/Slack/Discord/WhatsApp/Linear) with approvals from phone, `cline schedule` cron, shadow-git 3-way restore, dynamic command-risk classifier, 40+ providers incl. LM Studio |
| 7 | **DeepSeek Harness (dsh)** 0.1-rc | 128k★ in 3 days (launched 2026-08-13) | "Everything is a plugin" TS harness on Cordis kernel; web-first UI | Runtime profiles (standard/code/minimal/creator), sub-agent providers incl. Claude-Code and Codex children, `ralph` fresh-agent loop, `goal` tools, `run_code`, persistent PTY `terminal_*`, `cordis_*` (agent inspects/mounts its own plugins), bridges Claude Code + Codex `hooks.json`, event-log sessions w/ OTel, Landlock/Seatbelt/Windows-restricted-token sandbox fail-closed |
| 8 | **Devin / Devin Desktop** (Cognition; ex-Windsurf) | $492M ARR | Cloud VMs w/ desktop + local Rust agent in VS Code fork; Agent Command Center | Devin-manages-Devins, Confidence scores, `/handoff` local→cloud (open-source plugin for Claude Code/Codex/Cursor), Arena blind model battles, 12-event Cascade hooks via MDM, Smart approval mode, Fusion model routing, Devin Review stacked-PR merge, hosts other agents via ACP |
| 9 | **Gemini CLI** 0.55 (→ Antigravity CLI) | 106k★ · npm 332k/wk (enterprise-only since Jun 18) | Ink TUI; open core + SDK; ACP + A2A native | TOML policy engine w/ admin tiers, per-tool SandboxManager (Seatbelt/Docker/gVisor/LXC/bwrap/Windows), Before/AfterModel hooks, remote A2A sub-agents, model steering mid-turn, local Gemma router, Auto Memory → SKILL.md drafts, extensions gallery (~1.5k), `browser_agent` |
| 10 | **Factory Droid** 0.197 | vendor claims; TB1.0 #1 (Sep 2025) | Terminal droids + Factory app; Spec & Mission modes | Mission mode (orchestrator/planner/workers/validators, 1–500 features), custom droids, `droid exec` stream-jsonrpc, Droid Shield secret detection, BYOK incl. Ollama/LM Studio, sandbox + filtering proxy |

Next five: **OpenHands** (84k★, research reference harness), **Kilo Code** (27k★, OpenCode-based; Agent Manager with multi-model side-by-side, mobile remote control), **Goose** (53k★, Linux Foundation; recipes, Adversary Mode LLM watchdog, prompt-injection classifier, embedded llama.cpp, tool shim for non-tool models, symmetric ACP), **Aider** (48k★, stale since Feb 2026; repo map, architect/editor split, `AI!` watch-files), **Amp** (no approval prompts, orbs w/ webhooks + self-scheduling, Puck meta-agent). Also tracked: Zed (ACP hub, per-edit checkpoints, worktree threads), Warp (multi-harness orchestration bus, Full Terminal Use, custom model routers), Kiro (EARS specs + property-based tests, Cedar permissions, Powers), Qwen Code (`/arena`, auto-mode classifier, `/learn` auto-skills, `/dream` memory consolidation, Vision Bridge, IM channels), Muse Code, Grok Build, Junie, Crush.

**Interop standards that now define "generic":** ACP (Agent Client Protocol; v1 SDKs 1.0, 50+ agents, clients Zed/JetBrains/VS Code/nvim/Emacs/mobile), AGENTS.md (AAIF, 60k+ repos), Agent Skills `SKILL.md` (32+ tools), Agent Plugins 1.0 bundle spec, MCP 2026-07-28 (stateless, MCP Apps, tasks), Microsoft AHP, Codex app-server, A2A 1.0.

## 2. Capability matrix

### A. Instructions, memory, skills, agents (standards)
| Capability | Best-in-class | Us | Gap | Prio |
|---|---|---|---|---|
| Read `AGENTS.md` (+ walk-up, nested dirs, `AGENTS.override/local`) and fall back to `CLAUDE.md`/`GEMINI.md`/`.cursorrules`/`.github/copilot-instructions.md` | all ten | ❌ HARNESS.md only | load AGENTS.md → CLAUDE.md → HARNESS.md chain, subdir files on access, `@imports` | **P0** |
| Skills from standard dirs (`.agents/skills`, `~/.agents/skills`, `.claude/skills`, project `.harness/skills`), `paths:` gating, model-invoked + `/name` | Claude Code, Codex, Copilot, OpenCode, Goose | ◐ plugin skills only | discover standard dirs; SKILL.md frontmatter (`allowed-tools`, `model`, `effort`, `context: fork`) | **P0** |
| Named custom agents in markdown (`.harness/agents/*.md`, also read `.claude/agents`, `.cursor/agents`, `.codex/agents`): model, tools, permission mode, background, isolation | Claude Code, Cursor, Copilot, OpenCode, Kilo, Goose, Qwen | ❌ generic `spawn_agent` | agent registry + `@agent` mention + `subagent_type` | **P0** |
| Project slash commands from markdown (`.harness/commands/*.md`, `$ARGUMENTS`, `` !`shell` ``, `@file`) | Claude Code (merged into skills), OpenCode, Kilo, Qwen, Cline workflows | ◐ plugin commands only | project/user dirs; shell/file interpolation | P1 |
| Rules with path globs (`.harness/rules/*.md` `paths:`) | Claude Code, Cursor `.mdc`, Cline, Windsurf | ❌ | small addition on top of instructions loader | P1 |
| Auto-memory consolidation (`/dream` nightly), team memory in repo, memory citations | Qwen, Copilot Memory/Chronicle, Gemini Auto Memory→skills, Warp | ◐ reflection → BRAIN.md | scheduled consolidation pass; `.harness/memory/` shared, secret-scanned; auto-skill drafts | P1 |
| `/learn <url|dir>` → generate a skill; `/curator` hygiene | Qwen | ❌ | P2 |
| Plugin bundle spec (skills+agents+hooks+MCP+rules+themes), marketplaces with token-cost estimates; import Claude Code / Codex plugins | Claude Code, Codex, Copilot Agent Plugins 1.0, Cursor, Kiro Powers | ◐ GitHub-topic plugins | accept `.claude-plugin/plugin.json` + Agent Plugins 1.0 layout | P1 |

### B. Core loop
| Capability | Best-in-class | Us | Gap | Prio |
|---|---|---|---|---|
| **File checkpoints** (shadow git object DB): `/undo`, `/redo`, restore code and/or conversation, revert-to-message | Claude Code, OpenCode, Cline, Kilo, Copilot (`/rewind` w/o git), Qwen `/restore`, Zed per-edit | ❌ `/rewind` = conversation only | snapshot before each mutating tool into `~/.config/harness/snapshots/<session>` (git plumbing, untracked ≤2 MB), `/undo`, rewind menu (code / conversation / both) | **P0** |
| Session **fork** (`/fork`, fork from message, edit-and-regenerate) | Claude Code, Codex Esc-Esc, OpenCode, Cline, Goose, Qwen `/branch`, Kiro | ❌ | copy session + truncate; `--fork-session` | **P0** |
| Plan mode with plan file, plan editor, "implement in fresh context" | Claude Code (`Ctrl+G`, plansDirectory), Codex, Gemini, Kiro specs, Droid Spec | ◐ `/plan` read-only | write plan to `.harness/plans/`, approve → execute, `/plan` view | P1 |
| **`/goal <condition>`** keep working until a checker (fast model) says met; `/autopilot` | Claude Code, Codex GA, Copilot, Kiro, Qwen, DSH | ❌ | goal state + aux-model check each turn; headless too | P1 |
| Sub-agents: background by default, fork type (inherit context + cache), depth >1, per-agent model/effort, permission prompts surfaced from children | Claude Code, Codex, Cursor, Copilot, Qwen | ◐ parallel depth-1, worktree isolation | background + `/tasks`, `subagent_type: fork`, depth 2–3, budget caps | P1 |
| Agent teams / orchestrator with shared task board + mailbox; `/fleet` w/ validators; Mission mode | Claude Code teams, Cline teams, Copilot `/fleet`, Droid Mission, Devin-manages-Devins | ◐ workflows + mailbox | team primitive on top of sub-agents + todo graph | P2 |
| Model-written orchestration scripts (JS workflow / code mode `run_code`, `js_repl`) | Claude Code Workflows, Codex js_repl, Goose Code Mode, DSH run_code, OpenCode codemode | ❌ | `run_code` tool (deno/node or rhai) that can call our tools; ties to workflows | P1 |
| Hooks: `http`, `prompt` (LLM-judged), `agent` executors; more events (`PostToolUseFailure`, `PermissionRequest/Denied`, `PreCompact/PostCompact`, `Notification`, `Before/AfterModel`, `FileChanged`, `WorktreeCreate`), `once`/`async`/`if`, `updatedInput` rewrite | Claude Code (~31), Copilot (15, reads Claude hooks), Codex, Gemini, Cursor, Kiro | ◐ 9 events, command only | add http+prompt executors, `matcher` regex, tool-input rewrite, 8 more events; read `.claude/settings.json` hooks | P1 |
| Compaction: pre/post hooks, "summarize from here" via rewind menu, tool-output spill to disk, separate compaction model | Claude Code, Copilot, Gemini, OpenCode | ◐ good `/compact` | spill big results to disk + `read_file` back; compaction model = aux | P2 |
| Loop guard as a permission (`doom_loop`), consecutive-mistake limit | OpenCode, Cline | ✅ loop detection | expose as rule | — |
| Message steering mid-turn (Enter steers, Tab queues) | Codex, Cline, OpenCode V2, Gemini model steering | ◐ queue + `/next` | steer = inject user message at next tool boundary | P1 |
| `/btw` side question without polluting context; `/tangent` nestable | Claude Code, Codex `/side`, Kiro, Droid | ❌ | cheap: aux call w/ transcript excerpt | P2 |

### C. Tools
| Capability | Best-in-class | Us | Gap | Prio |
|---|---|---|---|---|
| **Persistent PTY** tool (agent drives REPL/debugger/vim; `terminal_*`, `unified_exec`) | Codex, Warp Full Terminal Use, DSH, OpenCode V2, Gemini PTY shell | ❌ bash + background only | `terminal` tool: create/write/read/wait/kill over portable-pty; needed for interactive installers/debuggers | P1 |
| Auto-formatters after edits (prettier/ruff/rustfmt…) + LSP diagnostics after every edit; auto-download language servers | OpenCode (24 formatters, 30+ LSP), Kilo | ◐ LSP client, `diagnostics` | formatter table keyed by extension; run after write/edit; auto-download rust-analyzer/pyright/tsserver | P1 |
| Repo map (tree-sitter + graph rank) under token budget for small-context models | Aider, Cline `list_code_definition_names`, Kiro tree-sitter | ❌ | matters for Qwen at 128k; `repo_map` tool + injected summary | P1 |
| Browser: bundled DevTools/Playwright agent, computer use, screenshots to model | Claude Code Chrome + computer use, Codex, Cursor, Gemini `browser_agent`, Qwen cua, Goose Peekaboo | ◐ chrome-devtools MCP default | `browser` sub-agent preset + `screenshot` tool (`screencapture`); computer use later | P2 |
| Web search: hosted/cached index, provider choice (Exa/Tavily/Brave/SearXNG), Search grounding | Codex cached/indexed, OpenCode Exa, Gemini grounding | ◐ DuckDuckGo HTML | provider config + result caching | P2 |
| MCP: OAuth (DCR/device), sampling, elicitation, roots, prompts as commands, resources `@server:uri`, tool-search deferral, MCP 2026-07-28 stateless + MCP Apps | all | ◐ stdio/HTTP + resources | OAuth + elicitation + prompts; deferred tool loading (ToolSearch-style) | P1 |
| MCP **server** mode (`harness mcp serve` exposing the agent as a tool) | Claude Code, Codex, Goose | ◐ `mcp-proxy` exposes tools | expose `harness` + `harness-reply` tools | P2 |
| Structured questions/forms to user, elicitation UI | Claude Code AskUserQuestion, OpenCode V2 forms, Goose elicitation | ✅ `ask_user` | — | |
| Vision bridge (text-only main model gets image transcription from vision model) | Qwen | ◐ aux model | route `view_image` through vision aux when main lacks vision | P2 |
| Notebook, PDF, archives, image gen | Claude Code, Codex image gen | ✅ (no image gen) | user undecided on image-gen backend | P2 |

### D. Permissions & sandbox
| Capability | Best-in-class | Us | Gap | Prio |
|---|---|---|---|---|
| **LLM-judged approvals**: allowlist → sandbox → classifier (fail-closed), NL allow/deny instructions; Guardian/Adversary reviewer | Claude Code auto mode (default Aug 14), Codex Guardian, Cursor Auto-review, Copilot `/allow-all auto`, Devin Smart, Goose smart_approve + Adversary, Qwen auto | ❌ heuristic auto | `auto` = classifier via aux model with `permissions.auto.allow/soft_deny/hard_deny`; adversary check for shell | **P0** |
| Rules: parameter matching (`Bash(git * main)`, `WebFetch(domain:)`, `mcp__*`, `Agent(model:)`), `.env` read denied by default, `external_directory` ask, protected paths, `rm -rf` breaker | Claude Code, OpenCode, Codex execpolicy, Gemini policy TOML, Kiro Cedar | ◐ glob rules | deny `.env*` reads (allow `.env.example`), external-dir ask, tool-arg matchers, ordered last-match-wins | **P0** |
| Sandbox: Linux Landlock+seccomp, native Windows sandbox, **network allowlist proxy** (domains, credential masking) | Codex, Claude Code (cred masking), Cursor, Zed, Droid, Gemini SandboxManager, DSH | ◐ seatbelt/bwrap, deny_network | HTTP(S) proxy with domain allowlist + secret masking; Windows restricted token; per-tool sandbox | P1 |
| Prompt-injection detection on tool results, secret detection before send (Droid Shield) | Goose, Droid | ◐ redaction | injection classifier (aux) on web/MCP results, flag banner | P2 |
| Managed/enterprise settings hierarchy (managed → CLI → local → project → user), MDM | all commercial | ◐ single harness.toml + overlay | layered `.harness/settings.toml` (project) + `settings.local.toml`, `--setting-sources` | P1 |
| Trusted folders w/ safe mode disabling project hooks/MCP/env | Gemini, Claude Code, Codex | ✅ trust + sanitize | — | |

### E. Sessions & surfaces
| Capability | Best-in-class | Us | Gap | Prio |
|---|---|---|---|---|
| **ACP server** (`harness acp`): run inside Zed, JetBrains, VS Code, Neovim, Emacs, mobile ACP clients | Claude, Codex, Gemini, Copilot, Cursor, Devin, Kiro, Droid, OpenCode, Goose, Cline, Qwen | ❌ | JSON-RPC over stdio: `initialize`, `session/new|prompt|cancel|load`, `session/update` streams (tool_call, diff, plan, thought), `request_permission`; our Event stream maps 1:1 | **P0** |
| ACP **client** / harness picker: run Codex, Gemini CLI, OpenCode, Copilot as backends the way we run Claude Code | Goose (ACP providers), Warp universal agents, Zed terminal threads, Copilot Agent HQ, DSH sub-agent providers | ◐ Claude Code backend only | generic `provider = "acp:<cmd>"` backend using the same bridge as Claude Code | P1 |
| Client/server split: `harness serve` + **attach the TUI** to a remote server; sessions outlive clients; mDNS; phone/web remote control with QR + push | OpenCode, Cline hub, Kilo, Qwen serve, Goose serve, Claude Remote Control, Codex Remote, Copilot `/remote`, Warp Remote Control | ◐ web UI only | `harness attach <url>`; TUI as thin client of serve; QR pairing; push via ntfy/APNs-less web push | P1 |
| Local↔cloud handoff, self-hosted runner accepting remote tasks | Claude self-hosted runners, Devin `/handoff` (open-source plugin), Amp runners, Warp, Cursor `&`, Kiro cloud | ❌ | `harness runner` daemon (accept tasks from serve/mailbox); cloud VMs out of scope | P2 |
| Share links / export+import sessions / import Claude Code & Codex transcripts | OpenCode, Kilo, Codex `/import`, Zed, Claude `/import`, Copilot `/share` | ◐ export md | `harness import ~/.claude/projects/...jsonl`, `/share` static HTML/gist | P1 |
| Session picker w/ search by content/branch, `--from-pr`, `--worktree=<PR url>` | Claude Code, Codex, Copilot | ◐ picker + fuzzy | content search index (SQLite FTS) | P2 |
| Native IDE extension (VS Code sidebar, diff in editor) | all commercial + OpenCode/Cline | ❌ (ACP covers JetBrains/Zed/nvim) | after ACP: thin VS Code ext | P2 |
| Mobile app / chat connectors (Telegram/Slack/Discord/WhatsApp/Linear) with approvals from phone | Cline, Qwen channels, Goose Telegram, Kilo, Cursor iOS, Kiro iOS, Claude mobile | ❌ | Telegram bot connector on the mailbox/inbox (approvals + messages) | P2 |

### F. Models & providers
| Capability | Best-in-class | Us | Gap | Prio |
|---|---|---|---|---|
| **Tool shim / XML fallback** for models without native tool calling; parallel tool calls; per-model-family prompt variants | Goose `GOOSE_TOOLSHIM`, Cline XML fallback + family prompts, Qwen | ❌ | critical for local models: parse `<tool_call>` blocks when server lacks tools; compact prompt for small models | **P0** |
| Prompt caching control (`cache_control` breakpoints, stable prefix ordering, cache stats, keepalive warming) | Claude Code, Aider, OpenCode V2 warming, Qwen, Cline | ❌ | Anthropic `cache_control` on system+tools; show cache-hit % in `/cost`; optional warming | P1 |
| **Multi-model arena / best-of-n**: same task, N models, isolated worktrees, judge picks | Cursor `/best-of-n`, Qwen `/arena`, Windsurf Arena, Kilo Multi-Version | ❌ | natural fit: worktree tool + arbiter as judge → `/arena a,b,c "task"`; also feeds eval data | P1 |
| Model routing: auto tiers, custom YAML routers, per-role models (fast/compaction/vision/voice/subagent), fallback chains, `/fast` | Warp, Cursor Router, Kilo Auto, Gemini Auto+Gemma router, Zed per-purpose, Claude fallbackModel | ◐ main + aux | per-role model table in config; fallback chain on 5xx | P1 |
| Native providers: Gemini/Vertex, Bedrock, Azure/Foundry, Copilot OAuth, ChatGPT/Codex OAuth reuse; models.dev catalog | OpenCode (75+), Cline (40+), Goose (60+), Zed subscriptions | ◐ OpenAI-compat + Anthropic + presets | Gemini native adapter; Bedrock/Vertex via SigV4/ADC; models.dev catalog for context sizes/pricing | P2 |
| $ cost tracking with pricing table, per-tool/skill/agent breakdown, budgets `--max-budget-usd` | Claude `/usage`, Aider, OpenCode `stats`, Qwen `/stats`, Copilot credits | ◐ tokens only | pricing table (models.dev) → $ in `/cost`; budget cap | P2 |
| Effort/thinking controls per agent/skill; `ultrathink` | Claude, Codex, OpenCode variants | ✅ effort (Claude), thinking_budget (Anthropic) | per-agent effort | — |

### G. Automation & integration
| Capability | Best-in-class | Us | Gap | Prio |
|---|---|---|---|---|
| Headless: `--output-format text|json|stream-json`, **`--json-schema` structured output**, `--input-format stream-json` (bidirectional, Claude-Code-compatible schema), exit codes, `--max-turns/--max-budget` | Claude Code, Codex `--output-schema`, Copilot, Gemini, Amp (Claude-compatible), OpenCode | ◐ JSONL sink | Claude-Code-compatible stream-json in/out (we already speak it as a client!), `--json-schema` | **P0** |
| **SDK / API**: OpenAPI HTTP server, TS + Python SDK, JSON-RPC app-server | Claude Agent SDK, Codex SDK+app-server, Copilot SDK (6 langs), OpenCode SDK, Cline SDK, Goose UniFFI, DSH py | ◐ `serve` SSE (undocumented) | document `serve` as OpenAPI; ACP doubles as programmatic API; thin Python client | P1 |
| GitHub Action / App (`@harness` on issues/PRs, `/review`, cron `prompt`) | claude-code-action, codex-action, run-gemini-cli, OpenCode app, goose action, droid-action | ❌ | `harness-action` composite action using headless mode | P1 |
| Persistent scheduler (`cron` w/ history, pause/resume, delivery), `/loop`, self-scheduling, webhook triggers | Cline schedule, Goose schedule, Claude Routines/CronCreate, Cursor Automations, Amp orbs, Qwen `/loop` | ◐ in-memory `schedule` tool | persist to `~/.config/harness/schedules.json`, `harness daemon` runs them; webhook endpoint in `serve` | P1 |
| OpenTelemetry (GenAI semconv) metrics/traces/logs; usage analytics API | Claude Code, Codex, Gemini, Copilot, Goose, Qwen, DSH | ❌ event logs only | OTLP exporter from Event stream (feature-flagged) | P2 |
| PR review bot w/ learned rules, autofix, `REVIEW.md`, stacked PRs | Cursor Bugbot, Devin Review, Kilo, Copilot code review, Claude ultrareview | ◐ `/review`, `/pr-comments` | `harness review --pr N --comment/--fix` via `gh` | P2 |
| Spec-driven dev: EARS requirements/design/tasks + property-based test generation | Kiro, Droid Spec | ❌ | `/spec` workflow (plan-mode variant) later | P2 |

### H. UI / UX
| Capability | Best-in-class | Us | Gap | Prio |
|---|---|---|---|---|
| Diff review pane: per-hunk accept/reject, comments routed back to agent, word-level diffs | Zed multibuffer, Warp Code Review (R/E), Cursor, Kilo, Cline, Copilot `/diff` | ◐ `/diff` display | interactive `/review-diff`: hunk list, `a` accept / `r` reject → `git checkout -p` semantics, comment → prompt | P1 |
| Statusline custom script; footer items | Claude, Codex, Copilot, Qwen | ❌ | `ui.statusline = "cmd"` receiving JSON | P2 |
| Themes (10+ built-ins, custom JSON, auto-detect terminal bg), screen-reader mode, i18n | OpenCode, Gemini, Copilot, Claude | ◐ light/dark | theme files in `~/.config/harness/themes/`; auto via OSC 11 | P2 |
| Voice dictation (on-device Whisper) | Claude, Codex, Copilot, Kiro, Cursor, Qwen, Goose | ❌ | `/voice` via whisper.cpp binary in tools bin | P2 |
| Notifications: sounds, OSC progress, `terminalSequence`, push to phone | Claude, OpenCode attention, Codex | ◐ macOS notify | OSC 9/777, sound packs; push via serve | P2 |
| Prompt history search `Ctrl+R`, `Ctrl+G` external editor, `@` mentions, `!` shell mode, image chips | all | ◐ ↑/↓ history, tab completion | `!cmd` shell mode, `Ctrl+R` fuzzy history, `Ctrl+G` $EDITOR, `@path` completion | P1 |
| Transcript search `/`, `/focus`, highlight-reel timeline, `/recap`, `/insights` | Claude, Copilot | ◐ fold/scroll | `/` search in transcript; `/recap` via aux | P2 |
| Native binary, PowerShell/Windows without Git Bash | Claude native binary, Codex Rust | ✅ Rust binary; ◐ Windows | Windows runtime validation | P1 |

### I. Self-improvement & evaluation (our differentiator)
| Capability | Others | Us | Gap | Prio |
|---|---|---|---|---|
| Eval-gated self-improvement loop, arbiter, BRAIN reflection | none ship it in-product (Cline & goose blog it; Qwen's release pipeline runs SWE-bench/TB via Harbor; Roo had an evals dashboard; opencode-bench, Aider polyglot) | ✅ unique | keep; add Terminal-Bench-2 style task import (Harbor format) so we can compare externally | P1 |
| Eval corpus size/quality (26 tasks) | SWE-bench Verified 500, TB2.1 89, Aider polyglot 225 | ◐ | 50+ tasks, repo-level, timing/cost in score, flaky averaging | P1 |
| Arena/best-of-n as data source for the arbiter | Cursor/Qwen/Windsurf arenas | ❌ | see F | P1 |
| Prompt-as-data (system prompt, tool descriptions, compaction prompt overridable files) | Goose `prompts/*.md`, Gemini `GEMINI_SYSTEM_MD` | ✅ `prompts/system.md` | tool descriptions + compaction/plan/subagent prompts as files | P2 |

## 3. Roadmap distilled from the matrix

**P0 — do next (parity items every top-10 harness has, cheap relative to value):**
1. Instruction files: `AGENTS.md` → `CLAUDE.md` → `HARNESS.md` chain, walk-up + nested, `@imports`; skills from `.agents/skills`/`.claude/skills`/project; agents from `.harness/agents` (+ `.claude/agents`); rules with `paths:`.
2. File checkpoints (shadow git) with `/undo` `/redo` and rewind menu (code / conversation / both); `/fork`.
3. `harness acp` (ACP v1 server) — turns the harness into a Zed/JetBrains/VS Code/nvim agent for free.
4. Tool shim for non-tool-calling local models + compact prompt variant.
5. Auto mode = LLM classifier (aux model) with allow/soft-deny/hard-deny + `.env`/external-dir/tool-arg rules.
6. Headless stream-json in/out (Claude-Code-compatible) + `--json-schema`.

**P1:** PTY tool; formatters + auto LSP; repo map; hooks http/prompt + more events; sub-agent background/fork/depth; `/goal`; steering; `run_code`; arena/best-of-n via arbiter; per-role model table + fallback chain; prompt caching; ACP client backend (Codex/Gemini/OpenCode as engines); TUI attach to `serve` + remote control; import Claude Code/Codex sessions + share; persistent scheduler + daemon; GitHub Action; interactive diff review; layered project settings; network-allowlist proxy; `Ctrl+R`/`Ctrl+G`/`@`; TB2-format eval import; Windows runtime validation.

**P2:** browser/computer-use presets, web-search providers, MCP OAuth/elicitation/Apps, MCP server mode, `/btw`, agent teams, spec mode, memory consolidation + `/learn`, OTel, PR bot, statusline/themes/voice/i18n/sounds, native providers (Gemini/Bedrock/Vertex), $ cost + budgets, connectors (Telegram), runner mode.

**Out of scope for now:** hosted cloud VMs / marketplaces / SSO-SCIM-MDM enterprise controls / IDE-native tab completion — we are a local, single-binary harness; ACP + serve give the integration surface instead.

## 4. Where we are ahead
Eval-gated self-improvement with an arbiter (nobody ships it); Claude Code as a *backend* (subscription reuse) with sub-agents bridged through MCP; single static Rust binary with dashboard (tok/s, temps, power); precise LLM compaction with before/after context map; drag-select copy + toast, font control; cross-session messaging (Claude Code only added it Aug 7); TOML deterministic workflows; task hand-off queue + loop detection; 29-tool integration + 61-step e2e harness for the harness itself.

---
# Appendix — earlier gap analyses (kept for history)

## Original analysis (2026-08-16 morning, updated overnight)

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


## Update 2026-08-16 (afternoon) — parity pass merged
Added: ask_user (AskUserQuestion), Inbox + wakeups, monitor, schedule, notify, report_findings, mcp_resources,
plan_mode, agents, run_workflow, todo task graph, worktree tool, sub-agents under the Claude Code backend +
Agents panel + /agents attach/kill/steer, Claude Code backend (subscription) with /backend, effort, remote
/compact, context window detection, hooks parity, project-scoped permissions + trust, fuzzy resume + auto
titles, markdown tables/lists, vim mode + keybindings, cross-session messaging, provider presets, bwrap,
plugin update-all, versioning 1.0.NNN, 25 eval tasks, 29-tool integration script, 60-step e2e.
Remaining: mid-turn steering of Claude sub-sessions, hosted/cloud sessions + phone push, MCP OAuth,
Windows/Linux runtime validation beyond CI, native Gemini/Bedrock/Vertex adapters, marketplace ratings.

---
# Part II — full research notes (verbatim from the 2026-08-16 research passes)

Everything below is the raw material behind Part I: the popularity ranking with sources, then per-harness
feature inventories. Items marked *(unverified)* were not confirmed against a primary source.

## II.1 Ranking data (GitHub API / npm / PyPI queried 2026-08-16)

| # | Agent | Vendor / OSS | GitHub stars (repo) | Latest release / cadence | Adoption evidence | Benchmarks | One-liner |
|---|---|---|---|---|---|---|---|
| 1 | Claude Code | Anthropic (source-available repo, npm binary) | 141,641 (anthropics/claude-code) | v2.1.233 on 2026-08-14; near-daily | npm 19.69M/wk; ~$2.5B run-rate Feb 2026 → ~$8B May 2026 *(secondary)*; "4% of public GitHub commits" *(secondary)*; 18% workplace usage in JetBrains AI Pulse Jan 2026 | Terminal-Bench 2.1 #1: 83.8% (Fable 5); Opus 5 86.7% per Meta's Muse Code comparison | Terminal-native agentic coder; subagents, hooks, MCP, SDK |
| 2 | OpenAI Codex (CLI + app + cloud) | OpenAI, Apache-2.0 | 106,234 (openai/codex) | rust-v0.148.0-alpha.20 on 2026-08-16; multiple alphas/day | npm 13.71M/wk; 5M+ weekly users (OpenAI, 2026-06-02; up from 600k Jan 2026) | TB 2.1: 83.1% (GPT-5.5), 78.4% (GPT-5.6 Terra) | Rust terminal agent + IDE ext + cloud tasks; sandboxed by default |
| 3 | Cursor (Agent, Cursor 3, Cursor CLI) | Anysphere, closed | n/a (closed) | Cursor 3 "Glass" 2026-04-02; CLI updated 2026-06-30 | $3B ARR / $29.3B valuation Jul 2026 *(secondary)*; 1M+ DAU (2025); 18% in Stack Overflow 2026 survey; 18% JetBrains AI Pulse | TB 2.1: Cursor CLI 79.3% (Grok 4.5) | Agent-first IDE + headless `cursor-agent` CLI, background agents |
| 4 | GitHub Copilot (agent mode + coding agent + Copilot CLI) | Microsoft/GitHub, closed (CLI repo is tracker) | 11,099 (github/copilot-cli) | CLI v1.0.80 on 2026-08-14; ~daily | npm @github/copilot 1.56M/wk; 4.7M paid subs (MSFT FY26 Q2, Jan 2026); 20M total users (Jul 2025); 68% of AI-tool users in SO 2026 survey; ~140k enterprise orgs; agent mode GA Mar 2026, usage billing Jun 2026 | not on TB leaderboard | Broadest install base; agent in VS Code/JetBrains/Xcode, async coding agent on github.com, terminal CLI |
| 5 | OpenCode | Anomaly (ex-sst), MIT | 198,019 (anomalyco/opencode) — most-starred coding agent | v1.18.18 on 2026-08-13; several/wk | npm opencode-ai 2.35M/wk; "7.5M monthly developers" *(unverified vendor claim)*; engine reused by Kilo Code | not listed on TB 2.1 | Provider-agnostic open-source TUI agent; server used by other tools |
| 6 | Cline | Cline Bot Inc, Apache-2.0 | 66,272 (cline/cline) | v4.1.10 on 2026-08-14; ~weekly | 5M+ VS Code installs *(secondary)*; CLI 2.0 headless; SDK | not listed | VS Code/JetBrains extension + CLI + SDK, BYOK, plan/act |
| 7 | DeepSeek Harness (dsh) | DeepSeek, MIT | 128,353 (deepseek-ai/deepseek-harness) — repo created 2026-08-13 | v0.1 dev preview 2026-08-13; too new for cadence | npm @deepseek-ai/dsh 195,945/wk in first week; ~27.5k stars day 1 → 92.7k day 2 → 128k day 3 (VentureBeat / justin3go) | none published yet | "Everything is a plugin" harness on Cordis; ships with V4-Pro-0813 |
| 8 | Devin / Devin Local (Devin Desktop, ex-Windsurf) | Cognition, closed | n/a | Windsurf → Devin Desktop 2026-06-02; Cascade EOL 2026-07-01 | $492M ARR May 2026, $1B raise at ~$25-26B; user counts undisclosed | historically SWE-bench; no current TB entry | Cloud autonomous engineer + Rust local agent in VS Code fork with Agent Command Center |
| 9 | Gemini CLI | Google, Apache-2.0 (enterprise-only access since 2026-06-18) | 106,531 (google-gemini/gemini-cli) | v0.56.0-nightly 2026-08-16; still nightly | npm 332k/wk (down; free/Pro/Ultra cut off 2026-06-18, migrated to closed Antigravity CLI); 6,000+ external PRs merged | TB 2.1: 65.8% (Gemini 3.1 Pro) | Terminal agent; now enterprise-licensed, successor Antigravity CLI is closed Go binary |
| 10 | Factory Droid | Factory AI, closed (npm/curl CLI) | n/a | continuous; npm @factory/cli only 2.5k/wk (mainly curl-installed) | "hundreds of thousands of developers daily" (Factory, Jun 2026, unverified); $150M Series C at $1.5B (Apr 2026); Nvidia/Adobe/EY customers | #1 Terminal-Bench 1.0 (58.75%, Sep 2025); no TB 2.1 entry found | Multi-model terminal droids for enterprise, CI headless mode |

Next 5 (honorable mentions):
- **OpenHands** (OpenHands/OpenHands, MIT) — 84,193 stars; v1.13.0 2026-08-13 (weekly); PyPI 174k/wk; 72–77.6% SWE-bench Verified; research reference harness; primary surface now Agent Canvas web UI.
- **Kilo Code** (Kilo-Org/kilocode, Apache-2.0) — 26,887 stars; v7.4.22 2026-08-13; "3M+ users, 40T tokens" *(vendor)*; rebuilt on OpenCode server Apr 2026; absorbed Roo Code users (Roo archived 2026-05-15 at 24,338 stars, 3M installs).
- **Goose** (aaif-goose/goose, Apache-2.0, now under Linux Foundation AAIF, ex-Block) — 52,864 stars; v1.46.0 2026-08-12 (~biweekly).
- **Aider** (Aider-AI/aider, Apache-2.0) — 48,267 stars; PyPI still 193k/wk but last release v0.86.2 2026-02-12, last push 2026-05-22 — declining maintenance.
- **Amp** (Sourcegraph, closed) — npm @sourcegraph/amp 46k/wk; "40,000 teams in first two months of 2026" *(review blog, unverified)*; ad-supported free tier.

Also considered (excluded / lower): Kiro (AWS; closed, tracker repo 4,191 stars, Q Developer folded into it May 2026); Zed agent (zed 88,685 stars, v1.15.0 2026-08-12; ACP hub for 20+ agents); Warp (64,251 stars on tracker repo, closed; hosts other agents; claims 75.8% SWE-bench Verified); Continue (35,501 stars, v2.1.0 2026-06-19; CLI 4.2k/wk); Qwen Code (27,067 stars, v0.21.12-preview 2026-08-16, npm 47.8k/wk); Charm Crush (27,409 stars, v0.89.0 2026-08-12, npm 7.6k/wk); Augment (auggie; npm 35.6k/wk); Junie CLI (JetBrains, GA 2026-06-17, 372 stars, npm 1.9k/wk); Trae (ByteDance; 1.6M MAU / 6M registered Dec 2025; trae-agent 12,023 stars, last push 2026-02-05); Grok Build (xAI, 2026-05-14), Muse Code (Meta, beta 2026-08-05, TB 2.1 82.9% claimed), Antigravity CLI (Google, closed), Jules (Google, GA Apr 2026): too new / no adoption data; Replit Agent, Bolt, Lovable: app builders, excluded.

Ranking justification: ordered by weighted adoption evidence — paid/weekly users and revenue first, then distribution (npm/PyPI weekly downloads), then stars, then benchmark presence. Claude Code and Codex lead on every measurable axis. Cursor and Copilot are closed but have the largest disclosed user/revenue bases. OpenCode is the largest OSS project by stars and downloads. Cline leads the VS Code-extension class. DSH is placed on momentum alone (128k stars / 196k npm in 72 h) — provisional. Devin ranks on ARR only. Gemini CLI has stars but shrinking reach post-June-18 cutoff. Droid is the weakest #10; OpenHands would be a defensible swap.

Sources: GitHub API repos/releases (2026-08-16); api.npmjs.org; pypistats.org; https://www.tbench.ai/leaderboard/terminal-bench/2.1; https://www.morphllm.com/best-ai-coding-agents-2026; https://venturebeat.com/technology/deepseek-harness-launches-as-open-source-rival-to-claude-code-alongside-v4-pro-on-api-with-higher-prices; https://justin3go.com/en/posts/2026/08/15-deepseek-harness-review; https://tech-insider.org/ie/openai-codex-5-million-users-2026/; https://www.gradually.ai/en/codex-statistics/; https://www.gradually.ai/en/claude-code-statistics/; https://aibusinessweekly.net/p/claude-code-statistics; https://www.getpanto.ai/blog/cursor-ai-statistics; https://blog.mean.ceo/cursor-news-july-2026/; https://axis-intelligence.com/github-copilot-statistics/; https://www.solidaitech.com/2026/06/github-copilot-complete-guide.html; https://byteiota.com/stack-overflow-dev-survey-2026-ai-at-84-trust-at-3/; https://www.developersdigest.tech/blog/opencode-developer-guide-2026; https://ghtrends.dev/anomalyco/opencode/; https://thenewstack.io/google-antigravity-cli/; https://developers.googleblog.com/an-important-update-transitioning-gemini-cli-to-antigravity-cli/; https://kilo.ai/compare/roo-vs-cline-vs-kilo; https://theaiagentindex.com/compare/roo-code-vs-kilo-code; https://webdeveloper.com/news/windsurf-devin-desktop-cascade-eol/; https://enterprisedna.co/resources/news/cognition-devin-1-billion-25-billion-valuation-2026/; https://www.developersdigest.tech/blog/factory-droid-review-setup-2026; https://factory.ai/news/terminal-bench; https://baeseokjae.github.io/posts/amp-code-review-2026/; https://tensorfeed.ai/harnesses/openhands; https://www.openhands.dev/blog/openhands-index; https://techcrunch.com/2026/08/05/meta-launches-muse-code-an-ai-agent-for-large-code-bases/; https://x.ai/news/grok-build-cli; https://cursor.com/blog/cli; https://www.learncursor.dev/guides/cursor-cli; https://andrew.ooo/answers/jetbrains-junie-ga-out-of-beta-june-2026/; https://news.aibase.com/news/18830; https://www.digitalapplied.com/blog/amazon-kiro-aws-agentic-ide-complete-guide

## II.2 Feature inventory — Claude Code, OpenAI Codex, Gemini CLI, GitHub Copilot CLI

Method: four parallel research passes over official docs/changelogs, plus direct verification of Claude Code weekly digests (code.claude.com/docs/en/whats-new), Codex changelog/releases, Gemini CLI changelog/releases, Copilot CLI changelog.

### Claude Code (Anthropic) — CLI v2.1.233 (2026-08-14) + Desktop + VS Code/JetBrains + claude.ai/code web + mobile + Chrome
Sources: https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md · https://code.claude.com/docs/en/whats-new · https://code.claude.com/docs/llms.txt

Core loop
- Plan mode: `Shift+Tab` cycle, `/plan [desc]`, `--permission-mode plan`; read-only exploration; approve→pick next mode; `Ctrl+G` edit plan in `$EDITOR`; rejection feedback; `plansDirectory`; built-in Plan subagent; `opusplan` alias (Opus plans, Sonnet executes).
- Ultraplan (cloud plan drafting w/ web editor) shipped early preview 2.1.101 (Apr 2026), removed in v2.1.222 (Aug 2026).
- `/goal <condition>` (v2.1.139, May 2026): fast model checks condition after each turn, keeps working until met; works in interactive, `-p`, Remote Control.
- Sub-agents: `.claude/agents/*.md` (user/project/plugin/managed/`--agents` JSON); frontmatter `model`, `tools`, `disallowedTools`, `permissionMode`, `skills`, `hooks`, `memory` scope, `isolation: worktree`, `background`, `effort`, `maxTurns`, `initialPrompt`, `mcpServers`. Built-ins: Explore, Plan, general-purpose, statusline-setup, claude-code-guide. Background by default (v2.1.198, Jul 2026); nested subagents (default depth 3, `CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH`; background chains capped 5); 20 concurrent (`CLAUDE_CODE_MAX_CONCURRENT_SUBAGENTS`); 200/session cap removed (v2.1.224); permission prompts surface in main session; `--max-budget-usd` enforced on subagents.
- Fork subagents (`subagent_type: "fork"`, inherits full context + prompt cache) default on v2.1.232 (Aug 13); `/subtask`.
- Agent teams (experimental, `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1`, v2.1.32 Feb 2026): lead + teammates + shared task list + mailboxes; tmux/iTerm2 split-pane display; `TeammateIdle`/`TaskCompleted` hooks.
- Dynamic workflows (v2.1.154, May 28): Claude writes JS orchestration script driving dozens–hundreds of subagents (16 concurrent, 1000/run); `/workflows`; `ultracode` keyword; bundled `/deep-research`.
- Agent view `claude agents` (research preview, v2.1.139): dashboard of all background sessions (running/blocked/done), dispatch flags (`--add-dir --settings --mcp-config --plugin-dir --permission-mode --model --effort`), attach/peek/pin, `claude attach|logs|stop|respawn|rm`, `claude daemon`; background sessions auto-commit/push/draft-PR (v2.1.198+); `/fork` copies session to a background session in its own worktree (v2.1.212/2.1.224).
- Cross-session messaging (v2.1.224, Aug 7; macOS/Linux): `ListAgents` + `SendMessage` between your sessions; `/list-agents`; `crossSessionInbound` accept/hold/refuse.
- Worktrees: `claude --worktree|-w [name]`, `EnterWorktree`/`ExitWorktree` tools, `worktree.baseRef`, `.worktreeinclude`, `WorktreeCreate/Remove` hooks; `--worktree` accepts PR/MR URLs (v2.1.233); isolation enforced on Bash + git redirects too (v2.1.224).
- Background tasks: `Ctrl+B` backgrounds bash/agents; auto-background long bash; `/tasks`; Monitor tool streams process/WebSocket events into conversation (v2.1.98); MCP calls >2 min auto-background; progress heartbeat for long tools.
- Hooks: types `command` (exec-form `args`), `http`, `prompt`, `agent`, `mcp_tool`; ~31 events incl. `SessionStart/End`, `Setup`, `UserPromptSubmit`, `PreToolUse` (allow/deny/`defer`, `updatedInput`), `PermissionRequest/Denied`, `PostToolUse` (`continueOnBlock`), `PostToolUseFailure`, `Stop`, `SubagentStart/Stop`, `TeammateIdle`, `TaskCreated/Completed`, `Notification`, `InstructionsLoaded`, `ConfigChange`, `CwdChanged`, `FileChanged`, `WorktreeCreate/Remove`, `PreCompact/PostCompact`, `Elicitation`; `if` filters, `once`, `async`, `terminalSequence` output; `$CLAUDE_EFFORT`, `CLAUDE_PROJECT_DIR`.
- Checkpoints/rewind: auto file snapshots; `/rewind`/`Esc Esc`; restore code+conversation / conversation / code; "Summarize from/up to here" (compress context via Rewind menu); rewind past `/clear` (v2.1.191).
- Compaction/context: auto-compact, `/compact [focus]`, `/autocompact <tokens>`, `/context` grid; big tool results spilled to disk; MCP result cap override to 500K; 1M context (Opus 4.6+/Sonnet 4.6+/Sonnet 5/Opus 5/Fable 5), `[1m]` alias; `CLAUDE_CODE_MAX_CONTEXT_TOKENS`.
- Memory/instructions: CLAUDE.md hierarchy (managed → `~/.claude/CLAUDE.md` → project (walk-up) → `CLAUDE.local.md`), lazy subdir CLAUDE.md, `@path` imports; `.claude/rules/*.md` w/ `paths:` globs; auto memory (`MEMORY.md` index, `/memory`); `/init` (reads Cursor/Copilot rules); `/import codex|gemini`; output styles; `--append-system-prompt`.
- Tasks: `TaskCreate/…` w/ dependencies (Jan 2026), `Ctrl+T`; task tools removed on Opus 4.8/Sonnet 5/Fable 5+ (v2.1.233), `CLAUDE_CODE_ENABLE_TODO_TOOLS=1`.
- Thinking/effort: extended/adaptive thinking; `ultrathink`; `/effort` slider `low|medium|high|xhigh|max` (+`ultracode`), `--effort`, per-skill/agent effort; default high, xhigh recommended on Opus 4.7+.

Tools
- Built-ins: Bash, PowerShell (Windows; no Git Bash needed since ~v2.1.120), Read (PDF/images), Write, Edit, NotebookEdit, Glob, Grep, WebFetch, WebSearch, Agent, Skill, AskUserQuestion, EnterPlanMode/Exit, EnterWorktree/Exit, Monitor, LSP, Task*, Cron*, ScheduleWakeup, SendMessage, ListAgents, PushNotification, SendUserFile, Artifact, RemoteTrigger, Workflow, ToolSearch, MCP resource tools, EndConversation.
- Shell mode `!cmd` (Claude responds to output, v2.1.186); `Ctrl+E` command-risk explanation; memory cgroup limit; shell snapshots.
- Vision: paste/drag images (`[Image #N]` chips), images from phone via Remote Control.
- LSP tool (Dec 2025): def/refs/hover/symbols/call hierarchy/diagnostics via `.lsp.json` plugins.
- Browser: Claude in Chrome (`--chrome`, GA v2.1.198 Jul 1 2026: DOM/console, forms, uploads, GIF recording); Desktop in-app browser (w28); computer use in Desktop (Mar) + CLI (Apr, macOS Pro/Max); Desktop iOS Simulator pane (w30).
- MCP client: stdio/HTTP/SSE/WebSocket, OAuth (`claude mcp login|logout` v2.1.186), claude.ai connectors, elicitation, `roots`, resources `@`-mention, prompts as commands, ToolSearch auto-defer, `mcp__server__*` perms, `managed-mcp.json`, `--strict-mcp-config`. MCP server: `claude mcp serve`.
- Channels (research preview v2.1.80): MCP servers push events (Telegram/Discord/iMessage) into running session; phone permission relay.
- Skills (`SKILL.md`; merged w/ slash commands): frontmatter `context: fork/background`, `allowed-tools`, `model`, `effort`, `hooks`, `paths`; `$ARGUMENTS`, `!cmd`, `@file`; hot reload; stacked `/a /b`; bundled `/code-review`(`/review`), `/simplify`, `/verify`, `/run`, `/loop`, `/batch`, `/dataviz`, `/security-review`, `/team-onboarding`.
- Plugins + marketplaces: bundle skills/agents/hooks/MCP/LSP/themes/monitors/bin; `claude plugin …`; `--plugin-dir` (.zip), `--plugin-url`; sources git/GitHub/GitLab/npm/zip `archive` (SHA-256)/`command`; official `anthropics/claude-plugins-official`; `/plugin` w/ token-cost estimates; Claude Security plugin (multi-agent vuln scan, w30).
- ~100 slash commands (`/btw`, `/cd`, `/diff`, `/export`, `/insights`, `/recap`, `/usage`, `/doctor`, `/radio`, `/powerup` …).

Permission & sandbox
- Modes: `default` (labelled Manual), `acceptEdits`, `plan`, `auto`, `dontAsk`, `bypassPermissions`; `--dangerously-skip-permissions`; `--permission-prompt-tool` (MCP-handled prompts headless).
- Auto mode (classifier reviews actions; research preview v2.1.83 Mar 24 → Pro plan → Bedrock/Vertex/Foundry → default for new sessions on Pro/Max/Team from Aug 14 2026); `autoMode.allow/soft_deny/hard_deny`, blocks transcript tampering & destructive git; classifier calls don't count against usage.
- Rules: `permissions.allow/ask/deny` w/ `Bash(git * main)` wildcards, path rules, `WebFetch(domain:)`, `mcp__*`, `Agent(model:opus)` param matching (v2.1.178); `/permissions` UI; workspace trust; protected paths; `rm -rf` circuit breaker.
- OS sandbox (Oct 2025+): macOS Seatbelt, Linux/WSL2 bubblewrap+socat; filesystem/network allowlists, `tlsTerminate` proxy, credential masking (`mode: "mask"`, JWT decode, AWS SigV4 re-sign; v2.1.221–224), `sandbox.filesystem.disabled`; not native Windows.
- Managed settings: `managed-settings.json` + `.d/`, macOS plist/Windows registry, server-managed via claude.ai admin; `allowManaged*Only`, `strictKnownMarketplaces`, `disableSideloadFlags`, `requiredMinimumVersion`, `availableModels`, `forceLoginMethod`.
- `--safe-mode` (all customizations off), `--bare`, `--tools ""`.

UI/UX
- Custom keybindings `~/.claude/keybindings.json` (chords, contexts; Jan 2026); themes incl. custom JSON/plugin themes (Apr 2026), auto-match terminal; vim mode (NORMAL/INSERT/VISUAL, text objects, `jj` remaps); fullscreen renderer w/ mouse, transcript search `/`, `/focus`; `/diff` viewer; `/statusline` custom script (context %, cost, rate limits, effort, PR badge); notifications (desktop, bell, OSC progress, `terminalSequence`, mobile push); `/voice` dictation (20 langs); `Ctrl+R` history, `Ctrl+G` editor, `@` mentions, prompt suggestions, emoji shortcodes, `/recap`, `/btw`; screen-reader mode (v2.1.208); native binary CLI (Apr 2026); `/doctor` full checkup.
- VS Code extension: Focus view (w32), diff viewer, resume claude.ai sessions, `/remote-control`.
- Desktop (macOS/Windows/Linux beta w27): parallel sessions, visual diff, terminal/editor, in-app browser, iOS Simulator, scheduled tasks.

Sessions
- `--continue`, `--resume [id|name]` (PR-URL search), `--from-pr`, `--fork-session`, `--name`, `/rename`, `/branch`, `/fork` (background copy in own worktree), `claude project purge`, `/export`.
- Claude Code on the web (claude.ai/code): cloud VMs or self-hosted environments (`claude self-hosted-runner`, public beta Team/Enterprise, v2.1.224), teleport web↔CLI (`--teleport`, `--cloud`, `&` prefix), auto-fix PRs, sharing links, GHES.
- Remote Control (`--rc`, `claude remote-control`): drive local session from claude.ai/mobile; Trusted Devices; push notifications; disabled with API-key auth / 3P providers.
- Artifacts: live shareable pages that update as session works; call viewers' MCP connectors (w29); GA on Pro/Max/Team/Enterprise.
- Deep links `claude-cli://open?q=…`; Desktop↔IDE↔web "Continue in".

Team/enterprise
- Settings precedence: managed > CLI args > local > project > user; `--setting-sources`; `ConfigChange` hook audit.
- OpenTelemetry metrics/events/traces (OTLP/Prometheus, mTLS, `OTEL_LOG_USER_PROMPTS`, `X-Claude-Code-Session-Id`).
- Auth: claude.ai OAuth/SSO, Console keys, `claude setup-token`, WIF, `forceLoginOrgUUID`; providers Bedrock/Vertex (Google Cloud Agent Platform)/Microsoft Foundry; Claude apps gateway (`claude gateway`: IdP SSO, group model allowlists, spend limits, OTLP).
- Analytics dashboard (Team/Enterprise), `/usage` per-skill/subagent/plugin/MCP breakdown, `--max-budget-usd`, org default model & per-role effort caps, version pinning.
- Managed PR reviewer (Team/Enterprise research preview), Slack app / Claude Tag.

Models
- Opus 5 (default Opus, Jul 24 2026, 1M ctx, fast $10/$50), Sonnet 5 (default Pro/Team Std/Enterprise seats, Jun 30, native 1M), Fable 5 (Jun 9, `/model fable`/`best`), Opus 4.8/4.7/4.6, Sonnet 4.6, Haiku 4.5; aliases `opus/sonnet/haiku/best/fable/opusplan/[1m]`.
- Fast mode `/fast` (~2.5× speed; Opus 5 & 4.8 only now); `fallbackModel` chains (3); classifier content fallback; `/advisor` second-model consult (experimental, API only).
- Prompt caching automatic (`ENABLE_PROMPT_CACHING_1H`, cache-hit breakdown in `/usage`).
- Third-party: Bedrock/Vertex/Foundry/LLM gateways via `ANTHROPIC_BASE_URL` (feature deltas: no fast mode/Remote Control/Chrome/voice). Local/non-Claude models: not supported officially; community shims exist *(unverified)*.

Automation
- `claude -p`, `--output-format text|json|stream-json`, `--json-schema` structured output, `--input-format stream-json`, `--bare`, `--max-turns`, `--max-budget-usd`, `--allowedTools`, `defer`+`-p --resume`, Setup hooks `--init/--maintenance`.
- Agent SDK (Python `claude-agent-sdk`, TS `@anthropic-ai/claude-agent-sdk`): hooks, subagents, MCP-as-callbacks, `canUseTool`, sessions, structured output.
- GitHub Actions `anthropics/claude-code-action` (`@claude`, `prompt` mode, Bedrock/Vertex/Foundry), GitLab CI/CD (beta), `claude ultrareview --json` in CI.
- Scheduling: `/loop` (self-pacing), `CronCreate` in-session cron, Routines on web (`/schedule`; schedule/API POST/GitHub-event triggers), Desktop scheduled tasks.
- Webhooks: HTTP hooks (out), Routine API triggers (in), Channels (push in), Monitor (WebSocket).

Distinctive / new in 2026: auto-mode classifier permissions (default Aug 14); cross-session messaging (Aug 7); self-hosted cloud runners (Aug 7); dynamic workflows (May 28); `claude agents` supervisor + `/goal` (May 11); fork subagents (Aug 13); agent teams (Feb 5); credential-masking sandbox proxy (Aug); Fable 5/Sonnet 5/Opus 5; fast mode; Remote Control + push; Routines; Ultrareview; Chrome GA + CLI computer use + iOS Simulator; Artifacts w/ MCP connectors; Channels; Monitor tool; 31-event hook system w/ http/prompt/agent hooks; screen-reader mode; apps gateway; task tools retired on 5.x models; native binary + PowerShell/Windows-no-Git-Bash.

### OpenAI Codex — CLI rust-v0.147.0 (2026-08-07; 0.148.0-alpha.20 Aug 16) + IDE ext + ChatGPT desktop app + Codex cloud + GitHub/Slack/Linear + Remote + SDK
Sources: https://learn.chatgpt.com/docs/changelog · https://learn.chatgpt.com/docs/llms.txt · https://github.com/openai/codex/releases

Core loop
- Plan mode `/plan [prompt]` (default-on since 0.94.0), dedicated plan view, `plan_mode_reasoning_effort`, implement in fresh context (0.122).
- Goal mode `/goal <objective>` (view/edit/pause/resume/clear; GA 2026-05-21; multi-hour/day).
- Sub-agents default-on: built-ins `default|worker|explorer`; custom TOML agents in `~/.codex/agents/` / `.codex/agents/` (`developer_instructions`, `model`, `sandbox_mode`, `mcp_servers`); `[agents] max_concurrent_threads_per_session`; `/agent`; `spawn_agents_on_csv` fan-out; path addresses; delegation mode disabled/explicit/proactive ("Ultra" = proactive).
- Parallel/worktrees: desktop app Local / Worktree / Cloud environments + Handoff between them; IDE `/worktree`; CLI has no built-in worktree manager *(unverified)*.
- Background: `unified_exec` PTY tool, `/ps`, `/stop`; `!cmd` concurrent w/ turn; side chats `/side` (`/btw`).
- Hooks (GA 2026-05-14): `SessionStart/End`, `SubagentStart/Stop`, `PreToolUse`, `PermissionRequest`, `PostToolUse`, `PreCompact/PostCompact`, `UserPromptSubmit`, `Stop`; `~/.codex/hooks.json`, `.codex/hooks.json`, `[hooks]` in config, plugin `hooks/hooks.json`, managed `requirements.toml`; regex matchers, JSON stdin/out, `decision: block`, `async: true`, `/hooks` trust-by-hash.
- Rewind: no filesystem checkpoint/undo in CLI (docs recommend git); `Esc Esc` edits prior message + forks a contextual branch (0.145); app-server `thread/rollback`; desktop app fork-from-message + review-pane revert.
- Compaction: `/compact`, `model_auto_compact_token_limit`, remote (server-side) compaction incl. Bedrock (0.147), Pre/PostCompact hooks; `/status`, `/usage daily|weekly`, rollout token budgets (0.142).
- AGENTS.md: `~/.codex/AGENTS.override.md` → `~/.codex/AGENTS.md`; project root→cwd chain of `AGENTS.override.md`/`AGENTS.md`/`project_doc_fallback_filenames`, `project_doc_max_bytes` 32 KiB; `/init`; `## Code Review Rules` section drives GitHub review.
- Local memories (experimental opt-in `features.memories`, `/memories`); `personality`; steering (Enter mid-turn) / queueing (Tab); `request_user_input`; Fast mode `/fast` (service tier); realtime voice + hold-space dictation in TUI.
- Reasoning: `model_reasoning_effort minimal|low|medium|high|xhigh` (+`max`, `ultra`), `Alt+,`/`Alt+.`, `model_reasoning_summary`, `model_verbosity`.

Tools
- Shell (`shell`, `unified_exec`, `shell_environment_policy`, `shell_snapshot`), `apply_patch`, bundled ripgrep, `@` fuzzy mention.
- Web search hosted: `web_search = disabled|cached|indexed|live` (`cached` = OpenAI index default; `--search` live), `allowed_domains`.
- Browser/computer: desktop app in-app browser (annotations, CDP dev mode), Codex Chrome extension (2026-05-07), Computer Use (macOS Apr 16, Windows May 29), Appshots (⌘⌘).
- Images `-i`, paste; `view_image` tool; image generation on by default. Notebook tool: none documented; LSP: none documented *(unverified/absent)*.
- `js_repl` / Code mode (model calls tools from JS; remote Code Mode host over WSS 0.146).
- MCP client: stdio + Streamable HTTP, OAuth (`codex mcp login`), `auth = oauth|chatgpt`, per-tool approval modes, tool search default (0.143), MCP 2026-07-28 protocol (0.147), MCP Apps/elicitations; `codex mcp add|list|…`. MCP server: `codex mcp-server` (`codex`, `codex-reply` tools).
- Apps/connectors `/apps`, `$app` mentions. Skills (agentskills.io; `.agents/skills`, `~/.agents/skills`, `/etc/codex/skills`; `$skill`; bundled skill-creator/plan; Record & Replay GUI demo → skill draft, 2026-06-18).
- Plugins (2026-03-25; `.codex-plugin/plugin.json` + skills/MCP/hooks/apps), marketplaces (`~/.agents/plugins/marketplace.json`, GitHub/git/npm), `codex plugin …`, `/plugins` (OpenAI Curated/Workspace/Personal); portable Agent Plugins (0.147); not in IDE ext. Custom prompts deprecated (2026-01-22).
- Slash commands: `/permissions /vim /keymap /theme /statusline /diff /review /model /fast /plan /goal /fork /side /ps /stop /resume /new /rename /archive /usage /import /pets …`.

Permission & sandbox
- `approval_policy = untrusted | on-request | never | {granular}`; `sandbox_mode = read-only | workspace-write | danger-full-access`; presets Auto; `--yolo` (`--dangerously-bypass-approvals-and-sandbox`); `--full-auto` removed for exec in 0.147; `--approve-for-me` (0.147).
- Permission profiles (beta): filesystem read/write/deny globs, network domain allow/deny, unix sockets, proxy; managed `allowed_permission_profiles`; in-sandbox network proxy w/ DNS-rebinding checks.
- OS sandbox: macOS Seatbelt, Linux bwrap+seccomp (from Landlock, 0.115), native Windows sandbox (elevated/unelevated), `codex sandbox <os> CMD` tester.
- Auto-review / Guardian reviewer agent adjudicates boundary-crossing approvals (`approvals_reviewer = auto_review`), fails closed; execpolicy `prefix_rule` `.rules` files (`codex execpolicy check`); trusted projects; `requirements.toml` (`/etc/codex`, MDM, cloud-managed bundles) pins approval/sandbox/web-search/MCP/features/hooks.

UI/UX
- Ratatui TUI: `@` search, `Ctrl+R` history, `Ctrl+G` editor, `Ctrl+T` transcript, Esc-Esc edit, `Alt+,/.` effort, `/keymap` custom bindings, vim mode `/vim`, `/theme` + custom `.tmTheme`, `/diff`, `/statusline` items, `/title`, notifications (`tui.notifications`, osc9/bel, `notify` external cmd), OSC-8 links, `file_opener`, `/pets`; mouse support *(unverified)*.
- IDE ext: sidebar, `/ide-context`, `/local`↔`/cloud`, `/worktree`, background-agent panel. Desktop app: review pane (multi-repo), Git/PR board, terminals, model slider (Faster↔Smarter/Max/Ultra), pop-out composer, Sites hosted deploys, artifact viewer, dictation + ChatGPT Voice, `codex://` deep links, Codex Micro keypad. Mobile: Codex Remote tasks/approvals/diffs.

Sessions
- Rollout JSONL `~/.codex/sessions/…` + SQLite state; `codex resume [--last|--all]`, `/resume` search by content/branch, `codex exec resume`, `codex fork`, `/fork`, temporary forks, `/new <name>`, `/rename`, `codex archive|delete`, pins + persistent sections (0.147); `--ephemeral`.
- Handoff `/app` (CLI→desktop), Local↔Worktree↔host handoff, IDE `/cloud`; Codex cloud `codex cloud [exec|list]`, `codex apply` diffs; Codex Remote (`codex remote-control`, QR pairing, SSH hosts; GA 2026-06-25); `codex --remote ws://` to app-server; `/import` from Claude Code/Cowork/Cursor w/ auto-sync. Share links: none for CLI *(unverified)*.

Team/enterprise
- Config: flags/`-c` → project `.codex/config.toml` (trusted) → profile → `~/.codex/config.toml` → `/etc/codex/config.toml`; Team Config in repo; `requirements.toml`/MDM/cloud bundles; `/debug-config`.
- Auth: ChatGPT OAuth (device auth), API key, access tokens, service accounts (2026-05), `forced_chatgpt_workspace_id`, keyring store; `[otel]` OTLP http/grpc events/metrics; Codex analytics dashboard + Analytics API + Compliance API; usage limits/spend controls; Codex Security product; Daybreak Blue/Red cyber tiers (2026-08-10); data residency `enforce_residency`.

Models
- GPT-5.6 Sol/Terra/Luna (272k ctx), gpt-5.3-codex-spark (Pro, ~1000 tok/s), legacy 5.5/5.4 (5.4 retires from ChatGPT auth Aug 31); effort minimal→xhigh/max/ultra; providers built-in `openai|ollama|lmstudio|amazon-bedrock`, `--oss --local-provider`, custom `[model_providers]` (Responses wire API; Chat Completions deprecated), Azure, Bedrock first-class (Jun 2026); prompt caching reported (`cached_input_tokens`) but no user config *(unverified)*; service tiers fast/flex.

Automation
- `codex exec` (`--json` JSONL events, `-o` last message, `--output-schema`, `--ephemeral`, `--sandbox`, resume); `codex review --uncommitted|--base|--commit`; SDK TS `@openai/codex-sdk` + Python `openai-codex`; app-server JSON-RPC (stdio/WS) v2 thread/turn APIs; GitHub Action `openai/codex-action@v1` (safety-strategy, effort, output-file); Codex in GitHub (`@codex review/fix`, auto PR review, `@codex security review`); Slack `@Codex`; Linear assignment; Automations (desktop/web scheduled tasks, RRULE, thread automations); webhooks none *(unverified)*.

Distinctive / new in 2026: Guardian auto-review of approvals; cached/indexed web search; permission profiles + network proxy; native Windows sandbox; goal mode GA; Codex Remote + host handoff; Computer Use + Chrome ext + Record & Replay + Computer History (Aug 13); plugins unified w/ ChatGPT directory; Codex merged into ChatGPT desktop app (Jul 9), Linux preview (Aug 11), Sites; import from Claude Code/Cursor w/ sync; Code mode/js_repl; Bedrock provider; Codex-Spark; Codex Micro hardware; Daybreak tiers; MCP 2026-07-28; enterprise service accounts/analytics API.

### Gemini CLI (Google) — v0.55.1 stable (2026-08-11), v0.56.0-preview.1, nightly 20260816
Status: Google announced (I/O, 2026-05-19) transition to closed-source Go Antigravity CLI (`agy`); on 2026-06-18 Gemini CLI + Code Assist IDE ext stopped serving consumer Google-login tiers; Code Assist Standard/Enterprise, Vertex, paid API keys continue; repo in maintenance mode (Apache-2.0). https://developers.googleblog.com/an-important-update-transitioning-gemini-cli-to-antigravity-cli/ · https://github.com/google-gemini/gemini-cli/discussions/27274

Core loop
- Plan mode (stable v0.37 Apr 8): Shift+Tab cycle Default→Auto-Edit→Plan, `/plan`, `--approval-mode=plan`; `enter/exit_plan_mode` tools; plans in `~/.gemini/tmp/<proj>/<sess>/plans/`; research subagents; Pro plans/Flash executes (`general.plan.modelRouting`).
- Model steering mid-turn (`experimental.modelSteering`).
- Subagents (default on v0.35): `invoke_subagent`, `@agent_name`; built-ins `codebase_investigator`, `generalist`, `cli_help`, `browser_agent`, memory-manager; custom Markdown+YAML in `~/.gemini/agents/`, `.gemini/agents/`, extensions (`tools`, `mcpServers`, `model`, `max_turns`); parallel invocation; remote A2A subagents (`kind: remote`, agent cards, OAuth/ADC).
- Git worktrees `gemini --worktree` (v0.36). Background shell (`is_background`/`&`), `/shells`, Ctrl+B, `backgroundCompletionBehavior`.
- Hooks (on by default v0.27): `SessionStart/End`, `BeforeAgent/AfterAgent`, `BeforeModel/AfterModel`, `BeforeToolSelection`, `BeforeTool/AfterTool`, `PreCompress`, `Notification`; settings.json or extension `hooks/hooks.json`; JSON in/out (`decision`, `hookSpecificOutput` incl. tool_input rewrite, `llm_request/response`, `clearContext`); `/hooks list|enable|disable`.
- Checkpointing (`general.checkpointing.enabled`, shadow git repo, `/restore`); `/rewind` (Esc Esc: conversation/code/both).
- Compression: `/compress` (`/compact`), `model.compressionThreshold` 0.5, iterative anchored compression, tool-output masking, JIT context discovery, Chapters/topic narration, ContextManager/Sidecar (v0.39); 1M-token window.
- GEMINI.md: global → workspace ancestors (`memoryBoundaryMarkers`) → JIT subdirs; `context.fileName` (array, e.g. AGENTS.md), `@file.md` imports; `/memory show|refresh|add|inbox`; four-tier memory (v0.40); Auto Memory inbox mining sessions into patches + SKILL.md drafts (v0.42).
- `write_todos` (Ctrl+T), experimental Task Tracker; thinking via `modelConfigs` `thinkingBudget/thinkingLevel`; loop detection.

Tools
- `run_shell_command` (PTY interactive shell), `read_file`, `write_file`, `replace`, `list_directory`, `glob`, `grep_search` (bundled ripgrep v0.40), `read_many_files`, `google_web_search` (Search grounding), `web_fetch` (20 URLs), `ask_user`, `activate_skill`, MCP resource tools, `update_topic`.
- Browser: `browser_agent` bundling chrome-devtools-mcp (v0.31). Multimodal `@image.png`, clipboard paste all OSes, drag-drop. Notebook: `.ipynb` special-cased in write only; LSP: none *(unverified/absent)*.
- MCP client: stdio/SSE/HTTP, OAuth (`dynamic_discovery`, `google_credentials`, SA impersonation), `gemini mcp add|list|remove|enable|disable`, `/mcp auth|schema|reload`, prompts as slash commands, `@server://resource`. Not an MCP server; instead ACP + A2A server.
- Extensions (`gemini-extension.json`: mcpServers, contextFileName, commands TOML, hooks, skills, agents, policies, themes, settings→keychain); `gemini extensions install|update|link|new|validate|config`; gallery geminicli.com/extensions (~1,480 entries, ~65 first-party: conductor, security, code-review, cloud-run, firebase, genkit, flutter, stitch, nanobanana, jules, workspace, bigquery, gke…).
- Custom commands TOML (`{{args}}`, `!{shell}`, `@{file}`); Skills (agentskills.io; `.gemini/skills`, `.agents/skills`; `gemini skills install`; built-in skill-creator/pr-creator/code-reviewer/ci…; stable v0.27).
- Voice mode (Gemini Live cloud or local Whisper; v0.41–42).

Permission & sandbox
- Approval modes `default|auto_edit|plan|yolo` (`--approval-mode`, `--yolo`, Ctrl+Y, `security.disableYoloMode`); allow once/session/"all future sessions" (writes policy).
- Policy engine TOML `[[rule]]` (toolName wildcards, `commandPrefix/Regex`, `argsPattern`, `modes`, `subagent`, priority; tiers Default<Extension<Workspace<User<Admin; admin dirs `/etc/gemini-cli/policies`; `--policy`, `--admin-policy`); `tools.core`/`tools.allowed`; shell prefix allowlisting w/ chain splitting.
- Sandbox: `-s`/`GEMINI_SANDBOX=docker|podman|sandbox-exec|runsc|lxc`; macOS Seatbelt profiles (permissive/restrictive/strict ×open/proxied, custom `.sb`); container image + custom Dockerfile; gVisor/LXC/bubblewrap; native Windows sandbox (v0.36); per-tool SandboxManager (v0.35) w/ dynamic expansion requests.
- Trusted folders (untrusted by default v0.24; safe mode disables project settings/MCP/hooks/env), `--skip-trust` for headless; env-var redaction; `security.auth.enforcedType`; Conseca LLM security policy (opt-in).

UI/UX
- Ink TUI; themes (Dracula, GitHub, Tokyo Night, colorblind, custom, auto by terminal bg); vim mode `/vim`; custom `~/.gemini/keybindings.json` (v0.35); mouse/alt-buffer, Ctrl+S; F12 DevTools; `/editor` external editors, "Modify with external editor" on diffs; `/footer` custom, context %, `/stats`; notifications osc9/osc777/bell; `/copy`, `/settings`, `/bug`, `/directory add`, `/terminal-setup`; screen-reader mode; IDE Companion (VS Code + forks: native diff, open files, selection); JetBrains/Zed via ACP.

Sessions
- Auto-saved JSONL sessions; `-r/--resume [latest|id]`, `--list-sessions`, `--delete-session`, `/resume` browser, `/chat save|resume|share`, export/import (v0.43), 30-day retention; ACP `loadSession`. No first-party phone/remote session feature *(unverified/absent)*; sibling Jules async cloud agent + `@google/jules` CLI.

Team/enterprise
- Precedence: defaults → system-defaults → user → project → system (`/etc/gemini-cli/settings.json`) → env → flags; enterprise wrapper doc; cloud Admin Controls (Strict Mode, extensions/MCP toggles, MCP allowlist, required MCP servers, skills off); OTel (`telemetry.target local|gcp`, OTLP, traces, GCP Cloud Trace/Monitoring); Code Assist Standard 1,500 / Enterprise 2,000 req/day; Vertex ADC/SA/express; GDC air-gapped; `agy plugin import gemini` migration.

Models
- `gemini-3.1-pro-preview`, `gemini-3.5-flash` (GA May 2026), `gemini-3-flash-preview`, `3.1-flash-lite`, 2.5 family, Gemma 4 (via API, default v0.42); aliases `auto|pro|flash`; unified Auto router (v0.44) w/ classifier + local Gemma router (`gemini gemma setup`, LiteRT-LM); `modelConfigs.modelChains` fallback; thinking config; auth Google login (consumer ended), API key, Vertex, Compute ADC, gateway base URL; token caching auto for API key/Vertex; no OpenAI-compatible endpoints; local chat models none (only router).

Automation
- `-p` headless, stdin piping, `--output-format text|json|stream-json`, exit codes 42/53; `--session-summary`; `.env` loading; `GEMINI_SYSTEM_MD` override.
- GitHub Action `google-github-actions/run-gemini-cli` (v0.1.22; triage/review/dispatch workflows, WIF, `@gemini-cli /review`); ACP `--acp` (Zed/JetBrains); SDK `@google/gemini-cli-sdk` (v0.30); A2A server package; Docker image; scheduling only via Action cron.

Distinctive / new in 2026: weekly Tuesday preview/stable + nightly cadence; open-source TUI+core+SDK+A2A; Search grounding tool; TOML policy engine + cloud Admin Controls; gVisor/LXC/bwrap/Seatbelt/Windows per-tool sandboxing; local Gemma router + Gemma 4; browser agent; remote A2A subagents; model steering; Chapters; Auto Memory skill extraction; voice mode; extension gallery; ACP-native; superseded by Antigravity CLI (adds async `/agents`, `/tasks`, `/permissions`, OS sandbox, multi-model incl. Claude/gpt-oss).

### GitHub Copilot CLI (v1.0.80, 2026-08-14; GA 2026-02-25) + Copilot cloud agent + VS Code agent mode + Agent HQ + Copilot app
Sources: https://github.com/github/copilot-cli/blob/main/changelog.md · https://docs.github.com/en/copilot · https://github.blog/changelog/

Core loop
- Modes: interactive / plan / autopilot (Shift+Tab; `--mode`, `--plan`, `--autopilot`; `/autopilot`=`/goal`; `--plan --mode autopilot` plan-then-implement v1.0.79); plan panel, plan approval dialog, `/model plan` separate model; autopilot until `task_complete`, `--max-autopilot-continues`; `/allow-all auto` LLM safety-judge approvals (experimental).
- `/fleet` parallel orchestrator w/ validation (GA v0.0.411, Feb 2026); Critic agent; Rubber-duck agent (default on); `/research`, `/refine`, `/ask`, `/security-review`, `/review`; `update_todo`.
- Custom agents `.github/agents/*.agent.md`, `~/.copilot/agents`, `.claude/agents`, plugins (frontmatter `tools`, `model`, `mcp-servers`, `skills`, `target`); built-ins explore/task/general-purpose/code-review/research/critic/configure-copilot; background multi-turn subagents (`write_agent`, `/tasks` tree, teleport into subagent), `subagents.maxDepth`.
- Hooks `.github/hooks/*.json`, `~/.copilot/hooks/`, settings.json (also reads `.claude/settings.json`): sessionStart/End, userPromptSubmitted/Transformed, preToolUse, postToolUse(+Failure), preCompact, permissionRequest, agentStop, subagentStart/Stop, errorOccurred, notification, preMcpToolCall; types `command|http|prompt`; Claude/VS Code PascalCase names + tool-name mapping accepted; `permissionDecision`, `modifiedArgs`, `additionalContext`.
- `/rewind` (`/undo`, Esc Esc; conversation and/or Copilot-changed files; no git needed v1.0.78); `/diff` viewer; `/fork`; `/worktree`, `/new-worktree`, `--worktree`.
- Context: auto-compaction ~80%, `/compact`, checkpoint summaries, "infinite sessions", `/context`, context tiers (1M), large output spill, deferred tool loading, embedding-based skill/MCP retrieval; Copilot Memory (repo facts w/ citations, `/memory`, 28-day expiry); `/chronicle` SQLite cross-session store (standup, cost-tips, skill drafts).
- Instructions: `.github/copilot-instructions.md`, `.github/instructions/*.instructions.md` (`applyTo`), AGENTS.md, CLAUDE.md, GEMINI.md, `~/.copilot/…`, `@`-imports, `/init`, `/instructions`.

Tools
- Shell (bash/pwsh, background `read_bash/write_bash`, `$` interactive), view/create/edit/`apply_patch`, grep/glob (`tgrep` trigram monorepo index v1.0.79), `web_fetch`, GitHub MCP web_search, `ask_user`, `store_memory`, `create_pull_request`, sql, LSP tools (`~/.copilot/lsp-config.json`, `.github/lsp.json`; `/lsp`).
- Images (`@`, drag/paste, PDFs/HEIC, `--attachment`, inline Kitty rendering); voice input `/voice` (on-device). Notebook tool none *(unverified)*.
- MCP client: stdio/HTTP/SSE, OAuth (DCR/device/client_credentials/Entra), sampling, elicitation, resources, tasks; `~/.copilot/mcp-config.json`, `.mcp.json`, `.github/mcp.json`; `/mcp …`, registry install, org allowlists; built-in GitHub MCP server (toolset flags). Not an MCP server; exposes ACP (`--acp`) and `--server` JSON-RPC.
- Browser: none bundled in CLI (add Playwright MCP); cloud agent ships Playwright MCP.
- Skills (`.github/skills`, `.claude/skills`, `.agents/skills`, `~/.copilot/skills`; `/skills`, `copilot skill add`); `.claude/commands` → skills; `.github/prompts/*.prompt.md` VS Code format (CLI support *unverified*).
- Plugins: `copilot plugin install|marketplace …`, `/plugins`; Agent Plugins 1.0 open spec GA 2026-08-12 across VS Code/CLI/SDK/app; reads `.claude-plugin/plugin.json`; marketplaces `copilot-plugins`, `awesome-copilot`; CLI JS Extensions (`@github/copilot-sdk`, canvases). Copilot Extensions (GitHub Apps) sunset 2025-11-10.
- `/delegate` / `&` → cloud agent draft PR; `/pr view|create|fix ci|fix conflicts|auto|automerge`; `#123` refs; Issues/PRs/Gists tabs.

Permission & sandbox
- Per-tool approvals once/session/folder-remembered; `--allow-all-tools|paths|urls`, `--allow-all`/`--yolo`, `--allow-tool 'shell(git:*)'`/`--deny-tool`, `--add-dir`, `/permissions`, `/allow-all auto`; enterprise `permissions.disableBypassPermissionsMode`; trusted-folder dialog; `-p` never prompts.
- Local OS sandbox (public preview, Microsoft MXC: macOS Seatbelt, Linux bubblewrap, Windows ProcessContainer; `/sandbox`, `--sandbox`; RW/RO/deny paths, network, MCP/LSP sandboxing; MDM floor v1.0.77/79). Cloud sandbox `copilot --cloud` ephemeral GitHub-hosted session (preview 2026-06-02).
- Cloud agent: GitHub Actions ephemeral runner, firewall allowlist (custom domains), `copilot-setup-steps.yml`, MCP config in repo settings, read-only GitHub token, 59-min timeout, secret scanning/CodeQL pre-PR.

UI/UX
- New terminal UI GA 2026-06-23: tab bar (Session/Gists/Issues/PRs), sessions sidebar, `/theme` (default/dim/high-contrast/colorblind/tritanopia, auto light/dark), highlight-reel timeline, screen-reader mode; `@file`, `!`/`$` shell, Shift+Enter, Ctrl+G editor, Ctrl+R history, Ctrl+F timeline search, Ctrl+T reasoning, Ctrl+Y open plan, message queueing/steering, prompt pinning; vim keys in pickers/`/diff`; mouse; `/usage`, `/model` picker (effort ←/→, context tier), `/statusline`/`/footer` scripts, `/settings`, desktop notifications, `/streamer-mode`; canvases; `/app` → GitHub Copilot desktop app (GA 2026-06-17).
- VS Code: Agents window, harness picker (Copilot SDK / Claude Agent SDK / Codex), worktrees, Plan agent, checkpoints, hooks, integrated browser, `/btw`, dictation.
- github.com/copilot/agents page + Automations; GitHub Mobile sessions + QR remote.

Sessions
- `--resume`, `--continue`, `--session-id`, `--name`, `/resume` picker (local/remote), `/fork`, `/rename`, `/session …`, `/cd`; `~/.copilot/session-state/` + SQLite; session sync to GitHub; `/share [file|gist|html]`, `--share-gist`; remote control `/remote`, `--connect` (steer from github.com/Mobile, SSO policy); cloud sessions `/delegate`, `--resume` cloud sessions locally, `--cloud`, `gh agent-task create|list|view`; Agent HQ / Mission Control w/ Claude + Codex third-party agents (preview 2026-02-04).

Team/enterprise
- Precedence: defaults → MDM → user `~/.copilot/settings.json` → repo `.github/copilot/settings.json` → `.local.json` (also `.claude/settings.json`) → env → flags; enterprise-managed settings (server/MDM/file; model, plugins, MCP allow/deny, remote control, telemetry, sandbox floor).
- Org policies (CLI enable, billed-to-org, cloud agent, third-party agents, MCP registries, memory, models, marketplaces); content exclusion; agentic audit-log events; OTel GenAI-conformant telemetry (OTLP, mTLS); OAuth/PAT/`COPILOT_GITHUB_TOKEN`, GHE.com; AI-credit billing (2026-06-01; per-model token pricing, budgets, `/limits`, usage metrics API); BYOK (`COPILOT_PROVIDER_*` openai/azure/anthropic/OpenAI-compatible incl. Ollama/vLLM/Foundry Local, `COPILOT_OFFLINE`).

Models
- GPT-5.x incl. 5.6 Luna/Sol/Terra, GPT-5.3-Codex; Claude Haiku 4.5, Sonnet 4.5/4.6/5, Opus 4.5–4.8/5, Fable 5; Gemini 3.1 Pro, 3.5/3.6/3.7 Flash; MAI-Code; Raptor mini; Kimi K2.7/K3; Grok 4.5/4.6; Auto server-side routing (discounted); `--effort low|medium|high|xhigh|max`; 1M context tiers; per-agent/repo-pinned models; local via BYOK/Ollama.

Automation
- `copilot -p`, stdin, `-s`, `--output-format json` (JSONL), `--no-ask-user`, `--share`, `-p --autopilot`; in Actions (`copilot-requests: write`, billed-to-org); `/every`, `/after`, `/loop` in-session schedules; cloud Automations (cron/issue/PR triggers); cloud agent triggers (issue assign, `@copilot`, REST `POST /agents/repos/{o}/{r}/tasks`, MCP `create_pull_request_with_copilot`, Slack/Teams/Jira/Linear/Raycast); Copilot code review; Copilot SDK GA 2026-06-02 (TS/Python/Go/.NET/Rust/Java; CLI as JSON-RPC server); GitHub Agentic Workflows (markdown workflows, engines Copilot/Claude/Codex/Gemini, preview); ACP.

Distinctive / new in 2026: `/delegate` + `/pr auto` drive-to-green loop; `/fleet`; Critic/Rubber-duck complementary-model agents; Chronicle + Copilot Memory w/ citations; MXC local sandbox on 3 OSes + `--cloud` sessions; remote control from github.com/Mobile; native Issues/PRs/Gists TUI tabs; LLM-judge `/allow-all auto` w/ enterprise policy; Claude/VS Code hook & config compatibility (reads `.claude/*`, CLAUDE.md, GEMINI.md); Agent Plugins 1.0 open spec; Agent HQ w/ third-party agents (Claude, Codex) + VS Code harness picker; multi-vendor model catalog + BYOK; AI-credit billing; multi-language SDK GA; desktop app GA; Agentic Workflows.

### Cross-tool quick matrix (mid-2026)

| Capability | Claude Code | Codex | Gemini CLI | Copilot CLI |
|---|---|---|---|---|
| Plan mode | yes | yes (`/plan`) | yes | yes |
| Goal/autopilot loop | `/goal` | `/goal` GA | — | `/autopilot`, `/goal` |
| Sub-agents / custom agents | yes, background default, forks, teams, JS workflows | yes, TOML agents, proactive delegation | yes, incl. remote A2A | yes, `/fleet`, background |
| Hooks | ~31 events, cmd/http/prompt/agent/mcp | 11 events, cmd, async | 11 events incl. Before/AfterModel | 15 events, cmd/http/prompt |
| Rewind/checkpoints | file+conv rewind | Esc-Esc fork only; no FS undo | `/restore` + `/rewind` | `/rewind` no-git |
| Memory files | CLAUDE.md + rules + auto-memory | AGENTS.md + opt-in memories | GEMINI.md + auto memory | AGENTS/CLAUDE/GEMINI/copilot-instructions + Memory |
| OS sandbox | Seatbelt/bwrap + cred masking | Seatbelt/bwrap+seccomp/Windows native | Seatbelt/Docker/gVisor/LXC/bwrap/Windows | MXC Seatbelt/bwrap/Windows (preview) |
| Auto/classifier approvals | auto mode (default) | Guardian auto-review | Conseca (opt-in) | `/allow-all auto` |
| MCP client / server | both | both | client (+ACP/A2A) | client (+ACP) |
| Browser/computer use | Chrome ext, computer use, iOS sim | app browser, Chrome ext, computer use | browser_agent (CDP MCP) | via MCP; cloud Playwright |
| Cloud sessions / phone | web + self-hosted runners, Remote Control | Codex cloud, Codex Remote | none (Jules sibling) | cloud agent, `--cloud`, remote control |
| Local models | not supported | Ollama/LM Studio built-in | Gemma router only | BYOK/Ollama |
| Multi-vendor models | Claude only | OpenAI (+Bedrock) | Gemini/Gemma | Claude/OpenAI/Gemini/xAI/Kimi/MAI |
| Headless JSON | `-p --output-format json/stream-json`, `--json-schema` | `exec --json --output-schema` | `-p --output-format json/stream-json` | `-p --output-format json` |
| SDK | Agent SDK Py/TS | Codex SDK TS/Py + app-server | `@google/gemini-cli-sdk` | Copilot SDK 6 langs |
| Scheduling | `/loop`, cron, Routines | Automations (app/web) | Action cron only | `/every`, Automations |
| Status | v2.1.233 active | 0.147 active | maintenance → Antigravity CLI | 1.0.80 active |

## II.3 Feature inventory — OpenCode, Aider, Cline, Roo Code + Kilo Code, Goose, Qwen Code, DeepSeek Harness

### OpenCode (anomalyco/opencode, formerly sst/opencode) — TUI + Desktop + Web
Status: stable v1.18.18 (2026-08-13); repo moved to `github.com/anomalyco/opencode`; ~195k stars; MIT; Bun/TypeScript (OpenTUI+SolidJS TUI, Electron desktop). V2 beta ships as `opencode2` (`@opencode-ai/cli@beta`, docs at opencode.ai/v2/docs).
Sources: https://github.com/anomalyco/opencode · https://opencode.ai/docs/ · https://opencode.ai/v2/docs · https://opencode.ai/changelog

Core loop
- Client/server architecture: `opencode` starts local HTTP server + TUI client; multiple clients (TUI, desktop, web, IDE, phone browser) attach to one server, sharing sessions.
- Primary agents (Tab cycles): `build` (full access, default) and `plan` (edit/bash default to `ask`); `default_agent` config.
- Built-in subagents: `general` (full access, multi-step), `explore` (read-only fast codebase search), `scout` (read-only, clones dependency repos for upstream research; experimental). Hidden system agents: `compaction`, `title`, `summary`.
- Subagent invocation: automatic via `task` tool or `@agent` mention; child sessions navigable; `subagent_depth` (default 1); `task` permission whitelists spawnable subagents; `hidden: true` agents callable only programmatically.
- Background subagents: v1.16.2 "send running subagent to background" (`ctrl+b`); `task(background=true)` gated by env; V2: `subagent` tool foreground/background + `POST /session/{id}/background` + session inbox/steer/queue endpoints (queue or steer prompts mid-run).
- Multi-session/parallel: multiple agents on same project; sessions movable between workspaces/dirs (v1.16); managed workspace cloning keeping dirty/untracked files; V2 worktree API; Desktop "workspaces" = git worktrees (auto branch+dir, auto-cleanup).
- Custom agents: JSON (`agent.<name>`) or Markdown (`.opencode/agents/*.md`, `~/.config/opencode/agents/`) with `description, mode (primary/subagent/all), model, prompt ({file:…}), temperature, top_p, steps (max iterations), permission, tools, color, hidden, disable`; `opencode agent create` wizard.
- Hooks/plugins: JS/TS plugins in `.opencode/plugins/`, `~/.config/opencode/plugins/`, or npm. Hooks: `tool.execute.before/after`, `chat.message`/`chat.params`, `permission.asked/replied`, `session.*` (created/idle/compacted/error), `file.edited`, `shell.env`, `lsp.*`, `message.*`, `tui.prompt.append/command.execute/toast.show`, `experimental.session.compacting`, `todo.updated`, `command.executed`, `installation.updated`; plugins can register custom tools. V2 plugin API rewrite (`Plugin.define`, transform hooks for agents/catalog/commands/skills/tools/references; runtime hooks `context`, `sdk`, `http.request/response`, `execute.before/after`).
- Checkpoints/undo: snapshots default on; captured before each model call and after completion; stored in a separate internal git object DB in OpenCode data dir (never touches repo commits/branches); tracks tracked+untracked (non-ignored, ≤2 MB) files; requires git repo. `/undo` (hides messages, restores files, refills composer), `/redo`, message-level revert, `/fork` (current or from earlier message; `--fork` on `run`/`attach`), timeline (`<leader>g`) rewind/fork/copy. Cannot undo shell side effects.
- Compaction: `compaction.auto` (default true), `prune` (drop old tool outputs), `reserved` buffer; `/compact` (`<leader>c`); auto-retry compaction on provider context-overflow (v1.17); v1.18.17 keeps recent turns for smaller models; V2: `keep.tokens` (15000) + `buffer` (20000), tool outputs capped 2000 chars. `OPENCODE_DISABLE_AUTOCOMPACT`, `OPENCODE_DISABLE_PRUNE`.
- Doom-loop guard: `doom_loop` permission (default `ask`) fires on 3 identical tool calls; extended to repeated reasoning/output.
- Rules/memory: `AGENTS.md` (project, walked upward; `.opencode/AGENTS.md`), global `~/.config/opencode/AGENTS.md`; falls back to `CLAUDE.md`/`~/.claude/CLAUDE.md`/`~/.claude/skills` unless `OPENCODE_DISABLE_CLAUDE_CODE*`; `instructions: [globs, URLs]` (e.g. `.cursor/rules/*.md`); `/init` generates/improves AGENTS.md; `references` config (`@alias` for local dirs or git repos injected into context). No built-in persistent memory store (community plugin `opencode-supermemory`).
- V2 session warming: periodic no-op requests to keep provider prompt caches warm (`warming: true`, 4-min interval, 30-min window).

Tools
- Built-ins: `bash`, `edit` (exact string replace), `write`, `apply_patch`, `read`, `grep`, `glob`, `list`, `todowrite`/`todoread`, `webfetch`, `websearch` (Exa; or provider-native), `codesearch` (unverified), `question` (ask user mid-run; V2 structured forms API), `skill`, `task` (V2 `subagent`), `lsp` (experimental). `fff`-backed fast file search (v1.17). Enable/disable per agent via `tools: {name: false}` (glob patterns incl. `mcp_*`).
- Shell: `!cmd` prefix runs shell and injects output; configurable `shell`; V2 shell/PTY HTTP endpoints (background shells, WebSocket PTY).
- Web: `webfetch`, `websearch`; V2 `/api/websearch` pluggable providers. No built-in browser automation (use MCP e.g. Playwright).
- Images/attachments: `@file` fuzzy reference, paste image `ctrl+v`, drag-and-drop; `--file/-f` on `run` (V2: ≤100 files, 10 MiB each); `attachment.image` auto-resize; PDFs for Copilot PDF-vision models (v1.18.17); directories attach as listings.
- LSP: 30+ pre-configured servers (tsserver, eslint, deno, oxlint, pyright, gopls, rust-analyzer, jdtls, clangd, intelephense, ruby-lsp, lua, dart, elixir, haskell, kotlin, svelte, vue, astro…), auto-download, diagnostics fed back to model after edits; custom servers via `lsp.<name>`; `find.symbols` in SDK.
- Formatters: 24+ built-ins (prettier, biome, oxfmt, ruff, gofmt, rustfmt, rubocop, pint, clang-format…) auto-run after write/edit; custom command/extensions.
- MCP client: local (`command`, `environment`, `cwd`) and remote (`url`, `headers`), OAuth (auto DCR on 401, pre-registered, or `oauth:false`); `enabled`, `timeout`; tools named `<server>_<tool>`; per-agent/global glob disable; `opencode mcp add|list|auth|logout|debug`; MCP resources listing (V2). No MCP server mode; ACP server (`opencode acp`) for editors instead.
- Skills: `SKILL.md` from `.opencode/skills/`, `.claude/skills/`, `.agents/skills/` (+ global); loaded via `skill` tool; per-skill allow/ask/deny. No official marketplace; ecosystem page lists 24 plugins, 11 projects (oh-my-opencode, ocx, opencode.nvim, OpenChamber, portal mobile UI, CodeNomad).
- Custom tools: `.opencode/tools/*.ts|js` using `tool()` from `@opencode-ai/plugin` (Zod args, `execute(args, ctx)`); can override built-ins. `codemode` package: `execute` tool runs model-written code in isolated runtime.
- Custom commands: `.opencode/commands/*.md`; `$ARGUMENTS`, `$1..$n`, `` !`shell` `` injection, `@file`; frontmatter `agent`, `model`, `subtask`; `/name` in TUI; `opencode run --command`.

Permission & sandbox
- `allow | ask | deny` per key: `read, edit (covers write/patch), glob, grep, list, bash, task, external_directory, todowrite, webfetch, websearch, lsp, skill, question, doom_loop`; `"*"` catch-all.
- Pattern rules with globs (`"git *": "allow"`, `"rm *": "deny"`; edit/read path globs; `~`/`$HOME` expansion); last matching rule wins; command patterns match parsed command structure.
- Defaults: most `allow`; `doom_loop` and `external_directory` `ask`; `.env` read denied by default (`.env.example` allowed).
- Prompt options: once / always (session-scoped; V2 durable project-scoped approvals, cannot override deny) / reject.
- `--auto` (TUI or `run`): auto-approve everything not explicitly denied. `OPENCODE_PERMISSION` env for inline JSON.
- Per-agent overrides; V2 ordered `permissions` array `{action, resource, effect}`.
- Policies (`experimental.policies`): allow/deny `provider.use` by pattern; enterprise: central/remote config (`.well-known/opencode`), MDM `.mobileconfig`, SSO gateway auth, disable share.
- Sandboxing: no built-in OS sandbox; community plugins (`opencode-sandbox` seatbelt/bubblewrap via `@anthropic-ai/sandbox-runtime`, `opencodebox`, `opencode-daytona`, `opencode-devcontainers`). Desktop workspaces = worktree isolation only.

UI/UX
- TUI: leader key `ctrl+x`, `ctrl+p` command palette, which-key `ctrl+alt+k`; ~90 rebindable actions; sidebar `<leader>b` (files changed, context %, cost, todos, subagents); status view `<leader>s`; `/details` tool-output toggle; `/thinking` reasoning toggle; conceal/diff-wrap/animation toggles; `/editor` external `$EDITOR`; prompt history; input undo/redo; copy last message `<leader>y`.
- Slash commands: `/connect /compact /details /editor /exit /export /help /init /models /new /redo /undo /sessions /share /unshare /themes /thinking /fork` (+ V2 `/report`).
- Model UX: `/models`, provider list `ctrl+a`, favorites `ctrl+f`, recent-model cycle `F2`, variant (reasoning effort) cycle `ctrl+t`, agent cycle Tab, `/connect` wizard.
- Themes: `opencode` default + system (adapts to terminal bg/ANSI), tokyonight, everforest, ayu, catppuccin(+macchiato), gruvbox, kanagawa, nord, matrix, one-dark; custom JSON themes; truecolor required.
- Mouse on by default (scroll speed/acceleration, cursor style); `diff_style` auto/stacked; diff-section navigation. No documented vim mode.
- Notifications: `attention` — desktop notifications when terminal blurred, sounds with volume/sound packs, per-event (question, permission, error, done, subagent_done); terminal title updates. Voice: none.
- Share: `/share` → `opncd.ai/s/<id>` public page; `/unshare`; modes manual/auto/disabled.
- Desktop app (beta; macOS/Windows/Linux; Electron 42 + SolidJS, sidecar server; `opencode://` deep links; auto-updater; WSL server mgmt): multi-project tabs, workspaces (git worktrees), review/diff panel, file tabs, integrated terminal tabs, thinking-level selector, themes, Servers tab (multiple local/remote), permission auto-accept per server, i18n incl. RTL, JSON transcript export.
- Web UI: `opencode web` (server + browser UI; `--port/--hostname/--mdns/--cors`, basic-auth) — usable from phone over LAN/Tailscale; `opencode attach <url>`.
- IDE: VS Code/Cursor/Windsurf/VSCodium extension (auto-installs in integrated terminal; `Cmd+Esc`, `Cmd+Shift+Esc`, `Cmd+Opt+K` insert `@File#L37-42`); ACP for Zed, JetBrains, Avante.nvim, CodeCompanion.nvim (`opencode acp`).

Sessions
- Local storage under `~/.local/share/opencode/` (SQLite; `opencode db path`).
- Resume: `/sessions`, `opencode --continue|-c`, `--session|-s <id>`, `--dir` filter; rename (`ctrl+r`), delete, auto titles (`small_model`); `opencode session list|delete`.
- Fork/branch: `/fork`, `--fork`, `POST /session/{id}/fork`; child sessions for subagents; move across projects/workspaces.
- Export/import: `/export` markdown; `opencode export [id] --sanitize` JSON; `opencode import <file|share-url>`; `run --replay`.
- Share: public link (opncd.ai); enterprise self-hosted share on roadmap.
- Remote: `opencode serve` headless server (`OPENCODE_SERVER_PASSWORD`, mDNS `opencode.local`, CORS), attach TUI/desktop/web/phone from anywhere; Desktop connects to multiple servers. No first-party hosted cloud runtime (Zen is a model gateway, not compute). Community mobile clients (MobileCode, OpenCode Mobile Android, portal).

Model support
- 75+ providers via Models.dev + Vercel AI SDK: Anthropic, OpenAI, Google (AI Studio/Vertex), Bedrock, Azure, OpenRouter, Groq, Together, Fireworks, DeepInfra, HF, Modal, NVIDIA, DeepSeek, xAI, Cerebras, Mistral, Cohere, GitHub Copilot, GitLab Duo, SAP AI Core, Snowflake Cortex, Vercel/Cloudflare AI Gateway, Helicone, ZenMux, Poolside, Venice, etc.; `enabled_providers`/`disabled_providers`; model whitelist/blacklist.
- Auth: API keys via `/connect`; OAuth logins for Anthropic (Claude subscription), GitHub Copilot, OpenAI ChatGPT Plus/Pro, GitLab Duo, Snowflake, DigitalOcean, xAI device-code.
- Local: Ollama (`:11434/v1`), LM Studio (`:1234/v1`), llama.cpp (`:8080/v1`), vLLM, Atomic Chat via `@ai-sdk/openai-compatible` custom provider (`options.baseURL`, `models{limit.context/output, tool_call, reasoning}`).
- Routing: `model` default, `small_model` for titles/light tasks, per-agent `model` (V2 `provider/model#variant`), per-command `model`, `--model`; priority CLI > config > last used.
- Variants: reasoning effort/thinking budgets (Anthropic high/max, OpenAI none…xhigh, Google low/high; custom); `ctrl+t`, `--variant`.
- Prompt caching: provider `options` incl. cache keys; Zen cached-token pricing; V2 session warming; Copilot token-based billing tracking.
- Cost/usage: TUI footer/sidebar tokens/%/$; `opencode stats --days --models --tools --project`.
- OpenCode Zen: paid gateway (pay-per-token, 60+ curated/benchmarked models incl. GPT-5.x, Claude, Gemini 3.x, Grok, Qwen, DeepSeek, Kimi, GLM, MiniMax; 7 free trial models; caching discounts; teams/BYOK). OpenCode Go: $5 first month then $10/mo, 19 open models (Qwen3.8 Max, Kimi K3, DeepSeek V4, GLM-5.x, MiniMax M3…), 5h/weekly/monthly limits.

Automation
- Headless: `opencode run "prompt"` with `--format json`, `--model`, `--agent`, `--file`, `--command`, `--continue/--session/--fork`, `--share`, `--title`, `--dir`, `--auto`, `--thinking`, `--variant`, `--attach <server-url>`, `--replay`; `--pure`.
- Server/API: `opencode serve` → OpenAPI 3.1 at `/doc`; SSE events; endpoints for sessions/messages/commands/shell/files/find/config/providers/agents/permissions/questions/TUI control/MCP/LSP/formatter/PTY; V2 adds fork/revert/inbox-steer/background/wait/generate/forms/worktrees/VCS diff/websearch/integrations.
- SDK: `@opencode-ai/sdk` (JS/TS; `createOpencode`, `session.prompt` with JSON-schema structured output, `event.subscribe`, `find.*`, `file.*`, `tui.*`); V2 `@opencode-ai/client` (+ Effect variant) and in-process `@opencode-ai/sdk-next` (private beta). No official Python/Go SDK.
- GitHub: `opencode github install`, GitHub App `opencode-agent`, action triggered by `/opencode` or `/oc` in issue/PR comments; explains issues, fixes → branch+PR, reviews PRs, triage; scheduled runs via cron with `prompt` input. `opencode pr <n>`. GitLab: CI component + GitLab Duo `@opencode`.
- Slack: `@opencode-ai/slack` bot package. Community Discord/Telegram/Matrix bridges. Scheduling: only via GitHub Actions cron; no built-in scheduler.

Self-improvement / eval: no in-product self-improvement loop or eval runner. Adjacent: `/init` writes/improves AGENTS.md; `opencode stats`; `run --replay`; separate `anomalyco/opencode-bench` (LLM-judge benchmark on real GitHub commits, 5 dimensions, community voting) used to vet Zen models.

Distinctive: true client/server split; provider-agnostic (75+, OAuth reuse of Claude/ChatGPT/Copilot subscriptions) + own gateway (Zen) + cheap open-model subscription (Go); built-in LSP diagnostics loop + auto-formatters after every edit; snapshot undo/redo/revert/fork via shadow git object DB; doom-loop detection as a permission; `external_directory` gating; `.env` read denied by default; ordered glob permission rules; provider policy layer; MDM/remote enterprise config; public share pages + import from share URL; session move across projects; desktop worktree workspaces; Claude Code compatibility fallbacks (CLAUDE.md, ~/.claude/skills, .cursor rules); full plugin hook surface, Zod-typed custom tools; V2 in-process SDK.

### Aider (Aider-AI/aider)
Status: latest PyPI `aider-chat` 0.86.2 (2026-02-12; maintenance/model-metadata bump); last GitHub Release v0.86.0 (2025-08-09); last push 2026-05-22; ~48.3k stars; cadence sharply slowed since Aug 2025. Unreleased main: Claude 4.5/4.6/4.7, Gemini 3, GPT-5.1–5.5 settings, `/ok` command, more repo-map languages. No MCP, agent mode, sub-agents, tool-calling, or hooks upstream (MCP PRs closed; #5539 open 2026-08-08). Active fork aider-ce → cecli (cecli-dev/cecli; PyPI `cecli-dev` 1.2.1, 2026-08-13) adds agent mode, MCP, sub-agents, hooks, skills, ACP, prompt queue, persistent memory, TUI.
Sources: https://pypi.org/project/aider-chat/ · https://github.com/Aider-AI/aider · https://aider.chat/HISTORY.html · https://cecli.dev/docs/

Core loop
- Chat modes: `code` (default), `ask` (no edits), `architect` (two-model: architect proposes, editor model applies), `help`, `context` (identifies files to edit, enlarges repo map). Per-message `/code`, `/ask`, `/architect`, `/help`, `/context`; persistent via `/chat-mode` or `--chat-mode`; `--architect`.
- Architect flow: `--editor-model`, `--editor-edit-format`, `--auto-accept-architect` (default true); `/editor-model`, `/weak-model`, `/model` live switch. `/ok` = "go ahead" after `/ask` plan (main branch).
- Single-agent request→edit loop. No sub-agents, parallel agents, background tasks, hooks (only git pre-commit hooks via `--git-commit-verify`).
- Auto-fix loop: auto-lint (default on) + optional auto-test; on failure asks "Attempt to fix?" and feeds output back.
- Reflection loop: LLM asks for file not in chat → aider offers to add and re-submits; malformed edit blocks → error feedback and bounded retry.
- Checkpoints/undo: git-based. Auto-commit after every LLM edit with LLM-generated commit message (weak model, Conventional Commits, `--commit-prompt`); dirty commits of user changes first; `/undo` reverts last aider commit; `/diff`; `/commit`, `/git`; `--no-auto-commits`, `--no-dirty-commits`, `--no-git`, `--dry-run`.
- Context management: repo map (tree-sitter tags + graph ranking; `--map-tokens` default 1k; `--map-refresh auto|always|files|manual`; `--map-multiplier-no-files`; `/map`, `/map-refresh`); `/add`, `/drop`, `/read-only`, `/ls`, `/clear`, `/reset`, `/tokens`; automatic chat-history summarization in background thread when exceeding `--max-chat-history-tokens` (weak model); `/save`/`/load` file-context scripts; `.aiderignore`; `--subtree-only`.
- Memory/rules: no built-in memory. Conventions via read-only files: `--read CONVENTIONS.md` / `/read-only`; persist via `read:` in `.aider.conf.yml` (home, git root, cwd; `--config`); `.env`/`AIDER_*` env vars mirror all options. System prompts only customizable by editing source.
- Reasoning: `--reasoning-effort`, `--thinking-tokens`, `/reasoning-effort`, `/think-tokens`; `accepts_settings` validation; `reasoning_tag` to strip `<think>`.

Tools
- Shell: `/run <cmd>` (optionally add output), `/test <cmd>`, `/git`, `/lint`; LLM-suggested shell commands (`--suggest-shell-commands`, default on) with confirmation; cwd = repo root.
- Edit formats: `whole`, `diff` (SEARCH/REPLACE), `diff-fenced` (Gemini), `udiff`, `udiff-simple`, `patch` (GPT-4.1), `editor-diff`, `editor-whole`, `editor-diff-fenced`, `architect`, `context`; `--edit-format`; per-model defaults; "infinite output" via assistant prefill.
- Search: repo map only (no grep/semantic tool); `/context` mode for file discovery.
- Web: `/web <url>` scrapes via Playwright (httpx fallback) → markdown; pasted URLs auto-detected (`--detect-urls`).
- Browser GUI: `aider --gui`/`--browser` (Streamlit, experimental).
- Images/vision: `/add image.png`, `/paste` (clipboard image/text), `aider screenshot.png`; needs vision model.
- LSP: none. Lint = tree-sitter syntax checks + flake8; `--lint-cmd "lang: cmd"`.
- MCP client/server: none upstream. Extensions/custom modes: none. Voice: `/voice` (OpenAI Whisper; `--voice-format`, `--voice-language`, `--voice-input-device`).
- Editor: `/editor` opens $AIDER_EDITOR/$VISUAL/$EDITOR; Ctrl-X Ctrl-E.

Permission & sandbox
- Confirmation prompts `(Y)es/(N)o/(A)ll/(S)kip all/(D)on't ask again` for adding files, URLs, creating files, editing files not in chat, running suggested commands, adding output, fixing lint/test errors, token-limit warnings.
- `--yes-always` auto-answers yes except prompts flagged `explicit_yes_required` (LLM-suggested shell commands default to "no"). `--dry-run` disables writes.
- No sandboxing: shell runs on host; git auto-commits as safety net. Analytics opt-in (PostHog), `--analytics-disable`.

UI/UX
- prompt_toolkit terminal: emacs (default)/vi (`--vim`) keybindings, Ctrl-C interrupt (partial reply kept), Ctrl-Z, Ctrl-Up/Down history, tab-completion, `--shell-completions`; multiline via `{ }`, Meta-Enter, `--multiline`/`/multiline-mode`, `/paste`, `/editor`.
- Output: rich markdown streaming, `--pretty`, `--dark-mode`/`--light-mode`, per-element colors, `--code-theme`, `--show-diffs`, spinner, per-message "Tokens: X sent, Y received (cache write/hit). Cost: $a message, $b session."
- `/settings`, `/models <search>`, `/report` (opens GitHub issue), `/copy`, `/copy-context`.
- Notifications: `--notifications` (terminal-notifier/AppleScript, notify-send/zenity, PowerShell), `--notifications-command` (Apprise → Slack/Discord).
- Watch-files: `--watch-files` scans repo for `# AI`, `// AI`, `-- AI` comments; `AI!` triggers edits, `AI?` question; works with any editor.
- Copy/paste mode: `--copy-paste` auto-copies context and applies pasted web-chat replies; `--apply-clipboard-edits`, `--apply FILE`.
- `--chat-language`, `--commit-language`.

Sessions
- History files: `.aider.chat.history.md`, `.aider.input.history`, `--llm-history-file`. `--restore-chat-history` reloads previous messages (summarized); `/save`/`/load`; `--load FILE`.
- Sharing: publish `.aider.chat.history.md` as gist → `https://aider.chat/share/?mdurl=<gist>`.
- No named sessions/resume picker, no cloud/remote sessions, one git repo per session. Runs in Codespaces/Replit/Docker.

Model support
- Any LiteLLM provider (OpenAI, Anthropic, Gemini, DeepSeek, OpenRouter, Azure, Bedrock, Vertex, Groq, Cohere, xAI, GitHub Copilot, OpenAI-compatible); `--api-key provider=key`, `--set-env`; `--list-models`, `/models` fuzzy search; model aliases (`sonnet`, `r1`, `flash`…), `--alias`.
- Local: Ollama (`ollama_chat/<model>`, `OLLAMA_API_BASE`, warns about 2k default ctx; `num_ctx` via `extra_params`), LM Studio (`lm_studio/<model>`, `LM_STUDIO_API_KEY=dummy`, `LM_STUDIO_API_BASE=http://localhost:1234/v1`), generic `--openai-api-base`.
- Roles: main model, `--weak-model` (commit messages, summarization), `--editor-model` (architect).
- Prompt caching: `--cache-prompts` (system prompt, read-only files, repo map, chat files), `--cache-keepalive-pings N` (5-min pings); Anthropic & DeepSeek; cache stats only with `--no-stream`.
- Cost: per-message and session cost line, `/tokens`; costs from LiteLLM metadata or `.aider.model.metadata.json`; unknown-model warnings.
- `.aider.model.settings.yml` keys: `edit_format`, `weak_model_name`, `use_repo_map`, `send_undo_reply`, `lazy`, `overeager`, `reminder`, `examples_as_sys_msg`, `extra_params`, `cache_control`, `caches_by_default`, `use_system_prompt`, `use_temperature`, `streaming`, `editor_model_name`, `editor_edit_format`, `reasoning_tag`, `system_prompt_prefix`, `accepts_settings`.

Automation
- CLI scripting: `aider --message "..." files` / `--message-file`, `--yes-always`, `--dry-run`, `--exit`, `--commit`, `--lint`/`--test` batch modes, `--load`.
- Python API: `Coder.create(main_model=Model(...), fnames=[...], io=InputOutput(yes=True)); coder.run("...")` — "not officially supported".
- No JSON/structured output mode, no headless server, no official GitHub Action. Docker image `paulgauthier/aider`. Watch-files as editor-driven trigger.

Self-improvement / eval
- Polyglot benchmark: 225 Exercism exercises (C++/Go/Java/JS/Python/Rust); leaderboard columns % correct, cost, edit-format compliance; leaderboard last updated ~2025-10-03; harness in `benchmark/` (Docker).
- "Aider wrote X% of the code in this release" per release (git-blame script; e.g. 88% v0.86.0); FAQ "usually about 70%".

Distinctive: repo map with tree-sitter + PageRank-style graph ranking under token budget; architect/editor two-model split with separate `editor-*` edit formats; copy/paste mode to use web-chat-only models as architect; watch-files `AI!`/`AI?` comment workflow from any editor; every edit auto-committed with attribution options → `/undo` and full git auditability; "aider wrote X%" self-measurement; multiple pluggable edit formats benchmarked per model; infinite-output prefill; public reproducible polyglot leaderboard with cost; terminal voice-to-code; prompt-cache keepalive pings; per-model YAML settings. Gaps: no tool-calling agent loop, MCP, sub-agents, hooks, sandbox, session resume — only in forks (cecli/aider-ce).

### Cline (cline/cline)
Status: VS Code extension v4.1.10 (2026-08-14; combined "Legacy" + "Next/SDK-based" A/B VSIX since 4.1.0); Cline CLI 3.0.55 (npm `cline`, 2026-08-14); `@cline/sdk` 0.0.75; `kanban` 0.1.70; Desktop app desktop-v0.0.13 (macOS); JetBrains plugin. 66.3k stars, Apache-2.0. Two runtimes coexist: legacy extension (`execute_command`, `read_file`, `replace_in_file`, `browser_action`, `focus_chain`…) and SDK runtime (`bash/run_commands`, `editor`, `read_files`, `apply_patch`, `search`, `fetch_web`, `ask_question`, `submit_and_exit`) used by CLI, Kanban, SDK, Desktop, "Next" extension.
Sources: https://github.com/cline/cline · https://docs.cline.bot/

Core loop
- Plan/Act: Plan = read-only investigation (SDK runtime hard-blocks file-mutating shell in plan mode since 4.1.4); Act = edits+commands; conversation carries over; separate model per mode; Tab toggles in TUI; model-initiated plan→act removed in 4.1.4 (user-driven only).
- `/deep-planning`: silent investigation → clarifying questions → `implementation_plan.md` → new task with trackable steps.
- Sub-agents: `use_subagents` tool (legacy v3.58+; "temporarily disabled" in SDK-based VS Code ext per 4.0.0) — parallel read-only research agents, isolated context/budget, per-subagent cost; can read/list/search/list_code_definition_names/execute_command/use_skill; cannot edit/browser/MCP/nest. `.cline/agents.yaml` config (skills + optional modelId). SDK: `enableSpawnAgent` (`spawn` tool).
- Agent Teams (SDK/CLI/Kanban): coordinator + specialists over shared task board; `team_spawn_teammate`, `team_delegate_task`, `team_check_status`, `team_get_result`; state in `~/.cline/data/teams/<name>/{task-board,mailbox,mission-log}.json`; `cline --team-name X`, `/team`; persistent across sessions; disabled by default in yolo/zen.
- Parallel agents: Kanban board — one git worktree + terminal per card, dependencies (auto-start downstream), inline diff comments, auto-commit/PR; runners: Cline CLI, Claude Code, Codex, OpenCode; `npx kanban` / `cline kanban`.
- Background tasks: `--zen` dispatches to background Hub daemon and exits (menubar app notifies); Hub sessions survive client exit; queued prompts mid-turn (4.0.0); background terminal execution mode with log files (default since 3.80).
- Hooks: file-based in `.cline/hooks/`, `~/.cline/hooks/`, `~/Documents/Cline/Hooks/` (`--hooks-dir`, `CLINE_HOOKS_DIR`): `TaskStart, TaskResume, TaskCancel, TaskComplete, TaskError, PreToolUse, PostToolUse, UserPromptSubmit, PreCompact, SessionShutdown` (+ legacy `Notification`); scripts `.sh/.js/.ts/.py/.ps1`; JSON stdin (incl. model info), can block/inject context. SDK plugin hooks: `session_start/shutdown, run_start/end, before_agent_start, tool_call_before/after, afterRun, beforeModel, error` with policies (blocking/async, timeout, retries, fail_open/closed).
- Checkpoints: shadow git repo (branch-per-task; captures untracked) snapshot after each tool use; Compare / Restore Files / Restore Task Only / Restore Files & Task; disabled in multi-root; SDK checkpoints re-implemented (true workspace rewind incl. created files; `/undo` in TUI). Session forking API + edit-and-regenerate previous message.
- Context: Auto Compact (agentic summarization default; `basic` truncation or `off` via `--compaction`), auto-recovery from context-overflow with force-compaction + retry; `/smol` & `/newtask` (IDE), `/compact` (TUI); context-window bar with clickable auto-condense threshold; file-read dedup cache; large output truncation; loop detection; consecutive-mistake limit (`--retries`, default 3); Focus Chain (legacy auto todo list).
- Rules/memory: `.clinerules` file or `.clinerules/*.md` (per-file toggles, YAML `paths:` globs); global `~/Documents/Cline/Rules` (or `~/.cline/rules`); auto-detects `.cursorrules`, `.windsurfrules`, `AGENTS.md`, `~/.agents/AGENTS.md`; `/newrule`; enterprise remote rules/skills; Workflows (`.clinerules/workflows/*.md`) as slash commands; Memory Bank = documented rule pattern (not built-in storage).

Tools
- Shell: `execute_command` (legacy; VS Code shell-integration or background process w/ logs, exit codes, timeouts) / `bash`+`run_commands` (SDK); PowerShell/Windows handling.
- Edit: `write_to_file`, `replace_in_file` (SEARCH/REPLACE streamed into VS Code diff view, user can edit in diff), `apply_patch` (unified diff), `editor` (view/edit), Background Edits (experimental); linter/compiler diagnostics fed back; VS Code Timeline entries.
- Read/search: `read_file` (chunked), `read_files` (batch), `list_files`, `search_files` (ripgrep), `list_code_definition_names` (tree-sitter, legacy), `search`/`search_codebase` (SDK).
- Web: `web_fetch`/`fetch_web`, `@url`, `web_search` (provider-executed, off by default; toggle 4.1.10).
- Browser: `browser_action` (legacy only) — headless Puppeteer/Chromium launch/click/type/scroll/screenshots+console; connect to local Chrome via remote debugging; not in SDK tool list.
- Image/vision: drag/drop, paste, `@./file.png` in CLI, PDFs/CSV/XLSX drop; image substitution for non-vision models; MCP image responses.
- LSP: none built-in; official `typescript-lsp-plugin` adds `goto_definition` (CLI plugin).
- MCP: stdio / streamable HTTP / SSE / remote with headers, OAuth, per-tool `autoApprove`, per-server timeout, `list_changed`, MCP prompts as `/mcp:<server>:<prompt>`, resources, rich responses; `load_mcp_documentation` + "add a tool that…" builds servers; MCP Marketplace (legacy) and Customize marketplace (Skills/MCP/Plugins, 4.0.0); `.cline/mcp.json`; `cline mcp` wizard; enterprise remote-synced servers.
- Skills: `SKILL.md` in `.cline/skills`, `.clinerules/skills`, `.claude/skills`, `~/.cline/skills`; auto-trigger or `/skill-name`; `use_skill`; `/skills`; enterprise globalSkills.
- Plugins (SDK/CLI/Kanban; not legacy IDE): TS/JS bundling tools (`createTool` w/ zod), commands, hooks, rules, events, MCP; `cline plugin install` from npm/git/file/local; subprocess sandbox; examples: web search, gitignore read guard, mac notify, metrics, custom compaction, background terminal.
- Slash commands (IDE): `/newtask`, `/smol`, `/newrule`, `/deep-planning`, `/reportbug`, workflows, skills, MCP prompts. TUI: `/settings /model /account /mcp /compact /undo /clear /history /help /quit /skills /team`.

Permission & sandbox
- Default: every edit/command/browser/MCP action needs approval (auto-approve off by default for new configs in 4.0; CLI headless defaults auto-approve on).
- Auto-approve bar: Read project/all files; Edit project/all files; Execute safe commands (dynamic risk classification) / all commands; Browser; MCP; OS notifications; per-MCP-tool; `attempt_completion` command.
- YOLO: legacy toggle merged into "auto-approve all" (4.1.8); CLI `-y/--yolo` (skip approvals, enable `submit_and_exit`, disable spawn/team), `--auto-approve <bool>`; Shift+Tab in TUI; enterprise can force-disable.
- Command policy: `CLINE_COMMAND_PERMISSIONS='{"allow":[...],"deny":[...],"allowRedirects":false}'`; SDK `toolPolicies` + `requestToolApproval` callback; plan mode blocks mutations.
- Sandbox: no OS-level sandbox; `CLINE_SANDBOX`/`--data-dir` = isolated state dir; plugins in subprocess sandbox. `.clineignore` (gitignore syntax; "deprecate soon").

UI/UX
- VS Code/Cursor/Windsurf/VSCodium/Antigravity webview: streaming chat, diff-view editing, terminal integration, Compare/Restore rows, task header tokens/cost/context bar, collapsible reasoning, mermaid & LaTeX, queued prompts, edit past messages, selectable options in questions/plans, model picker (Recommended/Free tabs, favorites), Customize marketplace, walkthrough onboarding, accessibility.
- @-mentions: `@file`, `@folder`, `@url`, `@problems`, `@terminal`, `@git-changes`, commits, `@workspace:/path`; drag/drop files/images/PDF/CSV/XLSX; context-menu Add/Fix/Explain/Improve; CMD+' add selection; commit-message generation.
- Voice: dictation (Aqua Voice STT) and experimental Voice Mode (legacy).
- Notifications: OS notifications on approval/completion; Notification hook; menubar app for zen tasks.
- CLI TUI (OpenTUI): markdown + syntax-highlighted diffs, mouse, Tab plan/act, Shift+Tab auto-approve, status bar (model, context %, cost, workspace/branch, git diff stats), `@file`, `/undo`, session picker.
- Cline Hub dashboard (browser) and Desktop app (Tauri + sidecar).

Sessions
- Task history w/ favorites, workspace filter, search; SDK sessions in SQLite + JSON `~/.cline/data/sessions/`; `cline history`, `--id` resume; session forking (SDK); export; `/task` deep-link URIs.
- Hub-spoke: singleton local daemon (127.0.0.1:25463) coordinates sessions; CLI, VS Code, JetBrains, connectors, desktop attach to same session; sessions outlive clients; backend modes `local|hub|remote|auto`; `remote` = WebSocket hub for self-hosted team deployments.
- Mobile: no native app; Kanban via `KANBAN_RUNTIME_HOST=0.0.0.0` + Tailscale; chat connectors (Telegram/Slack/Discord/Google Chat/WhatsApp/Linear) drive sessions from phone with approvals.
- Cline account: credits, usage, orgs, ClinePass.

Model support
- Providers: Cline gateway (100+ models, "exacto" tuned open models, free promos), ClinePass ($9.99/mo: GLM-5.2, Kimi K3/K2.7/K2.6, DeepSeek V4 Pro/Flash…), Anthropic, Claude Code (CLI subscription), OpenAI (+ Codex/ChatGPT OAuth, Responses API), OpenAI-compatible, OpenRouter, Bedrock, Vertex, Gemini, DeepSeek, Qwen/Qwen Code, Z.ai, MiniMax, Moonshot, Mistral, xAI, Groq, Cerebras, Fireworks, Together, Baseten, SambaNova, HF, Nebius, Requesty, LiteLLM, Vercel AI Gateway, SAP AI Core, Oracle Code Assist, AskSage, Dify, Doubao, Huawei MaaS, Nous, W&B/CoreWeave, Poolside, Chutes, Crusoe, VS Code LM API, Ollama, LM Studio, Atomic Chat; list generated from models.dev in SDK.
- Local: Ollama (native AI SDK provider, native tool calls, 5-min cold-load timeout), LM Studio (v0 API), llama.cpp via OpenAI-compatible; "Use Compact Prompt" for small models.
- Routing: separate provider/model for Plan vs Act; CLI multi-model orchestration via `--config` dirs.
- Native tool calling default (per-model allow-list, XML fallback), parallel tool calling, model-family prompt variants (GPT-5, Gemini, GLM, Hermes, Trinity, Devstral…), reasoning effort none→xhigh, thinking budgets.
- Prompt caching (Anthropic, Bedrock, OpenRouter, LiteLLM, Gemini/Vertex, GPT-5) with cache read/write reporting; cost/token per request & task; `-v` run stats.

Automation
- CLI headless: auto-activates on `--json`/piped stdin; NDJSON `{"type":"say|ask","text","ts","partial","reasoning"}`; `-p` plan, `-m/-P/-k`, `-s` system prompt, `-t` timeout, `--thinking`, `--compaction`, `--retries`, `--data-dir`, `--yolo`, `--zen`, `--team-name`, `--acp`; `cline auth --provider X --apikey` for CI; `cline doctor`.
- SDK: `@cline/sdk` (`ClineCore.create/start/send/list/readMessages/abort/subscribe`, `Agent` stateless loop, browser-safe `@cline/agents`, `@cline/llms`, custom tools, plugins, teams, checkpoints, MCP, cron); examples: cli-agent, code-review-bot, multi-agent SSE, desktop-app.
- Scheduling: `cline schedule` (cron, workspace, model, timeout, tags, pause/resume/trigger/history/export/import YAML), event-driven schedules, delivery to chat connectors; `.cline/cron/`.
- Connectors: `cline connect telegram|slack|discord|gchat|whatsapp|linear` with allow-lists.
- GitHub: sample Actions (issue `@cline` responder, PR review workflow w/ inline suggestions); ACP for Zed/JetBrains AI Assistant/Neovim/Emacs; Cline API (OpenAI-compatible endpoints); enterprise OpenTelemetry, remote config, SSO/RBAC.

Self-improvement / eval
- `evals/` in repo: smoke tests (5 scenarios, pass@k), cline-bench (12 real bug-fix tasks), failure-pattern classifier; partially disabled pending SDK re-wire; in-product evals tool removed 3.79.
- "Double-check completion" experimental verification (3.58); "Lazy Teammate Mode" (3.77).
- Blog "Recursive Self Improvement for Coding Agents" (2026-07-24): Cline harness + Kimi K3 ran 17h autonomously improving its own Terminal-Bench 2.1 score to 88.8% — a workflow, not a shipped feature.

Distinctive: hub-spoke daemon (one session live across CLI, IDE, desktop, chat connectors; sessions survive client exit; `--zen`); chat-platform connectors with approvals from phone; built-in cron scheduler with chat delivery; Kanban board orchestrating parallel worktree agents (also runs Claude Code/Codex/OpenCode); persistent Agent Teams with task board/mailbox; full runtime published as `@cline/sdk`; ACP mode; Plan/Act with per-mode model routing and hard-blocked mutations in plan; shadow-git 3-way restore; broadest provider matrix, ClinePass flat-rate; dynamic command-risk classification; cross-tool rule ingestion; Jupyter cell-level AI; multi-root workspaces.

### Roo Code + Kilo Code (one family)
Status: Roo Code sunset announced 2026-04-20; extension repo archived read-only; Roo Code Cloud + Router shut down 2026-05-15; final extension v3.54.0 (2026-05-15); Roo CLI v0.1.17 pre-release (2026-03-04). Team pivoted to Roomote (roomote.dev, source-available, Slack/Teams/Discord/Telegram-first cloud agent). Farewell recommends Cline and community fork ZooCode. Kilo Code: v7 rewrite GA 2026-04-02 — VS Code extension rebuilt on Kilo CLI / OpenCode server engine (Kilo CLI = fork of anomalyco/opencode). Latest: VS Code v7.4.22 (~2026-08-13), JetBrains 7.0.16 (2026-08-14). 26.9k stars, MIT.
Sources: https://github.com/RooCodeInc/Roo-Code · https://roocodeinc.github.io/Roo-Code/ · https://roomote.dev/ · https://github.com/Kilo-Org/kilocode · https://blog.kilo.ai/p/new-kilo-for-vs-code-is-live

Roo Code (final state v3.54.0)
- Core loop: built-in modes Code, Architect (edit restricted to markdown), Ask, Debug, Orchestrator (delegates via `new_task`); custom modes override by slug; sticky models per mode; boomerang subtasks (isolated context, summary-only return, recursive subtask tree; no true parallel); parallel tool calling (3.46); worktrees (3.44); message queueing; checkpoints (shadow git per task, "Restore Files Only" vs "Restore Files & Task", checkpoint navigation); context: 30% reserved, Intelligent Context Condensing, Smart Code Folding (3.45), 25% truncation on errors, read caps; todo (`update_todo_list`, `newTaskRequireTodos`, `preventCompletionWithOpenTodos`); rules `.roo/rules/`, `.roo/rules-{mode}/`, legacy `.roorules`, `.clinerules`; global `~/.roo/rules*`; `AGENTS.md` (+ `AGENTS.local.md`); no hooks.
- Tools: native tool-calling default (3.36.13+): `read_file` (multi-file, ranges, concurrent reads), `list_files`, `read_command_output`, `search_files` (ripgrep), `codebase_search` (semantic), `apply_diff`, `apply_patch`, `edit`, `edit_file`, `search_replace`, `write_to_file`, `generate_image` (experimental), `execute_command`, `run_slash_command`, `use_mcp_tool`, `access_mcp_resource`, `ask_followup_question`, `attempt_completion`, `switch_mode`, `new_task`, `update_todo_list`, `skill`, `fetch_instructions`; `browser_action` removed 3.48 → Playwright MCP; codebase indexing (tree-sitter chunking; embedders OpenAI/Gemini/Ollama/OpenAI-compatible/Mistral/Vercel/Bedrock/OpenRouter; vector DB Qdrant only); `@url` fetch; VS Code Problems integration; MCP stdio/HTTP/SSE w/ marketplace; skills (`.roo/skills/`, `.agents/skills/`, `skills-{mode}/`; skills as slash commands 3.51); Custom Tools (TS/JS in `.roo/tools/`, Zod, esbuild); custom modes (`.roomodes` / `custom_modes.yaml`: slug, roleDefinition, whenToUse, customInstructions, groups with `fileRegex`).
- Permissions: auto-approve panel (Read/Edit/Execute w/ allow/deny prefix lists, Browser, MCP, Mode switching, Subtasks, Retry, Follow-up auto-select, todo updates); "BRRR" = auto-approve all; `.rooignore`; no OS sandbox; command timeout.
- UI/UX: VS Code webview w/ mode/profile dropdowns, File Changes panel (3.49), diff view, Enhance Prompt, Suggested Responses, TTS, sound effects, 18 UI languages, settings import/export; @-mentions; code actions; Inline Terminal; CLI TUI pre-release.
- Sessions: local task history, subtask tree, export; Roo Code Cloud (shut down 2026-05-15) had Task Sync, Task Sharing, Analytics, Cloud Agents (Explainer/Planner/Coder/PR Reviewer/PR Fixer), Preview Environments, Team plan.
- Models: Anthropic, OpenAI (+ Codex OAuth), OpenAI-compatible, OpenRouter, Requesty, Bedrock, Vertex, Gemini, DeepSeek, Mistral, Moonshot, MiniMax, xAI, Z.ai, Fireworks, Baseten, SambaNova, LiteLLM, Vercel AI Gateway, VS Code LM API, Qwen Code (OAuth), Poe, Ollama, LM Studio; API Configuration Profiles per mode; prompt caching; native tool calling across 13+ providers.
- Automation: Roo CLI (`roo`; extension bundle via `@roo-code/vscode-shim`): `--print` w/ `--output-format text|json|stream-json`; `--stdin-prompt-stream` NDJSON protocol; `--oneshot`, `--ephemeral`, session resume; Extension API `RooCodeAPI`. Evals: `packages/evals` + `apps/web-evals` (Exercism-derived, Docker runners, dashboard).
- Distinctive: mode system with per-mode tool groups + fileRegex edit restrictions + sticky per-mode model profiles; orchestrator/boomerang isolated subtasks; shadow-git checkpoints "files only" vs "files+conversation"; Smart Code Folding; local TS Custom Tools without MCP; CLI = literal extension bundle under VS Code shim.

Kilo Code differences (v7.4.x VS Code / JetBrains 7.0.x / CLI)
- Engine: OpenCode-server core shared by CLI, VS Code, JetBrains, Cloud Agents; config `kilo.jsonc` (global `~/.config/kilo/`, project `./kilo.jsonc` or `.kilo/`; legacy `.kilocode/` migrated).
- "Modes" → agents: built-in code, ask (read-only), plan (edits only `.kilo/plans/`), debug; orchestrator deprecated (subagents native). Custom agents `.kilo/agents/*.md`; legacy `.kilocodemodes` auto-migrated. Subagents `general`, `explore`; via `task` tool or `@agent`; isolated sessions, run in parallel; `permission.task`; `kilo agent create/list`.
- Agent Manager (VS Code, Cmd/Ctrl+Shift+M): multiple parallel sessions each in isolated git worktree under `.kilo/worktrees/`; Multi-Version Mode (same prompt on up to 4 models side-by-side); live diff panel (unified/split, "Apply to local"), inline line-level review comments routed back to agent, PR status badges + PR comment actions (`gh`), per-session terminals, setup/run scripts, `.env` copy; `agent_manager` tool lets agent spawn sessions.
- Checkpoints → Snapshots: detached-worktree git repo outside project; snapshot before generation and after each tool call; "Revert to here" per user message, Redo/Redo All; `/undo` `/redo` in CLI.
- Context: auto-compaction (`compaction.auto`, `threshold_percent`, `prune`, `tail_turns`, `preserve_recent_tokens`, `reserved` ~20k), `/compact`, separate compaction model; context-usage graph.
- Rules: `AGENTS.md`/`AGENT.md` (root, per-subdirectory dynamically loaded, global), `CLAUDE.md`, `CONTEXT.md`, `instructions` array (files/globs/URLs), `.kilo/rules/`; Memory Bank deprecated in favor of AGENTS.md.
- Hooks: plugin system (TS/JS, `kilo plugin <npm>`, `~/.config/kilo/plugin/`, `.kilo/plugin/`): `tool.execute.before/after`, `tool.definition`, `chat.message/params/headers`, `permission.ask`, `command.execute.before`, `shell.env`, `auth`, `provider`, `event`, experimental `session.compacting`, `chat.messages.transform`; add/override tools; `KILO_PURE=1`.
- Tools: OpenCode-style names: `read`, `glob`, `grep`, `edit`, `write`, `apply_patch`, `bash`, `webfetch`, `websearch` (Exa/Parallel), `question`, `task`, `todowrite`, `todoread`, `plan`, `skill`, `agent_manager`, `semantic_search`, browser `kilo-playwright_browser_*` (bundled Playwright), MCP `{server}_{tool}`, `list_mcp_resources`/`read_mcp_resource`; `lsp` tool + experimental LSP diagnostics; codebase indexing (embedders incl. Ollama; vector stores LanceDB embedded default or Qdrant); MCP w/ OAuth + Marketplace; skills (`.kilo/skills/`, `~/.kilo/skills/`, `.agents/skills/`, `.claude/skills/`); workflows (`.kilo/commands/*.md`).
- Permissions: ordered rules per tool `allow|ask|deny`, globs, last-match-wins, per-agent + global; `external_directory`; `.env` sensitive; read-only agents block redirection/pipes; each shell sub-command checked; VS Code Auto Approve UI; CLI `/auto-approve`, `kilo run --auto`; no OS sandbox.
- UI/UX: VS Code chat + Agent Manager tab; reasoning blocks, side-by-side diff, Changes panel, context graph, model pricing in picker, starred models, Mermaid, `/export`, image paste, enhance prompt, commit message generation, voice transcription (Cmd/Ctrl+K), autocomplete (FIM ghost text; Codestral via gateway or free w/ Mistral BYOK); JetBrains native Swing UI over `kilo serve`; CLI TUI commands `/new /sessions /resume /continue /share /unshare /rename /timeline /fork /undo /redo /export /compact /diff /move /status /themes /reload /editor /auto-approve /privacy /agents /models /mcps /connect /teams /remote /review /resume-claude /resume-codex`.
- Sessions: local SQLite shared across CLI/VS Code/JetBrains; start in CLI resume in VS Code; `kilo session list --search`, `kilo export/import`, `--continue`, `--session`, `--fork`; share links; import Claude Code / Codex transcripts; Cloud Agent (app.kilo.ai; per-user Linux container, auto-commit/push, resume cloud↔local; webhook + scheduled triggers; `kilo cloud start/send/status/result`); remote control `/remote` exposes local CLI to web/mobile; mobile apps (Android live, iOS in review); Kilo Connect (`@Kilo` in Slack, `@kilocode-bot` on GitHub, `@kilo` in Linear).
- Models: Kilo Gateway (500+ models, `provider/model`, provider-price pass-through, `:free` models, `kilo-auto/free`, Auto Model tiers Frontier/Efficient/Free, BYOK, cache write/hit tracked, `kilo stats`, org pools/limits); direct providers incl. Ollama, LM Studio, Bedrock/Vertex; per-agent `model` + reasoning `variant`.
- Automation: `kilo run "msg" [--auto] [--format json] [--agent] [--model] [--session/--continue/--fork] [-f file] [--attach url] [--thinking]`; `kilo serve` (HTTP/SSE headless), `kilo daemon`, `kilo attach`, `kilo acp`, JS SDK; `kilo github`, `kilo pr`, `kilo review`; hosted Code Reviews (GitHub App/GitLab; `REVIEW.md`), Cloud Agent triggers, KiloClaw (hosted 24/7 agent), App Builder, Kilo Deploy.
- Distinctive: one OpenCode-derived core across CLI/VS Code/JetBrains/Cloud/mobile with shared SQLite sessions; Agent Manager (worktree-isolated parallel agents + multi-model comparison + inline diff review + PR badges); plugin hooks, ACP server, `kilo serve` daemon, import of Claude Code/Codex sessions; gateway w/ 500+ models, free tier, BYOK zero-markup, Auto Model routing; free Codestral autocomplete; voice; hosted extras.

### Goose (aaif-goose/goose, formerly block/goose)
Status: v1.46.0 (2026-08-12); repo moved to `github.com/aaif-goose/goose` (Agentic AI Foundation / Linux Foundation, 2026-04-07); docs at https://goose-docs.ai; ~52.9k stars. Rust core; Desktop (Electron → Tauri rewrite in progress), CLI, `goose serve`/ACP server, new TS TUI (`npx @aaif/goose`, beta).

Core loop
- Agent loop: request → provider → tool exec → response → context revision; v1.46 "unrolled agent loop" + cache-safe append-only turn assembly; errors returned as tool results.
- Modes: `auto` (default), `approve`, `smart_approve` (risk-classified via `permission_judge.md` prompt), `chat` (no tools); `goose configure`, `GOOSE_MODE`, `/mode` mid-session.
- Plan mode: CLI `/plan [msg]` / `/endplan`; dedicated planner via `GOOSE_PLANNER_PROVIDER/MODEL`; Lead/Worker model removed, replaced by planner mode.
- Sub-agents: built-in Summon platform extension (`delegate()`, `load()`); sequential default, parallel via NL; `delegate(async:true)` → background task id, `load(task_id)` awaits, "peek"; isolated sessions; cannot spawn sub-subagents; 5-min timeout, 25-turn default (`GOOSE_SUBAGENT_MAX_TURNS`), `GOOSE_MAX_BACKGROUND_TASKS`; auto mode only.
- Custom agents: markdown+YAML (`name`,`description`,`model`) in `~/.agents/agents/`, `.agents/agents/` (legacy `.goose/agents/`, `.claude/agents/`); `@agent-name` mention, delegate, or load into convo.
- Subrecipes: parent recipe gets one tool per subrecipe; isolated; no nesting; up to 10 concurrent workers; same-subrecipe-different-params auto-parallel; CLI progress dashboard.
- Scheduled tasks: `goose schedule add/list/remove/run-now/sessions/cron-help` (5/6/7-field cron incl. seconds); Desktop Scheduler page; `goose serve --enable-scheduler`.
- Hooks (2026-05): SessionStart/End, Stop, UserPromptSubmit, PreToolUse, PostToolUse, PostToolUseFailure, BeforeReadFile, AfterFileEdit, BeforeShellExecution, AfterShellExecution; `hooks.json` inside plugins (`~/.agents/plugins/<name>/hooks/hooks.json`); regex matchers; PreToolUse/Stop can block (exit 2 or `{"decision":"block"}`); fail-open.
- Checkpoints/rewind: no filesystem checkpoint/rewind; Desktop message edit-in-place (truncates later context) + fork from edited message; CLI `--resume --edit` (edit history as YAML) and `--fork`; docs advise git for rollback.
- Context: auto-compact at 80% (`GOOSE_AUTO_COMPACT_THRESHOLD`), `/compact`, Desktop "Compact now"; older tool outputs summarized (`GOOSE_TOOL_CALL_CUTOFF`); `GOOSE_CONTEXT_STRATEGY` = summarize/truncate/clear/prompt; `GOOSE_CONTEXT_LIMIT`; `GOOSE_MAX_TURNS` (1000); `compaction.md` template overridable; `GOOSE_MAX_TOOL_RESPONSE_SIZE` spills to temp file.
- Memory: Memory extension (`remember_memory/retrieve_memories/remove_*`, categories+tags, local `.goose/memory/` vs global `~/.config/goose/memory/`); Chat Recall platform ext (search past sessions in SQLite); Knowledge Graph Memory MCP (3rd-party).
- Project instructions: `.goosehints` + `AGENTS.md` (global `~/.config/goose/.goosehints`, `~/.agents/AGENTS.md`; local walk-up + nested dirs loaded on access; `CONTEXT_FILE_NAMES` env to add `CLAUDE.md`, `.cursorrules`); `@file` includes; Desktop Project Hints editor.
- Persistent instructions ("Top of Mind"/MOIM): `GOOSE_MOIM_MESSAGE_TEXT/FILE` re-injected every turn (64KB). Todo platform ext; Projects (Desktop). `/goal` slash command for agent self-evaluation (v1.46; details unverified).

Tools
- Developer (built-in default): `shell` (streaming output v1.46), `write`, `edit` (exact-text replace), `tree`, `read_image`; `GOOSE_SHELL` override; shell inherits `AGENT_SESSION_ID`, `GOOSE_TERMINAL=1`, `AGENT=goose`.
- Analyze platform ext (default): tree-sitter structure/semantic/focus modes, call graphs, multi-language.
- Computer Controller (opt-in): web scraping, automation scripts, PDF/Word/Excel processing, macOS UI automation via Peekaboo CLI (2026-04), app launching.
- Web/browser: no built-in browser; via MCP (Playwright, Chrome DevTools, Selenium, Browserbase, Fetch, Firecrawl, Tavily, Exa…).
- Image/vision: `read_image`; Desktop drag-drop/attach; images + MCP embedded blobs forwarded (v1.46); Nano Banana ext for generation.
- LSP: none documented; Analyze/tree-sitter is the code-intel path; JetBrains MCP ext.
- MCP client: stdio, streamable HTTP (SSE deprecated), OAuth (client-ID metadata, dynamic registration, pre-registered, proactive refresh), MCP Roots, Sampling, Elicitation (forms; interactive menu v1.46), MCP Apps (rich HTML UI in Desktop), tool-list change notifications; rmcp 3.0.
- MCP server: built-ins runnable standalone via `goose mcp <name>`; `goose acp` / `goose serve` expose goose itself.
- Extensions marketplace: https://goose-docs.ai/extensions (70+; deep links `goose://extension?...`); Extension Manager platform ext (agent enables/disables dynamically); "Smart Extension Recommendation"; extensions malware-checked before activation (docs claim); Container Use ext (Dagger).
- Extension types: `builtin`, `platform`, `stdio`, `streamable_http`, `frontend`, `inline_python`; per-extension timeout, `available_tools` filtering, `env_keys`, keyring secrets.
- Recipes: YAML/JSON (`instructions`, `prompt`, `extensions`, typed `parameters` incl. file/select/date, `settings` provider/model/temperature/max_turns, `activities`, `sub_recipes`, `response` JSON schema, `retry` w/ shell checks + on_failure), Jinja templating; `/recipe` generate from session; deeplinks; `GOOSE_RECIPE_PATH`, `GOOSE_RECIPE_GITHUB_REPO`; Recipe Library (Desktop) + Cookbook; trust prompt on first run.
- Skills: Anthropic-compatible `SKILL.md`; `~/.agents/skills/`, `.agents/skills/`, plugin skills `plugin:skill` (legacy `.goose/skills`, `.claude/skills`); auto-load by relevance or `/skills <name>`; `goose skills list`; built-in skills disable-able (v1.45); skills in Desktop composer.
- Plugins: `goose plugin install [--auto-update] <git-url>`, `plugin update`; `plugin.json` + `skills/` + `hooks/` (+ MCP extensions v1.46); Gemini-extension format supported.
- Custom slash commands: `slash_commands:` in config.yaml mapping to recipes.
- Prompt templates: override `system.md`, `compaction.md`, `plan.md`, `recipe.md`, `subagent_system.md`, `session_name.md`, `permission_judge.md`, `tiny_model_system.md` in `~/.config/goose/prompts/`.
- Code Mode platform ext: LLM writes JS executed by pctx (Deno); meta-tools `list_functions/get_function_details/execute_typescript`; batches/chains tool calls. Old vector "tool router" removed in favor of Code Mode.
- Apps platform ext: single-file HTML "goose apps" in sandboxed windows, exposed as MCP App resources. Auto Visualiser (charts), Tutorial, Todo, Chat Recall, Beads (git-backed issue tracker), goose Docs MCP.

Permission & sandbox
- Modes: auto / approve / smart_approve / chat; approve/smart only prompt for write-ish ops (edits, deletes, rm/cp/mv), reads pass.
- Per-tool permissions: Always Allow / Ask Before / Never Allow; `permissions/tool_permissions.json`; hooks PreToolUse denial.
- Adversary Mode: second silent LLM reviewer judges tool calls (default `shell`, extendable) vs original task; ALLOW/BLOCK; rules in `~/.config/goose/adversary.md`; fail-open.
- Prompt Injection Detection: pattern-based + optional ML classifier endpoint (`SECURITY_PROMPT_*`), Allow Once/Deny UI.
- Extension Allowlist: `GOOSE_ALLOWLIST=<url to YAML>` (enterprise).
- Sandboxing: no native sandbox — macOS seatbelt `GOOSE_SANDBOX` (v1.25) was experimental and removed; isolation via `--container <docker id>` (runs extensions/commands inside existing container), Container Use MCP (Dagger/Docker/Podman/Apple Container), devcontainers, official Docker images. SLSA/Sigstore provenance. Secrets: system keyring (`GOOSE_DISABLE_KEYRING` → `secrets.yaml`).

UI/UX
- CLI (Rust): `/help /builtin /extension /mode /plan /endplan /prompt(s) /recipe /compact /r /skills /t /clear /exit` (+ `/model`, `/status`, `/goal` per v1.46); Ctrl+C, Ctrl+J newline (`GOOSE_CLI_NEWLINE_KEY`), Ctrl+R history search, Cmd+Up/Down; themes light/dark/ansi + custom bat themes; markdown rendering, code-block truncation controls, thinking display (`GOOSE_CLI_SHOW_THINKING`), cost display (`GOOSE_CLI_SHOW_COST`), tool-output priority levels, `GOOSE_PROMPT_EDITOR`, message queueing (Enter while running), tab completion; no vim mode/mouse/voice documented; MCP elicitation forms in terminal.
- New TS TUI (beta, `npx @aaif/goose`): ACP-based; markdown, syntax highlight, tool calls, diff viewer (v1.46).
- Terminal integration: `goose term init <zsh|bash|fish|nu|pwsh>`, `@goose`/`@g` inline queries with shell-history context, unresolved-command handler, prompt segment.
- Desktop (macOS/Win/Linux): sidebar (Home/Chat/Recipes/Apps/Scheduler/Extensions/Settings), Quick Launcher (Cmd+Opt+Shift+G), customizable keyboard shortcuts, Cmd+F search, edit/fork messages, message queue w/ drag-reorder, drag-drop/`@` attach, voice dictation (Local Whisper, ElevenLabs, Groq, OpenAI; "submit" voice command), mid-session model/dir/extension/mode switching, token gauge + cost + per-message stats (tokens, cost, TTFT, tok/s), model-interactions viewer, tool-call chain cards, MCP Apps rendering, goose Apps windows, Gateways settings, Local Inference settings, i18n (15+ languages), theme light/dark.
- Sharing: recipe deeplinks; session export/import (JSON, Markdown v1.46), encrypted Nostr session sharing (`goose://sessions/nostr`); `goose://new-session`, `goose://resume`, `goose://extension` deep links.
- VS Code extension (experimental, ACP): chat panel, Cmd+Shift+G send selection, `@` files, session replay.

Sessions
- SQLite `sessions.db` shared by Desktop/CLI; auto-naming; IDs `YYYYMMDD_N`.
- CLI: `goose session [-n name] [-r] [--fork] [--edit] [--history] [--container id] [--max-turns] [--max-tool-repetitions]`; `session list/remove/export (md|json|yaml)/diagnostics`.
- Desktop: history, rename, duplicate, delete, import/export, per-project grouping, cross-session search (Chat Recall).
- Remote: `goose serve --host --port --tls --enable-scheduler [--platform desktop] [--dangerously-unauthenticated]` = ACP over HTTP/WebSocket, `X-Secret-Key` auth, TLS w/ cert pinning; Desktop "Use external server"; `goosed` REST/SSE being phased out for ACP.
- ACP: `goose acp` stdio agent for Zed, VS Code ext, any ACP client; multi-session, native diffs, mid-session model/mode switch, MCP servers passed from client. Goose can also consume ACP agents as providers (Claude ACP, Codex ACP, Amp ACP, Pi ACP).
- Mobile/messaging: Telegram Gateway (experimental; `goose gateway start|pair|stop telegram`; pairing codes).

Model support
- 60+ providers incl. Anthropic, OpenAI, ChatGPT Codex, Gemini, Vertex, Bedrock, SageMaker, Azure OpenAI/AI Foundry, Databricks, GitHub Copilot, OpenRouter, Tetrate, Groq, Cerebras, Mistral, xAI, Perplexity, LiteLLM, Snowflake, Venice, Together, Fireworks, Friendli, Sakana, NEAR AI, Novita, OVHcloud, Scaleway, Vercel AI Gateway, Routstr, Meta Models API, custom OpenAI-compatible (custom headers), declarative-JSON custom providers.
- Local: Ollama + Ollama Cloud (auto model discovery), LM Studio (:1234), Docker Model Runner, Ramalama, Atomic Chat, oMLX; built-in local inference (embedded llama.cpp; `goose local-models search|download|list|delete` from HF GGUF; Desktop Local Inference; MLX + Linux Vulkan); Mesh LLM p2p (early).
- Tool shim for non-tool-calling models (`GOOSE_TOOLSHIM`).
- Subscription reuse: ACP providers (Claude Code, Codex, Amp, Pi) and deprecated CLI providers (claude-code, codex, cursor-agent, gemini-cli).
- Routing: planner model, `GOOSE_FAST_MODEL` for auxiliary calls, per-recipe/per-agent model, `/model` mid-session; unified thinking-effort control.
- Prompt caching: Claude via Anthropic/Bedrock/Databricks/OpenRouter/LiteLLM; cache-safe request assembly; cache tokens in cost.
- Cost tracking: per-message tokens/cost/TTFT/tok-s, session totals, `GOOSE_CLI_SHOW_COST`, credit-low notifications; OTel (GenAI semconv), Langfuse, Laminar, MLflow tutorials.

Automation
- Headless: `goose run -t|-i file|-i - |--recipe --params --sub-recipe --system --no-session -q --output-format text|json|stream-json --provider --model --explain --render-recipe -s --max-turns`.
- `goose review [range]`: local diff review, parallel orchestrator, `.agents/checks/*.md` check files, `--severity`, `--files`, `--dry-run`.
- Recipes as CI units; retry/validation checks; Ralph-loop tutorial.
- SDK/API: `goose serve` ACP HTTP/WS; UniFFI goose-sdk → Python wheel `aaif-goose` on PyPI, Kotlin/Maven bindings.
- GitHub: official Marketplace action "goose ai developer agent" (label issue → PR; PR-comment aware); CI/CD tutorial. Scheduler cron (CLI + Desktop), Telegram gateway automation, custom distributions (branding, embedded keys, bundled extensions).

Self-improvement / eval: `goose bench` (JSON config: models, eval-suite selectors, repeats; status unverified); Harbor eval runner + `evals/harbor/cmd.py` (Terminal-bench runs); human-in-loop "self-improving" workflow (goose diffs pass/fail traces, human synthesizes, goose implements) — blog 2026-06-17; `/goal` self-evaluation slash command (v1.46); repo dev process: issues-first, agent-implemented "Ready" issues, agent review.

Distinctive: symmetric ACP (server for Zed/VS Code/TUI/Desktop/remote and client using Claude Code/Codex/Amp/Pi as backends w/ MCP passthrough); recipes as first-class artifacts (typed params, retry/verification, sub-recipe fan-out, deeplinks, cron scheduler, GitHub-repo recipe registry, GUI editor); Adversary Mode + prompt-injection classifier + enterprise extension allowlist; embedded llama.cpp local inference + Mesh LLM p2p; tool shim for non-tool models; Code Mode (JS/Deno meta-tool execution); goose Apps + MCP Apps rich UI; encrypted Nostr session sharing; Telegram gateway; `@goose` shell-history-aware terminal integration; MOIM every-turn instructions; cross-language SDK (UniFFI Python/Kotlin); Docker `--container` extension execution; 60+ providers; Desktop+CLI shared SQLite; fail-open hooks with file/shell-specific events; plugin format compatible with Claude/Gemini layouts.

### Qwen Code (QwenLM/qwen-code)
Status: v0.21.11 (2026-08-13); pre-release v0.21.12-preview.5 / nightly 20260816; 27.1k stars; Node ≥22. README explicitly targets Claude Code feature parity; Gemini-CLI lineage still visible.
Sources: https://github.com/QwenLM/qwen-code · https://qwenlm.github.io/qwen-code-docs/

Core loop
- Plan mode: read-only approval mode; `/plan`, `/plan <task>`, `/plan exit`; `exit_plan_mode` tool; explicit plan-exit approval.
- Sub-agents: markdown agents (`.qwen/agents/`, `~/.qwen/agents/`, extension agents); `agent` tool; fork subagent (`subagent_type:"fork"`, inherits parent context, `fork_turns`, `fork_tools` allowlist, named `fork_profile`s, shares parent prompt cache); top-level subagents run in background by default; `list_agents` / `send_message` continuation (survives session restore); per-agent `model:` (`inherit`/`fast`/`<id>`/`provider:id`), `approvalMode:`, `tools`/`disallowedTools`, `working_dir`, `isolation:"worktree"`; Claude-Code-compatible frontmatter; built-in Explore agent.
- Agent Teams (experimental) + `/coordinate` skill (read-only workers + optional single worktree writer, shared task list, teammate messaging).
- Background tasks: shell `is_background` (required param), `monitor` tool (streams long-running output), `Ctrl+B` promotes foreground shell to background, `/tasks`, Background-tasks dialog (pause/resume/stop/save script), `/workflows`.
- Goals: `/goal <condition>` keeps working until met (Goal v3 runtime; `usage_limited` state after evidence exhaustion); works headless.
- Hooks: 4 executor types (`command`, `http`, `function`, `prompt` [LLM-judged]); events: PreToolUse, PostToolUse, PostToolUseFailure, UserPromptSubmit, SessionStart/End/Delete, MessageDisplay, Stop, StopFailure, SubagentStart/Stop, PreCompact, PostCompact, Notification, PermissionRequest, PermissionDenied, TodoCreated, TodoCompleted; matchers; async hooks; `/hooks`.
- Checkpointing/restore: file checkpointing on by default interactive; `/restore` reverts files to pre-tool-call snapshot; `/rewind` (`/rollback`) conversation turns; `Esc Esc` rewind selector; `/branch` forks conversation into new session; `/fork <directive>` spawns background fork agent.
- Compaction: `/compress` (LLM summary), `/compress-fast` (model-free strip of old tool outputs/thinking); auto-compaction 3-tier ladder (warn/auto/hard), `context.autoCompactThreshold` (0.85); recent files/images retained; screenshot-count trigger; dedicated `compactionModel`; compression reuses main prompt cache; `/context` usage breakdown; tips at 50/80/95%.
- Memory: `QWEN.md` (`~/.qwen/QWEN.md`, project root, `.qwen/QWEN.local.md`; reads `AGENTS.md`; `@file` imports; `/init`); Auto-memory (writes `~/.qwen/projects/<project>/memory/*.md` + `MEMORY.md` index; `pinned/` protected; nightly `/dream` consolidation; `/remember`, `/forget`, `/memory`); opt-in team memory in `.qwen/team-memory/` (secret-scanned, optional git auto-sync); Auto Recall (mem0) opt-in.
- Todo (`todo_write`); `ask_user_question`; `/btw` side questions; `/recap`; `/summary`, `/insight`.
- Loop/scheduling: `/loop [interval] [prompt]` cron-style session-scoped scheduler; bare `/loop` = autonomous steward loop; channels have persistent scheduler.

Tools
- File system: `list_directory`, `read_file` (incl. Jupyter), `read_many_files`, `write_file`, `edit`, `notebook_edit`, `glob`, `grep_search` (ripgrep; respects `.gitignore`/`.qwenignore`); encoding preservation.
- Shell: `run_shell_command` (mandatory `is_background`, timeouts, interactive commands, allow/deny restrictions, shell-safety classification); `!` prefix; `monitor` tool.
- Web: `web_fetch`; `web_search` via MCP (Bailian, Tavily, GLM).
- Vision/multimodal: image paste (`Ctrl+V`), `@` image/PDF/audio/video input, Vision Bridge (`/model --vision`) transcribes images for text-only main model; `display_image` tool renders inline images in Kitty/Ghostty/chafa; image-generation model (`/model --image`); PDF vision-bridge fallback.
- LSP: experimental (`--experimental-lsp`), `.lsp.json`; definitions/references/hover/diagnostics/code actions/call hierarchy; `/lsp`.
- MCP client: stdio / streamable HTTP / SSE; `qwen mcp add|remove|list`, `/mcp`; scopes user/project; OAuth; MCP prompts as slash commands; resources via `@server:uri`; per-server tool allow/deny; trust; `/import-config` from Claude Code/Claude Desktop; ToolSearch deferred loading (`tools.toolSearch.*`).
- Computer Use: built-in `computer_use__*` deferred tools via cua-driver (click/type/screenshot/apps), on by default, macOS Accessibility+Screen Recording.
- Extensions: `qwen-extension.json` bundles (prompts, MCP servers, subagents, skills, commands, hooks, LSP); install from git/local/archive/npm/marketplace; supports Gemini CLI extensions, Claude Code Marketplace plugins, Qoder plugins, Agent Plugins v1 (0.21.11); `/extensions` manager with hot-reload; `/reload-plugins`.
- Custom commands: Markdown (recommended) or TOML (deprecated) in `.qwen/commands/`; `{{args}}`, `!{shell}`, `@{file}`.
- Skills: `SKILL.md` (personal `~/.qwen/skills/`, project `.qwen/skills/`, extension); model-invoked or `/<skill>`; `/skills` panel; `paths:` gating; `/learn <url|dir|video|text>` auto-generates skills; `/curator` skill hygiene; built-in skills `/review`, `/coordinate`, `/loop`, `/simplify`, `/qc-helper` (README also lists `/batch`, `/bugfix`, `/verify` — unverified).
- Worktrees: `enter_worktree`/`exit_worktree` tools; `qwen --worktree[=name|#PR|url]`; subagent `isolation:"worktree"`; stale cleanup.

Permission & sandbox
- Five approval modes: `plan`, `default` (Ask), `auto-edit`, `auto` (LLM classifier judges shell/network/out-of-workspace edits; fail-closed; loop guard; NL hints), `yolo`; `Shift+Tab` cycles; `/approval-mode [mode] [--project|--user]`; `--approval-mode`, `--yolo`.
- `permissions.allow` / `ask` / `deny` rule lists (deny > ask > allow), `/permissions`; `tools.disabled`.
- Trusted folders (`/trust`); untrusted folders block privileged modes; `--safe-mode` disables customizations.
- Sandbox: macOS Seatbelt (`sandbox-exec`, profiles permissive-open/closed/proxied, restrictive-*, custom `.qwen/sandbox-macos-<name>.sb`); Docker/Podman image `ghcr.io/qwenlm/qwen-code:<ver>`; `-s/--sandbox[=provider]`, `QWEN_SANDBOX`; network allowlist via `QWEN_SANDBOX_PROXY_COMMAND`; `--yolo` does NOT imply sandbox.
- Run-level budgets: `--max-session-turns`, `--max-wall-time`, `--max-tool-calls` (exit codes 53/55).

UI/UX
- Ink-style TUI; in-app scrollable viewport, SGR mouse (wheel, click-to-position, text selection incl. word/line), `Ctrl+O` expand thinking/tool detail, tool-use summaries (fast-model batch labels), followup ghost-text suggestions, `/theme` (dark/light + auto detection via OSC 11/COLORFGBG + custom themes incl. diff colors), `/vim` mode, `/editor`, `/terminal-setup`, `Ctrl+R` history search, `Ctrl+S` prompt stash, `Ctrl+Q` queue prompt, `@` completion, `!` shell mode, `/directory` multi-dir, `/cd`.
- `/diff` interactive diff viewer, Markdown rendering (Mermaid/tables/LaTeX; `Alt+M` raw), inline terminal images, `/copy`.
- Status line: `/statusline` presets or agent-generated command; footer with worktree, approval mode, background-task pill; live-agent panel; Arena tab bar; window-title status symbols; `general.terminalBell`; Notification hook; contextual tips; custom banner.
- Voice: `/voice` dictation (hold/tap), `voiceModel`; experimental Live Voice (Web Shell macOS).
- Stats: `/stats` dashboard, `/stats model` (per-model tokens + cost), `/stats tools|skills|daily|monthly|export`; cache hit %.
- i18n: `/language ui` (en, zh, ru, de, ja, pt-BR, fr, ca…), `/language output`.
- Other surfaces: VS Code extension (Beta, native panel, multi-session), Zed & JetBrains via ACP, Web Shell browser UI (via `qwen serve`; artifact panel, git status/diff/log, image drag-drop, channels sidebar), Qwen Code Desktop (Tauri), Local Control QR pairing, Chrome extension + mobile-mcp.

Sessions
- Project-scoped JSONL under `~/.qwen/projects/<cwd>/chats`; `/resume` picker, `--continue`, `--resume <id>` (headless too); `/rename`, `/delete`, `/export html|md|json|jsonl`, `/branch`, `/rewind`, `/history collapse`, `/clear`/`/new`; `qwen sessions list [--json]`; per-worktree isolation; crash recovery. No sharing/publish feature found.

Model support
- Qwen OAuth free tier discontinued 2026-04-15.
- `/auth`: Alibaba ModelStudio (Coding Plan weekly quota; Token Plan; DashScope key), Third-party (DeepSeek, MiniMax, Z.AI, Idealab, ModelScope, OpenRouter, Requesty), Custom provider (OpenAI / Anthropic / Gemini / Vertex-AI protocols).
- `modelProviders` in settings.json (`openai`, `anthropic`, `gemini`, `vertex-ai`, custom); `/model` runtime switching; local servers (Ollama, vLLM, LM Studio) via OpenAI-compatible baseUrl.
- Coding Plan models incl. qwen3.5/3.6/3.7-plus, qwen3-coder-plus/next, qwen3-max, glm-5, kimi-k2.5, MiniMax; Qwen 3.8 (qwen3.8-max) reasoning controls.
- Per-role models: `fastModel`, `compactionModel`, `visionModel`, `voiceModel`, `imageModel`, `agents.builtin.exploreModel`, subagent `model:`; Arena multi-model.
- Reasoning: `/effort low|medium|high|xhigh|max`, `generationConfig.reasoning`; thinking-tag leak defense.
- Prompt caching: `enableCacheControl` (Anthropic/DashScope), stable tool-schema ordering, compression cache sharing, fork cache-prefix sharing; `/stats` cache %; `contextWindowSize` per model; `QWEN_CODE_UNATTENDED_RETRY`.

Automation
- Headless `qwen -p` / stdin; `--output-format text|json|stream-json`, `--include-partial-messages`, `--input-format stream-json` (bidirectional); `--json-schema` structured output; `--system-prompt`, `--append-system-prompt`; `--exclude-tools`, budgets; consistent exit codes.
- Dual Output: `--json-fd`/`--json-file` sidecar JSON events + JSONL command file to drive TUI externally.
- SDKs: TypeScript `@qwen-code/sdk` (`query()`, permission handlers, SDK-embedded MCP servers), Python `qwen-code-sdk` (alpha), Java (alpha); daemon client SDK.
- `qwen serve` daemon (experimental, HTTP+SSE/ACP, multi-client, Web Shell :4170, bearer token); `qwen channel` IM bots (Telegram, DingTalk, WeChat, Feishu, QQ Bot, WeCom) + GitHub/GitLab channels (autofix, review comments).
- GitHub Action `qwen-code-action` (`@qwencoder` mentions, `/setup-github`); ACP for Zed/JetBrains; `acpx`/"Qwen Code Claw"; OpenTelemetry.

Self-improvement / eval
- README: "actively iterating on itself — using its own agent and models to file issues, submit PRs, review code, run tests"; many commits by `@qwen-code-dev-bot`; `/review` (14 parallel agents, sharded verification, reverse audit, `--comment`, `--fix`); GitHub-channel autofix with round limits.
- Release benchmark pipeline: stable releases trigger self-hosted runs of SWE-bench Verified (500) and Terminal-Bench 2.0 (89) via Harbor, publishing result JSON + trajectories to the Release; no published scores located. Arena doubles as in-repo model benchmarking.

Distinctive: Agent Arena (`/arena --models a,b,c "task"`: parallel top-level agents in isolated worktrees, metrics + diff comparison, pick winner); Auto approval mode with LLM classifier; Auto-memory + `/dream` + team memory; `/learn` auto-skills + `/curator`; `/goal`; `/loop` steward; fork subagents with cache sharing; Agent Teams `/coordinate`; multi-protocol (OpenAI/Anthropic/Gemini/Vertex) + per-role model routing; Vision Bridge for text-only models; `qwen serve` + Web Shell + Desktop (Tauri) + IM channels + Computer Use + voice; extension compatibility with Gemini CLI, Claude Code marketplace, Qoder, Agent Plugins v1.

### DeepSeek Harness (`dsh`) — official developer preview
- Repo github.com/deepseek-ai/deepseek-harness ("DeepSeek Harness: Everything is a Plugin."), created 2026-08-13, MIT, TypeScript on vendored Cordis plugin kernel; developer preview, no tagged release; npm `@deepseek-ai/dsh` 0.1.0-rc.6 (2026-08-13); ~128k stars / 12.8k forks by 2026-08-16. Site https://deepseek.com/harness/en/ ; docs https://deepseek-harness.github.io/deepseek-harness/en/guide/quickstart. Positioned as "Agent = Model + Harness".
- Primary UI is a Web UI, not a TUI: `npx @deepseek-ai/dsh web` → http://127.0.0.1:3080; `dsh --profile <name>`, `dsh --profile headless "job"` (one-shot); profiles = ordered plugin-bundle patch layers (`dsh-base`, `dsh-web-app`, `dsh-headless`); TUI only via out-of-tree plugin (docs example `turtle-ui`, repo not found — unverified).
- Runtime modes: Standard (edit/shell/search/plan/subagents/workflows), Code (adds TS `run_code` orchestration), Minimal (`bash` + `str_replace_editor` only, for benchmarking), Creator (runtime inspection + plugin experimentation).
- Tools: `read/write/edit/read_image`, `glob/grep` (ripgrep), `bash` (+ `run_in_background`), `pwsh`, persistent PTY `terminal_*`, `str_replace_editor`, `web_fetch/web_search`, `todo_write`, `ask_user_question`, `exit_plan_mode`, `subagent`/`subagent_fork`, `list_agents/send_message/interrupt_agent`, `report`, `job_*`, `workflow`, `ralph` (fresh-agent loop), `create/get/update_goal`, `schedule_*`, `lsp`, `skill` (SKILL.md discovery), `session_search/trace/event_*` (searchable session log), `run_code`, `cordis_*` (agent inspects/mounts its own plugins = self-modification), MCP client (`mcp__<server>__…`, stdio).
- Subagent providers: in-process spawn/fork, ACP child, Codex app-server child, Claude Code child via Claude Agent SDK, dsh-SDK child. Hooks: bridges that run existing Claude Code and Codex `hooks.json` hooks. ACP server (automation-only). SDKs: TypeScript JSON-RPC client; Python `deepseek-harness-sdk` (bundled runtime, no Node needed).
- Permission/sandbox: presets `workspace-write` (sandbox + approval `ask`) and `danger-full-access`; approval fail-closed; sandbox modes `read-only|workspace-write|danger-full-access` via Linux bwrap→Landlock, macOS Seatbelt, Windows restricted-token ACL (`SANDBOX_UNAVAILABLE` fail-closed); E2B provider (POC).
- Sessions: append-only event log, JSONL or SQLite, checkpoint policy, resumable/forkable, LLM titles, OTel telemetry, compaction (summarizing backend + model-free tool-result pruner), token meter, settings.yaml, i18n docs (en/zh).
- Models: direct `llm-deepseek` adapter (chat-completions route text-only) + `llm-pi-ai` multi-provider adapter (OpenAI, Anthropic, Bedrock, Vertex, Azure, Codex OAuth, custom OpenAI-compatible; per-model `input:[text,image]`, reasoning level, retry policy); models e.g. deepseek-v4-flash/-pro.
- Caveats: "THERE WILL BE COMPATIBILITY-BREAKING CHANGES"; no benchmark numbers published; web-first; heavy plugin/config (cordis.yml) surface; press over-engineering critiques (unverified).
- Sources: https://github.com/deepseek-ai/deepseek-harness · https://deepseek.com/harness/en/ · https://www.npmjs.com/package/@deepseek-ai/dsh · https://technode.com/2026/08/14/against-claude-cowork-deepseek-opens-its-open-source-harness-to-developers/ · https://thenewstack.io/deepseek-harness-open-source-plugins/ · https://justin3go.com/en/posts/2026/08/15-deepseek-harness-review
- Before Aug 2026: no official "DeepSeek CLI" existed in the deepseek-ai org; "DSH" = DeepSeek Harness only. Official curated list deepseek-ai/awesome-deepseek-agent (Jun 2026) documents using DeepSeek-V4 inside third-party agents (Claude Code, Codex, Qwen Code, OpenCode, Crush, Pi, Cline, Kilo, Copilot CLI, Hermes, OpenClaw…). Community DeepSeek-native CLIs (unofficial): Deep Code (`@vegamo/deepcode-cli`), DeepSeek-TUI (github.com/Hmbown/DeepSeek-TUI; Rust, Codex-style, sandboxed tools, MCP client+server), Reasonix (`npx reasonix code`), plus awesome-deepseek-harness (0xsline) plugin list for dsh.

## II.4 Feature inventory — Cursor, Devin, Windsurf→Devin Desktop, Amp, Kiro, Warp, Zed, Factory Droid, Jules/Antigravity + interop standards

### Cursor (Anysphere → SpaceX, acquisition closed 2026-08-14)
Versions: 3.11 (Jul 10, 2026) last minor with changelog notes; 3.12.x patch builds after Jul 17; 2.0 shipped Oct 29, 2025; 3.0 Apr 2, 2026. (https://cursor.com/changelog, https://cursor.com/blog/joining-spacex)

Core loop
- Modes: Agent (default), Plan (clarifying questions, interactive plan UI, Mermaid, plan-in-background, plans shareable in transcripts), Ask (read-only, RO terminal), Debug (runtime instrumentation/root-cause, 2.2).
- Subagents (2.4, Jan 2026): built-in Explore/Bash/Browser; custom subagents as Markdown+YAML in `.cursor/agents/`, `.claude/agents/`, `.codex/agents/` (or `~/…`); frontmatter `name/description/model` (`inherit` or e.g. `claude-opus-5[effort=high,context=300k]`), `readonly`, `is_background`; foreground/background, nested, resumable by ID; async subagents w/ tree coordination (2.5); `/multitask`, "Build in Parallel", `/in-cloud` (cloud subagents in separate VMs), `/babysit` (PR prep).
- Parallel/multi-agent: Agents Window (3.0) runs many agents locally, in git worktrees (`/worktree`), in cloud, over remote SSH; `/best-of-n` (same task across models in separate worktrees + judge); agent tabs/tiles/full-screen; multi-root workspaces (3.2); long-running agents (research preview, Feb 2026, Ultra/Teams/Ent).
- Cloud Agents: isolated VMs; start from desktop, cursor.com/agents, iOS/iPad, Android PWA, Slack, GitHub/Bitbucket comments, Linear, Jira, MS Teams, API/SDK; `.cursor/environment.json` (Dockerfile) or snapshots; "Development Environments" (multi-repo, build secrets, version history/rollback, May 2026); "builds" (pre-built envs, revert-to-last-good, Aug 2026); computer use w/ artifacts (screenshots/video/logs, Feb 2026); team-scoped secrets, outbound domain restrictions, Tailscale; self-hosted Cloud Agents (Mar 25, 2026).
- Local↔cloud handoff: prefix message with `&` (CLI/editor); 3.7 local-to-cloud handoff.
- Hooks: `sessionStart/End`, `preToolUse/postToolUse/postToolUseFailure`, `subagentStart/Stop`, `beforeShellExecution/afterShellExecution`, `beforeMCPExecution/afterMCPExecution`, `beforeReadFile/afterFileEdit`, `beforeSubmitPrompt`, `preCompact`, `stop`, `afterAgentResponse/afterAgentThought`, Tab hooks, `workspaceOpen`; command-based (JSON stdin/stdout, exit 2 = block) or prompt-based (LLM-judged); options `timeout`, `loop_limit`, `failClosed`, `matcher`; scopes: Enterprise MDM → Team dashboard → project `.cursor/hooks.json` → user `~/.cursor/hooks.json`; cloud agents run command hooks only.
- Checkpoints: auto snapshots before significant changes, restore, stored outside git; queued/reorderable follow-ups.
- Context: `/summarize`, auto compaction (`preCompact`), context-usage breakdown/canvas (3.3/3.7), `.cursorignore`, @-mention transcripts, 1M context on select models.
- Rules/memory: `.cursor/rules/*.mdc` (Always / Apply Intelligently / glob / Manual), `AGENTS.md` (root+subdirs), User Rules, Team Rules (enforceable), `~/.cursor/rules`, Skills (`SKILL.md`), Team Commands, `/rules` generator in CLI; Memories = persistent notes across Automation runs.
- Plugins: Marketplace Plugins (2.5) bundle skills/subagents/MCP/hooks/rules; Team Marketplaces (2.6) with Default Off/On/Required; "Customize Cursor" page (3.9).

Tools: search/read/edit files, shell (background procs, `Await` tool), web search/fetch, Browser (screenshots, navigation, component tree, style editor, Design Mode ⌘⇧D, multi-tab), image generation, ask-user, MCP (stdio/HTTP, OAuth one-click, MCP resources/elicitation, MCP Apps interactive UI since 2.6 Mar 3 2026), Canvases (Apr 2026), Instant Grep, LSP, Jupyter.

Permissions & sandbox: Run modes: Auto-review (default since 3.6: allowlist → sandbox → LLM classifier (Claude 4.5 Haiku / GPT-5.4 Mini) judging Shell/MCP/Fetch; `permissions.json` `allow_instructions`/`block_instructions`), Allowlist, Run Everything; "Ask Every Time" deprecated 3.5. Sandbox: macOS Seatbelt, Linux Landlock+seccomp (bubblewrap fallback), Windows via WSL2 *(unverified)*; workspace R/W, protected `.git/config|hooks`, `.vscode`, `.cursorignore`; network blocked by default; `sandbox.json` network allowlist modes; browser/file-deletion/external-file/dotfile approvals; MCP allowlist; enterprise egress policies.

UI/UX: Agents Window, side chats, redesigned PR review (Reviews/Commits/Changes), diff review pane w/ per-file accept + commit&push, word-level diffs, Canvases, Design Mode, in-editor browser, 4 layouts, OS notifications, Voice Mode (batch STT), Cursor Blame (AI attribution), transcript search. Mobile: iOS app (Jun 29, 2026; Live Activities, push, PR review, artifacts), iPad (Jul 29; Apple Pencil markup), Android PWA. Integrations: Slack (@cursor, plans-before-execution, emoji triggers), GitHub/GitLab/GHES/Bitbucket/ADO, Linear, Jira, MS Teams, Sentry, PagerDuty, Google Workspace plugins, JetBrains via ACP (Mar 4, 2026).

Sessions: CLI `agent ls`, `agent resume`, `--continue`, `--resume=<id>`; shared read-only/forkable transcripts; cloud agent URLs shareable; iOS remote control.

Models: Composer 2 / 2.5 (+Fast), Grok 4.5/4.6; Anthropic Claude 4.x–Fable 5; OpenAI GPT-5.x incl. 5.6; Gemini 2.5–3.7; GLM 5.2; Kimi K2.7/K3. Auto Cost/Balance/Intelligence; Cursor Router (Jul 22, 2026; per-request routing, admin-governed). No local/self-hosted models. Token Rate $0.25/M third-party (Teams/Ent).

Automation: CLI headless `agent -p "…" --output-format text|json|stream-json`, `--force/--yolo`, `--sandbox`, `CURSOR_API_KEY`; `agent acp` ACP server mode; `@cursor/sdk` TypeScript (Apr 29, 2026; local in-process or cloud; custom tools via built-in MCP, autoReview, custom stores, nested subagents); Cloud Agents API (SSE streaming, lifecycle); Bugbot (PR review GitHub/GHES/GitLab/Bitbucket/ADO, learned rules `@cursor remember`, `.cursor/BUGBOT.md`, Autofix via cloud agent, effort levels, `/review`); Security Review (PR checks + scheduled Vulnerability Scanner); Automations (Mar 5, 2026: cron, GitHub/GitLab/Bitbucket events, Slack, Linear, Sentry, PagerDuty, webhooks; Memories; computer use; `/automate`, `/loop`).

Team/Enterprise: SAML SSO, SCIM, RBAC, SOC 2 II, ZDR, Privacy Mode; model/provider/MCP/repo allow-blocklists; sandbox/network/browser controls; spend limits; billing groups; service accounts; audit logs; AI code tracking API; conversation insights; team rules/hooks/commands/MCP distribution; Team Marketplaces; Organizations; self-hosted cloud agents; AWS-hosted only. Pricing: Hobby free; Pro/Pro+/Ultra; Teams $40/user; Enterprise; Cursor Start ₹649 (India).

Distinctive: `/best-of-n` multi-model race + judge; Design Mode + Canvases + MCP Apps host; Cursor Router w/ admin modes; prompt-based (LLM) hooks w/ `failClosed`; Auto-review classifier run mode; native iOS/iPad; Bugbot learned rules + Autofix; ACP agent (runs in JetBrains/Zed).

### Devin (Cognition) incl. Devin Desktop (ex-Windsurf)
Versions: Devin 2.0 (Apr 3, 2025), 2.1 (Jun 10, 2025), 2.2 (Feb 24, 2026, last numbered); continuous release notes to Aug 14, 2026; API v3 (Feb 20, 2026); Devin CLI (Apr 27, 2026); Devin Desktop replaced Windsurf (Jun 2, 2026); SWE-1.5 (Oct 2025), SWE-1.6 (Apr 7, 2026), SWE-1.7 (Jul 8, 2026); Devin Fusion (Jun 29, 2026). (https://docs.devin.ai/release-notes/overview)

Core loop
- Planning: Interactive Planning (2.0), Ask Devin Ask/Plan modes, `/plan /review /test /think-hard`; Confidence Scores 🟢🟡🔴 (2.1; waits for approval on 🟡/🔴); Devin Local plan mode → `~/.devin/plans/`, `megaplan`; Devin Coach prompt suggestions (Aug 2026).
- Sub-agents/parallel: parallel Devins each w/ own VM+IDE (2.0); Devin manages Devins (Mar 19, 2026: coordinator spawns child sessions w/ prompt/playbook/ACU limits, messages/pauses/terminates; structured output); Security Swarm "Agentic MapReduce" (Jul 2026); Auto-Triage; CLI/Local subagents (`agents/<name>.md`, fg/bg).
- Cloud: every session = isolated VM w/ shell, browser, VS Code IDE, Linux desktop (VNC), video recordings; Windows VMs (`!windows`, May 2026), Android emulators; environment blueprints (declarative, git-backed, snapshot builds); Devin Outposts (Jul 22, 2026: self-hosted data plane on VMs/K8s/Mac mini; partners Namespace, Modal, NVIDIA Brev, Daytona, E2B, Cloudflare).
- Hooks: CLI/Local lifecycle hooks (`/hooks`, `devin migrate hooks`); Desktop Cascade Hooks (12 events).
- Checkpoints/rewind: CLI `/steps`, `/revert <step>`, `/fork [step]`; session duplication; wake sleeping sessions.
- Context: `/compact`, `/context`, `/usage`; SWE-1.7 self-compaction; Fusion switches models at compaction; Devin Local ~30% fewer tokens.
- Memory/rules: Knowledge (org notes, suggested knowledge, API), Playbooks (procedures, structured output schema, `/name`), "Skills & Rules", `AGENTS.md`, `.devin/wiki.json`, `REVIEW.md`; `devin rules|skills|plugins|migrate`; Desktop legacy Cascade Memories/Workflows.

Tools: shell, VS Code IDE (real-time collab/takeover), browser + computer use (VNC), test video, App Deploys, web search, Data Analyst "Dana", DeepWiki/Ask Devin, MCP marketplace (1000+; custom OAuth, secret scoping, read-only mode, run-from-Devin-servers), Devin MCP server `https://mcp.devin.ai/mcp` (wiki/session/playbook/knowledge/schedule ops), Secrets & cookies, VPN, side chats, `/btw`.

Permissions & sandbox: Cloud: Guardrails V3, network access requests (approve from Slack), session ACU hard caps, Secure mode, IP allowlists, MCP approval/read-only, enterprise MCP registry, security profiles for automations. CLI/Local: modes Normal / Accept-Edits / Smart (fast model judges) / Plan / Bypass-Dangerous / Autonomous (requires `--sandbox`); Deny>Ask>Allow over file R/W, commands, HTTP, MCP; OS sandbox (Linux bubblewrap+socat; macOS *unverified*; Windows unsupported); domain allow/deny via loopback proxy; enterprise Sandbox Required.

UI/UX: web app + PWA, unified plan→review UI (2.2), worklog/timeline, Tasks/Changes tabs w/ word-level diffs, session folders, command palette, Mermaid/Figma embeds, voice input (Mar 2026), Japanese localization; Devin Review (smart diff grouping, copy/move detection, severity, CWE security findings, auto-fix, auto-merge, stacked PRs, `devinreview.com` URL trick, `npx devin-review`, GitLab); Slack (@Devin, bang commands `!ask !deep !fast !ultra !lite !fusion !swe !windows !dana`, approvals from Slack), MS Teams, GitHub/GHES/GitLab/Bitbucket/ADO, Linear, Jira. Native mobile app: none; PWA.

Sessions: CLI `--continue/--resume`, `/fork`, `--export` (ATIF); `/handoff [task]` packages conversation+branch+diffs into cloud VM; open-source Devin Handoff plugin for Claude Code, Codex, Cursor, shell; share links, message permalinks, Slack thread sync.

Models: Anthropic (Opus 4.7, Fable 5), OpenAI (GPT-5.5/5.6, Codex), Gemini, Cognition SWE-1.5/1.6/1.7 (~1000 TPS via Cerebras; SWE-1.7 Kimi K2.7-based), SWE-grep, SWE-Check, DeepSeek/Kimi K3/GLM, Grok 4.5; Adaptive router ($0.50/$2.00/M); Devin Fusion (frontier + sidekick mid-session routing, 35–60% cheaper); BYOK *(unverified)*; no local models.

Automation: API v3 (sessions, PR reviews, knowledge, playbooks, secrets, blueprints, automations, enterprise; Terraform provider; PATs; OIDC tokens; service users); CLI `devin -p`, `devin acp` (ACP server for JetBrains/Zed/Xcode), `devin worker start --outpost=`; Automations (Slack/GitHub/GitLab/Linear/Jira/RRULE schedule/webhook/files-changed triggers; Triage Devin; ACU caps; queueing; API v3 GA Aug 7, 2026); Devin Review PR bot (auto-fix/auto-merge, CI check, REST API); Security Swarm scheduled scans; scheduled sessions ("Devin schedules Devins", Mar 20, 2026).

Enterprise: SOC 2 II, ZDR, FedRAMP High In-Process (Jul 2026), SSO/SCIM GA, RBAC/custom roles, audit logs, IP allowlists, enterprise MCP registry, ACU pools, dedicated tenant, Outposts, Cerebras enterprise. Pricing: Free / Pro $20 / Max $200 / Teams $80 min / Enterprise (ACU).

Distinctive: per-session full VM w/ desktop, Windows/Android targets and video; Devin-manages-Devins; Devin Review stacked-PR merge; DeepWiki; Slack bang-modes; Outposts; Fusion routing; SWE-1.x at ~1000 TPS; Security Swarm; open-source `/handoff` plugin usable from Cursor/Claude Code/Codex.

### Windsurf Cascade → Devin Desktop (Cognition)
Versions: Wave 12 (Oct 2025: Codemaps, Fast Context/SWE-grep, dev containers), Wave 13 v1.13.3 (Dec 24, 2025: parallel Cascade sessions, worktrees, Cascade Hooks, context meter, SWE-1.5 default/free), Skills (Jan/Mar 2026), Adaptive router (Apr 6, 2026), Windsurf 2.0 v2.0.44 (Apr 15, 2026: Devin Cloud in-editor, Agent Command Center, Spaces, Windsurf Browser), Devin for Terminal v2.1.29 (Apr 29), Devin Review v2.2.17 (May 6), rebrand to Devin Desktop v3.0.12 (Jun 2, 2026), plugins v3.2.16 (Jun 16), v3.6.21 (Jul 29), latest v3.7.25 (Aug 13, 2026). Two agents coexist: legacy Cascade (phasing out; disabled by default for enterprise Aug 7, 2026) and Devin Local (default; same harness as Devin CLI); Migration Wizard. Legacy `.windsurfrules`/`.windsurf/` still honored, `.devin/` preferred. (https://docs.devin.ai/desktop/changelog)

Core loop
- Modes: Code / Plan (persistent md plan, `megaplan`) / Ask; ⌘+. switch; JetBrains plugin also has Planning + Turbo.
- Cascade: auto-todo lists, named checkpoints + revert, mid-turn revert, message queuing (editable), auto-continue, real-time awareness of user edits, auto lint-fix, simultaneous Cascades, @-mention previous conversations, voice input.
- Subagents (Devin Local only): fg/bg, custom `agents/<name>.md`, subagents call MCP directly, default subagent model. Cascade has none.
- Parallel/worktrees: worktree-backed sessions w/ merge back, side-by-side panes; Arena mode (2 models on same prompt in isolated worktrees, blind battle groups Frontier/Fast/Hybrid; not in Devin Local yet).
- Cloud: one-click hand-off to Devin Cloud VM; review Devin changes/tests in editor; multi-device sync via web links; shared quota.
- Cascade Hooks (12 events): `pre/post_read_code`, `pre/post_write_code`, `pre/post_run_command`, `pre/post_mcp_tool_use`, `pre_user_prompt`, `post_cascade_response`, `post_cascade_response_with_transcript` (JSONL), `post_setup_worktree`; exit 2 blocks; JSON payload; system / user `~/.codeium/windsurf/hooks.json` / workspace `.windsurf/hooks.json`; MDM push.
- Memory/rules: auto Memories (workspace-scoped, `~/.codeium/windsurf/memories/`; Devin Local lacks persistent memories → skills), Rules (`global_rules.md`, `.devin/rules/*.md` / `.windsurf/rules/*.md`, `/etc/devin/rules/`, triggers `always_on|model_decision|glob|manual`), AGENTS.md (root+subdirs), Workflows (`.windsurf/workflows/*.md`, `/name`, manual-only, not in Devin Local), Skills (`.windsurf/skills/`, `permissions:` frontmatter), Plugins (enterprise preview; team marketplace).
- Context: context-window meter, Fast Context (SWE-grep), Codemaps @-mention, remote/self-hosted indexing, `.devinignore`.

Tools: terminal (dedicated profile), file R/W, web & docs search, MCP (stdio/HTTP/SSE + OAuth, `${env:}`/`${file:}`, Plugin Store, 100-tool cap, admin registries/allowlists; Devin Local uses `.devin/config.json`, per-server Always allow), package install, images (drag-drop), Previews (in-IDE web view, "Send element", console errors; proxies local dev servers to remote/ACP agents), Windsurf Browser (2.0), App Deploys, Codemaps, DeepWiki, AI commit messages, Jupyter diffs. ACP host: run Claude Agent, Codex CLI, OpenCode, Junie, Gemini CLI inside Desktop via `~/.windsurf/acp/registry.json`.

Permissions & sandbox: Cascade: auto-execution levels (4 incl. Turbo) + team terminal allow/deny (enforced Jun 2026). Devin Local: Deny/Ask/Allow over read/write/exec/HTTP/MCP at project/user/org; OS sandbox (FS bounds + domain filtering); modes normal/accept-edits/smart/dangerous/autonomous(--sandbox); editable command approvals w/ fast-model rewrite; enterprise can force sandbox.

UI/UX: Agent Command Center (kanban of local/cloud/CLI/ACP sessions; duplicate/branch), Spaces (sessions+PRs+files), Agent/Editor mode switch, streaming diff highlighting, native notifications, Quick Review (local secondary review) + Devin Review (cloud PR review), Tab/Supercomplete/Tab-to-Jump/Import, conversation sharing (sanitized transcript upload), JetBrains plugin v2.12.26 (Jul 29, 2026) + VS Code/Eclipse/VS plugins. Mobile: none (web links).

Sessions: Devin CLI `-c`/`-r <id>`, `devin list`, `/fork`, `--export`; CLI sessions shared with Desktop; hand-off to cloud; desktop↔web sync.

Models: SWE-1.5/1.6/1.7 (free), SWE-1.7 Lightning, swe-check, Adaptive router; OpenAI GPT-4o…5.6, Anthropic Opus 4.1→5/Fable 5/Sonnet/Haiku, Gemini 2.5→3.6, Grok, DeepSeek V4, Kimi K2.5–K3, GLM-5.x, Nemotron, Minimax; per-token pricing + credit multipliers; enterprise model filtering. BYOK/local: not documented.

Automation: Devin CLI (Rust) `devin -p`, `devin --print -- cmd`, `devin list --format json`, `devin mcp|plugins|models`, `--permission-mode`, `--sandbox`, ACP; Devin Review PR bot; Analytics API v1/v2 + service keys; scheduling via cloud Devin.

Enterprise: SSO (Okta/Entra/Google/SAML/OIDC), SCIM, RBAC, IP lists, Admin Portal, model/MCP/terminal policies, hooks/skills/rules via MDM (Group Policy/macOS profiles), remote indexing, FedRAMP guide, attribution filtering, quota controls, hybrid/dedicated deployment.

Distinctive: local IDE + one-click cloud VM handoff in one product; hosts competitor agents via ACP; Arena blind model battles; Codemaps; 12-event hooks w/ MDM distribution; free SWE-1.x models.

### Amp (Amp Frontier Corp., spun out of Sourcegraph Dec 2, 2025)
State: npm `@ampcode/cli` (May 14, 2026); editor extensions killed ("The Coding Agent Is Dead" Feb 19, 2026; VS Code/Cursor ext self-destructed Mar 5) — surfaces = CLI/TUI, web, mobile web, Slack; Amp Tab removed (Jan 15, 2026); "Amp, Rebuilt"/Neo (May 6, 2026; remote-controllable, compaction-first, plugin-powered, durable), sole Amp since May 27; news through Aug 13, 2026. (https://ampcode.com/news)

Core loop
- The Dial (Jul 9, 2026): `low` (GLM-5.2 / GPT-5.6 Terra), `medium` (GPT-5.6 Sol, default), `high` (Sol xhigh, Fable reviews), `ultra` (Claude Fable 5 + Sol oracle).
- Subagents: Task (parallel/isolated), Oracle (second opinion), Librarian (GitHub repo search), finder, Painter (image gen), custom subagents/agent modes via plugins; agent-to-agent spawn/message/file exchange across threads (Jul 17); Puck meta-agent on ampcode.com/Slack (Jul 20).
- Orbs (cloud): remote Debian machines per thread, 5 sizes $0.08–$1.32/hr per-minute, auto-pause; `amp -ox "prompt"`; `amp sync <thread>`; `.agents/setup|resume` scripts; `.amp/services.yaml` portals; OIDC; event-driven via plugin `amp.createWebhook` (GitHub/Linear/Discord; wakes orb, Jul 23); self-scheduling agents (Jul 21); Multiplayer shared orb control (Jul 22).
- Runners (Jul 8): `amp --no-tui --runner-id X` on your own machine, accepts remotely created threads.
- Context: automatic compaction replaces Handoff; "Read Bigger Threads"; `@thread-id` referencing; `look_at` for PDFs/images. Message queue + "steer" to jump the line.
- Checkpoints: file rollback removed in rebuild; edit/restore/fork prior messages.
- Rules/memory: `AGENTS.md` (cwd+parents+subtree, `globs` frontmatter; `~/.config/amp/AGENTS.md`; `/etc/ampcode/AGENTS.md`); Skills (`.agents/skills/`, admin-pushed global; lazy MCP loading via skills); Plugins (TS/JS `.amp/plugins/`; events `session.start`, `agent.start/end`, `tool.call/result`; register tools/commands/skills/modes/subagents; UI `notify/confirm/input/select`; `amp.ai.ask()`); Checks `.agents/checks/*.yaml`.

Tools: Bash, Read, edit_file, create_file, undo_edit, glob, Grep, finder, web_search, read_web_page, oracle, Task, mermaid, look_at, get_diagnostics, Painter; Attach Anything (video/audio/logs/PDF/spreadsheets to orbs, transcription); MCP (local+remote OAuth, precedence CLI>workspace>user>skills, enterprise registry allowlists); no first-party browser tool *(unverified)*.

Permissions: no approval prompts by default; customize via Plugin API; legacy `amp.permissions`/`amp.guardedFiles`/`amp.dangerouslyAllowAll` rules; Proof of Human passkey for sensitive ops (May 27); isolation via orbs, no local sandbox.

UI/UX: TUI (Ctrl+O palette, Ctrl+S dial, Ctrl+\ threads sidebar), Diffs web/mobile review (Jun 16, 2026; duplicate-block detection, request changes, interactive staging), Ship to origin/main, Agents Panel, `amp review` + checks, thread labels/map, Feed, voice dictation, images, notifications; Slack `@Amp` → Puck; web + mobile web remote control of all threads; thread sharing private/group/workspace/unlisted (public deprecated Jun 2, 2026).

Sessions: `amp threads new/continue [id] [--execute]`, remote control from ampcode.com, `amp sync`, runners, fork threads.

Models: Amp picks per dial mode; no user picker/BYOK; link ChatGPT subscription ("A Dial for You" Aug 10) or X Premium+; models seen: GPT-5.3-Codex→5.6, Claude Opus 4.5–4.8, Fable 5, Gemini 3, GLM-5.2, GPT Image 2; no local.

Automation: `amp -x`, `--stream-json`, `--stream-json-input` (Claude-Code-compatible schema), `AMP_API_KEY`; Python SDK (Dec 2025); webhooks/orbs as CI/PR bots; scheduled agents; workspace API `/api/v2/openapi.json`.

Enterprise/pricing: Workspaces, managed settings, SSO/SAML + directory sync, Minimal Data Retention, entitlements, MCP registry allowlists, IP allowlisting, analytics API; Megawatt $20 / Gigawatt $200 (Jul 18, 2026); Amp Free (ad-supported → ad-free Mar 30, 2026, being reduced).

Distinctive: no approval prompts; always-on orbs w/ webhooks + self-scheduling + agent-to-agent messaging; Puck; runners; ChatGPT-subscription linking; deliberate removal of tab, extensions, model choice.

### Kiro (AWS)
Versions: GA Nov 17, 2025 (IDE 0.6, CLI 1.20 succeeding Amazon Q Developer CLI); IDE 1.0 Jun 25, 2026, latest 1.0.309 (Aug 13, 2026); CLI 2.0 (Apr 13, 2026), CLI v3 harness EA (2.8, Jun 17), latest CLI 2.18.0 (Aug 12, 2026); Kiro Web (app.kiro.dev, May 2026), Kiro for iOS (Jun 17, 2026), Kiro Crew OSS orchestrator (Aug 4, 2026). (https://kiro.dev/changelog)

Core loop
- Specs: `requirements.md` (EARS), `design.md`, `tasks.md`; Requirements-First / Design-First / Bugfix / Quick Spec / Plan-only; "Analyze Requirements"; parallel task waves (0.12); CLI `/spec new`, spec review screen. Correctness/property-based testing: EARS → properties → generated PBT with shrinking (IDE).
- Modes: Chat(Vibe) / Spec / Plan / Bug Fix; Autopilot vs Supervised (hunk-level review). Agent Focus Mode (1.0): sessions rail, parallel sessions, attention cards, IDE→CLI hand-off; nested AGENTS.md (1.0.309).
- Sub-agents: `.kiro/agents/`, `invoke_subagent`, parallel isolated contexts, DAG stages w/ review loops; built-ins context-gathering/general/Plan/Introspect; `/spawn`, `/plan`.
- Cloud Sessions (preview Aug 2026): IDE + CLI `--cloud [--repo]` in managed AWS sandboxes; enterprise opt-in.
- Hooks (`.kiro/hooks/*.json`): `PostFileSave/Create/Delete`, `PreToolUse` (block), `PostToolUse`, `UserPromptSubmit` (block), `SessionStart`, `Stop`; regex matcher; shell command (STDIN context) or agent prompt; NL hook creation; global `~/.kiro/hooks/`.
- Checkpoints: IDE checkpoints rewind code+conversation; CLI `/checkpoint`, `/rewind` (fork at turn), `/tangent` (nestable side-conversations), `/goal` (autonomous loop w/ verification), mid-turn queue steering.
- Context: `#codebase #file #folder #git diff #terminal #problems #url #docs #spec #steering #mcp…`; CLI `@path`, `/context` per-tool tokens, `/compact`, auto-summarization at 80%, PDF/spreadsheet attachments, `/knowledge` semantic index, on-demand MCP tool loading, Skills (`.kiro/skills/`).
- Steering: `.kiro/steering/*.md` (product/tech/structure), inclusion `always | fileMatch | manual | auto`; global `~/.kiro/steering/`; `#[[file:path]]` refs; AGENTS.md; agent learns from PR review comments (Web).
- Custom Agents: `.kiro/agents/*.md|json` w/ tools tags, model, permissions, resources, mcpServers, hooks; hot-reload. Powers: MCP + skills + steering bundles = Agent Plugins format; keyword activation; marketplace (Stripe, Supabase, Datadog, Figma…); "Authorize powers" OAuth.

Tools: file R/W, fuzzy `file_search`, `grep_search`, shell + background procs, web search/fetch, tree-sitter code intelligence (18 langs)/LSP, `invoke_subagent`, `todo_list`, diagnostics, dev-server management, MCP (stdio/HTTP/SSE, OAuth, `kiro://` install, prompts/resources/elicitation), images, CLI `/voice` (on-device Whisper, 2.18); no browser tool (issue #9039); IDE = Code OSS fork (.vsix); ACP server (CLI 1.25, Feb 2026).

Permissions & sandbox: capability-based permissions on Cedar policy engine (`permissions.yaml`, ask/allow/deny per `fs_read`/shell/MCP/web), consent persistable, trusted workspaces, dangerous-command detection; headless `--trust-all-tools`/`--trust-tools=…`; cloud sandbox w/ domain allowlist, secrets, IAM role creds.

UI/UX: hunk-level supervised diffs, inline diffs, "Ask Kiro to Fix", dockable chat, multi-window sync, session search/export, credit meter; Kiro for iOS (start/steer sessions, review diffs, approve, PR); GitHub `kiro` label / `/kiro` comment, PR `/kiro fix`; GitLab (Web); no direct Slack trigger (Kiro Crew adds Slack/Telegram/Discord/WeChat); CLI TUI.

Sessions: CLI `--resume/--resume-id/--resume-picker`, `/chat save|load`, IDE↔CLI hand-off, mobile sees IDE/Web sessions, Cloud Sessions cross-machine.

Models: Auto (default), Claude Opus 5 / Sonnet 5 (1M), Opus 4.6–4.8, Haiku 4.5, GPT-5.6 Sol/Terra/Luna, open-weight MiniMax M2.5, DeepSeek 3.2, Qwen3 Coder Next, GLM-5; `/effort`; regions us-east-1/eu-central-1/GovCloud; no BYOK/local *(unverified)*.

Automation: `kiro-cli chat --no-interactive` + `KIRO_API_KEY`, GH Actions example; Web Automations (cron, → PR, Jun 2026); autonomous agent issue→PR; Kiro Crew (OSS 24/7 orchestrator, heartbeat jobs, memory, chat surfaces); ACP server; `/issue`.

Enterprise/pricing: IAM Identity Center (Okta/Entra/SCIM), Builder ID, MCP registry + model governance, activity CSV, Service Quotas caps, HIPAA eligible, GovCloud, proxy support. Free/Pro $20/Pro+ $40/Pro Max $100/Power $200; add-on credits $0.04.

Distinctive: EARS specs + PBT correctness loop; Cedar-verified permissions; one ACP-based harness across IDE/CLI/Web/iOS; Powers marketplace; Kiro Crew; AWS-native governance/IAM sandboxes.

### Warp (terminal ADE + Oz cloud platform)
Versions: weekly releases; latest 2026.08.13; Warp Code Sep 3, 2025; Agents 3.0 Nov 19, 2025; Oz launch Feb 10, 2026; Universal Agent Support Apr 14, 2026; open-sourced client (AGPL) Apr 27–28, 2026; multi-harness Oz + orchestration + memory May 18–19; Warp Agent CLI Aug 4, 2026; "Oz" agent UI renamed "Warp Agent" Aug 13, 2026. (https://docs.warp.dev/changelog/2026/)

Core loop
- `/plan` → versioned rich-text plan saved to Warp Drive, `@plans`, partial execution, task-list + Code Review panel.
- Multi-agent orchestration (May 2026): parent/child, local↔cloud in any direction, supervisor/worker, fan-out/in, critic, DAG, swarm; durable server-backed message bus, harness-agnostic (Warp Agent, Claude Code, Codex); `run_agents` permission.
- Parallel: multiple sessions, git worktrees, tab configs, ambient/cloud-mode panes, auto-handoff to cloud before macOS sleep.
- Cloud agents (Oz): Warp-hosted or self-hosted (K8s); Docker Environments; persistent shareable runs; oz.warp.dev; harnesses Warp Agent/Claude Code/Codex. `/handoff` local→cloud (transcript + workspace changes), `/continue-locally`.
- Hooks: no user-defined lifecycle hooks found *(unverified)*; automation via Skills/triggers/Actions.
- Checkpoints: `/fork`, `/fork-and-compact`, `/compact-and`; no `/rewind`-style checkpoints found *(unverified)*.
- Context: `@` menu (files, blocks, images, URLs, conversations, notebooks, rules, workflows, skills, plans), Codebase Context (semantic indexing), `/compact`, per-profile max context, `/queue`, clarifying questions.
- Rules/memory: `AGENTS.md` (`/init`), `WARP.md` legacy, hierarchy, links to CLAUDE.md/.cursorrules/GEMINI.md; Global Rules in Warp Drive; Agent Memory (research preview: personal/agent/team stores, auto-extraction, citations, shared across harnesses); Skills (`.agents/`, `.claude/`, `/{skill}`, `SKILLS_DIRS`, schedulable).

Tools: Full Terminal Use (agent drives REPLs/debuggers/vim/htop, long-running procs), shell, file edit (native LSP editor), web search, URL fetch, Computer Use + Browser Use (cloud-only, Chromium, screenshots, annotated video artifacts to PRs), images, MCP (stdio/HTTP/SSE, OAuth, one-click, auto-detect Claude/Codex MCP configs, team sharing), Mermaid, file artifacts; voice input; third-party CLI agents inside Warp (Claude Code, Codex, Gemini CLI, OpenCode, Copilot CLI, goose, Mistral Vibe, Antigravity, etc.) get rich input, notifications, review routing, remote control.

Permissions & sandbox: Agent Profiles: base model + autonomy per action type (apply diffs, read files, plans, execute, interact, ask, run_agents) = Agent Decides / Always Ask / Always Allow; command allow/denylist (deny wins); MCP allow/deny; "Run until completion"; `/fast-forward`; onboarding autonomy Full/Partial/None; secret redaction; cloud sandboxes w/ managed secrets + short-lived AWS/GCP creds.

UI/UX: Terminal vs Agent modes; Warp Code Review pane (hunk nav, `R` refine in NL, `E` edit, comments to any running CLI agent); Agent Management Panel; vertical tabs/tab configs; notifications center; Warp Drive (workflows, notebooks, prompts, env vars, rules, plans, MCP); integrations Slack, Linear, Jira, GitHub (`@oz-agent`, label), GitLab, Bitbucket, ADO; no native mobile app — Remote Control (`/remote-control`) publishes any agent session to web/phone w/ live cursors, QR; Agent Session Sharing.

Sessions: Cloud-Synced Conversations (Feb 2026), `warp --resume <token>`, `/conversations`, bidirectional handoff.

Models: GPT-5.2–5.6, Claude Opus 5/Fable 5/Sonnet 5, Gemini 3.x, Grok 4.x (`/connect-grok`), open-weight via Fireworks; Auto routers (responsive/cost-efficient/genius/open-weights) + custom model routers (YAML/NL rules); BYOK (Anthropic/OpenAI/Google, OpenAI-compatible + Anthropic-schema endpoints incl. OpenRouter/LiteLLM; Bedrock/Vertex for Enterprise; not for cloud agents/Auto); local via OpenAI-compatible endpoint *(unverified as documented)*.

Automation: Warp Agent CLI (`warp`, headless via `WARP_API_KEY`); `oz` CLI (`oz agent run|run-cloud`, `--harness oz|claude|codex`, `--output-format`); REST API + Python/TS SDKs; scheduled agents (cron); GitHub Action `oz-agent-action` (PR review, CI-fix PRs, digests).

Enterprise/pricing: Business $50/user (SAML, ZDR, admin denylists/profiles, team API keys), Enterprise (self-hosted execution, BYOLLM Bedrock/Vertex, analytics API, OIDC), SOC 2 II; Free / Build $20 / Max $200; benchmarks Terminal-Bench 52%, SWE-bench Verified 75.8%.

Distinctive: terminal-native full-terminal-use w/ PTY mux; multi-harness cloud control plane + cross-harness message bus + shared memory; bidirectional handoff w/ workspace state; Remote Control of any CLI agent; custom model routers; open-source (AGPL) client.

### Zed Agent Panel + ACP
Versions: Zed 1.0 Apr 29, 2026 (ACP headline feature); 1.12.0 (Jul 23: ACP elicitations default), 1.14.2 (Aug 5: sandboxing), latest 1.15.0 (Aug 12, 2026); Delta multiplayer app private beta Aug 12, 2026. (https://zed.dev/releases/stable, https://zed.dev/blog/zed-1-0)

Core loop
- Threads: concurrent independent threads, Threads Sidebar grouped by project, history/archive/search, import Claude Code/Codex threads. Parallel Agents (Apr 22, 2026): mix Zed Agent / external ACP agent / Terminal Thread; multi-root; git worktree isolation per thread (`create_worktree` hook, `ZED_WORKTREE_ROOT`).
- Subagents: `spawn_agent` tool (0.227.1, Mar 2026), `agent.subagent_model`. Steering/queueing mid-generation (Zed Agent only). Checkpoints: auto "Restore Checkpoint" per edit. Context: auto-compaction at 90% (`agent.auto_compact`, `compaction_model`), `/compact`, "New from summary".
- Instructions: `.rules`, `.cursorrules`, `.windsurfrules`, `.clinerules`, `.github/copilot-instructions.md`, `AGENT.md`, `AGENTS.md`, `CLAUDE.md`, `GEMINI.md`; `~/.config/zed/AGENTS.md`; Rules Library replaced by Skills (`SKILL.md` in `~/.agents/skills/`, `<worktree>/.agents/skills/`, `/skill`, share links). Profiles Write/Ask/Minimal + custom. Hooks: only `create_worktree`.

Tools: `read_file`, `list_directory`, `find_path`, `grep`, `diagnostics`, `fetch`, `search_web` (Zed Pro), `edit_file`, `write_file`, `create_directory`, `copy/move/delete_path`, `terminal`, `skill`, `spawn_agent`; MCP context servers (local/remote/extension; forwarded to ACP agents); `@`-mentions (files, symbols, threads, skills, diagnostics, branch diffs, URLs), images; no browser tool, no voice.

Permissions & sandbox: `agent.tool_permissions` global + per-tool allow|confirm|deny w/ Rust regex `always_allow/deny/confirm` (terminal command, paths, URLs, `mcp:<server>:<tool>`); hardcoded `rm -rf /` blocks; OS sandbox (1.14.2): macOS Seatbelt, Linux Bubblewrap, Windows via WSL; applies to `terminal`+`fetch`; write only project dirs, network via approved HTTP proxy allowlist; `agent.sandbox_permissions`.

UI/UX: multibuffer diff review (`ctrl-shift-r`), accept/reject per hunk/file/all, inline per-file review option, follow-agent crosshair, desktop+sound notifications, keyboard-driven nav, export thread md, favorite models; Terminal Threads (run Claude Code/Amp/OpenCode/Pi/Codex TUIs in sidebar-managed terminals); public Agent Metrics dashboard per ACP agent; no mobile/Slack/Linear. Delta (DeltaDB CRDT sync, shared threads w/ line comments, cloud runners, browser access).

Models: Zed-hosted (Claude Fable 5/Opus 5/Sonnet 5, GPT-5.6, Gemini 3.x); BYO keys Anthropic/OpenAI/Google/Mistral/DeepSeek/xAI/OpenRouter/Vercel/OpenCode; local: Ollama, LM Studio, llama.cpp (auto-discovery), OpenAI-compatible; subscriptions ChatGPT/Claude/Copilot/Cursor; per-purpose models (default/inline/commit/summary/compaction/subagent); no auto routing. Edit Prediction: Zeta/Zeta2.1 (open-source), Copilot, Codestral, Mercury, Ollama, self-hosted.

Automation: no headless Zed agent/CLI; use external agents' headless modes. Pricing: Free (own keys, external agents), Pro $10, Business $30/seat.

External agents: curated Claude Agent (`claude-code-acp` v0.69.0, Aug 16, 2026), Codex (`codex-acp` v1.4.0 at github.com/agentclientprotocol), Gemini CLI, OpenCode, Copilot, Cursor, Pi; `agent_servers` custom `{command,args,env}`; `zed: acp registry`.

### Factory Droid and Jules / Antigravity (brief)
Factory Droid (factory.ai)
- Versions: Droid CLI/App v0.197.0 (Aug 15, 2026); Factory 2.0 "Software Factory" Jun 15, 2026; Series C $150M.
- Modes: Normal / Spec (read-only planning, `--spec-model`, save to `.factory/docs`) / Mission (orchestrator+planner+workers+validators, Mission Control `Ctrl+T`, `droid exec --mission`, 1–500 features).
- Custom droids `.factory/droids/*.md` (model/reasoningEffort/tools/mcpServers; Task tool; built-in `worker`/`explorer`; no nesting). Hooks: PreToolUse, PostToolUse, UserPromptSubmit, SessionStart/End, Stop, SubagentStop, PreCompact, Notification; `permissionDecision`. AGENTS.md, Skills, custom slash commands, Plugins + marketplaces, AutoWiki. `/context`, `/compress`, `/btw`, `/rewind-conversation`, `/fork`, `/tree`, `-w` worktrees, background tasks.
- Tools: Read/LS/Grep/Glob, Create/Edit/ApplyPatch, Execute (`!`), WebSearch/FetchUrl, MCP, images/PDF, Droid Control (terminal/browser/desktop). Autonomy Off/Low/Med/High, allow/deny/blocklists, Droid Shield secret detection; sandbox macOS Seatbelt, Linux bubblewrap+seccomp, filtering proxy.
- UI: keyboard-first CLI; Factory App (desktop+web; inline diff comments, artifacts, Design mode; Jira/Notion/Slack/Linear/PagerDuty); VS Code/Cursor/Windsurf ext; JetBrains + Zed via ACP; Slack `@Factory` → Droid Computer sessions. Sessions `--resume`, `/share`, forking, Sessions API; Droid Computers (managed cloud VMs or BYOM).
- Models: Anthropic/OpenAI/Google/xAI + "Droid Core" open models; Factory Router auto; BYOK `customModels` (anthropic/openai/generic; Bedrock, OpenRouter, Ollama/LM Studio/vLLM local).
- Automation: `droid exec` (`--auto`, `-o stream-json|stream-jsonrpc`, multi-turn JSON-RPC), TS/Python SDKs, GitHub Action `Factory-AI/droid-action`, Software Factory (triage, review, security, QA, release gates, incident response). Pricing Pro $20/Plus $100/Max $200; Enterprise SSO/SCIM/audit/ZDR/CMEK/on-prem/air-gapped.

Jules (Google) + Antigravity
- Jules: async cloud agent, GitHub repo → Google Cloud VM, plan → approval (or auto w/ Planning Critic), PR; auto-fixes GitHub Actions failures on its PRs (Feb 2026); memory (user + repo), AGENTS.md, env setup scripts/snapshots, scheduled tasks, suggested tasks, repoless sessions, web browsing, MCP servers (Linear, Stitch, Neon, Supabase…), Render deploys; Jules Tools CLI (`jules remote new --repo --session --parallel N`, TUI), Gemini CLI `/jules` extension, REST API; models Gemini 3.1 Pro (Pro/Ultra), 3 Flash; limits Free 15/day, AI Pro 100, Ultra 300; changelog last Mar 9, 2026.
- Antigravity 2.0 (I/O May 19, 2026; app v2.8.1 Aug 13, 2026): agent-first desktop app w/ Agent Manager (multi-agent, worktree mode), `/schedule`, native voice, `/goal`, `/grill-me`, `/btw`, custom hooks + subagents, MCP w/ admin policies; browser subagent (Chrome control, allow/deny lists) + artifacts (plans, walkthroughs, screenshots, recordings); Antigravity CLI + SDK; models Gemini 3.5 Flash default, Claude, GPT-OSS; Google AI Pro/Ultra pricing.

### Interop standards between editors and agents
Agent Client Protocol (ACP) — agentclientprotocol.com
- Origin: Zed, Aug 2025; JetBrains co-develops (Oct 6, 2025); JetBrains' Sergey Ignatov Lead Maintainer (Feb 18, 2026); ACP Registry launched Jan 28, 2026 (registry.json at cdn.agentclientprotocol.com; 40+ agents by Apr 2026, 50+ by late Jun); Rust/TS SDKs 1.0.0 Jun 24–25, 2026; latest Rust crate v1.6.0 / schema v1.20.0 (Jul 21, 2026); v2 draft (Jul 20, 2026; schema 2.0.0-alpha.2).
- v1 wire: JSON-RPC 2.0 over stdio (agent as subprocess); Transports WG (Apr 2026) for remote transports. Agent methods: `initialize`, `authenticate`, `session/new`, `session/prompt`, `session/load` (replay), `session/list`, `session/resume`, `session/close`, `session/delete`, `session/set_mode`, `session/set_config_option`, `session/cancel`, `logout`, `$/cancel_request`. Client methods: `session/request_permission` (allow/reject once/always), `fs/read_text_file`, `fs/write_text_file`, `terminal/create|output|wait_for_exit|kill|release`, `elicitation/create`. Notifications `session/update` kinds: agent/user message chunks, thought chunks, `tool_call`/`tool_call_update` (kinds read/edit/delete/move/search/execute/think/fetch/other; content|diff|terminal; `locations` follow-along), `plan`, `available_commands_update` (slash commands), `current_mode_update`, `config_option_update` (select/boolean; categories mode/model/model_config/thought_level), `usage_update`, `session_info_update`. Session setup: `cwd`, `mcpServers` passthrough (stdio required, http optional, sse deprecated), `additionalDirectories`. Capabilities negotiated (fs, terminal, elicitation, loadSession, promptCapabilities image/audio/embeddedContext). Extensions via `_`-prefixed methods and `_meta`.
- v2 draft changes: `auth/login|logout`, `session/resume {replayFrom}` replaces load, modes → config options, client fs/terminal methods removed (terminals agent-owned display), `session/prompt` returns immediately + `state_update`, required `messageId`, unified `tool_call_update` + streaming content, `cancelled` status, diffs → `changes[]`+`git_patch`, MCP stdio|http only, JSON-RPC batches.
- Clients: Zed, JetBrains IDEs, VS Code (3 extensions), Neovim (CodeCompanion, agentic.nvim, avante.nvim, hermes.nvim), Emacs, Qt Creator, Pulsar, Unity, Obsidian, marimo, Jupyter, TUIs (Toad, acpx…), desktop (Kepler/GitKraken, Codeg…), mobile (Happy, Shellular, Agmente, Ferngeist), messaging bridges, frameworks (LangChain, LlamaIndex, Mastra); Devin Desktop hosts ACP agents.
- Agents: Claude Agent, Codex CLI, Gemini CLI, GitHub Copilot (preview), Cursor (`agent acp`), Devin CLI (`devin acp`), Kiro CLI, Factory Droid, Junie, OpenCode, Goose, Cline, OpenHands, Kimi CLI, Qwen Code, Mistral Vibe, Augment, Poolside, cagent, Hermes, etc.

Other emerging standards
- MCP spec 2026-07-28: stateless architecture (session header + `initialize` handshake removed; version/caps in `_meta`), `Mcp-Method`/`Mcp-Name` routing headers, `ttlMs`/`cacheScope`, first-class extensions framework, MCP Apps as official extension (SEP-1865; first shipped Jan 26, 2026; `ui://` resources, sandboxed iframe, JSON-RPC bridge), Tasks extension (`tasks/get|update`), multi-round-trip input requests, OAuth issuer binding + Client ID Metadata Documents; deprecations: Roots, Sampling, Logging, HTTP+SSE. Hosts rendering MCP Apps: Claude Desktop, VS Code/Copilot, Goose, Cursor (2.6).
- Agent Host Protocol (AHP) — Microsoft, MIT-licensed (github.com/microsoft/agent-host-protocol): VS Code 1.129 (Jul 16, 2026) / 1.133 (Aug 12, 2026) run Copilot, Claude (Claude Agent SDK), Codex adapters in a separate Agent Host process; JSON-RPC, URI-addressed channels (sessions/chats/terminals/changesets) w/ snapshot + ordered actions, reconnect/replay, multi-window/remote/cloud session sharing; SDKs Rust/TS/Kotlin/Go/Swift; community OpenCode plugin.
- Codex App Server protocol (OpenAI): "JSON-RPC lite" over stdio/WebSocket/Remote Control; thread/turn/item primitives; server-initiated approval requests; Python SDK; `codex-exec-server` for remote sandboxed exec; used by VS Code ext, desktop, web, TUI; ACP via `codex-acp` adapter.
- A2A (Linux Foundation): v1.0 Apr 2026 — signed Agent Cards, gRPC binding (`a2a.proto` normative), multi-tenancy; 150+ orgs; relevance to coding harnesses is agent-to-agent, not editor↔agent.
- Agentic AI Foundation (AAIF) under Linux Foundation (Dec 2025; Anthropic/OpenAI/Block founding): governs MCP, goose, and AGENTS.md (60k+ repos; native in Codex, Cursor, Copilot, Gemini CLI, Windsurf/Devin, Zed, Factory, Jules, Devin, Amp, Warp, Kiro, Junie…).
- Agent Skills (Anthropic, open standard Dec 18, 2025, agentskills.io): `SKILL.md` folders; adopted by 32+ tools by Mar 2026 (Claude Code, Codex, VS Code/Copilot, Gemini CLI, Junie, Kiro, Goose, Amp, Cursor, Zed, Warp, Factory, Devin).
- Agent Plugins format (Kiro Powers, Cursor/Amp/Factory/Devin plugin marketplaces converging on bundles of skills+MCP+hooks+rules/subagents; Copilot Agent Plugins 1.0 GA 2026-08-12).
- Cross-tool config compat: subagent dirs `.cursor/agents`/`.claude/agents`/`.codex/agents` (Cursor); rules-file discovery lists (Zed, Warp); Amp `--stream-json` Claude-Code-compatible; Devin `migrate` from other tools; Warp/Zed auto-detect Claude/Codex MCP configs.
