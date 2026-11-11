#!/usr/bin/env bash
# Capture every screen and dialog of the README gallery in both themes:
#   scripts/capture-screens.sh [output-dir]   (default: target/screens)
# Writes <output-dir>/{light,dark}/<view>.png. Needs a display; on Linux CI run
# it under `xvfb-run -a -s "-screen 0 1600x1200x24"`.
set -euo pipefail
cd "$(dirname "$0")/.."
output="${1:-target/screens}"

# Keep this list in sync with the README's "Every screen and dialog" table.
views=(
  arrangement mixer piano audio automation expression library assistant analysis
  instrument effects effect-editor routing properties channel recording export history
  setup setup-appearance setup-storage setup-plugins setup-ai unsaved-changes about
)

cargo build --locked -p daw-app --bin rill
binary="target/debug/rill"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cargo run --locked -q -p daw-core --example screens_project -- "$work/screens.rill"

for theme in Light Dark; do
  directory="$output/${theme,,}"
  mkdir -p "$directory" "$work/$theme"
  printf '{"appearance":"%s"}\n' "$theme" > "$work/$theme/settings.json"
  for view in "${views[@]}"; do
    RILL_CONFIG_DIR="$work/$theme" RILL_CACHE_DIR="$work/cache" \
      timeout 120 "$binary" --screenshot "$directory/$view.png" \
      --project "$work/screens.rill" --view "$view" > /dev/null
    test -s "$directory/$view.png"
  done
done
echo "Captured ${#views[@]} views in light and dark to $output"
