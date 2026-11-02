#!/bin/sh
# Copies the vault CLI's Rust source from its home in cstack into vault/, which Herdr builds into
# the app for browser autofill. Usage: scripts/sync-vault.sh [path to cstack/vault]
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
from=${1:-$HOME/code/cstack/vault}
[ -f "$from/Cargo.toml" ] && [ -d "$from/rust" ] || { echo "no vault at $from" >&2; exit 1; }
mkdir -p "$root/vault"
rm -rf "$root/vault/rust" "$root/vault/tests"
cp -R "$from/rust" "$from/tests" "$root/vault/"
cp "$from/Cargo.toml" "$from/Cargo.lock" "$root/vault/"
echo "vault/ now matches $from ($(git -C "$from" rev-parse --short HEAD 2>/dev/null || echo "not a git checkout"))"
