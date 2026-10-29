# Application E2E tests

`assistant_scope_e2e` opens AI from the piano roll, composition dialog, audio
editor, Mixer, Routing, automation lane, instrument, effect and effect-chain
headers. Scripted provider calls exercise real note, audio, parameter, chain,
mix and automation edits, including atomic rejection of foreign targets and
linked-clip propagation. It browses the context-filtered Tools page, checks the
visible and outbound prompt scope, and verifies that global AI clears local
conversation history and can edit the whole song. Selection changes keep the
captured scope. A gated isolated parameter worker checks that switching songs or
deleting the target prevents pending contextual work from editing the song.
Replacing an instrument on the same channel requires reopening its AI context
and clears the previous processor's conversation before editing its parameters.
Narrow composition dialogs are exercised in both appearances. These tests need
no provider credentials, network, display, audio device or installed plug-in.

```sh
cargo test --locked -p daw-app --test assistant_scope_e2e -- --test-threads=1
```

`assistant_guidance_e2e` browses musical examples and prompting tips through the
running Assistant, checks editable insertion without automatic submission, and
keeps the composer and actions accessible at 800×600 in both appearances. A
deterministic provider fixture turns an edited example into eight bars of chords,
arpeggio and bass, then refines the mix through a follow-up and mouse-driven
Undo. It also covers conversation continuity despite many tool activities,
failed-prompt recovery, New chat without losing a draft or editing the song,
selected musical context, nested AI settings, and clearing stale conversation
scope when changing documents. These scenarios need no provider credentials,
network, display, audio device or installed instrument.

```sh
cargo test --locked -p daw-app --test assistant_guidance_e2e --test assistant_e2e --test assistant_settings_e2e --test composer_e2e -- --test-threads=1
```

`visual_polish_e2e` selects clips and channels through real mouse actions and
checks bounded halo meshes, continuous routing attachment, source/destination
color gradients, and inactive-background blur layering without song edits.
It exercises both appearances and includes a native pixel test that distinguishes
blur from dimming by measuring normalized image detail.

```sh
cargo test --locked -p daw-app --test visual_polish_e2e --test mixer_flow_e2e -- --test-threads=1
xvfb-run -a -s '-screen 0 1600x1200x24' cargo test --locked -p daw-app --test visual_polish_e2e native -- --ignored --test-threads=1
```

`track_reorder_e2e` drives real title/grip drags in Arrangement and Mixer, including
all track kinds, nested/collapsed folders, explicit placement previews, invalid
targets, cancellation, edge scrolling, synchronized channel numbers, Undo/Redo
and portable recall. `track_reorder_playback_e2e` verifies continuous audio and
stable engine/plug-in identities through channel moves and subsequent live edits.

```sh
cargo test --locked -p daw-app --test track_reorder_e2e --test track_reorder_playback_e2e -- --test-threads=1
```

`automation_color_e2e` checks actual orange curves, nodes, handles and owned
controls across Arrangement, the automation editor, Mixer, Properties and Routing.
It distinguishes coral-red manual overrides from muted retained curves, verifies
contrast in both themes, and ensures view/theme changes preserve the project and
history. Opt-in paint diagnostics include paths, circles and text colors.

`slider_scale_e2e` compares actual painted ticks and handle center lines with
mouse edits at range endpoints and fractional positions. It checks adaptive
scales, endpoint/current captions, compact headers, both themes and display
densities. `slider_automation_e2e` creates gain, pan and send lanes directly from
controls, checks exact ownership, Undo/Redo, override and Resume. The optional
`inspect` paint output includes `painted_lines` for these visual checks.

```sh
cargo test --locked -p daw-app --test slider_scale_e2e --test slider_automation_e2e -- --test-threads=1
```

`practical_controls_e2e` opens exact-value menus on Mixer pan, compact header
gain/pan and Routing sends using the mouse. It checks visible ranges and current
values, selected numeric drafts, explicit Apply/Cancel, invalid and out-of-range
input, fractional precision, one-step Undo and saved recall. Automated controls
retain their curves through typed and explicit overrides, and Resume returns to
the held curve value. Actual painted handle markers distinguish orange automated,
coral overridden and gray bypassed controls. The Channel properties dB readout is edited directly.
These scenarios require no display, audio device or plug-in.

```sh
cargo test --locked -p daw-app --test practical_controls_e2e --test mixer_gain_entry_e2e --test slider_scale_e2e --test slider_automation_e2e -- --test-threads=1
```

`automation_curves_e2e` drives per-segment spline presets and diamond handles,
manual takeover, Resume, Undo/Redo and portable recall. It compares rendered
audio before the first point, inside shaped segments and after the last point,
and checks live override/resume without replacing the audio engine.
`automation_indicators_e2e` covers native pan/send controls and arranger status
menus. Parameter-search and native-parameter-automation tests cover browser and
native plug-in gestures, exact instance ownership, override recall and resume,
including a delayed native setter acknowledgement arriving after Resume.
`automation_tools_e2e` verifies that tool-driven mix changes enter override,
preserve curves and Undo/Redo, and respect explicit Resume in an atomic batch.
These tests require no audio device, display or installed plug-in.

```sh
cargo test --locked -p daw-app --test automation_curves_e2e --test automation_indicators_e2e --test automation_tools_e2e --test automation_e2e --test parameter_search_e2e --test native_parameter_automation_e2e -- --test-threads=1
```

`mixer_readability_e2e` checks channel separation, destination color markers,
bus input counts, insert summaries and mouse access to output/send routing.
`effect_chain_layout_e2e` covers bounded effect names, status, pinned insertion,
bypass/reordering with Undo, and calibrated fader interaction. `studio_windows_e2e`
opens and closes on-demand utilities in both appearances and at narrow/wide
sizes, preserving the song while keeping the wrapped transport accessible.
`control_roles_e2e` also checks whole-family tool wrapping and mouse selection.
`routing_layout_e2e` follows Properties into Routing through live appearance,
display-density and viewport changes. It keeps automated sends visible beneath
the contextual AI action, scrolls a long send list to its output selector, and
checks mouse edits and Undo at 800×600 and 1100×850.

```sh
cargo test --locked -p daw-app --test mixer_readability_e2e --test effect_chain_layout_e2e --test routing_layout_e2e --test studio_windows_e2e --test control_roles_e2e -- --test-threads=1
```

`clip_surface_e2e` verifies opaque audio/MIDI clip bodies, separate title and
preview geometry, extreme-pitch thumbnail visibility, compact rows and mouse
selection through the running app in both appearances and display densities.

`composer_e2e` exercises the native composition module through the running
assistant and MCP boundary. A deterministic provider fixture creates chord,
arpeggio, and bass clips in one history entry; pointer-driven Undo/Redo checks
the same song state. The scenarios verify exact pitches/timing, full clip
ranges, read-only theory queries, instrument/routing preservation, and atomic
rejection of stale or invalid composition calls. They require no model API key,
audio device, display, or installed plug-in code.

`shared_theory_e2e` selects expanded chord/scale choices through the piano roll,
including a deep scrolling menu. It compares editor highlighting, octave
stacks, Generate previews/commits, and AI tool generation against the shared
catalogue, while preserving legacy Add9 and scale choices.

