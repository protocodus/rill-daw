#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
. "$project_dir/scripts/package-common.sh"
case "$(uname -s)" in
  Darwin|Linux) ;;
  *) echo 'Use package-windows.ps1 on Windows.' >&2; exit 1 ;;
esac
rill_build
package_name="rill-$rill_version-$rill_target"
stage=$(mktemp -d "${TMPDIR:-/tmp}/rill-package.XXXXXX")
trap 'rm -rf "$stage"' EXIT HUP INT TERM
package_dir="$stage/$package_name"
mkdir -p "$package_dir" "$project_dir/dist"
cp README.md LICENSE "$package_dir/"
cp -R docs "$package_dir/docs"
python3 scripts/collect-licenses.py "$package_dir"
case "$(uname -s)" in
  Darwin)
    RILL_PACKAGE_SKIP_BUILD=1 ./scripts/package-macos.sh
    ditto dist/Rill.app "$package_dir/Rill.app"
    cat > "$package_dir/INSTALL.txt" <<'TEXT'
Rill for macOS

Move Rill.app to Applications and launch it. The application and its plug-in
helper are bundled together; keep the app bundle intact. This build has no
Developer ID signature or Apple notarization. macOS may require approval in
Privacy & Security before its first launch. No installer or system changes run.
Use the archive matching your Mac: aarch64 for Apple silicon, x86_64 for Intel.
Separately installed VST3 plug-ins must match the host architecture.
TEXT
    executable="$package_dir/Rill.app/Contents/MacOS/rill"
    archive="$project_dir/dist/$package_name.zip"
    ;;
  Linux)
    cp "$rill_bin_dir/rill" "$rill_bin_dir/daw-plugin-host" "$package_dir/"
    chmod 755 "$package_dir/rill" "$package_dir/daw-plugin-host"
    mkdir -p "$package_dir/share/applications" "$package_dir/share/icons/hicolor/256x256/apps"
    cp crates/daw-ui/assets/brand/rill-icon-256.png "$package_dir/share/icons/hicolor/256x256/apps/dev.rill.daw.png"
    cp scripts/dev.rill.daw.desktop "$package_dir/share/applications/"
    cp scripts/install-desktop.py "$package_dir/"
    python3 scripts/test-desktop-install.py
    python3 scripts/verify-branding.py --linux "$package_dir"
    cat > "$package_dir/INSTALL.txt" <<'TEXT'
Rill for Linux

Extract this directory and run ./rill. Keep daw-plugin-host beside rill.
CI builds use Ubuntu 22.04 (glibc 2.35); use that version or a compatible newer
distribution. A graphical session (X11 or Wayland), OpenGL-capable driver,
ALSA, udev, X11 input/cursor, xkbcommon, and Wayland client libraries are required
for desktop use. The X11 libraries are loaded at runtime and may not appear in ldd.
Ubuntu 22.04 runtime packages:
  sudo apt install libstdc++6 libasound2 libudev1 libxkbcommon0 libxkbcommon-x11-0 \
    libwayland-client0 libx11-6 libx11-xcb1 libxi6 libxcursor1 libxrandr2 libgl1 libegl1
Ubuntu 24.04 uses libasound2t64 in place of libasound2.
No installation is needed for the --headless test driver or offline rendering
beyond shared-library dependencies. Use the archive matching your CPU.
Separately installed VST3 plug-ins must match the host architecture.
Native plug-in editors need X11 or XWayland. Editor thumbnail capture requires
the XRes and Composite extensions on that display.

Optional application-menu launcher (Python 3, current user only):
  python3 ./install-desktop.py
Keep this extracted directory in its final location before installing the launcher.
Run the command again after moving it. Remove the launcher with:
  python3 ./install-desktop.py --remove
The bundled share/ directory also provides a standard desktop entry and icon
for system package maintainers; its rill executable must be on PATH.
TEXT
    executable="$package_dir/rill"
    archive="$project_dir/dist/$package_name.tar.gz"
    ;;
esac
RILL_CONFIG_DIR="$stage/smoke-config" "$executable" --render-demo "$stage/smoke.wav"
test -s "$stage/smoke.wav"
case "$(uname -s)" in
  Darwin) ditto -c -k --sequesterRsrc --keepParent "$package_dir" "$archive" ;;
  Linux) tar -C "$stage" -czf "$archive" "$package_name" ;;
esac
(
  cd "$project_dir/dist"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$(basename "$archive")" > "$(basename "$archive").sha256"
  else
    shasum -a 256 "$(basename "$archive")" > "$(basename "$archive").sha256"
  fi
)
if [ -n "${GITHUB_ENV:-}" ]; then
  # Test what users download, including archive layout and executable permissions.
  verify_dir=$(mktemp -d "$project_dir/dist/verify-$rill_target.XXXXXX")
  case "$(uname -s)" in
    Darwin)
      ditto -x -k "$archive" "$verify_dir"
      verified_executable="$verify_dir/$package_name/Rill.app/Contents/MacOS/rill"
      ;;
    Linux)
      tar -xzf "$archive" -C "$verify_dir"
      verified_executable="$verify_dir/$package_name/rill"
      ;;
  esac
  test -x "$verified_executable"
  test -x "$(dirname "$verified_executable")/daw-plugin-host"
  printf 'RILL_E2E_BINARY=%s\n' "$verified_executable" >> "$GITHUB_ENV"
fi
echo "Packaged $archive"
