#!/bin/zsh
# WO-0003 feel-pass perf run (macOS): loops Standard7 -> High8 -> Ultra9 with
# smooth shading + render detail on, and writes
# docs/results/perf-feelpass-<machine>.json.
#
# caffeinate -dimsu: scripted runs hang at their first stage if the display
# sleeps (CLAUDE.md lesson) — keep display/system awake for the whole run.
set -u
cd "$(dirname "$0")/.." || exit 1
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"

MACHINE=$(hostname -s)
OUT="docs/results/perf-feelpass-${MACHINE}.json"
STDERR_LOG="${TMPDIR:-/tmp}/worldmaker-perf-stderr.log"

echo "Building WorldMaker (release)..."
cargo build --release -p worldmaker-app || exit 1

echo "Perf run (L7 -> L8 -> L9, each with a full 500 My world build)..."
caffeinate -dimsu ./target/release/worldmaker-app --perf-out "$OUT" "$@" \
  2> "$STDERR_LOG"
STATUS=$?

# D4: fail loudly if the binary warned about any ignored argument — an old
# binary silently swallowing new flags must never produce a "successful" run.
if grep -q "ignoring" "$STDERR_LOG"; then
  echo "PERF RUN INVALID: the binary ignored one or more arguments:" >&2
  grep "ignoring" "$STDERR_LOG" >&2
  exit 2
fi
if [ $STATUS -ne 0 ]; then
  echo "PERF RUN FAILED: worldmaker-app exited with status $STATUS" >&2
  exit $STATUS
fi
if [ ! -f "$OUT" ]; then
  echo "PERF RUN FAILED: $OUT was not written" >&2
  exit 3
fi
echo "Perf results written to $OUT"
