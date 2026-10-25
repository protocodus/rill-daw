#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
if [ "$(uname -s)" != Darwin ]; then
  echo 'This script packages the current macOS architecture.' >&2
  exit 1
fi
. "$project_dir/scripts/package-common.sh"
if [ "${RILL_PACKAGE_SKIP_BUILD:-0}" != 1 ]; then
  rill_build
fi
bundle="$project_dir/dist/Rill.app/Contents"
mkdir -p "$bundle/MacOS" "$bundle/Resources"
# Replace executables atomically so an already running app retains its inode.
for executable in rill daw-plugin-host; do
  staged_executable=$(mktemp "$bundle/MacOS/.$executable.XXXXXX")
  cp "$rill_bin_dir/$executable" "$staged_executable"
  chmod 755 "$staged_executable"
  mv -f "$staged_executable" "$bundle/MacOS/$executable"
done
cp LICENSE "$bundle/Resources/LICENSE"
cp crates/daw-ui/assets/brand/rill.icns "$bundle/Resources/rill.icns"
# This bundle can also be distributed independently of the release archive.
mkdir -p "$bundle/Resources/licenses/rill-fonts"
cp crates/daw-ui/assets/fonts/*.txt crates/daw-ui/assets/fonts/README.md "$bundle/Resources/licenses/rill-fonts/"
mkdir -p "$bundle/Resources/licenses/signalsmith-stretch"
cp crates/daw-engine/vendor/signalsmith/LICENSE-*.txt crates/daw-engine/vendor/signalsmith/README.md "$bundle/Resources/licenses/signalsmith-stretch/"
python3 scripts/native_dsp_licenses.py "$bundle/Resources"
cat > "$bundle/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>rill</string>
<key>CFBundleIdentifier</key><string>dev.rill.daw</string>
<key>CFBundleName</key><string>RILL</string>
<key>CFBundleDisplayName</key><string>RILL</string>
<key>CFBundleIconFile</key><string>rill.icns</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>$rill_numeric_version</string>
<key>CFBundleShortVersionString</key><string>$rill_numeric_version</string>
<key>LSMinimumSystemVersion</key><string>11.0</string>
<key>NSHighResolutionCapable</key><true/>
<key>NSMicrophoneUsageDescription</key><string>RILL records and monitors audio from the input device you select.</string>
</dict></plist>
PLIST
plutil -lint "$bundle/Info.plist"
python3 scripts/verify-branding.py --macos "$project_dir/dist/Rill.app"
"$bundle/MacOS/rill" --help >/dev/null
echo "Built $project_dir/dist/Rill.app"
