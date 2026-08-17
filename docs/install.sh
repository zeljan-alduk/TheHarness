#!/bin/sh
# TheHarness installer — https://github.com/zeljan-alduk/TheHarness
#
#   curl -fsSL https://zeljan-alduk.github.io/TheHarness/install.sh | sh
#
# macOS on Apple Silicon. Everything the harness owns lives under ~/.config/harness, so
# uninstalling is `rm -rf ~/.config/harness ~/Applications/TheHarness.app ~/.local/bin/harness`.
#
#   harness    prebuilt binary from the latest GitHub release (falls back to `cargo install --git`)
#   kitty      the terminal the TUI is built for — inline images, font control. Vendor installer, no sudo
#   MLX        ~/.config/harness/runtime/mlx: a private venv with mlx-lm + mlx-vlm, the fastest way
#              to run Qwen3.8-27B on an M-series chip
#   claude     the Claude Code CLI: the harness works on Claude while the local model downloads
#   TheHarness.app in ~/Applications, aliased on the Desktop, opens the harness in kitty
#
# The model itself is NOT downloaded here — the harness does that on first run, so it can show
# progress, speed and ETA in the side panel and resume a partial download.
#
# Nothing needs sudo; nothing is written outside $HOME. Knobs:
#   DRY_RUN=1     print every step instead of doing it
#   NO_KITTY=1  NO_MLX=1  NO_CLAUDE=1  NO_APP=1      skip that step
#   WITH_OLLAMA=1                                    also install Ollama (GGUF models, ~150MB)
#   PREFIX=~/.local                                  where bin/ and kitty.app go
set -eu

REPO=zeljan-alduk/TheHarness
PREFIX="${PREFIX:-$HOME/.local}"
BIN="$PREFIX/bin"
APPS="$HOME/Applications"
HOME_DIR="${HARNESS_HOME:-$HOME/.config/harness}"
RUNTIME="$HOME_DIR/runtime"
DRY_RUN="${DRY_RUN:-0}"

if [ -t 1 ]; then B=$(printf '\033[1m'); D=$(printf '\033[2m'); G=$(printf '\033[32m'); Y=$(printf '\033[33m'); R=$(printf '\033[31m'); N=$(printf '\033[0m'); else B=; D=; G=; Y=; R=; N=; fi
say()  { printf '%s\n' "${B}▸${N} $*"; }
ok()   { printf '%s\n' "  ${G}✓${N} $*"; }
skip() { printf '%s\n' "  ${D}·${N} $*"; }
warn() { printf '%s\n' "  ${Y}!${N} $*"; }
die()  { printf '%s\n' "  ${R}✗${N} $*" >&2; exit 1; }
run()  { if [ "$DRY_RUN" = 1 ]; then printf '%s\n' "  ${D}would run:${N} $*"; else "$@"; fi; }
runsh(){ if [ "$DRY_RUN" = 1 ]; then printf '%s\n' "  ${D}would run:${N} $*"; else sh -c "$*"; fi; }

# ── preflight ─────────────────────────────────────────────────────────────────
say "Checking this machine"
[ "$(uname -s)" = Darwin ] || die "TheHarness targets macOS on Apple Silicon. (The tree still carries Linux/Windows code paths — open an issue if you want them built.)"
[ "$(uname -m)" = arm64 ]  || die "this needs an M-series Mac: MLX is Apple-Silicon-only. On Intel, run the harness against any OpenAI-compatible server instead."
command -v curl >/dev/null || die "curl is required"
TARGET=aarch64-apple-darwin
FREE_GB=$(df -g "$HOME" | awk 'NR==2 {print $4}')
ok "macOS $(sw_vers -productVersion) · Apple Silicon · ${FREE_GB}GB free in \$HOME"
[ "${FREE_GB:-0}" -ge 25 ] || warn "the local model needs 17–30GB depending on the quant you pick — only ${FREE_GB}GB free"
run mkdir -p "$BIN" "$APPS" "$HOME_DIR" "$RUNTIME"

# ── harness ───────────────────────────────────────────────────────────────────
say "Installing the harness"
ASSET="harness-$TARGET.tar.gz"
URL=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null \
      | awk -v a="$ASSET" -F'"' '$2=="browser_download_url" && $4 ~ a {print $4; exit}') || URL=""
if [ -n "${URL:-}" ]; then
    TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT INT TERM
    run curl -fSL --progress-bar "$URL" -o "$TMP/$ASSET"
    run tar -xzf "$TMP/$ASSET" -C "$TMP"
    run rm -f "$BIN/harness"   # rm before cp: macOS SIGKILLs a binary overwritten in place
    run cp "$TMP/harness" "$BIN/harness"
    run chmod +x "$BIN/harness"
    ok "harness → $BIN/harness (prebuilt)"
