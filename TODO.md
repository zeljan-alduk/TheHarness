# TODO — tool parity with Claude Code CLI

Gaps found by comparing Claude Code's tools reference (code.claude.com/docs/en/tools-reference) with
`Registry::defaults` in `src/tools/mod.rs`. Ordered by usefulness for a local coding harness.
Status: ☐ open · ◐ partial · ☑ done.

## P0 — most useful

- ☑ **`ask_user`** (Claude Code `AskUserQuestion`) — multiple-choice / free-text clarification prompt shown in the
  TUI (and desktop/web UIs) with an optional auto-continue timeout; headless runs answer "no user present".
- ☑ **`monitor`** (`Monitor`) — `monitor {start {cmd, filter?, timeout_secs?, max_lines?}|stop|list}`: background
  command whose (regex-filtered) output lines arrive as inbox events (coalesced ~1/s) plus an exit note.
- ☑ **`worktree`** (`EnterWorktree` / `ExitWorktree`) — `worktree {create|enter|exit|list|remove}` under
  `.harness/worktrees/<name>` (excluded via `.git/info/exclude`); `enter` switches every tool's working directory until
  `exit` (`ToolCtx::effective`, TUI mode line shows it); `spawn_agent {isolation:"worktree"}` gets a fresh worktree.
- ☑ **MCP resources** (`ListMcpResourcesTool` / `ReadMcpResourceTool`) — `mcp_resources {list|read|templates, server?, uri?}`
  (image blobs returned as images; servers registered globally in `mcp.rs`).

## P1

- ☑ **`notify`** (`PushNotification`) — `notify {message, title?, subtitle?, sound?}` via `osascript` / `notify-send` /
  PowerShell toast; `HARNESS_NO_NOTIFY` suppresses.
- ◐ **`schedule`** (`CronCreate/List/Delete`, `ScheduleWakeup`) — `schedule {add {prompt, delay_secs|at HH:MM, every_secs?}|list|remove|clear}`
  fires prompts into the inbox; session-scoped (not yet restored on resume).
- ◐ **`report_findings`** (`ReportFindings`) — `report_findings {findings:[{file,line?,severity,title,summary,...}], summary?}`
  validated, sorted, saved to `.harness/findings/<ts>.json` + `latest.json`; rendered as text (no dedicated UI panel yet).
- ☑ **Task graph** (`TaskCreate/Get/List/Update`) — `todo` items carry `blocked_by`, `owner`, `details`; `get`, blocked
  `start` refused unless `force`, `next` skips blocked; TUI marks ⏳/@owner. `process kill` covers `TaskStop`.
- ☑ **Model-driven plan mode** (`EnterPlanMode` / `ExitPlanMode`) — `plan_mode {enter|exit {plan}}`: sets the shared
  policy to Plan, presents the plan via ask_user-style question (approve / approve+ask / revise), restores the mode.

## P1 + P2 passes from docs/GAPS.md — DONE 2026-08-17

Everything in the P1 and P2 lists is implemented; see docs/GAPS.md §2c for the itemised status and the
short "deliberately not done" list (Bedrock/Vertex signing, i18n, cloud/enterprise surfaces). The only
open item from the original roadmap is runtime validation on Windows/Linux beyond what CI covers.

## P0 pass from docs/GAPS.md — DONE 2026-08-16

Instruction files (AGENTS.md/CLAUDE.md chain, @imports, path-scoped rules), skills + custom agents from
the standard directories, file checkpoints with /undo · /redo · /rewind · /fork, `harness acp` (Agent
Client Protocol server), the tool-call shim for models without function calling, permission rules with
argument matchers + built-in guards + the auto-mode classifier, and headless stream-json in/out with
`--json-schema`. See docs/GAPS.md §2b. Remaining work is the P1/P2 lists there.

## P2

- ◐ **Agent messaging** (`SendMessage` / `ListAgents`) — `agents {list|send {id,message}|kill|wait}` (steer via the
  sub-agent inbox); resuming a finished agent / teams not yet.
- ☑ **`Workflow`** as a tool — `run_workflow {list|run {name, args?}}`.
- ☐ **`powershell`** — native PowerShell tool on Windows (today: shell selection inside `bash`).
- ☐ **`WaitForMcpServers` / `ToolSearch`** — lazy MCP connection + deferred tool loading for very large tool sets.
- ☐ **`SendUserFile`** — hand a generated file to the user (open in Finder / attach in web UI).

## Not planned (cloud-only)

- `Artifact`, `RemoteTrigger`, `ShareOnboardingGuide`, `EndConversation`.

## Harness-only tools (no Claude Code equivalent)

`apply_patch`, `diagnostics`, `view_image`, `read_pdf`, `pdf_edit`, `extract_archive`, `download_file`, `memory`,
`list_dir`, `spawn_agent {read_only}`.

## Coordination — DONE (merged into main 2026-08-16): feature/parity-b + wt/todo-tools
The following are being implemented concurrently in a separate git worktree and will be merged into main;
skip them to avoid conflicts: hooks parity (SessionStart/End, SubagentStop, PreCompact, Notification, matchers),
permission scopes (project always-rules, /permissions add/remove, directory trust), session picker/fuzzy resume +
auto titles, markdown tables/lists rendering, vim mode + custom keybindings, cross-session messaging
(send_message/list_sessions), provider presets (/backend gemini|openai|openrouter), bwrap sandbox on Linux,
plugin auto-update, more eval tasks. Keep committing your own items normally; avoid touching src/tui.rs
rendering functions and src/permissions.rs beyond what your items need.
