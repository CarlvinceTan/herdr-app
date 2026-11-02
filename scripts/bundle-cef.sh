#!/bin/sh
# Packages a dev build as an app with CEF inside. CEF only runs from a bundle: its framework and
# helper apps are found by their place in one, and helpers by the main executable's name. Usage: scripts/bundle-cef.sh [debug|release]
set -eu
profile=${1:-debug}
cef=${CEF_PATH:-$HOME/.local/share/cef}
root=$(cd "$(dirname "$0")/.." && pwd)
app="$root/target/bundle/Herdr.app"
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
<key>CFBundleVersion</key><string>0.1.11</string>
<key>NSPrincipalClass</key><string>NSApplication</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.1.11</string>
<key>NSHighResolutionCapable</key><true/>
$3
</dict></plist>
PLIST
}

plist Herdr dev.herdr.app.fork "<key>CFBundleIconFile</key><string>Herdr</string>" > "$app/Contents/Info.plist"
cp /Applications/Herdr.app/Contents/Resources/Herdr.icns "$app/Contents/Resources/" 2>/dev/null || true
for suffix in "" " (GPU)" " (Renderer)" " (Plugin)" " (Alerts)"; do
  name="Herdr Helper$suffix"
  helper="$app/Contents/Frameworks/$name.app"
  mkdir -p "$helper/Contents/MacOS"
  cp "$root/target/$profile/herdr-cef-helper" "$helper/Contents/MacOS/$name"
  plist "$name" "dev.herdr.app.fork.helper$(echo "$suffix" | tr -d ' ()' | tr '[:upper:]' '[:lower:]')" "<key>LSUIElement</key><true/>" > "$helper/Contents/Info.plist"
done
# Signed for this computer, helpers before the app that holds them. Herdr's script engine needs JIT.
sign() { codesign --force --sign - --entitlements "$root/scripts/macos-entitlements.plist" --timestamp=none "$1"; }
for helper in "$app/Contents/Frameworks"/*.app; do sign "$helper"; done
codesign --force --sign - --timestamp=none "$app/Contents/Frameworks/Chromium Embedded Framework.framework"
sign "$app"
echo "$app"