elif command -v cargo >/dev/null; then
    warn "no published release yet — building from source (a few minutes)"
    run cargo install --git "https://github.com/$REPO" --root "$PREFIX" harness
    ok "harness → $BIN/harness (built from source)"
else
    die "no release asset and no cargo. Install Rust (https://rustup.rs) and re-run."
fi

# ── kitty ─────────────────────────────────────────────────────────────────────
say "Installing kitty"
KITTY=""
for k in "$(command -v kitty 2>/dev/null || true)" "$HOME/.local/kitty.app/Contents/MacOS/kitty" /Applications/kitty.app/Contents/MacOS/kitty; do
    [ -n "$k" ] && [ -x "$k" ] && { KITTY=$k; break; }
done
if [ -n "$KITTY" ]; then skip "already installed ($KITTY)"
elif [ "${NO_KITTY:-0}" = 1 ]; then skip "skipped (NO_KITTY=1)"
else
    runsh 'curl -fsSL https://sw.kovidgoyal.net/kitty/installer.sh | sh /dev/stdin launch=n'
    KITTY="$HOME/.local/kitty.app/Contents/MacOS/kitty"
    [ "$DRY_RUN" = 1 ] || [ -x "$KITTY" ] || die "kitty install failed"
    run ln -sf "$KITTY" "$BIN/kitty"
    run ln -sf "$HOME/.local/kitty.app/Contents/MacOS/kitten" "$BIN/kitten"
    ok "kitty → $KITTY"
fi
# The TUI changes the font size (ctrl+= / ctrl+-) over kitty's remote control; off by default in kitty.
KCONF="$HOME/.config/kitty/kitty.conf"
if [ "${NO_KITTY:-0}" != 1 ] && ! grep -qs '^allow_remote_control' "$KCONF" 2>/dev/null; then
    run mkdir -p "$(dirname "$KCONF")"
    runsh "printf '\n# added by TheHarness: lets the TUI change the font size (ctrl+= / ctrl+-)\nallow_remote_control yes\n' >> '$KCONF'"
    ok "allow_remote_control yes → $KCONF"
fi

# ── MLX runtime, inside the harness directory ──────────────────────────────────
say "Installing the MLX runtime"
if [ "${NO_MLX:-0}" = 1 ]; then skip "skipped (NO_MLX=1)"
else
    if command -v uv >/dev/null; then UV=$(command -v uv); skip "uv already installed"
    else
        runsh 'curl -fsSL https://astral.sh/uv/install.sh | sh'
        UV="$HOME/.local/bin/uv"
        ok "uv installed"
    fi
    # A venv the harness owns, so the runtime is versioned with the harness and removed with it.
    run "$UV" venv --python 3.12 "$RUNTIME/mlx"
    # mlx-lm serves the text path (it knows the qwen3_5 architecture); mlx-vlm adds the vision tower.
    run "$UV" pip install --python "$RUNTIME/mlx/bin/python" --quiet --upgrade mlx-lm mlx-vlm
    ok "mlx-lm + mlx-vlm → $RUNTIME/mlx"
fi

# ── ollama (optional second runtime) ──────────────────────────────────────────
if [ "${WITH_OLLAMA:-0}" = 1 ]; then
    say "Installing Ollama (optional)"
    if command -v ollama >/dev/null; then skip "already installed"
    else
        TMP2=$(mktemp -d)
        run curl -fSL --progress-bar https://ollama.com/download/ollama-darwin.tgz -o "$TMP2/ollama.tgz"
        run mkdir -p "$RUNTIME/ollama"
        run tar -xzf "$TMP2/ollama.tgz" -C "$RUNTIME/ollama"
        run rm -rf "$TMP2"
        [ "$DRY_RUN" = 1 ] || { O=$(find "$RUNTIME/ollama" -type f -name ollama -perm -u+x | head -1); [ -n "$O" ] && ln -sf "$O" "$BIN/ollama"; }
        ok "ollama → $RUNTIME/ollama (models would live in $HOME_DIR/models)"
    fi
fi

# ── claude code cli ───────────────────────────────────────────────────────────
say "Installing the Claude Code CLI"
if command -v claude >/dev/null; then skip "already installed ($(command -v claude))"
elif [ "${NO_CLAUDE:-0}" = 1 ]; then skip "skipped (NO_CLAUDE=1)"
else
    runsh 'curl -fsSL https://claude.ai/install.sh | bash'
    ok "claude installed — it asks you to log in the first time it runs"
fi

