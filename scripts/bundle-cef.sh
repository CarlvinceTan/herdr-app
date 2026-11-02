#!/bin/sh
# Packages a dev build as an app with CEF inside. CEF only runs from a bundle: its framework and
# helper apps are found by their place in one, and helpers by the main executable's name. Usage: scripts/bundle-cef.sh [debug|release]
# HERDR_BUNDLE_ID (default dev.herdr.app.fork, the test copy), HERDR_BUNDLE_DIR (default target/bundle),
# HERDR_SIGN_IDENTITY (default "-": signed for this computer only).
set -eu
profile=${1:-debug}
cef=${CEF_PATH:-$HOME/.local/share/cef}
root=$(cd "$(dirname "$0")/.." && pwd)
id=${HERDR_BUNDLE_ID:-dev.herdr.app.fork}
app="${HERDR_BUNDLE_DIR:-$root/target/bundle}/Herdr.app"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Frameworks" "$app/Contents/Resources"
cp "$root/target/$profile/herdr-app" "$app/Contents/MacOS/Herdr"
cp "$root/target/$profile/herdr-autofill-host" "$app/Contents/MacOS/"
# The vault CLI autofill runs (vault/), built here as it's a package of its own.
if [ "$profile" = release ]; then vault_flag=--release; else vault_flag=; fi
cargo build $vault_flag --quiet --manifest-path "$root/vault/Cargo.toml" --target-dir "$root/target/vault"
cp "$root/target/vault/$profile/vault" "$app/Contents/MacOS/"
cp -R "$cef/Chromium Embedded Framework.framework" "$app/Contents/Frameworks/"

plist() { # executable, identifier
cat <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>$1</string>
<key>CFBundleIdentifier</key><string>$2</string>
<key>CFBundleName</key><string>$1</string>
<key>CFBundleDisplayName</key><string>$1</string>
<key>CFBundleVersion</key><string>$version</string>
<key>NSPrincipalClass</key><string>NSApplication</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>$version</string>
<key>NSHighResolutionCapable</key><true/>
$3
</dict></plist>
PLIST
}

# Without these, macOS ends the app the moment a page touches one (a phone passkey uses Bluetooth).
usage='<key>NSBluetoothAlwaysUsageDescription</key><string>Web pages in browser panes use Bluetooth for passkeys on your phone and for security keys.</string>
<key>NSCameraUsageDescription</key><string>Web pages in browser panes can ask to use the camera.</string>
<key>NSMicrophoneUsageDescription</key><string>Web pages in browser panes can ask to use the microphone.</string>
<key>NSLocationUsageDescription</key><string>Web pages in browser panes can ask for your location.</string>
<key>NSLocationWhenInUseUsageDescription</key><string>Web pages in browser panes can ask for your location.</string>'
plist Herdr "$id" "<key>CFBundleIconFile</key><string>Herdr</string>
$usage" > "$app/Contents/Info.plist"
cp "$root/assets/icons/Herdr.icns" "$app/Contents/Resources/"
for suffix in "" " (GPU)" " (Renderer)" " (Plugin)" " (Alerts)"; do
  name="Herdr Helper$suffix"
  helper="$app/Contents/Frameworks/$name.app"
  mkdir -p "$helper/Contents/MacOS"
  cp "$root/target/$profile/herdr-cef-helper" "$helper/Contents/MacOS/$name"
  plist "$name" "$id.helper$(echo "$suffix" | tr -d ' ()' | tr '[:upper:]' '[:lower:]')" "<key>LSUIElement</key><true/>
$usage" > "$helper/Contents/Info.plist"
done
# Helpers before the app that holds them. Herdr's script engine needs JIT. With a real identity,
# macOS keeps the permissions it gave the app (Bluetooth, microphone) across updates.
identity=${HERDR_SIGN_IDENTITY:--}
sign() { codesign --force --sign "$identity" --entitlements "$root/scripts/macos-entitlements.plist" --timestamp=none "$1"; }
for helper in "$app/Contents/Frameworks"/*.app; do sign "$helper"; done
codesign --force --sign "$identity" --timestamp=none "$app/Contents/MacOS/herdr-autofill-host"
codesign --force --sign "$identity" --timestamp=none "$app/Contents/MacOS/vault"
codesign --force --sign "$identity" --timestamp=none "$app/Contents/Frameworks/Chromium Embedded Framework.framework"
sign "$app"
echo "$app"
