#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
# Each scenario runs concurrent audio/plugin workers with real IPC deadlines.
# Avoid oversubscribing shared runners with several complete DAWs at once.
cargo test --locked -p daw-app --tests -- --test-threads=1 --nocapture