`piano_roll_refinement_e2e` checks blue velocity shades and optional red scale
warnings in both themes, selection outlines, independent green guides, and
mouse-driven Randomize/Quantize/Legato actions. It verifies preview cancellation,
seeded rerolls, selection scope, stretched source timing, both legato modes,
single-step Undo/Redo, and semantic tool parity.

```sh
cargo test --locked -p daw-app --test piano_roll_refinement_e2e
cargo test --locked -p daw-composer -p daw-tools
cargo test --locked -p daw-app --test composer_e2e
cargo test --locked -p daw-app --test shared_theory_e2e
```

Run `bash scripts/test-e2e.sh` locally, or run the same suite in a Linux container:

```sh
docker build -f Dockerfile.e2e -t rill-e2e .
docker run --rm rill-e2e
```

The suite runs on macOS and in a Linux arm64 container with Rust 1.92. The headless app tests need no display server, audio device, or installed VST plug-in. The container also starts a temporary Xvfb display for native tests that check real window pixels, offscreen embedded-child pixels, and full application startup/rendering/shutdown across Arrangement, Instrument, Mixer, Setup, and Properties. Xvfb uses software OpenGL; the container and Linux CI explicitly install the dynamically loaded X11 input/cursor and GL/EGL runtime libraries.

The integration tests launch the production `rill --headless` executable as a separate process. It runs the real application and egui frame processing without opening a native window or audio device. Pointer clicks use visible labels, and WAV imports use normal hover and file-drop events. Mixdown uses the same asynchronous export worker as File → Export WAV. Each test supplies a separate temporary configuration, cache, and scratch directory; closing exercises the application's session recovery path there.

The runner executes scenarios one at a time, including in CI and containers. Each app still runs its concurrent audio and isolated plugin workers with the production response deadlines. Serial scenario scheduling avoids turning functional tests into a test of several complete DAWs competing for limited runner CPUs; sample, continuity, crash-isolation and deadline assertions remain unchanged.

By default the tests launch Cargo's application binary. Set `RILL_E2E_BINARY` to an absolute executable path to test a packaged build instead. CI runs all subprocess scenarios again against the executable extracted from each native distribution before uploading the archive.

The audio scenario creates five tracks through the Track menu, drops generated WAV files with different lengths and sample rates onto their rows, exports the complete arrangement, and checks duration including the release tail, finite samples, nonzero stereo content, a distinct audible frequency from each of the five sources, 48 kHz / 32-bit floating-point format, recovered session contents, and successful process exit. Another scenario drops multiple files into empty arrangement space and checks that new tracks appear. Launch-and-quit and File → Quit scenarios verify display-free startup and shutdown. Further scenarios cover track creation and Undo/Redo, instrument/library selection, effect insertion/bypass/reordering and bus routing, mixer mute/solo, folder membership/collapse, and Setup Apply/Cancel persistence. The Add MIDI track scenario verifies automatic assignment-chooser opening and its four-beat pattern; choosing an instrument preserves that pattern. The `mixer_e2e` target drags arranger volume/pan controls, checks channel/destination labels against Mixer, verifies that folders do not consume channel numbers, exercises mute/solo and Undo/Redo, saves/reopens the session, and verifies that the exported stereo balance and level reflect those edits. All subprocess scenarios share `crates/daw-app/tests/common/mod.rs`.

Header-picker scenarios change the output bus directly from the sequencer, return to Master, cancel, undo/redo, and verify the saved route in exported audio. They check that self-routing and transitive feedback destinations are absent and that compact rows retain the picker. MIDI scenarios open the instrument chooser from an unselected track's header, assign and replace its VST, and verify that other tracks, existing patterns, and track count remain unchanged.

The headless tests cover application behavior, UI event handling, model changes, decoding, rendering, export, and shutdown. Metadata-only VST fixtures verify folder setup, scans, search, multi-class instrument assignment, insert chains, and automation without claiming to execute a real plug-in. Native window tests are separate; platform file dialogs and hardware drivers still need manual testing.

`source_preservation_e2e` rejects Save As and export destinations that alias audio
referenced by the project, including inactive takes and muted tracks. It checks
direct paths, relative paths, hard links and symbolic links; distinct destinations
remain usable. Byte comparisons cover original WAVs, full portable assets and
freshly extracted copies, with command-line and engine export coverage too.

`focus_e2e` checks that only the foreground view exposes interactive controls, while retained caller paint remains visible beneath utilities. It checks backdrop source/depth, ignores clicks on a former background Back target, restores nested Properties and the editor viewport, and keeps navigation out of project history. It exercises both themes at 1100×760. Editor and library scenarios also exercise their on-demand menus through real pointer events.

`foreground_shortcuts_e2e` uses F and plus/minus with the pointer over the toolbar
in Arrangement, MIDI, audio and automation. It checks selection and history stay
intact and guards text/numeric entry, menus, utilities and document decisions.
`beat_ruler_e2e` checks readable beat and subdivision labels across those same
editors, including 3/4 and 4/4, both themes and display densities.
`mixer_gain_entry_e2e` types an exact dB value, checks Undo/Redo and portable
recall against exported level, and ensures middle-drag scrolling and stored
values outside the editing range do not change gain.

`main_toolbar_roles_e2e` checks complete control groups at 800–1600 px, their
painted action/view roles, and real transport/Click/Loop behavior. The spectrum
view tests check intensity and both cached palettes in actual uploaded textures
and painted quads, with unchanged playback, source files and exports.
`arranger_layout_e2e` drives label and global row resizing, verifies the loop's
separate painted lane, and creates/reparents nested folders with safe choices,
Undo and portable recall. `mixer_flow_e2e` verifies signal-stage order and actual
arrow geometry, multiple pre/post bus sends against rendered samples, feedback
rejection, retained route preview, Undo and save/reopen.
`midi_overlap_merge_e2e` creates and moves fully covered notes through normal
pointer/keyboard/native clipboard input. It checks release-time merging,
selection, Undo, stretched ghosts, untouched imported overlaps and saving during
a held draw. Chords and partial overlaps remain distinct.

`popup_backdrop_e2e` inspects actual paint order to require one theme-aware dim layer behind the active popup. It covers utilities, nested navigation, menu-bar and context menus, output and color pickers, combo boxes over both workspaces and utilities, and the unsaved-project confirmation. Closing or cancelling restores the caller and removes the dim when no popup remains; both appearances are exercised.

`parameter_search_e2e` exercises instrument/effect parameter search, keyboard and double-click activation, empty-result safety, automated/last-touched filters, instance-specific initial values, lane reuse, Undo/Redo, and project recall. It compiles a test-only Rust metadata host and points only its child app at that host through `DAW_PLUGIN_HOST`. The app uses its production asynchronous sandbox protocol; the fixture provides metadata and saved values, with no DSP or native editor. It requires no installed plugins, display, or audio device and works in containers. Third-party compatibility and native editor coverage remain in the opt-in installed-plugin tests.

`editor_unavailable_e2e` holds native-editor startup while Save or WAV export is
requested. An explicit unsupported-view response before the editor becomes ready
must continue the operation using the project state, including a real audible
VST3 export. The same error after a working editor reports parameter changes
must still cancel saving and preserve recovery. This requires no desktop or
installed plug-in and uses the production DSP worker.

