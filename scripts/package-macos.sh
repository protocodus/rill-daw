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
cp "$rill_bin_dir/rill" "$rill_bin_dir/daw-plugin-host" "$bundle/MacOS/"
cp LICENSE "$bundle/Resources/LICENSE"
cat > "$bundle/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>rill</string>
<key>CFBundleIdentifier</key><string>dev.rill.daw</string>
<key>CFBundleName</key><string>Rill</string>
<key>CFBundleDisplayName</key><string>Rill</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>$rill_numeric_version</string>
<key>CFBundleShortVersionString</key><string>$rill_numeric_version</string>
<key>LSMinimumSystemVersion</key><string>11.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
plutil -lint "$bundle/Info.plist"
"$bundle/MacOS/rill" --help >/dev/null
echo "Built $project_dir/dist/Rill.app"
