#!/bin/bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "package-macos: macOS is required (Darwin host)" >&2
  exit 1
fi

for tool in cargo hdiutil iconutil sips; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "package-macos: required tool not found: $tool" >&2
    exit 1
  fi
done

script_dir=$(cd "$(dirname "$0")" && pwd -P)
root_dir=$(cd "$script_dir/.." && pwd -P)
target_triple=aarch64-apple-darwin
target_dir="$root_dir/target"
if ! target_libdir=$(rustc --print target-libdir --target "$target_triple" 2>/dev/null) || [[ ! -d "$target_libdir" ]]; then
  echo "package-macos: Rust target $target_triple is not installed" >&2
  echo "Install it with: rustup target add $target_triple" >&2
  exit 1
fi

version=$(cargo metadata --manifest-path "$root_dir/Cargo.toml" --no-deps --format-version 1 \
  | sed -n 's/.*"name":"denon-avr-desktop","version":"\([^"]*\)".*/\1/p' | head -n 1)
if [[ -z "$version" ]]; then
  echo "package-macos: unable to determine the Cargo workspace version" >&2
  exit 1
fi

output_dir="$target_dir/release/macos"
work_dir=$(mktemp -d "${TMPDIR:-/tmp}/denon-avr-remote-macos.XXXXXX")
cleanup() { rm -rf "$work_dir"; }
trap cleanup EXIT

echo "Building denon-avr-remote-gui $version for $target_triple..."
cargo build --manifest-path "$root_dir/Cargo.toml" --release --target "$target_triple" \
  -p denon-avr-desktop --bin denon-avr-remote-gui

app="$work_dir/Denon AVR Remote.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$work_dir/dmg"
cp "$target_dir/$target_triple/release/denon-avr-remote-gui" "$app/Contents/MacOS/denon-avr-remote-gui"
chmod 755 "$app/Contents/MacOS/denon-avr-remote-gui"

iconset="$work_dir/DenonAVRRemote.iconset"
mkdir -p "$iconset"
icon_source="$root_dir/tests/visual-baselines/macos/receivers-100pct.png"
if [[ ! -f "$icon_source" ]]; then
  echo "package-macos: icon source is missing: $icon_source" >&2
  exit 1
fi
for icon in \
  "icon_16x16.png:16" "icon_16x16@2x.png:32" \
  "icon_32x32.png:32" "icon_32x32@2x.png:64" \
  "icon_128x128.png:128" "icon_128x128@2x.png:256" \
  "icon_256x256.png:256" "icon_256x256@2x.png:512" \
  "icon_512x512.png:512" "icon_512x512@2x.png:1024"; do
  name=${icon%%:*}
  size=${icon##*:}
  sips -z "$size" "$size" "$icon_source" --out "$iconset/$name" >/dev/null
done
if ! iconutil --convert icns --output "$app/Contents/Resources/DenonAVRRemote.icns" "$iconset"; then
  # Some newer SDK/tool combinations reject otherwise valid iconsets. Keep
  # packaging usable with the native system application icon in that case;
  # supported SDKs use the generated branded icon above.
  fallback_icon="/System/Library/CoreServices/CoreTypes.bundle/Contents/Resources/GenericApplicationIcon.icns"
  if [[ ! -f "$fallback_icon" ]]; then
    echo "package-macos: iconutil could not build the icon and no fallback exists" >&2
    exit 1
  fi
  cp "$fallback_icon" "$app/Contents/Resources/DenonAVRRemote.icns"
fi

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDisplayName</key><string>Denon AVR Remote</string>
  <key>CFBundleExecutable</key><string>denon-avr-remote-gui</string>
  <key>CFBundleIdentifier</key><string>com.denonavr.remote</string>
  <key>CFBundleIconFile</key><string>DenonAVRRemote.icns</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>Denon AVR Remote</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleSupportedPlatforms</key><array><string>MacOSX</string></array>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSArchitecturePriority</key><array><string>arm64</string></array>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSRequiresNativeExecution</key><true/>
</dict>
</plist>
PLIST

cp -R "$app" "$work_dir/dmg/"
ln -s /Applications "$work_dir/dmg/Applications"
dmg="$work_dir/Denon-AVR-Remote-$version-arm64.dmg"
hdiutil create -quiet -volname "Denon AVR Remote" -srcfolder "$work_dir/dmg" \
  -ov -format UDZO "$dmg"

mkdir -p "$output_dir"
rm -rf "$output_dir/Denon AVR Remote.app"
rm -f "$output_dir/Denon-AVR-Remote-$version-arm64.dmg"
mv "$app" "$output_dir/"
mv "$dmg" "$output_dir/"
echo "Created unsigned artifacts in $output_dir"