`error_details_e2e` exercises real export failures and isolated editor failures through
the running app. It checks the complete diagnostic text, clipboard copy, retained
caller, and safe dismissal. `render_error_e2e` runs a native VST3 whose processing
exceeds the sandbox deadline alongside healthy audio: the Details view identifies
the failed channel/plugin and cause, healthy audio continues while the dialog is
open, and a dismissed failure does not repeat on every block. These tests use no
desktop or audio hardware. Run them after building the workspace binaries:

```sh
cargo test --locked -p daw-app --test error_details_e2e --test render_error_e2e
```

`plugin_loading_e2e` compiles a native VST3 fixture and runs it through the production
`daw-plugin-host`, with no protocol substitute. The fixture terminates its process
if the host attempts to create an editor during audio loading or parameter reads.
Its lifecycle calls take 200 ms to guard against applying audio-block deadlines
to legitimate instrument initialization. Rill permits up to 30 seconds for
start/stop and configuration while retaining 100 ms limits for audio processing;
the native plugin tests also verify both timeout classes terminate stalled workers.
The app scans the bundle, opens an assigned MIDI project, edits its Level parameter
through the parameter browser, exports finite non-silent audio with the expected
level change and note-off silence, then saves and restarts to verify identical
audio from the recalled archive. It needs no installed plugin, desktop, or audio
hardware. Build the production worker before running this target alone:

```sh
cargo build --locked --workspace --bins
cargo test --locked -p daw-app --test plugin_loading_e2e
```

`native_parameter_automation_e2e` uses an isolated editor-process fixture to send the production native automation IPC event into the running app. It checks exact instrument/effect ownership, parameter ID zero, current-value initialization, reuse, Undo/Redo, save/reopen, and rejected requests during document confirmation. An opaque-preset scenario checks that the snapshot precedes lane creation, so Undo removes only the lane and preserves the current preset. The fixture never opens a native window or audio device. Host COM tests separately invoke the actual VST3 context-menu interfaces, covering both plugin-rendered copied targets and host-rendered popup callbacks, eligibility, stale targets, and thread/lifetime checks. Shared-menu and worker tests cover nested/disabled entries and process cancellation/failure. These automated layers do not claim compatibility with every third-party editor; native editors must expose the VST3 host-menu hook.

```sh
cargo test --locked -p daw-app --test native_parameter_automation_e2e
cargo test --locked -p vst3-host --lib component_handler_tests
cargo test --locked -p daw-plugins --lib editor_context_menu
```

On Linux, `cargo test --locked -p daw-plugins --test native_context_menu -- --ignored` creates its own private Xvfb display and drives the real menu worker with X11 pointer/keyboard events. It checks selection of action zero, disabled items, Escape cancellation, and physical positioning at 1× and 2× scale. It requires Xvfb and Mesa, connects only to its private display, and is included in container tests and Linux CI.

`workspace_switch_e2e` exercises the persistent Arranger/Mixer switch from both workspaces, Clip Editor, and contextual utilities, including selecting the already active caller to dismiss a popup. It checks bounds at 800×600 and 1100×720, both appearances and densities, and preserves project/history/selection. `channel_properties_e2e` verifies separate Clip/Track/Channel scopes, audio-only clip gain, channel gain/pan, and navigation among Mixer, Channel, Effects, Instrument, and Routing. `instrument_context_e2e` checks concise native-editor overviews, the on-demand **Automate…** parameter page, contextual titles, effect-editor-to-instrument navigation, and clean failure of metadata-only plugin fixtures.

`header_channel_controls_e2e` drags the graphical gain/pan sliders in normal and compact track headers, verifies the same mixer values and undo/redo, preserves clip data, saves/reopens the project, and checks the exported audio level/balance. On Linux, `channel_meter_e2e` runs a real render worker and CPAL stream against an isolated ALSA null PCM. Its child-only `ALSA_CONFIG_PATH` cannot choose host audio hardware. It validates distinct source, silent-channel, and bus levels after gain/pan, then zero readings on pause/stop. This target requires no display or physical sound device; non-Linux builds skip it.

`live_edit_e2e` drives volume/pan, MIDI notes, audio/MIDI clip edits, source imports, and Undo/Redo while the production render worker is active. A device-free sink consumes the same bounded audio queue and callback path through explicit `pump_audio` commands; it never opens physical audio hardware. A temporary Rust instrument/effect worker speaks the real isolation protocol and retains sustained voices plus a 32-frame delay. Tests inspect actual output samples, engine/generation identity, sandbox process identity, lifecycle/MIDI logs, and applied revisions. Audio is also consumed between pointer Down/Move/Up events: fader previews must sound before release, a gesture returning to its starting value must restore audio without an Undo entry, and dragging a sounding MIDI note must not add a duplicate audition voice. Rapid edits must converge on the latest project; failed source preparation must preserve the previous audio; structural changes still use controlled engine replacement. This deterministic suite verifies musical continuity and instance preservation, not real-time hardware deadline performance or native VST compatibility. It runs on every platform and in containers:

```sh
cargo test --locked -p daw-app --test live_edit_e2e
```

`track_color_picker_e2e` clicks the arranger's color strip and edits native RGB controls, then checks actual painted track markers, clip strokes, and full colored mixer names with subdued strip tints. It covers audio, MIDI, and bus colors, neutral Master, selected and unselected channels, tint restoration through undo/redo and portable save/reopen, both appearances and densities, and compact/folder tracks without changing selection, cursor, clip contents, or folder expansion. Rectangle paint diagnostics are requested only by `inspect` with `paint: true`.

`mixer_height_e2e` resizes the running mixer between 1100×720, 1440×1100, and 800×600 in light and dark appearances. It checks the actual strip, fader, and stereo-meter paint bounds, full-height expansion, aligned controls, compact-window scrolling, Effects navigation, and a resized fader edit with Undo.

