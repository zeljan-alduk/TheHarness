# TODO — tool parity with Claude Code CLI

Gaps found by comparing Claude Code's tools reference (code.claude.com/docs/en/tools-reference) with
`Registry::defaults` in `src/tools/mod.rs`. Ordered by usefulness for a local coding harness.
Status: ☐ open · ◐ partial · ☑ done.

## P0 — most useful

- ☑ **`ask_user`** (Claude Code `AskUserQuestion`) — multiple-choice / free-text clarification prompt shown in the
  TUI (and desktop/web UIs) with an optional auto-continue timeout; headless runs answer "no user present".
- ☐ **`monitor`** (`Monitor`) — run a command in the background and stream each output line (or matching lines)
  back into the conversation so the model can react to logs / polled status. Today `process tail` is polling only.
- ☑ **`worktree`** (`EnterWorktree` / `ExitWorktree`) — `worktree {create|enter|exit|list|remove}` under
  `.harness/worktrees/<name>` (excluded via `.git/info/exclude`); `enter` switches every tool's working directory until
  `exit` (`ToolCtx::effective`, TUI mode line shows it); `spawn_agent {isolation:"worktree"}` gets a fresh worktree.
- ☐ **MCP resources** (`ListMcpResourcesTool` / `ReadMcpResourceTool`) — the harness bridges MCP *tools* only;
  expose `mcp_resources {list|read, server, uri}`.

## P1

- ☐ **`notify`** (`PushNotification`) — desktop notification when a long task finishes / needs input
  (`osascript` / `notify-send` / PowerShell toast).
- ☐ **`schedule`** (`CronCreate/List/Delete`, `ScheduleWakeup`) — session-scoped one-shot / recurring prompts
  ("in 10 min re-run the tests and report"), restored on resume.
- ☐ **`report_findings`** (`ReportFindings`) — structured code-review findings (file, line, summary, severity,
  failure scenario) rendered as a list in the UI and saved as JSON.
- ☐ **Task graph** (`TaskCreate/Get/List/Update`) — `todo` items with dependencies (`blocked_by`), owner and
  details; `process kill` already covers `TaskStop`.
- ☐ **Model-driven plan mode** (`EnterPlanMode` / `ExitPlanMode`) — the model may enter plan mode and present a
  plan for approval; today `/plan` is user-toggled only.

## P2

- ☐ **Agent messaging** (`SendMessage` / `ListAgents`) — talk to a running sub-agent / resume it with a follow-up
  message; agent teams.
- ☐ **`Workflow`** as a tool — run a `harness workflow` TOML from inside a session.
- ☐ **`powershell`** — native PowerShell tool on Windows (today: shell selection inside `bash`).
- ☐ **`WaitForMcpServers` / `ToolSearch`** — lazy MCP connection + deferred tool loading for very large tool sets.
- ☐ **`SendUserFile`** — hand a generated file to the user (open in Finder / attach in web UI).

## Not planned (cloud-only)

- `Artifact`, `RemoteTrigger`, `ShareOnboardingGuide`, `EndConversation`.

## Harness-only tools (no Claude Code equivalent)

`apply_patch`, `diagnostics`, `view_image`, `read_pdf`, `pdf_edit`, `extract_archive`, `download_file`, `memory`,
`list_dir`, `spawn_agent {read_only}`.

## Coordination — handled by the *other* Claude session on branch `feature/parity-b` (do NOT implement here)
The following are being implemented concurrently in a separate git worktree and will be merged into main;
skip them to avoid conflicts: hooks parity (SessionStart/End, SubagentStop, PreCompact, Notification, matchers),
permission scopes (project always-rules, /permissions add/remove, directory trust), session picker/fuzzy resume +
auto titles, markdown tables/lists rendering, vim mode + custom keybindings, cross-session messaging
(send_message/list_sessions), provider presets (/backend gemini|openai|openrouter), bwrap sandbox on Linux,
plugin auto-update, more eval tasks. Keep committing your own items normally; avoid touching src/tui.rs
rendering functions and src/permissions.rs beyond what your items need.
