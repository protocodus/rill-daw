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

`focus_e2e` checks that only one workspace renders its controls, Back returns to Arrangement, nested Properties restores the editor viewport, and navigation leaves the project unchanged. It exercises both themes at 1100×760. Editor and library scenarios also exercise their on-demand menus through real pointer events.

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
| `viewport` | `width`, `height`, optional `pixels_per_point` | Resize the headless logical viewport (800–4096 by 600–4096 points) and display density (0.75–3). |
| `system_theme` | `dark` (boolean) | Deliver operating-system appearance input to egui; `effective_theme` and `theme_colors` report the active theme. |
| `save` | `path` | Save the project through the same native state-flush path as File → Save. |
| `open` | `path` | Open a saved project through File → Open's state barrier, preserving the outgoing session in recovery. |
| `click` | `label`, optional `occurrence` | Click the center of a visible text label using normal pointer press/release events. Occurrences start at zero. |
| `pointer` | `x`, `y`, `phase` (`click`, `double_click`, `drag`, `down`, `move`, `up`), drag `to_x`, `to_y`; optional `button` (`primary`, `middle`, `secondary`) and `shift`, `ctrl`, `alt`, `cmd` | Send real pointer events to canvas coordinates; `automation_plot_rect` in state locates the curve plot. |
| `scroll` | `x`, `y`, `delta`; optional `delta_x` and modifiers | Scroll at viewport coordinates; negative delta moves down the content. |
| `key` | `key`, optional `shift`, `ctrl`, `alt`, `cmd` | Send a keyboard press/release. Names include `Escape`, `Tab`, `Enter`, `Delete`, `A`, and `Z`. `cmd` is the platform command shortcut modifier. |
| `focused` | `focused` boolean | Change window focus for held-note release tests. |
| `text` | `text` | Send text input to the focused control. |
| `drop` | `paths`, optional `track`, `beat` | Drop files onto a visible audio track by project index, or omit `track` to create new tracks in empty arrangement space. Beat must be visible and align to current snap. |
| `export` | `path` | Start the application's background WAV export. |
| `wait_export` | optional `timeout_ms` | Pump application frames until export completes; maximum timeout is 60 seconds. |
| `quit` | — | Deliver a viewport close event and exit successfully. Wait for export before quitting. |

File → Quit also closes the process through the application's normal viewport command. Native file picker actions require a windowing environment; automation supplies the export path explicitly. `--headless` is an opt-in test driver, not a network service. Commands can operate on files with the current user's permissions, so feed it trusted input.

Snapshots also expose `native_editor` lifecycle and parameter metadata, `plugin_catalog` thumbnail status, and `instrument_parameter_rect`. These are observations for driving real UI interactions; they do not mutate the project. Save, Export, Quit, New, Open, and recovery transitions wait for an active native editor's state with a bounded timeout. Failed state synchronization cancels the operation and writes the last known project to recovery.

With `RILL_TEST_VST_INSTRUMENT` set to Surge XT, the ignored `installed_instrument_editor_thumbnail_parameter_automation_save_export_and_quit` test in `features_e2e` drives the complete production app: direct instrument creation, automatic native editor opening, note editing, last-touched automation, Undo/Redo protection, saved state restoration, cached thumbnails, two independent VST tracks with audible export, New/Open recovery, and shutdown. The regular suite separately covers keyboard and double-click creation using metadata fixtures.

The `library_e2e` target drives the large device browser through actual UI controls: name/category search, developer filtering, MIDI instrument creation and assignment, cached instrument/effect previews, insertion into the active chain, creation of routed send returns, and returning from the routing browser with Escape. Its drag test loads stateful effect slots, reorders them by pointer gesture, checks stable IDs/state/parameter overrides/automation, exercises Undo/Redo, and saves/reopens a `.rill` bundle. These metadata fixtures need neither installed plugins nor an audio device and run in containers. Real native preview capture remains covered by the opt-in installed-plugin tests.

```sh
cargo test --locked -p daw-app --test library_e2e
```