```sh
cargo test --locked -p daw-app --test workspace_switch_e2e --test channel_properties_e2e --test instrument_context_e2e --test header_channel_controls_e2e --test channel_meter_e2e
```

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
| `inspect` | optional `paint` | Advance frames and return the project, selection, popup state, settings, status, visible labels, and pointer/label geometry. With `paint: true`, also return clipped rectangle fills and strokes in `painted_rects` for visual-color assertions. |
| `pump_audio` | `frames` (1–8192) | Consume audio through the production queue/callback consumer and return stereo `audio_output` samples. Requires the isolated driver's `RILL_E2E_VIRTUAL_AUDIO` mode; `engine` reports playback identity, revisions, position, and counters. |
| `dummy_audio_device` | optional `input_channels` (1–32), `input_latency_frames`, `output_latency_frames` | Configure the device-free backend before recording. Omitted fields retain their values; defaults are two input channels and zero latency. Each latency is bounded to one second at the current sample rate. Requires `RILL_E2E_VIRTUAL_AUDIO`. |
| `pump_input` | `channels`, `samples` (interleaved float array) | Deliver input and consume an equal number of output frames in bounded chunks; return stereo `audio_output`. Runs the production capture, monitoring, and disk-writer path. Requires a configured virtual input with matching channel count. |
| `feed_input` | `channels`, `samples` (interleaved float array) | Deliver only input callbacks, without advancing output. Pair with `pump_audio` to test independent scheduling, queue overflow, and latency. Same virtual input requirements and bounds as `pump_input`. |
| `input_error` | — | Inject input disconnection into the virtual backend; active recording finalizes available audio and reports the failure. |
| `output_error` | — | Inject output failure into the virtual backend; active recording finalizes available audio and reports the failure. |
| `midi_input` | `bytes` (MIDI byte array), optional `offset_frames` (default `0`) | Queue a MIDI message through the dummy input callback, relative to the next output frame. Requires a selected and ready virtual MIDI port. Feed scheduled messages in timestamp order, then advance the device with `pump_audio` or `pump_input`; inspecting UI frames does not advance MIDI time. |
| `midi_error` | — | Disconnect the selected virtual MIDI input. Active capture retains completed notes and closes held notes; monitoring releases held instrument voices. |
| `viewport` | `width`, `height`, optional `pixels_per_point` | Resize the headless logical viewport (800–4096 by 600–4096 points) and display density (0.75–3). |
| `system_theme` | `dark` (boolean) | Deliver operating-system appearance input to egui; `effective_theme` and `theme_colors` report the active theme. |
| `save` | `path` | Save the project through the same native state-flush path as File → Save. |
| `open` | `path` | Execute an already-approved open through the document pipeline, preserving the outgoing session in recovery. Use `file_dialog` and File → Open to test the unsaved-change decision. |
| `file_dialog` | `kind` (`save`, `open`, `import`, `export`, or `export_midi`), `path` (or `null`), optional `replace_existing` | Supply the next matching native file picker's result; `null` cancels. It does not open or save anything until the actual UI requests that picker. If extension normalization changes the path to an existing destination, only `replace_existing: true` accepts the additional replacement decision; false or omitted cancels. Exact recognized-extension paths retain the native picker's original confirmation. |
| `close_window` | — | Deliver a native viewport close request and return the resulting guard or exit state. Unsaved work requires an explicit UI choice. |
| `click` | `label`, optional `occurrence` | Click the center of a visible text label using normal pointer press/release events. Occurrences start at zero; the default/zero occurrence prefers an active menu or utility window over an identically named inactive workspace control. |
| `pointer` | `x`, `y`, `phase` (`click`, `double_click`, `drag`, `down`, `move`, `up`), drag `to_x`, `to_y`; optional `button` (`primary`, `middle`, `secondary`) and `shift`, `ctrl`, `alt`, `cmd` | Send real pointer events to canvas coordinates; `automation_plot_rect` in state locates the curve plot. |
| `scroll` | `x`, `y`, `delta`; optional `delta_x` and modifiers | Scroll at viewport coordinates; negative delta moves down the content. |
| `key` | `key`, optional `shift`, `ctrl`, `alt`, `cmd`, `release_modifiers`, `repetitions` (1–32) | Send a keyboard press/release. Names include `Escape`, `Tab`, `Enter`, `Delete`, `A`, and `Z`. `cmd` is the platform command shortcut modifier. `release_modifiers: true` delivers the complete chord in one frame with no modifiers held at repaint; repeated presses are also delivered in one frame. Defaults preserve separate press/release frames. |
| `focused` | `focused` boolean | Change window focus for held-note release tests. |
| `text` | `text` | Send text input to the focused control. |
| `clipboard` | `action` (`copy`, `cut`, `paste`), optional `text`, `with_key` | Deliver native clipboard events from the desktop integration. Paste defaults to the last clipboard output; `with_key` also supplies the matching command key to check that an edit happens only once. |
| `drop` | `paths`, optional `track`, `beat` | Drop files onto a visible audio track by project index, or omit `track` to create new tracks in empty arrangement space. Beat must be visible and align to current snap. |
| `export` | `path` | Start the application's background WAV export. |
| `wait_export` | optional `timeout_ms` | Pump application frames until export completes; maximum timeout is 60 seconds. |
| `quit` | — | Explicitly finish the isolated test session through the approved shutdown pipeline, retaining native state barriers and recovery. It bypasses the unsaved-change decision; use `close_window` or File → Quit to test that decision. Wait for export before quitting. |

File → Quit closes the process through the application's normal viewport command after any unsaved-change decision. Native file picker presentation requires a windowing environment; the driver can supply Open/Save/Export picker results, or start a default export with an explicit path. These are automated UI regressions, not a replacement for native mouse-and-keyboard testing. `--headless` is an opt-in test driver, not a network service. Commands can operate on files with the current user's permissions, so feed it trusted input.

Snapshots also expose `native_editor` lifecycle and parameter metadata, `plugin_catalog` thumbnail status, version, architectures, first-indexed time, favorite and categories, `plugin_scanning`, and `instrument_parameter_rect`. The open library's `library` snapshot adds its pane rectangles, view, collection/developer/category filters, sort, result `order`, and background `sync` status. `popup_context` reports the inactive caller's source title, shape count, and nesting depth; `inspector_page` identifies the active property scope. `timeline.channels` reports the actual header slider/meter rectangles and engine peak readings. `recording` reports `phase` (`idle`, `starting`, `counting_in`, `recording`, `finalizing`), armed tracks, monitor modes, input readiness/channels/peaks, captured frames, completed passes, dropped frames, `dummy_device` configuration, the last published files/clip IDs, and error details. Its `midi` snapshot reports port readiness, capture activity, event and held-note counts; `timing` reports remaining count-in beats/frames and capture gates. These are observations for driving real UI interactions; they do not mutate the project. Save, Export, Quit, New, Open, and recovery transitions wait for an active native editor's state with a bounded timeout. Failed state synchronization cancels the operation and writes the last known project to recovery.

With `RILL_TEST_VST_INSTRUMENT` set to Surge XT, the ignored `installed_instrument_editor_thumbnail_parameter_automation_save_export_and_quit` test in `features_e2e` drives the complete production app: direct instrument creation, automatic native editor opening, note editing, last-touched automation, Undo/Redo protection, saved state restoration, cached thumbnails, two independent VST tracks with audible export, New/Open recovery, and shutdown. The regular suite separately covers keyboard and double-click creation using metadata fixtures.

The `library_e2e` target drives the large device browser through actual UI controls: name/category search, developer filtering, MIDI instrument creation and assignment, cached instrument/effect previews, insertion into the active chain, creation of routed send returns, and returning from the routing browser with Escape. Its drag test loads stateful effect slots, reorders them by pointer gesture, checks stable IDs/state/parameter overrides/automation, exercises Undo/Redo, and saves/reopens a `.rill` bundle. These metadata fixtures need neither installed plugins nor an audio device and run in containers. Real native preview capture remains covered by the opt-in installed-plugin tests.

`plugin_catalog_e2e` compiles a standard-library editor worker (`tests/common/preview_editor.rs`) that answers capture requests with generated panels, and launches the app with `Harness::with_plugin_previews`. It verifies that opening the app indexes the folders and captures verified previews in the background without opening the library, that selecting an unverified bundle captures it, that **Refresh preview** recaptures, that a restarted session reuses the index without relaunching editors, and that **Refresh** picks up installs and removals and sorts them by **Recently added**. Its second scenario exercises favorites (detail toggle, More menu, card badge), collection and category filters with removable chips, user categories, **Copy path**, the list view with double-click creation, persistence across restart, and the `list_plugins` (including `refresh`) and `organize_plugin` tools. Tests that seed the preview cache after launch call `common::refresh_library`, as a user would press **Refresh**.

