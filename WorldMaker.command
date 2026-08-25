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
  read -r "?Press Enter to close."
  exit 1
fi

echo "Building WorldMaker (release)..."
if ! cargo build --release -p worldmaker-app; then
  echo ""
  echo "Build failed. The full error is above."
  read -r "?Press Enter to close."
  exit 1
fi

# Detach the app from this Terminal window so closing the window doesn't
# close WorldMaker.
./target/release/worldmaker-app > /dev/null 2>&1 &
disown
exit 0
