#!/bin/zsh
# WorldMaker launcher (macOS): builds the app in release mode and runs it.
# First build takes a few minutes; later launches are quick.
cd "$(dirname "$0")" || exit 1

# Finder launches .command files in a fresh Terminal; make sure the Rust and
# Homebrew tool locations are on PATH even if the shell profile doesn't add them.
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"

if ! command -v cargo >/dev/null 2>&1; then
  echo "Rust is not installed or not on PATH."
  echo "Install it from https://rustup.rs and run this file again."
  read -r "?Press Enter, then close this window."
  exit 1
fi

echo "Building WorldMaker (release)..."
if ! cargo build --release -p worldmaker-app; then
  echo ""
  echo "Build failed. The full error is above."
  read -r "?Press Enter, then close this window."
  exit 1
fi

# A remapped cargo target dir (CARGO_TARGET_DIR / build.target-dir) would put
# the binary elsewhere and the launch below would silently do nothing.
if [ ! -x ./target/release/worldmaker-app ]; then
  echo "Build succeeded but ./target/release/worldmaker-app is missing."
  echo "(Is a custom cargo target directory configured?)"
  read -r "?Press Enter, then close this window."
  exit 1
fi

# Detach the app from this Terminal window so closing the window doesn't
# close WorldMaker. Startup errors land in the log file below; once running,
# the app logs to worldmaker_*.log next to the binary.
LAUNCH_LOG="${TMPDIR:-/tmp}/WorldMaker-launch.log"
./target/release/worldmaker-app > "$LAUNCH_LOG" 2>&1 &
disown
echo "WorldMaker is starting — you can close this window."
exit 0