```sh
cargo test --locked -p daw-app --test plugin_catalog_e2e
```

`device_cards_e2e` checks the responsive card grid using cached PNG fixtures, including actual painted image meshes, device/developer text, placeholders, both themes and display densities, compact-window bounds, virtualized scrolling, two-dimensional keyboard navigation, filtering, and safe click/double-click creation and assignment. `inspect` with `paint: true` includes `painted_images` alongside painted rectangles; normal frames do not collect these diagnostics. Shared component tests cover large catalogs, long Unicode names, focus, and stable geometry as previews arrive. Preview-cache tests cover bounded background decoding, refresh invalidation, failure memoization, and eviction.

```sh
cargo test --locked -p daw-app --test library_e2e
```

The editor suites (`audio_editor_e2e`, `midi_editor_e2e`, `timeline_e2e`) exercise shared gestures and Fit, audio edits/transient previews with rendered-sample oracles, MIDI note/chord/velocity editing, and audition release on mouse/focus/view changes. `audio_setup_e2e` switches conversion quality and sample rate through Setup and verifies real exported bandwidth, rate, persistence, and project preservation. `plugin_recall_e2e` covers failed effect-editor launches plus an opt-in CHOW workflow for automatic state, automation, portable save/reopen, audible recall, and helper crash recovery.

`midi_generate_e2e` uses the running piano roll to generate sustained chords,
arpeggios and seeded melodies. It verifies complete visible-bar defaults in two
meters and both themes, editable ranges, non-destructive colored previews,
cancel and keyboard guards, hidden-note selection safety when scale rows are
folded, toolbar wrapping, empty-range recovery, arpeggio controls, all melody
algorithms and accompanying voices. Committing a preview is checked against the
actual generated source notes, including trimmed/stretched ghost instances,
one-step undo/redo and portable project recall. Run it on its own with
`cargo test --locked -p daw-app --test midi_generate_e2e`; it needs no display,
audio device or installed plugin.

`midi_note_entry_e2e` exercises the shared drawing-length picker, remembered
Last note duration, drag-to-length gestures, the Clip end marker and drawing
beyond it, including one-step Undo and trimmed/stretched ghost source timing.
`midi_generation_memory_e2e` checks per-clip session choices after apply/cancel,
independent visual Grid scale, restored editor view, **Next range**, and bounds.
`novice_clip_e2e` covers editable Properties Start/Duration with move/trim and
Undo, Add pattern from the toolbar and context menu, empty-lane guidance, and
sparse Arrangement Fit row sizing.

`novice_library_e2e` distinguishes absent VST3 inventory from filtered results,
opens Plug-in folders and restores its caller, clears filters, and verifies
unassigned-instrument silence guidance without changing notes or history.
`novice_recovery_e2e` reaches Choose instrument directly from Clip Editor and
Audio setup from a device-error notice, preserving the caller and project.

```sh
cargo test --locked -p daw-app --test midi_note_entry_e2e --test midi_generation_memory_e2e --test novice_clip_e2e --test novice_library_e2e --test novice_recovery_e2e
```

Headless runs disable automatic physical-device startup for held-note previews. `RILL_E2E_AUDIO_DEVICE=1` enables it for a deliberate native-device test. The ordinary suite observes audition intent through `audition_request`; a native Surge renderer test verifies actual sound and note release without a hardware device.

Headless automation and app unit tests also disable automatic native plugin preview capture; `RILL_E2E_PLUGIN_PREVIEWS=1` (set by `Harness::with_plugin_previews`) enables it for a test that supplies its own editor worker through `DAW_PLUGIN_HOST`. Scanning remains metadata-only and cached previews still display. **Capture preview** and **Refresh preview** remain explicit actions for opted-in native tests; normal desktop use still captures previews automatically. `library_e2e` verifies that merely scanning and drawing frames leaves previews missing, while an explicitly requested capture of its metadata-only fixture fails safely without executable plugin code.

The `project_io_e2e` target covers background bundle saving and loading, later edits remaining dirty, failure preserving the previous archive/current document, Quit waiting for a save, deduplicated waveform preparation, and rejecting oversized WAV declarations before allocation. On Unix it uses a small OS pipe to hold a project read open while exercising real menus, then verifies ordered Save/New completion and outgoing recovery. It needs no device, display, or installed plugin.

`document_guard_e2e` covers New/Open/Quit and native close requests, Cancel/Escape preserving the previous workspace, blocked background edits, native picker cancellation, failed-save retry, and Save completing the actual archive before a transition. It exercises global Save/Open shortcuts from Properties and long Unicode project names in Light/Dark at 1×/2×. `document_guard` exposes the current intent, phase and error; `file_dialog_pending` reports an unconsumed picker result. The `--screenshot ... --view unsaved-changes` preview allows native rendering of the shared decision dialog with an isolated configuration.

It also checks Unicode names and Select All after completed/cancelled pickers.
`cargo test --locked -p daw-ui --test native_dialog_menu` checks the macOS native
menu key equivalent against an AppKit text view and verifies cancellation,
nested-scope and unwind cleanup. It runs on the process main thread without
opening a window, taking focus, or changing the clipboard. On Linux/Windows the
same target checks the platform-neutral callback behavior; actual native picker
keyboard behavior still requires macOS verification.

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

The `clones_e2e` suite creates MIDI/audio ghost copies through menus, edits
notes and Unicode names from different members, and checks chain indicators.
It exercises independent duplication, detaching, original deletion, local
moving/trimming/splitting, undo/redo, shared gain/fades/reverse/normalization,
and portable save/reopen after deleting the source WAV. An analytic rendered
sample oracle verifies the shared audio edits. App interaction regressions also
cover live note drags, quantize preview/cancel/apply, and rejected boundary edits.

```sh
cargo test --locked -p daw-app --test clones_e2e
```

`midi_clipboard_e2e` delivers native Copy/Cut/Paste events through the running
piano roll, including duplicate key events, text-field focus, menu copying,
relative note timing and velocities, and Undo/Redo. Shift-click selection toggles
must leave notes and history untouched while Shift-drag still duplicates.
`audio_common_e2e` also checks that **Duplicate selection** and **Duplicate clip**
name the active scope, with rendered sample checks in both appearances.

`song_workflow_e2e` builds an eight-bar song from three original generated WAV
parts and an unassigned MIDI guide. It draws notes, repeats linked patterns,
varies an independent ending, balances channels, and checks Undo/Redo. After
saving a portable project and deleting the original sources, a fresh app process
must export the same finite, unclipped 16-second audio with sound in every bar.
Set `RILL_SONG_ARTIFACT_DIR` to retain the app-produced `.rill` and `.wav` files.

`quiet_current_e2e` makes a second eight-bar song in 3/4 at 90 BPM, using audio
range variations, precise fades and channel balance, then checks portable recall.
The opt-in `cut_current_e2e` makes a third song entirely from four original audio
loops at 105 BPM. Normal UI gestures isolate and rearrange 93 drum hits and bass,
keys and chord cues; Slip, reverse, fades and precise channel gains complete its
eight-bar arrangement. Byte checks retain all four originals and verify complete
embedded WAVs, followed by identical export from a fresh process. Set
`RILL_FOUR_LOOP_SOURCES` to the four source loops and `RILL_SONG_ARTIFACT_DIR` to
retain the project, mix and intermediate checkpoints.

