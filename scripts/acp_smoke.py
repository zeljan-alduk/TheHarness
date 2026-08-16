#!/usr/bin/env python3
"""Minimal ACP client used to smoke-test `harness acp` end to end.

Usage: scripts/acp_smoke.py <harness-bin> <harness.toml> <workdir> "<prompt>" [extra harness args…]
Exits 0 when the prompt finishes with stopReason end_turn. Needs a reachable model server.
"""
import json, subprocess, sys, threading, os

HARNESS, CFG, CWD, PROMPT = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
ARGS = sys.argv[5:]
p = subprocess.Popen([HARNESS, "--config", CFG] + ARGS + ["acp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                     stderr=subprocess.PIPE, text=True, bufsize=1)
def send(msg): p.stdin.write(json.dumps(msg) + "\n"); p.stdin.flush()

state = {"session": None, "done": False, "stop": None, "updates": [], "perm": 0}
def reader():
    for line in p.stdout:
        line = line.strip()
        if not line: continue
        try: m = json.loads(line)
        except Exception: print("NONJSON", line); continue
        if m.get("method") == "session/update":
            u = m["params"]["update"]; state["updates"].append(u["sessionUpdate"])
            kind = u["sessionUpdate"]
            if kind == "tool_call": print(f"  ▶ tool_call {u['title']} [{u['kind']}]")
            elif kind == "tool_call_update": print(f"  ◀ {u['status']}")
            elif kind == "agent_message_chunk": print(u["content"]["text"], end="")
        elif m.get("method") == "session/request_permission":
            state["perm"] += 1
            print(f"\n  🔒 permission asked: {m['params']['toolCall']['title']} → allowing")
            send({"jsonrpc":"2.0","id":m["id"],"result":{"outcome":{"outcome":"selected","optionId":"allow"}}})
        elif "result" in m:
            r = m["result"]
            if m.get("id") == 1: print("initialize:", json.dumps(r)[:160])
            elif m.get("id") == 2: state["session"] = r["sessionId"]; print("session:", r["sessionId"], "modes:", r.get("modes",{}).get("currentModeId"))
            elif m.get("id") == 3: state["stop"] = r.get("stopReason"); state["done"] = True
threading.Thread(target=reader, daemon=True).start()

send({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{"fs":{"readTextFile":True,"writeTextFile":True}}}})
import time
time.sleep(0.5)
send({"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":CWD,"mcpServers":[]}})
for _ in range(100):
    if state["session"]: break
    time.sleep(0.2)
send({"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{"sessionId":state["session"],"prompt":[{"type":"text","text":PROMPT}]}})
deadline = time.time() + 420
while not state["done"] and time.time() < deadline: time.sleep(0.3)
print("\nstopReason:", state["stop"], "| updates:", {k: state["updates"].count(k) for k in set(state["updates"])}, "| permission prompts:", state["perm"])
p.kill()
sys.exit(0 if state["stop"] == "end_turn" else 1)
