#!/bin/sh
# Packages a dev build as an app with CEF inside. CEF only runs from a bundle: its framework and
# helper apps are found by their place in one, and helpers by the main executable's name. Usage: scripts/bundle-cef.sh [debug|release]
# HERDR_BUNDLE_ID (default dev.herdr.app.fork, the test copy) and HERDR_BUNDLE_DIR (default target/bundle).
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
# Signed for this computer, helpers before the app that holds them. Herdr's script engine needs JIT.
sign() { codesign --force --sign - --entitlements "$root/scripts/macos-entitlements.plist" --timestamp=none "$1"; }
for helper in "$app/Contents/Frameworks"/*.app; do sign "$helper"; done
codesign --force --sign - --timestamp=none "$app/Contents/Frameworks/Chromium Embedded Framework.framework"
sign "$app"
echo "$app"
