# Application E2E tests

Run `bash scripts/test-e2e.sh` locally, or run the same suite in a Linux container:

```sh
docker build -f Dockerfile.e2e -t rill-e2e .
docker run --rm rill-e2e
```

The suite runs on macOS and in a Linux arm64 container with Rust 1.92. The headless app tests need no display server, audio device, or installed VST plug-in. The container also starts a temporary Xvfb display for native tests that check real window pixels, offscreen embedded-child pixels, and full application startup/rendering/shutdown across Arrangement, Instrument, Mixer, and Setup. Xvfb uses software OpenGL; the container and Linux CI explicitly install the dynamically loaded X11 input/cursor and GL/EGL runtime libraries.

The integration tests launch the production `rill --headless` executable as a separate process. It runs the real application and egui frame processing without opening a native window or audio device. Pointer clicks use visible labels, and WAV imports use normal hover and file-drop events. Mixdown uses the same asynchronous export worker as File → Export WAV. Each test supplies a separate temporary configuration, cache, and scratch directory; closing exercises the application's session recovery path there.

By default the tests launch Cargo's application binary. Set `RILL_E2E_BINARY` to an absolute executable path to test a packaged build instead. CI runs all subprocess scenarios again against the executable extracted from each native distribution before uploading the archive.

The audio scenario creates five tracks through the Track menu, drops generated WAV files with different lengths and sample rates onto their rows, exports the complete arrangement, and checks duration including the release tail, finite samples, nonzero stereo content, a distinct audible frequency from each of the five sources, 48 kHz / 32-bit floating-point format, recovered session contents, and successful process exit. Another scenario drops multiple files into empty arrangement space and checks that new tracks appear. Launch-and-quit and File → Quit scenarios verify display-free startup and shutdown. Further scenarios cover track creation and Undo/Redo, instrument/library selection, effect insertion/bypass/reordering and bus routing, mixer mute/solo, folder membership/collapse, and Setup Apply/Cancel persistence. The Add MIDI track scenario verifies automatic assignment-chooser opening and its four-beat pattern; choosing an instrument preserves that pattern. The `mixer_e2e` target drags arranger volume/pan controls, checks channel/destination labels against Mixer, verifies that folders do not consume channel numbers, exercises mute/solo and Undo/Redo, saves/reopens the session, and verifies that the exported stereo balance and level reflect those edits. All subprocess scenarios share `crates/daw-app/tests/common/mod.rs`.

Header-picker scenarios change the output bus directly from the sequencer, return to Master, cancel, undo/redo, and verify the saved route in exported audio. They check that self-routing and transitive feedback destinations are absent and that compact rows retain the picker. MIDI scenarios open the instrument chooser from an unselected track's header, assign and replace its VST, and verify that other tracks, existing patterns, and track count remain unchanged.

The headless tests cover application behavior, UI event handling, model changes, decoding, rendering, export, and shutdown. Metadata-only VST fixtures verify folder setup, scans, search, multi-class instrument assignment, insert chains, and automation without claiming to execute a real plug-in. Native window tests are separate; platform file dialogs and hardware drivers still need manual testing.

Run the native application test on a desktop (or prefix with `xvfb-run -a` on Linux):

```sh
cargo test -p daw-app --test e2e native_window_renders_and_exits_after_screenshot -- --ignored --nocapture
```

Run the arranger mixer scenario alone with `cargo test -p daw-app --test mixer_e2e -- --nocapture`. Both the regular CI suite and extracted-distribution verification include `e2e`, `features_e2e`, `automation_e2e`, and `mixer_e2e`.

Installed-plug-in tests are opt-in and fail if their required environment variables are missing. `RILL_TEST_VST_INSTRUMENT` selects Surge XT and `RILL_TEST_VST_EFFECT` selects CHOWTapeModel for the native editor tests:

