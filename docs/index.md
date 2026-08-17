---
title: TheHarness
---

# TheHarness

A local-first, self-improving agentic coding harness for **macOS on Apple Silicon**. Rust core,
**Qwen3.8-27B on MLX**, with Claude as a second backend — or as an orchestrator that delegates to
the local model.

## Install

```sh
curl -fsSL https://zeljan-alduk.github.io/TheHarness/install.sh | sh
```

No sudo, nothing outside `$HOME`. It installs:

| | |
|---|---|
| `harness` | the binary, from the latest release (or built from source if there is none) |
| **kitty** | the terminal the TUI is built for — inline images, graphics protocol, font control |
| **MLX** | a private `mlx-lm` + `mlx-vlm` venv in `~/.config/harness/runtime` |
| **claude** | the Claude Code CLI, so you can work while the local model downloads |
| **TheHarness.app** | in `~/Applications`, with an alias on your Desktop |

Prefer to read before you run? It is [install.sh](install.sh) — and `DRY_RUN=1 sh install.sh` prints
every step instead of taking it. `NO_KITTY=1`, `NO_MLX=1`, `NO_CLAUDE=1`, `NO_APP=1` skip a piece;


## First run

The installer does **not** download the model — the harness does, so it can show you what is
happening. It offers Qwen3.8-27B as **4-bit (16 GB)**, **6-bit (23 GB)** or **8-bit (30 GB)**, pulls
the weights in parallel segments that resume if interrupted, and reports progress, speed and ETA in
the side panel (⌃P). Claude keeps you working meanwhile; when the weights land, the harness offers to
switch to the local model.

`harness` in any terminal re-opens itself in kitty. `HARNESS_NO_KITTY=1 harness` stays where it is.

## Uninstall

```sh
rm -rf ~/.config/harness ~/Applications/TheHarness.app ~/Desktop/TheHarness.app ~/.local/bin/harness
```

---

[Source on GitHub](https://github.com/zeljan-alduk/TheHarness) · MIT licensed
