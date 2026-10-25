#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
python3 scripts/test-desktop-install.py
cargo test --locked --workspace --exclude daw-app
cargo test --locked -p vst3-host --lib component_handler_tests
cargo test --locked -p vst3-host --lib host_notifications_and_restart_requests_round_trip_across_the_wire
cargo test --locked -p daw-plugins --test native_context_menu -- --ignored --nocapture
# Exercise real X11 pixel capture on an isolated virtual display, then run the
# application suite without any display or device.
xvfb-run -a -s "-screen 0 1280x1024x24" cargo test --locked -p daw-plugins --lib captures_real_x11_window_pixels -- --ignored --nocapture
xvfb-run -a -s "-screen 0 1600x1200x24" cargo test --locked -p daw-app --test e2e native_window_renders_and_exits_after_screenshot -- --ignored --nocapture
bash scripts/test-e2e.sh