`tool_hints_e2e` switches every supported arranger, MIDI and audio tool using
pointer and keyboard input. A compact summary stays visible by default; the
mouse-accessible **Tool help** disclosure reveals complete gesture and shortcut
instructions. Both states, including automation guidance, must stay readable in
narrow windows without changing selection or history. `waveform_scale_e2e` checks Default/Fit against actual painted meshes,
stereo balance, fractional trims, faded and very quiet audio, unchanged exports
and source bytes, and warm caching with hundreds of short visible clips.

```sh
cargo test --locked -p daw-app --test midi_clipboard_e2e --test audio_common_e2e --test song_workflow_e2e -- --test-threads=1
```

`control_roles_e2e` checks filled commands, persistent tool selection, and
outlined presentation controls in both appearances at narrow and wide sizes.
Commands operate on notes without changing the chosen tool; highlighting,
folding, and Fit preserve the project and history. Shared `control_roles` tests
also check native radio accessibility, keyboard activation, disabled behavior,
and style isolation. Audio editor tests resize through 800–1400 points, requiring
gain labels and controls to stay together, then enter precise dB/fade values and
Undo each edit.

Parameter-search regressions keep modified arrows and Enter out of list
navigation, including when modifiers are released before repaint. Shared timeline
tests nest a horizontal-only canvas in a real scroll area to check vertical-wheel
delegation, while automation E2E verifies that navigation preserves points,
selection, and project history.

`mixer_navigation_e2e` starts middle-drags directly over Mixer gain and pan,
checks that press/held/release preserve every channel and selection, and verifies
horizontal scrolling without a press jump or release fling. Primary fader edits
and Undo remain available. Both themes and display densities are covered.

```sh
cargo test --locked -p daw-ui --test control_roles --test timeline --test parameter_browser
cargo test --locked -p daw-app --test control_roles_e2e --test audio_editor_e2e --test automation_e2e --test parameter_search_e2e -- --test-threads=1
```

`components_e2e` checks shared controls and popup navigation at the 1100×720 minimum viewport in both
appearances and at 1×/2× display density. It exercises settings choices, text
fields, toggles, library search/filtering, and returning between focused views;
Cancel preserves the active settings and project. Surface tests verify bounded
mesh work, clipping, transparent edges, and unchanged interaction/layout. Shared
menu regressions cover fast nested-menu clicks, leaf selection, keyboard access,
Escape, and outside-click dismissal. Existing MIDI scenarios also enforce a
compact wrapped toolbar at a narrow desktop viewport.

`popup_close_e2e` clicks native popup close controls after text entry and checks
that Properties, Instrument, Automation, and Library restore their caller in
both themes and densities. It derives the click from rendered title and window
bounds (`window_rects` in driver snapshots). Shared component tests also click
the complete close target, including its corners, on small and large windows.

```sh
cargo test --locked -p daw-ui
cargo test --locked -p daw-app --test components_e2e
cargo test --locked -p daw-app --test popup_close_e2e
```

Layout regression coverage also includes eight long Unicode plug-in folders in
`setup_layout_e2e`, long effect/channel names with reorder and routing actions in
`channel_layout_e2e` (including twenty-insert growth/removal), and an 80-device catalog in `components_e2e`. Search and
creation actions remain fixed while scrolling or using arrow-key selection.
Join checks distinguish ghost instances by their visible one-based beat ranges;
mixer checks align master/channel controls and verify vertical overflow access.
These automated UI regressions are separate from an interactive desktop walkthrough.

`properties_e2e` checks focused Clip/Track/Channel pages across both themes and densities,
all track types, linked gain/fade edits, independence, and undo/redo. Valid stored
gains and latency above UI ranges survive loading and opening Arrangement,
Properties, Clip Editor, and Mixer without dirtying the project; Routing tests
cover the same rule for sends. Automation tests cover shared navigation/Fit,
one-based numeric point edits, point gestures and undo, bounded controls, and
refitting a reopened lane with the same ID but different content.

`export_feedback_e2e` verifies visible completion with long Unicode filenames,
Copy path through native clipboard output, dismissal, navigation, failure/retry,
and no project/history edits. It validates the rendered WAV as well. State
includes `exported_file`, `clipboard_text`, and `automation_editor` geometry for
observing these interactions. Clipboard diagnostics observe output emitted by
the headless app; they do not read the operating system clipboard.

`export_options_e2e` opens File → Export WAV, checks the displayed duration,
toggles Include effect tails, and validates an exact 60-second WAV versus the
default release tail. Popup and destination-picker cancellation preserve the
project and caller. Its opt-in native test uses `RILL_TEST_VST_INSTRUMENT`
(Surge XT) to verify both choices against an actual isolated plugin tail;
ordinary container runs require no installed plugins.

`novice_documents_e2e` saves and reopens through the actual menus, checks saved
filename/dirty identity and picker defaults, and exercises extensionless,
uppercase and legacy project filenames. It covers missing-instrument WAV
warnings, cancellation and explicit continuation, trimmed/empty MIDI exclusion,
mute/solo eligibility, normal audio export and named MIDI completion with Copy
path and MIDI reimport. Rendered WAV oracles require healthy audio to remain
nonzero and unassigned MIDI-only output to remain silent. State exposes
`project_title`, `file_dialog_defaults` and `export_missing_instruments` for
observing those decisions; native picker presentation still needs desktop tests.

```sh
cargo test --locked -p daw-app --test novice_documents_e2e --test export_options_e2e --test export_feedback_e2e
```

The vendored RFD Linux Zenity regression runs separately with
`cargo test --locked -p daw-app --test native_picker_arguments`.
It checks directory+basename construction without changing the portal's separate
fields; native Save as and WAV/MIDI picker checks additionally accept a
basename-only edit beside a saved project in a directory containing spaces.

`arranger_tools_e2e` exercises the common audio/MIDI toolset through real pointer and keyboard events: selected-result behavior for repeated Duplicate/Cut, Join and Ghost copy, both trim edges, Shift snap bypass, held Shift-duplicate, Stretch/Undo/return-to-origin, and stretch→trim→split composition. It verifies local ghost timing, linked/independent source edits, versioned portable recall, joined WAV duration/pitch, editor tool reset, and device-free audio continuity while stretching. `live_edit_e2e` additionally stretches MIDI and audio while retaining instrument/effect sandbox identities, sustained MIDI voices and continuous PCM, including while the pointer remains held.

```sh
cargo test --locked -p daw-app --test arranger_tools_e2e --test live_edit_e2e
```

The headless driver's default label click prefers an active utility over an identically named inactive workspace control (for example the Join tool behind the Join popup), while explicit occurrence indexing remains available.


`audio_pitch_e2e` changes transpose and processing engine through the running audio editor and Properties. It measures exported pitch, exact duration and stereo relationships for both bundled engines, checks Reset, ghost sharing and independent duplicates, Undo/Redo, archive/restart recall, and combined arranger stretching plus transposition. `live_edit_e2e` also changes the pitch and engine of sounding audio while checking continuous output, retained instrument/effect workers, sustained MIDI voices and unchanged transport/queue generation.