The editor suites (`audio_editor_e2e`, `midi_editor_e2e`, `timeline_e2e`) exercise shared gestures and Fit, audio edits/transient previews with rendered-sample oracles, MIDI note/chord/velocity editing, and audition release on mouse/focus/view changes. `audio_setup_e2e` switches conversion quality and sample rate through Setup and verifies real exported bandwidth, rate, persistence, and project preservation. `plugin_recall_e2e` covers failed effect-editor launches plus an opt-in CHOW workflow for automatic state, automation, portable save/reopen, audible recall, and helper crash recovery.

Headless runs disable automatic physical-device startup for held-note previews. `RILL_E2E_AUDIO_DEVICE=1` enables it for a deliberate native-device test. The ordinary suite observes audition intent through `audition_request`; a native Surge renderer test verifies actual sound and note release without a hardware device.

The `project_io_e2e` target covers background bundle saving and loading, later edits remaining dirty, failure preserving the previous archive/current document, Quit waiting for a save, deduplicated waveform preparation, and rejecting oversized WAV declarations before allocation. On Unix it uses a small OS pipe to hold a project read open while exercising real menus, then verifies ordered Save/New completion and outgoing recovery. It needs no device, display, or installed plugin.

Headless `open` and `save` continue to wait for native state acknowledgements and file completion by default. Add `"wait":false` to return after dispatch; `{"command":"wait_project","timeout_ms":120000}` waits for queued document actions and native barriers. `project_pending`, `modified`, `project_path`, `waveform_count`, and `waveform_warning` expose completion and recovery state. Failed operations cancel queued destructive transitions and leave the current document open. The driver accepts a bounded project wait of at most 120 seconds; a timed-out wait does not cancel filesystem work.

The `history_e2e` target checks named multi-step History jumps, saved checkpoints, redo branching, new-project reset, one-step multi-frame note drags, and actual clicks on labelled vector-icon tools in both arranger and piano roll. CI runs every application test target automatically, including tests added in future changes.


`typography_e2e` edits a Unicode/emoji track name, checks readable visible text
across Light/Dark and 1×/2× display density in a 1100×760 window, and saves/reopens
its portable project. A long Unicode device name is searched in the real library,
wraps in the detail title, remains intact in the catalog, and can still be added.
Shared typography tests verify actual glyph availability, proportional/monospace
fallbacks, full wrapped bounds, ellipsis, and dense-row label suppression.

```sh
cargo test --locked -p daw-ui --test typography
cargo test --locked -p daw-app --test typography_e2e
```

The `loop_e2e`, `crossfade_e2e`, and `joins_e2e` suites exercise transport range
editing, validation, grouped gestures, undo/redo, project recall, exclusive audio
priority, spline controls, and MIDI/audio clip consolidation through real menus
and pointer/keyboard events. WAV sample oracles check overlaps, ties, resumed
takes, complementary blends, and joined audio with channel processing applied
once. Joined audio can be split again and survives deleting its original scratch
WAV after portable saving. Loop settings do not truncate offline export.

```sh
cargo test --locked -p daw-app --test loop_e2e --test crossfade_e2e --test joins_e2e
```

The `clones_e2e` suite creates linked MIDI/audio instances through menus, edits
notes and Unicode names from different members, and checks chain indicators.
It exercises independent duplication, detaching, original deletion, local
moving/trimming/splitting, undo/redo, shared gain/fades/reverse/normalization,
and portable save/reopen after deleting the source WAV. An analytic rendered
sample oracle verifies the shared audio edits. App interaction regressions also
cover live note drags, quantize preview/cancel/apply, and rejected boundary edits.

```sh
cargo test --locked -p daw-app --test clones_e2e
```

`components_e2e` checks shared controls and popup navigation at the 1100×720 minimum viewport in both
appearances and at 1×/2× display density. It exercises settings choices, text
fields, toggles, library search/filtering, and returning between focused views;
Cancel preserves the active settings and project. Surface tests verify bounded
mesh work, clipping, transparent edges, and unchanged interaction/layout. Shared
menu regressions cover fast nested-menu clicks, leaf selection, keyboard access,
Escape, and outside-click dismissal. Existing MIDI scenarios also enforce a
compact wrapped toolbar at a narrow desktop viewport.

```sh
cargo test --locked -p daw-ui
cargo test --locked -p daw-app --test components_e2e
```
