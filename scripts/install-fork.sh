#!/bin/bash
# Builds this fork and installs it as /Applications/Herdr.app, the Mac's only Herdr, then reopens it.
# Terminals and agents live in herdr-core, so they keep running through the restart. See FORK.md.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
app=/Applications/Herdr.app
export CEF_PATH=${CEF_PATH:-$HOME/.local/share/cef} ZIG=${ZIG:-$HOME/.local/share/zig-0.16/zig} RUSTUP_TOOLCHAIN=${RUSTUP_TOOLCHAIN:-1.95.0}
lsregister=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister

cd "$root"
# Which certificate signs it: HERDR_SIGN_IDENTITY, or the one named in .herdr-sign-identity (kept out
# of git), or none ("-", this computer only, and macOS forgets its permissions on each update).
export HERDR_SIGN_IDENTITY=${HERDR_SIGN_IDENTITY:-$(cat "$root/.herdr-sign-identity" 2>/dev/null || echo -)}
cargo build --release
HERDR_BUNDLE_ID=dev.herdr.app HERDR_BUNDLE_DIR="$root/target/install" scripts/bundle-cef.sh release
codesign --verify --deep --strict "$root/target/install/Herdr.app"

rm -rf "$app.new"
ditto "$root/target/install/Herdr.app" "$app.new"
# The build copy has the same id, so macOS could pick it for "Herdr"; only the installed one stays.
"$lsregister" -u "$root/target/install/Herdr.app" 2>/dev/null || true
rm -rf "$root/target/install"

# Quit the running one and wait for it to go.
if pgrep -f "^$app/Contents/MacOS/Herdr\$" >/dev/null; then
  "$app/Contents/MacOS/Herdr" do app.quit >/dev/null 2>&1 || osascript -e 'tell application id "dev.herdr.app" to quit' || true
  for _ in $(seq 1 100); do pgrep -f "^$app/Contents/MacOS/Herdr\$" >/dev/null || break; sleep 0.2; done
  if pgrep -f "^$app/Contents/MacOS/Herdr\$" >/dev/null; then
    echo "Herdr didn't quit; nothing was replaced. Quit it and run this again." >&2
    rm -rf "$app.new"
    exit 1
  fi
fi

rm -rf "$app.old"
[ -d "$app" ] && mv "$app" "$app.old"
mv "$app.new" "$app"
rm -rf "$app.old"
"$lsregister" -f "$app"
open "$app"
echo "Installed Herdr $(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1) from $(git rev-parse --short HEAD) and reopened it."