The piano-roll suite checks independent scale/chord highlighting and scale-only rows, view changes without source/history edits, folded drawing/dragging/resize/audition/velocity, hidden-note safety for selection and deletion, and Fit/root changes with every note hidden. Short-note body dragging and fast copy/paste are regression cases. Shared component tests cover distinct action/choice/view menu styles in both themes, nested settings, Unicode accessible names, compact wrapping, keyboard dismissal and disabled controls.

### Audio comping and progressive preparation

`audio_gestures_e2e` covers body moves, both trim edges, independent Shift-copy,
time-range split/trim, pitch-preserving stretch, marker navigation, transient and
clip-edge magnet snapping with Shift bypass, and presentation-only view options.
Rendered sample and pitch oracles verify the gestures' musical results.

`audio_takes_e2e` imports alternatives through the actual Takes popup and native
picker result boundary, selects whole takes and time ranges, checks both smooth
comp boundaries against exported samples, splits a comp without changing audio,
undoes and archives retained alternatives, layers independent overdubs, and
changes takes through a running virtual audio callback. Cancelled/failed imports
must leave the project and undo history unchanged. The headless
`audio_take_files` command supplies only the next picker result (`paths`: array or
null); it performs no edit itself.

`live_edit_e2e` uses long tonal audio to exercise preview/refinement with the chosen
Signalsmith and Bungee engines while checking retained instrument/effect process
identities and musical continuity. Engine tests distinguish proxy/full cache
entries, reject cancelled/stale refinements, verify exact duration/pitch and stereo
relationships, assert allocation-free swaps, and keep offline exports at full
quality. None of these tests opens a native desktop or physical audio device.

### Audio input recording and dummy devices

Run the focused subprocess suite with:

```sh
cargo test --locked --release -p daw-app --test recording_e2e
```

`recording_e2e` uses the real Recording popup, input choices, arm checkbox,
monitoring modes, transport controls, take editor, settings, and document guards.
Its input boundary supplies deterministic samples to the production engine's
callback queues and WAV writer. Tests check exact mono/stereo samples, separate
armed channel routing, recording beyond an empty arrangement, Undo/Redo, archived
sources after restart, and session-only arm/monitor reset. They verify that dry
monitoring follows channel and bus controls while recorded samples remain raw,
and that Auto changes behavior between stopped, playback, and recording states.

Loop scenarios check complete passes, partial final passes, and the audible
complete take's full duration. The tonal comp oracle records C3 and D3 passes,
selects four ranges in the actual Takes popup to form C3/D3/C3/D3, exports WAV,
and compares samples against an independent expected signal. It checks hard
boundaries, smooth complementary crossfades, and a custom Bézier spline. Other
scenarios verify overdub layers preserve old audio and routing, Pause finalizes
once, New waits before the unsaved decision, and device/sample-rate changes finish
capture before reinitializing. A test protocol host verifies that stopping only
recording retains backing instrument/effect processes and transport continuity.

The dummy input/output devices are explicit and independently driven. Configure
them with `dummy_audio_device`, arm through the UI, and wait for
`recording.phase == "recording"`. `feed_input` queues interleaved frames without
advancing playback; `pump_audio` then consumes them. `pump_input` combines both
for ordinary scenarios. Each injection must contain complete frames, at most
1,048,576 sample values and at most 480,000 frames. Input queue capacity is 8,192
frames, so a larger unconsumed `feed_input` intentionally exercises overflow.
The writer queue is bounded to 16,384 frames; `pump_input` allows it to drain
between large offline chunks so fast test execution does not manufacture a disk
overload.

Independent-clock cases select the fourth of four inputs, vary callback delivery
sizes, and assert that feeding input alone does not record or move output. With
128 input and 256 output latency frames, tests require a 384-frame delayed tail,
exact loop splitting, and preserved samples after Stop. Continue delivering that
tail and pumping output until phase becomes `idle`; inspecting UI frames alone
does not advance dummy audio clocks. Input overflow and explicit input/output
failures must report details, retain valid audio, and allow another recording.
Engine unit tests separately cover partial first passes, rounding at 44.1 kHz,
callback allocation, queue bounds, and finalization. These checks run in a container and do not verify
microphone permission prompts, physical round-trip latency, or hardware drivers.

### MIDI input, pre-count and metronome

```sh
cargo test --locked --release -p daw-app --test midi_recording_e2e
```

`midi_recording_e2e` selects **Rill virtual MIDI input** in the real Recording
popup and sends timestamped MIDI bytes through the production input path. It
checks channel filtering, exact note starts/durations/velocities, chords,
same-pitch retriggers, velocity-zero note-offs, sustain release, and notes held
when recording stops. Delayed callbacks queued after the last output pump keep
their original note timing, including behind a future event; timestamps at or
after Stop stay outside the take. Loop recording splits held notes at the loop
boundary and merges every pass into a new MIDI clip while preserving existing
clips. Sample and plugin-message checks also require held live voices to remain
audible across multiple wraps without extra note-on/off messages. **Stop
recording** preserves held **Monitor On** voices during both MIDI and audio-only
capture while backing playback continues; actual key release ends monitoring.
Transport Stop, seeking and input reset retain their existing voice-reset behavior. Undo,
portable project recall, input failure/retry, and document/device transitions
exercise the same capture finalization used by audio recording.

The temporary protocol instrument produces a known level for each pitch and
velocity. Tests check real monitored/output samples and sandbox MIDI messages,
plus exact exported chord/overdub audio. This verifies live input reaches the
isolated instrument; it does not claim native VST or physical MIDI-driver
compatibility. No controller, audio hardware, desktop, or installed plugin is
required.

Combined audio/MIDI scenarios test no pre-count, one bar, and two bars. During
pre-count the cursor stays still and neither audio nor MIDI enters the take.
Notes first pressed during pre-count remain excluded even if released later;
events on the first capture frame are included. Count-in clicks are audible
even when the ordinary playback Click toggle is off. MIDI audition is suspended
during pre-count while the timeline is prepared; monitoring resumes when capture
starts. Audio dry monitoring remains available. Sample oracles check beat
boundaries, stronger downbeats, clean audio at the capture boundary, and absence
of clicks in exported WAV files. Recording into already-running playback skips
pre-count, and changing Click while playing preserves plugin instances.

The virtual port ID is `rill-virtual-midi`. Select/arm it through the UI and wait
for `recording.midi.input_ready` before sending `midi_input`. Offsets are in
output frames, not UI frames or wall-clock milliseconds; queue messages in time
order during ordinary playback. To simulate delayed native delivery, send
`{"command":"midi_input","bytes":[128,60,0],"delay_frames":2400}` after pumping
the output: its timestamp is 2,400 output frames before the current dummy clock.
`delay_frames` and `offset_frames` are mutually exclusive, default to zero, and
are limited to one minute. Stop tests deliberately combine a future event with
delayed events to check finalization independently of queue timestamp order.
For pre-count, wait for `recording.phase == "counting_in"` and continue
pumping the dummy output until capture begins. `pump_audio` accepts up to 8,192
frames per command, so longer musical spans use multiple calls. These fixtures
exercise deterministic musical timing and recovery, not hardware latency.

