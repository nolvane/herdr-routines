#!/bin/sh
# Builds the release binary and places it in the plugin root, where the
# manifest's startup and action commands expect it.
set -eu

cd "$(dirname "$0")/.."

if ! command -v cargo >/dev/null 2>&1; then
  for dir in "$HOME/.cargo/bin" "$HOME/.homebrew/bin" /opt/homebrew/bin /usr/local/bin; do
    if [ -x "$dir/cargo" ]; then
      PATH="$dir:$PATH"
      export PATH
      break
    fi
  done
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo not found: install Rust (https://rustup.rs or 'brew install rust')" >&2
  exit 1
fi

# Only the dependency versions pinned in Cargo.lock, never newer ones
# resolved at install time: a failed build stops here instead.
if ! cargo build --release --locked 2>&1; then
  echo "build failed; nothing was installed (Cargo.lock must match Cargo.toml)" >&2
  exit 1
fi
cp target/release/herdr-routines ./herdr-routines.new
mv -f ./herdr-routines.new ./herdr-routines
echo "built $(pwd)/herdr-routines"
