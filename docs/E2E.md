# Application E2E tests

Run `bash scripts/test-e2e.sh` locally, or run the same suite in a Linux container:

```sh
docker build -f Dockerfile.e2e -t rill-e2e .
docker run --rm rill-e2e
```

The suite runs on macOS and in a Linux arm64 container with Rust 1.92. The container run needs no display server, audio device, or installed VST plug-in.

The integration tests launch the production `rill --headless` executable as a separate process. It runs the real application and egui frame processing without opening a native window or audio device. Pointer clicks use visible labels, and WAV imports use normal hover and file-drop events. Mixdown uses the same asynchronous export worker as File → Export WAV. Each test supplies a separate temporary configuration and scratch directory; closing exercises the application's session recovery path there.

By default the tests launch Cargo's application binary. Set `RILL_E2E_BINARY` to an absolute executable path to test a packaged build instead. CI runs all subprocess scenarios again against the executable extracted from each native distribution before uploading the archive.

The audio scenario creates five tracks through the Track menu, drops generated WAV files with different lengths and sample rates onto their rows, exports the complete arrangement, and checks duration including the release tail, finite samples, nonzero stereo content, a distinct audible frequency from each of the five sources, 48 kHz / 32-bit floating-point format, recovered session contents, and successful process exit. Another scenario drops multiple files into empty arrangement space and checks that new tracks appear. Launch-and-quit and File → Quit scenarios verify display-free startup and shutdown. Five further subprocess scenarios cover track creation and Undo/Redo, instrument/library selection, effect insertion/bypass/reordering and bus routing, mixer mute/solo, folder membership/collapse, and Setup Apply/Cancel persistence. All subprocess scenarios share `crates/daw-app/tests/common/mod.rs`.

These tests cover application behavior, UI event handling, model changes, decoding, rendering, export, and shutdown. They do not test native window compositor rendering, platform file dialogs, hardware device drivers, or third-party plug-in installations. Native audio and installed VST smoke tests remain separate.

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
| `click` | `label`, optional `occurrence` | Click the center of a visible text label using normal pointer press/release events. Occurrences start at zero. |
| `pointer` | `x`, `y`, `phase` (`click` or `drag`), drag `to_x`, `to_y` | Send real pointer events to canvas coordinates; `automation_plot_rect` in state locates the curve plot. |
| `key` | `key`, optional `shift`, `ctrl`, `alt`, `cmd` | Send a keyboard press/release. Names include `Escape`, `Tab`, `Enter`, `Delete`, `A`, and `Z`. `cmd` is the platform command shortcut modifier. |
| `text` | `text` | Send text input to the focused control. |
| `drop` | `paths`, optional `track`, `beat` | Drop files onto a visible audio track by project index, or omit `track` to create new tracks in empty arrangement space. Beat must be visible and align to current snap. |
| `export` | `path` | Start the application's background WAV export. |
| `wait_export` | optional `timeout_ms` | Pump application frames until export completes; maximum timeout is 60 seconds. |
| `quit` | — | Deliver a viewport close event and exit successfully. Wait for export before quitting. |

File → Quit also closes the process through the application's normal viewport command. Native file picker actions require a windowing environment; automation supplies the export path explicitly. `--headless` is an opt-in test driver, not a network service. Commands can operate on files with the current user's permissions, so feed it trusted input.