### Common audio editing

```sh
cargo test --locked --release -p daw-app --test audio_common_e2e --test audio_silence_e2e --test audio_playback_edit_e2e
```

These running-app suites exercise range Cut/Copy/Paste/Duplicate/Delete,
track-local ripple deletion, source slip, zero-crossing snap with Shift bypass,
and shared Grid/1 ms/1 sample nudging. Rendered WAV oracles verify surviving
samples and silent gaps, including reversed stereo sources. Core regressions
also check moved/stretched take alignment and overlap priority after ripple edits.

Strip silence tests preview controls and retained spans, modal cancellation and
background input blocking, full silent-clip removal, Undo/Redo, and portable
project recall. The live-edit test selects, copies and cuts a future range while
the dummy device plays: transport and engine generation must stay unchanged,
with every output sample matching the edited song. Native egui clipboard-event
coverage complements the keyboard-driven E2Es. See
[audio editor coverage](AUDIO_EDITOR.md) for controls and remaining advanced tools.

### Spectral waveform previews

```sh
cargo test --locked --release -p daw-app --test spectrum_e2e
```

The fixtures contain alternating low and high tones, including opposing stereo
channels. Tests wait for background results and check time/frequency summaries,
then match spectral texture IDs to actual egui paint commands in the arranger,
focused audio editor, neighbouring clips and take thumbnails. Both themes keep
the spectral layer faint. Reverse, trim and stretch tests check the source-frame
mapping, and ghosts and repeated views must reuse one source analysis.

The dummy device verifies that analysis and view changes leave transport,
engine generation and output samples intact. The project, undo history and WAV
export also remain unchanged. Reload checks require a new cache generation and
only the replacement document's sources. Worker counters verify background work
was submitted without requiring an artificial minimum pending duration on fast
machines. These tests use generated WAV files and need no desktop, audio device,
installed plug-in or network connection.


## Returning-user feedback regressions

`numeric_replacement_e2e` uses mouse multi-click, typing and a mouse commit to replace complete tempo, gain and generator-range values without Select All. Shared `numeric_entry` tests also cover a coalesced click/text frame, native dragging and deferred commit.

`instrument_targets_e2e` keeps instrument assignment bound to its intended existing track, preserves intentionally empty tracks when explicitly adding a new instrument, and opens clips through Pencil double-click or their visible Edit action. `document_context_e2e` covers remembered first-save folders across New/restart, cancellation/failure/deleted-folder fallback, and consistent saved-file diagnostic identity.

`generation_workflow_e2e` checks full requested bar extents with shortened notes/rests, sequential additions without closing Generate, separate Undo steps, ghost/source bounds, maximum beat limits, all twelve root choices and explicit view-only grid-scale sync.

`offline_processing_e2e` runs a real native fixture that requires VST3 offline mode: a slow first block succeeds while a hung block remains bounded and preserves an existing WAV. `render_error_e2e` retains the independent live-playback timeout/isolation checks. `export_retry_e2e` drives File export and Retry through the running app, verifies retained mix/options after later edits, and checks that document switching waits for either completion or an explicit failure decision. `export_run` diagnostics expose filename, original document identity, tails choice, rate, failure and elapsed time.

```sh
cargo test --locked -p daw-app --test numeric_replacement_e2e --test instrument_targets_e2e --test document_context_e2e --test generation_workflow_e2e --test offline_processing_e2e --test export_retry_e2e --test render_error_e2e -- --test-threads=1
cargo test --locked -p daw-ui --test numeric_entry
```

`waveform_refinement_e2e` checks the immediate raw preview, subsequent exact source
peaks, and actual antialiased mesh/fringe geometry in Arrangement, Clip Editor,
neighbouring clips and takes. It verifies single-sample transient preservation,
gain/reverse/trim/stretch mapping, bounded visible-column work, cached redraws
without PCM access, and identical playback/export. A silent source window with
loud samples immediately outside both boundaries stays silent in forward and
reverse previews and WAV output. `inspect` with `paint: true` reports compact
`painted_meshes` summaries; ordinary snapshots include `waveform` cache/stage
observations. No test substitutes a diagnostic flag for actual mesh submission.

```sh
cargo test --locked -p daw-core waveform
cargo test --locked -p daw-ui --test waveform_refinement
cargo test --locked -p daw-app --test waveform_refinement_e2e --test spectrum_e2e --test audio_takes_e2e -- --test-threads=1
```

The optional `daw-ui` `waveform_bench` example compares bar drawing/tessellation
with rebuilt and cached envelopes at 1× and 2× density. It excludes source queries
and FFT work, so it measures painting rather than total application frame time.

### Phrase, cue and mouse-only workflows

`arranger_selection_e2e` covers cross-track marquee and time-block selection,
mouse Add/Remove, group move/copy/cut/duplicate/delete, preserved rests and
partial source windows, retained takes, Slip/fade handles, history cancellation
of held gestures, and nested folder Fit selection/Previous view.
`batch_audio_fades_e2e` checks mixed audio/MIDI selections, unchanged original
WAVs and unselected/interior PCM, musical fade preservation, idempotence,
Apply/Remove/Cancel, Undo/Redo and portable recall. Core/engine boundary fade
checks cover reverse, stretch, comp regions and live decoded-PCM reuse.

`cue_editor_flow_e2e` exercises header and View navigation, arrow selection,
retained tool/zoom, bounded grid preview/splitting, all-cue result selection,
exact exported PCM and source bytes. `precise_clip_editing_e2e` covers mouse
Rename and F2, exact Start/Duration/Source offset, explicit Trim/Stretch, ghosts,
retained sources, text-field arrows, Undo, save and recall.

`navigation_selection_e2e` checks notes, folded piano keys and automation
points: plain/Shift arrows select, modified arrows edit, mouse-only scattered
selection and group deletion, text/modal guards, auto-reveal, Fit selection and
Previous view. Shared `selection_navigation`, `curves` and `timeline_views`
checks verify deterministic ordering, focus handling and complete pan recall.

`finishing_actions_e2e` covers mouse-only channel batch routing, cycle rejection,
phrase repeat with rests and partial final windows, bounded allocations,
re-export after successful and failed attempts, remembered tail options, and
reset on Open. `time_signature_e2e` chooses 3/4 entirely through the UI, verifies
Undo/Redo and portable recall, and disables the choice during live capture.

## Empty workspaces and first actions

`empty_workspace_e2e` exercises the empty arranger's starting actions through the
running application. It checks layout at 800×600 and at 2× display scaling in
both appearances, opens and dismisses the instrument chooser without creating
tracks, and opens global AI examples without sending or replacing an existing
draft. Native picker results are supplied with `file_dialog` and `kind: "import"`;
Cancel, WAV import, MIDI import and mouse-driven Undo still run through the
production import flow. Imports retain source files and create exactly one
musical edit. The suite also protects direct file dropping and double-click
pattern creation in the blank canvas, and verifies that deleting the channel
being edited returns to the empty arranger without an extra history entry for
the navigation.

```sh
cargo test --locked -p daw-app --test empty_workspace_e2e --test novice_clip_e2e -- --test-threads=1
```
