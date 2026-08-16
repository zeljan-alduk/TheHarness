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
- ☐ **`schedule`** (`CronCreate/List/Delete`, `ScheduleWakeup`) — session-scoped one-shot / recurring prompts
  ("in 10 min re-run the tests and report"), restored on resume.
- ◐ **`report_findings`** (`ReportFindings`) — `report_findings {findings:[{file,line?,severity,title,summary,...}], summary?}`
  validated, sorted, saved to `.harness/findings/<ts>.json` + `latest.json`; rendered as text (no dedicated UI panel yet).
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