# ── config ────────────────────────────────────────────────────────────────────
say "Writing the default config"
if [ -f "$HOME_DIR/harness.toml" ]; then skip "keeping your $HOME_DIR/harness.toml"
else
    runsh "curl -fsSL 'https://raw.githubusercontent.com/$REPO/main/harness.toml' -o '$HOME_DIR/harness.toml'"
    ok "harness.toml → $HOME_DIR/harness.toml"
fi

# ── TheHarness.app + Desktop alias ────────────────────────────────────────────
say "Creating TheHarness.app"
if [ "${NO_APP:-0}" = 1 ]; then skip "skipped (NO_APP=1)"
else
    APP="$APPS/TheHarness.app"
    run mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
    if [ "$DRY_RUN" = 1 ]; then
        printf '%s\n' "  ${D}would write:${N} $APP/Contents/{Info.plist,MacOS/TheHarness,Resources/TheHarness.icns}"
    else
        cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>TheHarness</string>
  <key>CFBundleDisplayName</key><string>TheHarness</string>
  <key>CFBundleIdentifier</key><string>tech.aldo.theharness.launcher</string>
  <key>CFBundleVersion</key><string>1.0</string>
  <key>CFBundleShortVersionString</key><string>1.0</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleExecutable</key><string>TheHarness</string>
  <key>CFBundleIconFile</key><string>TheHarness</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
        cat > "$APP/Contents/MacOS/TheHarness" <<'LAUNCH'
#!/bin/sh
# Open the harness in kitty. This bundle exists only to give the Dock and Desktop something to click.
for k in "$HOME/.local/kitty.app/Contents/MacOS/kitty" /Applications/kitty.app/Contents/MacOS/kitty "$(command -v kitty 2>/dev/null || true)"; do
    [ -n "$k" ] && [ -x "$k" ] && { KITTY=$k; break; }
done
for h in "$HOME/.local/bin/harness" "$HOME/.cargo/bin/harness" "$(command -v harness 2>/dev/null || true)"; do
    [ -n "$h" ] && [ -x "$h" ] && { HARNESS=$h; break; }
done
[ -n "${KITTY:-}" ] && [ -n "${HARNESS:-}" ] || { osascript -e 'display alert "TheHarness" message "kitty or harness is missing — re-run the installer."'; exit 1; }
cd "${HARNESS_WORKDIR:-$HOME}" || exit 1
exec "$KITTY" -o allow_remote_control=yes --start-as=maximized -T TheHarness "$HARNESS"
LAUNCH
        chmod +x "$APP/Contents/MacOS/TheHarness"
        curl -fsSL "https://raw.githubusercontent.com/$REPO/main/docs/TheHarness.icns" -o "$APP/Contents/Resources/TheHarness.icns" 2>/dev/null \
            || warn "could not fetch the icon; the app gets the generic one"
        touch "$APP"   # nudge Finder into re-reading the bundle
    fi
    ok "TheHarness.app → $APP"
    run ln -sfn "$APP" "$HOME/Desktop/TheHarness.app"
    ok "Desktop alias → ~/Desktop/TheHarness.app"
fi

# ── PATH ──────────────────────────────────────────────────────────────────────
case ":$PATH:" in
  *":$BIN:"*) ;;
  *) say "Adding $BIN to PATH"
     for rc in "$HOME/.zshrc" "$HOME/.bashrc"; do
        [ -f "$rc" ] || continue
        grep -qs "$BIN" "$rc" && continue
        runsh "printf '\n# added by TheHarness installer\nexport PATH=\"%s:\$PATH\"\n' '$BIN' >> '$rc'"
        ok "$rc"
     done ;;
esac

# ── done ──────────────────────────────────────────────────────────────────────
printf '\n%s\n' "${B}TheHarness is installed.${N}"
cat <<EOF

  ${B}harness${N}                    start it in any terminal — it re-opens itself in kitty
  double-click ${B}TheHarness${N}    same thing, from the Desktop

The first run has no local model yet, so it offers the ${B}Qwen3.8-27B${N} MLX build in
${B}4-bit (16GB) · 6-bit (23GB) · 8-bit (30GB)${N} and downloads the one you pick in segments that
resume if interrupted — progress, speed and ETA in the side panel (⌃P). Claude keeps you working
meanwhile; when the weights land, the harness offers to switch, or to keep Claude as the
orchestrator that delegates to the local model.

  ${D}HARNESS_NO_KITTY=1 harness${N}  stay in the terminal you are in
  ${D}harness --help${N}              everything else
EOF
[ "$DRY_RUN" = 1 ] && printf '\n%s\n' "${Y}(dry run — nothing was changed)${N}"
exit 0
