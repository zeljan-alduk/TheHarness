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
