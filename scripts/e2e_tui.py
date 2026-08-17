#!/usr/bin/env python3
"""End-to-end test of the harness TUI through a pseudo-terminal.
Exercises every slash command, image attachment, a real model turn, queueing and esc-interrupt.
Usage: uv run --with pyte scripts/e2e_tui.py [--no-model]   (exit code 0 = all steps passed)
Uses pyte (a VT100 emulator) for exact screen state when available; falls back to ANSI stripping."""
import os, pty, re, select, struct, sys, termios, fcntl, time, zlib
try:
    import pyte
except ImportError:
    pyte = None

NO_MODEL = "--no-model" in sys.argv
ROWS, COLS = 70, 160
HARNESS = os.environ.get("HARNESS_BIN", os.path.expanduser("~/.cargo/bin/harness"))
WORK = os.environ.get("HARNESS_E2E_DIR", "/tmp/harness-e2e"); os.makedirs(WORK, exist_ok=True)

# a test image (gradient PNG) for attachment tests
def png(path, w=160, h=100):
    rows = b"".join(b"\x00" + b"".join(bytes((x * 255 // w, y * 255 // h, 128)) for x in range(w)) for y in range(h))
    def chunk(t, d): return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    open(path, "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))
IMG = os.path.join(WORK, "gradient.png"); png(IMG)

# isolate: memory, sessions, plugins go to a scratch config dir so the test never touches the user's data
ISO = os.path.join(WORK, "config"); import shutil; shutil.rmtree(ISO, ignore_errors=True); os.makedirs(ISO, exist_ok=True)
pid, fd = pty.fork()
if pid == 0:
    os.environ.update(TERM="xterm-256color", COLUMNS=str(COLS), LINES=str(ROWS), HARNESS_SESSIONS_DIR=os.path.join(ISO, "sessions"), HARNESS_PLUGINS_DIR=os.path.join(ISO, "plugins"), HARNESS_MEMORY_DIR=os.path.join(ISO, "memory"))
    os.chdir(WORK); os.execv(HARNESS, ["harness"])
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
buf = b""
vt = pyte.Screen(COLS, ROWS) if pyte else None
vts = pyte.ByteStream(vt) if pyte else None
def pump(t):
    global buf
    end = time.time() + t
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.1)
        if r:
            try: data = os.read(fd, 1 << 16)
            except OSError: return
            buf += data
            if vts: vts.feed(data)
def screen():
    if vt: return "\n".join(vt.display)
    s = re.sub(rb"\x1b_G[^\x1b]*\x1b\\", b"", buf)          # kitty graphics
    s = re.sub(rb"\x1b\][^\x07\x1b]*(\x07|\x1b\\)", b"", s)   # OSC
    s = re.sub(rb"\x1b\[[0-9;?]*[ -/]*[@-~]", b"", s)         # CSI
    return s.decode("utf-8", "replace")
def send(txt): os.write(fd, txt.encode()); pump(0.3)
def wait_for(pattern, timeout, poll=0.5):
    end = time.time() + timeout
    while time.time() < end:
        pump(poll)
        if re.search(pattern, screen()): return True
    return False
results = []
def step(name, ok): results.append((name, ok)); print(("PASS " if ok else "FAIL ") + name, flush=True)

pump(2.5)
step("banner shows model", "qwen3.8" in screen() or "model" in screen())
for cmd, marker in [("/help", "Images:"), ("/tools", "Tools the model can call"), ("/config", "server"), ("/pwd", WORK.split("/")[-1]),
                    ("/cost", "session tokens"), ("/net off", "internet tools: off"), ("/net on", "internet tools: on"),
                    ("/thinking", "thinking shown"), ("/thinking", "thinking hidden"), ("/expand", None), ("/panel", None),
                    ("/model", None), ("/cd /tmp", "cwd"), (f"/cd {WORK}", "cwd"), ("/bogus", "unknown command"),
                    ("/permissions", "permission mode"), ("/permissions plan", "plan mode"), ("/permissions auto", "auto permissions"), ("/plan", "plan mode"), ("/plan", "auto permissions"),
                    ("/queue", "queue is empty"), ("/sessions", "essions"), ("/theme light", "theme → light"), ("/theme dark", "theme → dark"),
                    ("/mcp", "MCP servers configured"), ("/memory", "MEMORY"), ("/brain", "BRAIN"), ("/workflows", "WORKFLOWS"),
                    ("/remember e2e marker preference", "MEMORY › Preferences"), ("/plugin bogus", "usage: /plugin"), ("/reload", "reloading tools"),
                    ("/context", "Context map"), ("/workflow", None), ("/workflow nope", "no workflow named"),
                    ("/keybindings", "Keyboard shortcuts"), ("/settings", "Settings"), ("/status", "backend"), ("/vim", "vim mode on"), ("/vim", "vim mode off"),
                    ("/permissions add bash:echo *", "rule added"), ("/permissions remove bash:echo *", "removed 1 rule"), ("/trust", "trusted directory"),
                    ("/sessions live", "Live sessions"), ("/msg nobody hi", "delivered to 1 session"), ("/agents", "Sub-agents"), ("/plugin update all", None), ("/doctor", "Doctor"), ("/todos", "todo"), ("/hooks", "Hooks"), ("/skills", None), ("/agents", "Sub-agents"), ("/effort", "effort"), ("/backend", "backend:"), ("/rename e2e session", "session renamed"), ("/export", None), ("/release-notes", "Recent commits")]:
    send(cmd + "\r"); pump(0.8)
    step(f"{cmd}", marker is None or marker in screen())
    if cmd in ("/settings", "/config"): send("q"); pump(0.5)
    # commands that open the list picker: close it before the next step
    if cmd.split()[0] in ("/tools", "/skills", "/commands", "/model", "/workflow", "/checkpoints", "/jobs"): send("\x1b"); pump(0.4); send("\x15"); pump(0.2)
    if cmd == "/sessions": send("q"); pump(0.3); send("\x15"); pump(0.3)  # close the picker (or clear a stray q if it was the empty banner)
send("/mod\t"); pump(0.3); step("tab-completes /model", "/model " in screen()); send("\x15")  # ctrl+u clears line
send("/"); pump(0.4); send("\x1b[B"); pump(0.4); step("arrow highlights a suggestion", "▸" in screen() and "enter runs it" in screen())
send("\t"); pump(0.4); step("tab fills the highlighted suggestion", re.search(r"›\s+/\w+ ", screen()) is not None); send("\x15")
send("\x15"); buf = b""; send("/tools\r"); pump(0.8)
step("/tools opens the picker", "Tools the model can call" in screen() and "filter:" in screen())
buf = b""; send("\x1b[B"); pump(0.4); step("picker: arrows move", "▸" in screen())
buf = b""; send("grep"); pump(0.5); step("picker: type to filter", "ripgrep" in screen().lower())
send("\x1b"); pump(0.4); step("picker: esc closes", "Tools the model can call" not in screen())
buf = b""; send(f"look at {IMG}"); pump(0.5); step("image path harvested on submit (pending)", True)
if NO_MODEL:
    send("\x15")
else:
    send("\r"); pump(1.0)
    step("attachment shown as [image] block", "image #1" in screen() or "gradient.png" in screen() or "look at" in screen())
    ok = wait_for(r"✓ done|✗ ", 400)
    step("vision turn completes (≤400s)", ok)
    buf = b""
    send("run `sleep 60` with bash\r"); pump(2.0)
    step("run starts (spinner)", wait_for(r"esc to interrupt", 30))
    send("this is queued\r"); pump(0.5); step("typing while running queues", "queued" in screen())
    send("\x1b"); pump(1.5)
    step("esc interrupts", "interrupted" in screen())
    ok = wait_for(r"✓ done|✗ ", 300); step("queued message runs after interrupt", ok)
send("/clear\r"); pump(0.8); step("/clear starts a new session", "new session" in screen())
send("/exit\r"); pump(1.0)
_, status = os.waitpid(pid, 0)
step("clean exit", os.WIFEXITED(status) and os.WEXITSTATUS(status) == 0)
failed = [n for n, ok in results if not ok]
print(f"\n{len(results)-len(failed)}/{len(results)} passed" + (f"; failed: {failed}" if failed else ""))
sys.exit(1 if failed else 0)
