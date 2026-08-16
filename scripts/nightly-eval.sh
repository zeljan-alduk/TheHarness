#!/bin/sh
# Nightly: run the full eval suite and judge every proposal/* branch; merge green ones.
# Schedule with launchd/cron, e.g. crontab: 0 3 * * * /path/to/TheHarness/scripts/nightly-eval.sh
set -e
cd "$(dirname "$0")/.."
H=${HARNESS_BIN:-$HOME/.cargo/bin/harness}
DAY=$(date +%F); OUT=${HARNESS_NIGHTLY_DIR:-$HOME/.config/harness/nightly}; mkdir -p "$OUT"
$H eval --out "$OUT/eval-$DAY.json" > "$OUT/eval-$DAY.log" 2>&1 || true
tail -2 "$OUT/eval-$DAY.log"
for b in $(git branch --list 'proposal/*' | sed 's/^[* ]*//'); do
  echo "== arbiter $b"; $H arbiter "$b" --baseline "$OUT/eval-$DAY.json" --merge > "$OUT/arbiter-$DAY-$(echo $b | tr / _).log" 2>&1 || true
  tail -3 "$OUT/arbiter-$DAY-$(echo $b | tr / _).log"
done
