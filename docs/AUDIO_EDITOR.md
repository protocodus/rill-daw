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
| Precise positioning | Nudge menu: Grid, 1 ms or 1 sample. Ctrl/Cmd+Left/Right moves selected clips; plain arrows navigate selection. Properties exposes Start, Duration (Trim or Stretch), and Source offset. One sample uses the configured output sample rate. With the grid off, Grid nudge uses a quarter beat. |
| Snap | Grid plus optional clip edges and transient markers. Optional Source zero crossings searches within 5 ms after grid snap for a quiet common stereo frame. Shift bypasses all snapping. |
| Gain and fades | Audio clip gain, normalization, fade-in/out and Linear, Equal power or S curve shaping. Mixer pan and gain remain channel properties. |
| Crossfades | Optional overlap crossfades and spline controls, including take comp boundaries. The newest-starting audio region wins when crossfading is off. |
| Time and pitch | Pitch-preserving stretch (`S`), transposition and selectable Signalsmith or Bungee processing. |
| Transients | Energy-rise or amplitude-gate analysis, sensitivity and minimum spacing, visible preview, marker navigation and splitting within a selected range. |
| Spectral preview | A faint frequency diagram appears beneath audio waveforms in the arranger, audio editor, neighbouring clips and take thumbnails. Background analysis and a shared source cache keep editing responsive. |
| Waveform amplitude | Arranger View → Waveform amplitude offers Default and Fit. Fit scales each clip's waveform to its available height, using one reference across stereo channels and the full trimmed source window. It changes the view without changing gain or audio. |
| Strip silence | Process → Strip silence previews retained regions before applying a source-preserving cut. Threshold, minimum silence and padding are adjustable; selected time limits the operation. |
| Takes and comping | Imported and recorded take groups, retained originals, selection of whole takes or selected regions, and crossfades at comp boundaries. |
| Recording and overdubs | Audio input selection, arming, monitoring and loop recording into separate takes. |
| Join, duplicate and ghosts | Join selected material into a new clip. Independent copies retain the selected source window; ghosts share source edits while keeping occurrence placement, trim and slip local. |
| Navigation | Middle-button drag or Alt+drag to pan; shared zoom gestures and Fit (`F`) show all clip content on both axes. |
| Recall and undo | Audio edits preserve source files and project state. Each clipboard command or drag is one undo transaction. |

## Editing many cues

Select (`Q`) can marquee clips across tracks; each clip's context menu offers
Add/Remove selection. Cursor (`P`) selects a time block across tracks. Duplicate
retains that block's exact duration, including trailing rests, and crops copies
to its boundaries. Group moves, copies, cuts, deletes and nudges are one Undo.

Clip Editor View → Previous clip / Next clip inspects adjacent cues while
retaining tool and zoom; Tab/Shift+Tab accelerates those commands. Clip → Slice
on grid previews up to 2,048 cuts before Split grid. Grid and transient splits
keep all resulting cues selected for subsequent group actions. View → Fit
selection and Previous view avoid repeatedly rebuilding the song overview.

Edit → Smooth edit boundaries applies short, occurrence-local fades with the
last accepted settings (initially 3 ms in / 5 ms out). Existing musical fades and
their inherited source anchors remain unchanged; already protected edges need
no additional fade. Boundary fade settings provides exact 0–100 ms controls,
Apply, Remove boundary fades and Cancel. Applying the same protection twice is
idempotent. Apply and Remove each use one Undo and never rewrite source audio.

Edit/context Rename opens an item-anchored name field with Apply and Cancel;
F2 is the shortcut. Properties → Clip provides exact Start (one-based beat),
Duration and Source offset values. Duration defaults to Trim and offers an
explicit Stretch choice. Source offset moves audio inside fixed clip edges and
is limited to the original source window.

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

Save and export reject destinations that are audio sources referenced by the
current project, including muted clips and retained takes. This includes symbolic
links and hard links to those sources. Choose a separate project or mixdown file.
Portable projects embed the complete referenced WAV files, preserving audio
outside the edited clip windows.

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

## Progressive waveform preview

Audio waveforms keep their amplitude outline and add a subtle spectral underlay:
low frequencies appear at the bottom and high frequencies at the top. The same
shared component appears in the arranger, the focused audio editor, neighbouring
clips and take thumbnails, in both light and dark themes. No extra panel or
analysis action is needed.

View menus offer spectrum **Intensity** from 0–100% and **Monochrome** or
**Blue–yellow** palettes. Zero hides the spectrum while retaining the waveform.
These session view settings apply across the arranger, audio editor and takes;
they change neither audio nor project history. Both palettes reuse one analysis,
and intensity changes only the painted opacity.

An immediate coarse waveform remains usable while a background worker prepares
source peaks, then the spectral image, from one decode. Peaks retain single-sample
transients and stereo channel differences. The refined outline is a continuous
envelope with a one-physical-pixel antialiasing fringe; at close zoom it becomes a
thin trace. No new control or analysis action is needed.

Ghosts, repeated clips and views reuse one source analysis. Trim, reverse, stretch,
gain and fade changes update the displayed passage; gain and fades are represented
at display resolution. The spectral underlay remains a source-frequency overview,
with transposition shifting its frequency display. Loading another project rejects
late results from the previous document. Reimport or reopen a changed source to
replace its loaded audio and previews together.

Only visible pixel columns are prepared, capped at 4,096. Unchanged views reuse
ready mesh geometry without scanning PCM again. Peak and spectral analysis share
a 32 MiB cache and one worker; view geometry has a separate 8 MiB / 512-entry cache.
These budgets are additional to the existing decoded-source cache. Under memory
pressure a coarse preview remains available; analysis does not enter a retry loop.

Arranger Fit uses a cached full-clip amplitude reference shared by stereo
channels. Unfaded source extrema refine with the peak cache; faded references
use 128 full-clip bins with the overview's envelope approximation. This keeps
the reference stable while zooming and panning. Up to 512 small references are
retained separately from the geometry byte budget.

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
`crates/daw-app/tests/waveform_refinement_e2e.rs` additionally checks progressive
stages, actual antialiased meshes, isolated transients, shared analysis and cached
redraws, both appearances and display densities, all waveform views, and impulses
excluded by forward/reverse trim boundaries. Core tests compare pyramid queries
against exact PCM extrema; shared renderer tests check physical edge width,
clipping, and mesh storage reuse.

Run the focused suite with:

```sh
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test --locked --release -p daw-core --test audio_common
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test --locked --release -p daw-app --test audio_common_e2e -- --test-threads=1
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test --locked --release -p daw-app --test audio_silence_e2e -- --test-threads=1
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo test --locked --release -p daw-app --test spectrum_e2e -- --test-threads=1
cargo test --locked -p daw-app --test waveform_refinement_e2e -- --test-threads=1
cargo run --locked --release -p daw-ui --example waveform_bench
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
