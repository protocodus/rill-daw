# Audio editing in RILL

RILL's audio editor and arranger share tools, timeline navigation, snapping and
clip operations. This is a coverage audit of common editing workflows, not a
claim of feature parity with every established DAW.

## Common workflows

| Workflow | RILL behavior |
| --- | --- |
| Select, move, trim | Select (`Q`), drag the body to move or either edge to trim; Shift-drag the body creates an independent copy. |
| Cursor and time selection | Cursor (`P`), click to seek or drag across audio to select time. |
| Cut, copy, paste, duplicate, delete | Cmd/Ctrl+X/C/V/D and Delete operate on selected time, or the complete clip when no time range is selected. The Clip and Edit menus use the same commands. Paste starts at the cursor; Duplicate starts at the selection end. Cut and Delete leave a gap. |
| Ripple delete | Clip → Ripple delete selection removes selected time across audible clips on the selected audio track and moves later clips left. Other tracks stay in place; one Undo restores the complete edit. |
| Split and crop | Cut (`E`), Split at cursor, Split selection, Trim to selection, Set start/end and Snap end. Source audio stays intact. |
| Slip source contents | Slip (`L`), drag the waveform inside fixed clip edges. Grid snapping applies to the movement; Shift bypasses it. Movement stops at the source file boundaries. Reversed and stretched clips retain their direction and source mapping. |
| Precise positioning | Nudge menu: Grid, 1 ms or 1 sample. Left/Right arrows move the selected clip in the arranger and audio editor. One sample uses the configured output sample rate. With the grid off, Grid nudge uses a quarter beat. |
| Snap | Grid plus optional clip edges and transient markers. Optional Source zero crossings searches within 5 ms after grid snap for a quiet common stereo frame. Shift bypasses all snapping. |
| Gain and fades | Audio clip gain, normalization, fade-in/out and Linear, Equal power or S curve shaping. Mixer pan and gain remain channel properties. |
| Crossfades | Optional overlap crossfades and spline controls, including take comp boundaries. The newest-starting audio region wins when crossfading is off. |
| Time and pitch | Pitch-preserving stretch (`S`), transposition and selectable Signalsmith or Bungee processing. |
| Transients | Energy-rise or amplitude-gate analysis, sensitivity and minimum spacing, visible preview, marker navigation and splitting within a selected range. |
| Spectral preview | A faint frequency diagram appears beneath audio waveforms in the arranger, audio editor, neighbouring clips and take thumbnails. Background analysis and a shared source cache keep editing responsive. |
| Strip silence | Process → Strip silence previews retained regions before applying a source-preserving cut. Threshold, minimum silence and padding are adjustable; selected time limits the operation. |
| Takes and comping | Imported and recorded take groups, retained originals, selection of whole takes or selected regions, and crossfades at comp boundaries. |
| Recording and overdubs | Audio input selection, arming, monitoring and loop recording into separate takes. |
| Join, duplicate and ghosts | Join selected material into a new clip. Independent copies retain the selected source window; ghosts share source edits while keeping occurrence placement, trim and slip local. |
| Navigation | Middle-button drag or Alt+drag to pan; shared zoom gestures and Fit (`F`) show all clip content on both axes. |
| Recall and undo | Audio edits preserve source files and project state. Each clipboard command or drag is one undo transaction. |

Range edits preserve reverse, stretch, source offsets and fade timing. Cutting a
comp region does not remove its retained takes, and cutting a ghost occurrence
does not cut the other occurrences. Copy/paste and independent duplication
detach ghost and take membership. A new pasted region has no inherited overlap
crossfade; transitions can be added explicitly at its new position.

Ripple editing keeps original takes retained. Comp regions moved across the
deleted time receive a separate take group with aligned source windows, so
choosing another take continues to use the correct passage. Newly joined
boundaries use a hard cut; add a crossfade when needed.
If conforming an alternate would exceed the supported stretch range, its
original remains retained in the original group instead of blocking the ripple
edit; the new group includes the compatible candidates.

Zero-crossing snapping is off by default. It uses the source waveform, with one
frame for all channels, and does not sum stereo channels when judging quietness.
Pitch/stretch processing may alter the final waveform. Use explicit fades or
crossfades when a guaranteed smooth transition matters. Transient snapping and
source analysis do not change the underlying audio file.

## Strip silence

Open Process → Strip silence from the audio editor. Adjust Threshold (dBFS),
Minimum silence (ms) and Padding (ms); background analysis updates the tinted
retained regions over the original waveform. Apply becomes available only once
the current preview is ready and there is silence to remove. Cancel or Escape
leaves the project untouched.