```sh
cargo build --workspace
RILL_TEST_VST_INSTRUMENT="/path/to/Surge XT.vst3" \
RILL_TEST_VST_EFFECT="/path/to/CHOWTapeModel.vst3" \
cargo test -p daw-plugins --test native_editor -- --ignored --nocapture
```

These tests initialize real isolated editors, validate detailed PNG captures in a temporary Rill cache, change parameters, and restore saved state. On macOS, a real native mouse drag additionally verifies last-touched parameter events without calling the parameter setter.

## JSON-lines automation

Set `RILL_AUTOMATION_DIR` to an absolute isolated working directory and `RILL_CONFIG_DIR` to a directory inside it. The configuration directory's parent must already exist. Run `rill --headless`. The first stdout line is a `ready` response. Send one JSON object per stdin line; each command receives one JSON response with `ok` and a `state` snapshot. Errors return `ok: false` with `error`. Diagnostics use stderr.

```json
{"command":"click","label":"Track"}
{"command":"click","label":"Add audio track"}
{"command":"drop","paths":["/tmp/tone.wav"],"track":0,"beat":0}
{"command":"export","path":"/tmp/mix.wav"}
{"command":"wait_export","timeout_ms":30000}
{"command":"quit"}
```

Supported commands:

| Command | Arguments | Behavior |
| --- | --- | --- |
| `inspect` | — | Advance frames and return the project, selection, popup state, settings, status, visible labels, and pointer/label geometry for diagnostics. |
| `save` | `path` | Save the project through the same native state-flush path as File → Save. |
| `open` | `path` | Open a saved project through File → Open's state barrier, preserving the outgoing session in recovery. |
| `click` | `label`, optional `occurrence` | Click the center of a visible text label using normal pointer press/release events. Occurrences start at zero. |
| `pointer` | `x`, `y`, `phase` (`click` or `drag`), drag `to_x`, `to_y` | Send real pointer events to canvas coordinates; `automation_plot_rect` in state locates the curve plot. |
| `scroll` | `x`, `y`, `delta` | Scroll at viewport coordinates; negative delta moves down the content. |
| `key` | `key`, optional `shift`, `ctrl`, `alt`, `cmd` | Send a keyboard press/release. Names include `Escape`, `Tab`, `Enter`, `Delete`, `A`, and `Z`. `cmd` is the platform command shortcut modifier. |
| `text` | `text` | Send text input to the focused control. |
| `drop` | `paths`, optional `track`, `beat` | Drop files onto a visible audio track by project index, or omit `track` to create new tracks in empty arrangement space. Beat must be visible and align to current snap. |
| `export` | `path` | Start the application's background WAV export. |
| `wait_export` | optional `timeout_ms` | Pump application frames until export completes; maximum timeout is 60 seconds. |
| `quit` | — | Deliver a viewport close event and exit successfully. Wait for export before quitting. |

File → Quit also closes the process through the application's normal viewport command. Native file picker actions require a windowing environment; automation supplies the export path explicitly. `--headless` is an opt-in test driver, not a network service. Commands can operate on files with the current user's permissions, so feed it trusted input.

Snapshots also expose `native_editor` lifecycle and parameter metadata, `plugin_catalog` thumbnail status, and `instrument_parameter_rect`. These are observations for driving real UI interactions; they do not mutate the project. Save, Export, Quit, New, Open, and recovery transitions wait for an active native editor's state with a bounded timeout. Failed state synchronization cancels the operation and writes the last known project to recovery.

With `RILL_TEST_VST_INSTRUMENT` set to Surge XT, the ignored `installed_instrument_editor_thumbnail_parameter_automation_save_export_and_quit` test in `features_e2e` drives the complete production app: direct instrument creation, automatic native editor opening, note editing, last-touched automation, Undo/Redo protection, saved state restoration, cached thumbnails, two independent VST tracks with audible export, New/Open recovery, and shutdown. The regular suite separately covers keyboard and double-click creation using metadata fixtures.
