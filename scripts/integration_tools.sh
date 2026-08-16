#!/bin/sh
# Integration tests for tools that need no model: run each via `harness tool` and check the output.
# Usage: scripts/integration_tools.sh   (exit 0 = all pass)
H=${HARNESS_BIN:-$HOME/.cargo/bin/harness}
D=${HARNESS_ITEST_DIR:-/tmp/harness-itest}; rm -rf "$D"; mkdir -p "$D"; cd "$D" && git init -q -b main && git -c user.name=t -c user.email=t@t commit -q --allow-empty -m init
pass=0; fail=0
t() { name=$1; shift; out=$("$H" -y tool -C "$D" "$@" 2>&1); if echo "$out" | grep -qE "$EXPECT"; then echo "PASS $name"; pass=$((pass+1)); else echo "FAIL $name"; echo "$out" | head -5 | sed 's/^/     /'; fail=$((fail+1)); fi; }
EXPECT="ok"                       t "bash"            bash '{"cmd":"echo ok"}'
EXPECT="created .*a.txt"           t "write_file"      write_file '{"path":"a.txt","content":"hello\nworld\n"}'
EXPECT="^\s*1\thello"              t "read_file"       read_file '{"path":"a.txt"}'
EXPECT="edited .*a.txt"            t "edit_file"       edit_file '{"path":"a.txt","old":"world","new":"there"}'
EXPECT="a.txt:2:there"             t "grep"            grep '{"pattern":"there"}'
EXPECT="a.txt"                     t "glob"            glob '{"pattern":"*.txt"}'
EXPECT="patch applied"             t "apply_patch"     apply_patch '{"patch":"--- a/a.txt\n+++ b/a.txt\n@@ -1,2 +1,2 @@\n hello\n-there\n+again\n"}'
EXPECT="a.txt"                     t "list_dir"        list_dir '{}'
EXPECT="python: ok|no known"       t "diagnostics"     diagnostics '{}'
EXPECT="started background process #1" t "bash background" bash '{"cmd":"sleep 5; echo bg","background":true}'
EXPECT="worktree ready|worktree|created" t "worktree enter" worktree '{"action":"enter","name":"itest"}'
EXPECT="itest"                     t "worktree list"   worktree '{"action":"list"}'
EXPECT="removed|exit|itest"        t "worktree exit"   worktree '{"action":"exit","name":"itest"}'
EXPECT="todo|#1|☐"                 t "todo set"        todo '{"action":"set","items":["one","two"]}'
EXPECT="no user|decide yourself"   t "ask_user headless" ask_user '{"question":"which?","options":[{"label":"a"},{"label":"b"}]}'
EXPECT="added to MEMORY|already"   t "memory append"   memory '{"action":"append","file":"MEMORY","section":"Ideas","text":"itest marker"}'
EXPECT="itest marker"              t "memory show"     memory '{"action":"show","file":"MEMORY"}'
EXPECT="removed 1"                 t "memory remove"   memory '{"action":"remove","file":"MEMORY","text":"itest marker"}'
EXPECT="no live sessions|●|\\d"     t "list_sessions"   list_sessions '{}'
EXPECT="delivered to 1"            t "send_message"    send_message '{"to":"nobody-itest","text":"ping"}'
EXPECT="monitor #.* started|started" t "monitor start"  monitor '{"action":"start","cmd":"echo line1; sleep 1; echo ERROR boom","pattern":"ERROR"}'
EXPECT="scheduled|#"               t "schedule add"    schedule '{"action":"add","in_secs":1,"prompt":"itest wake"}'
EXPECT="notification|sent|ok|not available|no notifier" t "notify" notify '{"title":"harness itest","message":"hello"}'
EXPECT="finding|saved|report|1"    t "report_findings" report_findings '{"findings":[{"file":"a.txt","line":1,"summary":"itest finding","severity":"low"}]}'
EXPECT="no MCP|servers|resource"   t "mcp_resources"   mcp_resources '{"action":"list"}'
EXPECT="plan|Plan|read-only|mode"  t "plan_mode"       plan_mode '{"action":"status"}'
EXPECT="agent|none|no sub-agents|Sub"  t "agents"      agents '{"action":"list"}'
EXPECT="workflow|Workflows|review|fix-tests|no workflow" t "run_workflow list" run_workflow '{"action":"list"}'
EXPECT="path escapes|error"        t "path jail"       write_file '{"path":"/etc/harness-itest","content":"x"}'

# file checkpoints (shadow git): a session-scoped tool call snapshots the tree; undo/redo restore it
c() { name=$1; shift; out=$("$H" checkpoint -C "$D" --session itest-cp "$@" 2>&1); if echo "$out" | grep -qE "$EXPECT"; then echo "PASS $name"; pass=$((pass+1)); else echo "FAIL $name"; echo "$out" | head -5 | sed 's/^/     /'; fail=$((fail+1)); fi; }
"$H" -y tool -C "$D" --session itest-cp write_file '{"path":"cp.txt","content":"one\n"}' >/dev/null 2>&1
"$H" -y tool -C "$D" --session itest-cp write_file '{"path":"cp.txt","content":"two\n"}' >/dev/null 2>&1
EXPECT="checkpoint" c "checkpoint list"  list
EXPECT="restored"   c "checkpoint undo"  undo
EXPECT="^one$"; if grep -qE "$EXPECT" "$D/cp.txt"; then echo "PASS checkpoint undo restored the file"; pass=$((pass+1)); else echo "FAIL checkpoint undo restored the file"; fail=$((fail+1)); fi
EXPECT="restored"   c "checkpoint redo"  redo
EXPECT="^two$"; if grep -qE "$EXPECT" "$D/cp.txt"; then echo "PASS checkpoint redo re-applied the change"; pass=$((pass+1)); else echo "FAIL checkpoint redo re-applied the change"; fail=$((fail+1)); fi

# project instruction files + path-scoped rules reach the model through tool results
mkdir -p "$D/.harness/rules"; printf -- '---\npaths: *.txt\ndescription: itest rule\n---\nITEST-RULE-BODY\n' > "$D/.harness/rules/itest.md"
EXPECT="ITEST-RULE-BODY"           t "path-scoped rule"  read_file '{"path":"cp.txt"}'
mkdir -p "$D/.harness/skills/itest-skill"; printf -- '---\nname: itest-skill\ndescription: itest\n---\nSKILL-BODY-OK\n' > "$D/.harness/skills/itest-skill/SKILL.md"
EXPECT="SKILL-BODY-OK"             t "load_skill"        load_skill '{"name":"itest-skill"}'

echo; echo "$pass passed, $fail failed"; [ "$fail" -eq 0 ]