Analysis measures source amplitude before clip gain, fades and pitch processing;
any channel above the threshold keeps that frame, including anti-phase stereo.
Minimum silence and padding use playback time, including reversed and stretched
clips. A time selection limits the edit; otherwise it covers the whole clip.
Gaps remain in place, and all retained pieces keep their source offsets,
processing and original fade timing. Other ghost occurrences and retained take
alternatives remain unchanged. One Undo restores the complete operation.

## Spectral waveform preview

Audio waveforms keep their amplitude outline and add a subtle spectral underlay:
low frequencies appear at the bottom and high frequencies at the top. The same
shared component appears in the arranger, the focused audio editor, neighbouring
clips and take thumbnails, in both light and dark themes. No extra panel or
analysis action is needed.

File reading and spectral analysis run on a background worker. The waveform is
available while the spectral image is pending. Ghosts, repeated clips and views
reuse one source analysis; clip trimming, reverse and pitch-preserving stretching
map that cached image to the correct source passage. Transposition shifts its
frequency display. Loading another project invalidates the previous generation
so a late result cannot attach itself to the new document.

The diagram pools overlapping analysis windows across the complete source into
an overview with 48 logarithmic frequency bands and at most 1,024 time columns.
It uses fixed level scaling,
rather than brightening every quiet section independently. It does not represent
the output of inserted effects or provide spectral repair. The preview changes
neither source audio nor project history, playback or exported sound. Memory and
background work are bounded; cache entries can be regenerated when revisited.

## Verification

The container-compatible running-app tests in
`crates/daw-app/tests/audio_common_e2e.rs` cover range clipboard operations,
silent gaps, track-scoped ripple deletion, exact rendered samples, reverse stereo slipping, cursor/cut/trim
zero-crossing snap, Shift bypass, shared sample/millisecond/grid nudge, one-step
undo, project recall and narrow/wide layouts. Existing audio editor, gesture,
pitch, takes and recording suites cover the remaining workflows above.
`crates/daw-app/tests/audio_silence_e2e.rs` additionally checks preview-before-apply,
parameter changes, modal cancellation, stereo and reversed rendered samples,
ghost preservation, one-step undo/redo and portable project recall.
`crates/daw-app/tests/spectrum_e2e.rs` covers source frequency/time summaries,
actual spectral image paint in every waveform view, both themes, reverse/trim/
stretch mapping, shared-source reuse, project reload, and unchanged history,
playback samples and WAV export.

Run the focused suite with:

```sh
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test --locked --release -p daw-core --test audio_common
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test --locked --release -p daw-app --test audio_common_e2e -- --test-threads=1
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test --locked --release -p daw-app --test audio_silence_e2e -- --test-threads=1
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test --locked --release -p daw-app --test spectrum_e2e -- --test-threads=1
```

## Remaining specialized workflows

RILL does not yet provide editable warp-marker maps, automatic audio
quantization, multitrack phase-locked comping/edit groups,
spectral repair, pitch-curve correction or a dedicated sample-level repair
pencil. Destructive file processing is also outside this editor's current scope.
These should be considered separately from
the common clip and take operations, and evaluated against actual user work.

## Reference audit

The common tools were checked against primary documentation:

- [Ableton: selecting clips and time](https://www.ableton.com/en/manual/arrangement-view/#selecting-clips-and-time) — selection-aware clipboard editing.
- [Ableton keyboard shortcuts](https://www.ableton.com/en/manual/live-keyboard-shortcuts/) and [REAPER user guide](https://dlx.reaper.fm/userguide/html/) — sliding audio contents within item boundaries.
- [Pro Tools: nudging clip contents](https://apps.avid.com/proToolsFirstHelp/version12.3/enu/Pro%20Tools%20First%20Help/ed4.clips.26.17.html) — fixed-boundary source movement.
- [Cubase: snap to zero crossing](https://www.steinberg.help/r/cubase-pro/15.0/en/cubase_nuendo/topics/project_window/project_window_snap_to_zero_crossing_c.html) — optional audio boundary snapping.
- [Cubase audio part editor toolbar](https://www.steinberg.help/r/cubase-artist/15.0/en/cubase_nuendo/topics/audio_part_editor/audio_part_editor_toolbar_r.html) — explicit nudge increments and editing actions.
- [Cubase: Detect Silence](https://www.steinberg.help/r/cubase-elements/15.0/en/cubase_nuendo/topics/audio_functions/audio_functions_silence_detect_r.html) — threshold, minimum silence, padding and visual analysis before cutting.
