#!/bin/sh
# Sourced by native Unix packaging scripts; callers set project_dir first.
set -eu

rill_target=$(rustc -vV | awk '/^host:/ { print $2 }')
case "$rill_target" in
  *-apple-darwin)
    MACOSX_DEPLOYMENT_TARGET=${MACOSX_DEPLOYMENT_TARGET:-11.0}
    export MACOSX_DEPLOYMENT_TARGET
    python3 - "$MACOSX_DEPLOYMENT_TARGET" <<'PY'
import sys
try:
    parts = sys.argv[1].split('.')
    if not 1 <= len(parts) <= 3 or not all(part.isdecimal() for part in parts):
        raise ValueError
    version = tuple(int(part) for part in parts) + (0,) * (3 - len(parts))
except ValueError:
    raise SystemExit('Invalid MACOSX_DEPLOYMENT_TARGET')
if version < (11, 0, 0):
    raise SystemExit('RILL requires macOS 11 or newer for CoreMIDI UMP input')
PY
    ;;
esac
if [ "${RILL_TARGET:-$rill_target}" != "$rill_target" ]; then
  echo "Packaging must run on the native target: expected $RILL_TARGET, found $rill_target" >&2
  exit 1
fi
if [ -n "${CARGO_BUILD_TARGET:-}" ] && [ "$CARGO_BUILD_TARGET" != "$rill_target" ]; then
  echo 'Cross compilation is not supported by native packaging scripts.' >&2
  exit 1
fi
rill_pkgid=$(cargo pkgid --locked -p daw-app)
rill_version=${rill_pkgid##*#}
rill_version=${rill_version##*@}
rill_numeric_version=${rill_version%%[-+]*}
rill_target_dir=$(cargo metadata --locked --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
rill_bin_dir="$rill_target_dir/release"
if [ -n "${CARGO_BUILD_TARGET:-}" ]; then
  rill_bin_dir="$rill_target_dir/$rill_target/release"
fi

rill_build() {
  cargo build --locked --release --workspace
  test -x "$rill_bin_dir/rill"
  test -x "$rill_bin_dir/daw-plugin-host"
}
