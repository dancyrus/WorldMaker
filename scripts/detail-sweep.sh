#!/bin/zsh
# WO-0003 render-detail sweep (d3a §12, graft 8): octaves {3,4,5,6} x
# A0 {120,220,350} m x seeds {cyrus, feelpass} at High8, Detail 1.0.
# Each run captures the two judged crops (deterministic coast close-up +
# mountains) into <out-dir>/oct{o}-amp{a}-{seed}-{coast|mountains}.png and the
# script writes an index.md panel table beside them.
#
# Usage: scripts/detail-sweep.sh [out-dir]   (default target/detail-sweep)
#
# caffeinate -dimsu: scripted runs hang if the display sleeps (CLAUDE.md).
set -u
cd "$(dirname "$0")/.." || exit 1
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"

OUT_DIR="${1:-target/detail-sweep}"
mkdir -p "$OUT_DIR" || exit 1
STDERR_LOG="${TMPDIR:-/tmp}/worldmaker-sweep-stderr.log"

echo "Building WorldMaker (release)..."
cargo build --release -p worldmaker-app || exit 1

OCTAVES=(3 4 5 6)
AMPS=(120 220 350)
SEEDS=(cyrus feelpass)

for seed in "${SEEDS[@]}"; do
  for o in "${OCTAVES[@]}"; do
    for a in "${AMPS[@]}"; do
      run_dir="$OUT_DIR/raw-o${o}-a${a}-${seed}"
      echo "sweep: octaves $o, amp $a m, seed $seed"
      caffeinate -dimsu ./target/release/worldmaker-app \
        --screenshots "$run_dir" --seed "$seed" --preset high8 \
        --detail 1 --detail-octaves "$o" --detail-amp-m "$a" \
        2> "$STDERR_LOG"
      rc=$?
      # D4: fail loudly if the binary ignored any argument (old binaries
      # swallow new flags silently — that must never look like success).
      if grep -q "ignoring" "$STDERR_LOG"; then
        echo "SWEEP INVALID: the binary ignored one or more arguments:" >&2
        grep "ignoring" "$STDERR_LOG" >&2
        exit 2
      fi
      if [ $rc -ne 0 ]; then
        echo "SWEEP FAILED: run o$o a$a $seed exited with code $rc" >&2
        exit $rc
      fi
      for crop in coast mountains; do
        src="$run_dir/$crop.png"
        dst="$OUT_DIR/oct${o}-amp${a}-${seed}-${crop}.png"
        if [ ! -f "$src" ]; then
          echo "SWEEP FAILED: $src was not captured" >&2
          exit 3
        fi
        mv "$src" "$dst"
      done
      rmdir "$run_dir" 2>/dev/null
    done
  done
done

# Panel index: one table per seed, rows = octaves, cols = amplitudes.
INDEX="$OUT_DIR/index.md"
{
  echo "# Render-detail sweep panel (High8, Detail 1.0)"
  echo
  for seed in "${SEEDS[@]}"; do
    echo "## seed \"$seed\""
    echo
    echo "| octaves | A0 120 m | A0 220 m | A0 350 m |"
    echo "|---|---|---|---|"
    for o in "${OCTAVES[@]}"; do
      row="| $o |"
      for a in "${AMPS[@]}"; do
        row="$row coast ![c](oct${o}-amp${a}-${seed}-coast.png) mountains ![m](oct${o}-amp${a}-${seed}-mountains.png) |"
      done
      echo "$row"
    done
    echo
  done
} > "$INDEX"
echo "Sweep complete: $(ls "$OUT_DIR" | grep -c '\.png$') crops + index.md in $OUT_DIR"
