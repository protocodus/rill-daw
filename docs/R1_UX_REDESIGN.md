# R1 device UX redesign

The nine built-in devices use a common preset header, signal-stage headings and
output strip. The interactive plots and controls edit the same parameters.
Instruments keep the audition keyboard and wheels below the output strip.

## Current panels

| Device | Editing layout |
| --- | --- |
| Mini | Both oscillators and the source mixer share **Oscillators**. **Filter & amp** groups the filter and its envelope with the amp envelope. **Modulation** pairs the LFO with its destinations. **Voices** covers voice allocation, glide and pitch response. |
| Sampler | **Sample** covers region and loop editing. **Pitch & voices** groups keyboard mapping and voice settings. **Filter & amp** contains the filter and envelopes. **Modulation** groups the LFO and its destinations. The waveform stays visible on every page. |
| Drums | Select a pad in the kit to see its waveform, trigger, playback region, tuning, envelopes and tone controls together. The footer holds pad level, pan and mute alongside kit volume. |
| Filter | Circuit and filter controls sit below the response. The LFO and envelope follower share the next row so you can adjust both while watching the cutoff. |
| Equalizer | Choose HP, one of four bands, or LP below the response. The inspector starts with shape or slope. Frequency stays in the same position across selections. |
| Saturator | Follow the input through low bypass and drive into the waveshaper, then adjust wet gain compensation and tone before mixing with the dry signal. |
| Compressor | Mode and character sit above the transfer curve, level history and reduction meter. The sidechain, gain curve and timing share one row. Makeup, mix and output follow. |
| Modulator | Choose mode and character above the plots. The LFO controls lead into the wet path, with mode-specific center, voices or stages, feedback and low cut. |
| Spacer | **Echo** groups repeats, the delay line and the echo return. **Reverb** groups the room, tail and reverb return. Each page opens with routing and the two engine switches above the timing display. Mix and output sit below the engine controls. |

The **Randomize** menu in the preset header offers 1%, 10% and 100% variations.
Each choice creates one Undo step. [Dark panels](../artifacts/r1-ux-redesign/overview-dark.png) · [Light panels](../artifacts/r1-ux-redesign/overview-light.png)

## November 15 design record

<details>
<summary>Original proposal, implementation and verification</summary>

## Design team and agreement

Five UX designers reviewed the existing panels and source independently:

| Designer | Responsibility | Agreement |
| --- | --- | --- |
| `ux_lead` | Shared principles, feasibility and final synthesis | Approved |
| `ux_instruments` | Mini, Sampler, Drums | Approved explicitly |
| `ux_tone` | Filter, Equalizer | Approved explicitly |
| `ux_dynamics` | Compressor, Saturator | Approved explicitly |
| `ux_space` | Modulator, Spacer | Approved explicitly |

All five designers agreed to the common contract and their final device
specifications on 2026-11-15. The implementation follows that agreement.

The main problem is information hierarchy and fit. Some instrument panels put
roughly fifty parameters and the keyboard in one tall view, extending well
beyond the host popup's usable height. Other devices give setup controls the
same prominence as the sound controls. This design starts each device from its
musical task while retaining a consistent family of interactions.

## Shared principles and implementation contract

1. **Start with the sound task.** The default workspace exposes the controls
   needed to make the device useful immediately. A device gets named task pages
   only when its complexity needs them. Simple devices remain a single view.
2. **Keep context and output visible.** Presets remain in the device header.
   Plots and sample/pad context stay visible across pages. The final Output
   strip stays in the same place on every page; instruments retain their
   keyboard and wheels. Output labels distinguish sample gain, pad volume and
   kit volume where they coexist.
3. **Fit the existing popup.** The complete device frame, including preset
   header, task navigation, output and instrument performance controls, must
   fit within 620 logical points at its declared width in both themes. This
   matches the existing effect-panel test budget for the default host window.
   No device-body scrolling. Reserve the largest task's body height so page,
   mode, band and pad changes do not resize the frame or move the output.
4. **Make navigation explicit and harmless.** Use the shared `popup_tabs` /
   `tab_choice` controls. Page and band selections are UI state only: changing
   them never writes a sound parameter, creates an undo step or changes a
   preset. Pages remember their selection while that device panel is open.
5. **Make control identity stable.** Each parameter belongs to exactly one
   workspace or persistent area, except a purposeful graphic/numeric editing
   pair or a shared engine selector repeated across task pages. Keep existing
   descriptor IDs, formatting/parsing, defaults, gestures, undo batching and
   automation menus. Every graphic edit retains a numeric alternative.
   Knob captions stay visible; their minimum, maximum and current value reveal
   on hover and during a drag. Reserve readout space so nothing moves. Compact
   inline knobs reveal their value in place and their range in a hover hint.
6. **Explain inactive controls without moving them.** Preserve disabled slots
   and use concise hover reasons for model/mode restrictions. Existing effective
   readouts (Auto release, actual auto gain, limiter ratio) remain accurate.
   A disabled EQ band remains editable so it can be prepared before enabling;
   only controls meaningless for its type are disabled.
7. **Use one component family.** Reuse `daw-ui` controls, shared typography,
   colors and spacing. Add any new generic wrapper to `daw-ui` first. Prefer
   concise full task/control names; explain specialized terms on hover. Align
   mixed rows with shared row helpers. Standard knobs are preferred; dense
   knobs are appropriate where needed to meet the measured height budget.

The twelve-column allocations below express grouping and relative width.
Implementers may adjust spans and control density to satisfy measured bounds,
but must preserve the agreed parameter ownership, task order and interactions.
Plots shrink before controls become unreadable. Their axes and handles must
remain usable at the final measured size.

## Individual device designs

### R1 Mini

Two tasks: **Sound** (default) and **Motion**. Volume and the performance
keyboard/wheels remain visible. The Sound task reads as source, color and shape;
voice behavior and modulation no longer interrupt the source controls.

- Sound: compare Oscillator 1 and 2 in aligned rows (model, wave, octave,
  width; oscillator 2 also semitones, detune and sync). Source mixer has
  Osc 1, Osc 2, Sub and Noise. The next row pairs Filter (model, Cutoff,
  Resonance, Drive, envelope amount, key tracking) with Amp ADSR.
  Suggested allocations: source row 8+4, filter/amplitude row 6+6.
- Motion: Filter ADSR; velocity to filter and amplitude; voice mode, voice
  count, Glide, Bend range and Drift; LFO Shape, Rate, Sync, Pitch, Cutoff,
  Width and Wheel. Suggested first row 4+2+6, then LFO across twelve.
- Keep pulse-width dependency, mono/poly behavior, audition and wheel behavior.
  The default Sound task must contain the means to build a basic playable tone.

### R1 Sampler

Three tasks: **Sample** (default), **Shape**, **Motion**. Keep the sample name,
Load/Clear actions and a compact 80–96 point waveform visible, with markers and
playheads. Pan, Volume and the performance controls remain visible.

- Sample: Region (Start, End, Reverse, Gain); Playback (loop/playback mode,
  Loop start, Loop end, Crossfade); Pitch (Root, Transpose, Fine, Key tracking).
  Suggested spans 3+5+4. Root belongs with pitch, rather than sample trimming.
- Shape: Filter with its enable header (Type, Cutoff, Slope, Resonance,
  envelope amount, key tracking); Filter ADSR and velocity; Amp AHDSR and
  velocity. Suggested spans 4+3+5.
- Motion: LFO Shape, Rate, Sync, Pitch, Cutoff, Level, Pan; Voices (Mode,
  voice count, Glide, Bend range). Suggested spans 8+4.
- Playback-mode hover explains that no-loop gated playback follows note-off
  while One-shot plays to End. Preserve existing loop/filter dependencies,
  sample import/drop/clear and marker edits. Empty state makes Load sample
  immediately available. Filter off leaves dependent controls disabled in place.

### R1 Drums

Two tasks: **Sample** (default) and **Shape**. Keep the 4×4 pad grid with at
least 85×70-point pads on the left; stack the selected pad's waveform, sample
actions and task controls on the right. Keep
the selected pad number and sample name apparent. Persistent output contains
**Pad volume**, Pan, Mute and a separately named **Kit volume**, plus existing
performance controls.

- Sample: Region (Start, End, Reverse); Tuning (Pitch, Fine); Trigger
  (One-shot/Gate, Choke). Suggested spans 4+4+4, one control line.
- Shape: Pitch envelope (Amount, Decay); Amp envelope (Attack, Hold, Decay,
  Velocity); Tone (Filter, Resonance, Drive). Suggested spans 3+5+4.
- Preserve height-based pad audition, drop-to-pad, waveform marker editing,
  muted indication, and existing empty-pad behavior. Empty pad offers loading.
  Selecting a pad changes the editing target without changing its parameters.
  All nineteen parameters of every pad remain reachable; Kit volume is global.
- Preserve the shared sample-player tree, pad naming, composite drops, Browse,
  and sample actions for trimming, reversing, exporting, loading and clearing.

### R1 Filter

Two tasks: **Tone** (default) and **Motion**. A 160–180 point response plot
and the Mix/Output strip remain visible.

- Tone: Sound spans five cells: Cutoff (hero, two cells), Resonance, Drive,
  Morph. Circuit spans seven: Model (two), Type (four), Slope (one).
- Motion: LFO spans eight: Shape (two), Rate, Sync (three), Depth, Stereo.
  Envelope follower spans four: Amount (two), Attack, Release.
- The plot's cutoff node uses a meaningful label and a cutoff readout.
  Preserve per-model Type options. Sync is the existing Off/division choice;
  it is not a new toggle. Rate is disabled while synced. Model-inapplicable
  controls retain their places and explain their state on hover.

### R1 Equalizer

One focused workspace replaces the all-band matrix. Keep a 180–200 point
response graph, then visible **HP / 1 / 2 / 3 / 4 / LP** selection synchronized
with its nodes. Default selection is Band 1. Full names appear in accessible
labels and hover. Selecting a node or band never enables or bypasses it.

- Show one selected-band inspector: enable in heading; Frequency (primary),
  Gain, Q, and named Type. HP/LP use Frequency and Slope in the same reserved
  inspector space; do not fabricate gain/Q parameters. Prefer a named selector
  over the previous tiny matrix glyph controls.
- Mode and Analyzer remain visible as compact global controls. The persistent
  Output strip contains Auto gain with its actual compensation and Output.
- Preserve double-click enable, wheel Q/slope, context menus, Shift precision,
  and locked plot range during dragging. Disabled bands can be edited; Notch
  Gain is inactive while Q remains available. Every band parameter must be
  reachable with the mouse through selector plus inspector.

### R1 Saturator

One workspace; no unnecessary task pages. Compact the transfer/harmonic visual
to about 150 points. Make Drive the primary control.

- Sound: Model, Drive, model-specific Character, Tone.
- Refinement: **Low bypass** (the existing Focus parameter), Quality.
  Low bypass reads Off at minimum; hover explains that frequencies below it
  stay clean. Preserve the actual parameter key/ID.
- Persistent Output: Auto gain with actual compensation, Mix, Output.
- Keep model-specific Hardness/Bias/Compress meanings and all existing editing
  paths. Sound shaping must precede quality/setup in the visual hierarchy.

### R1 Compressor

Two tasks: **Shape** (default) and **Detector**. Mode stays visible independent
of task; place it beside task tabs only if measured width permits, otherwise
use a fixed compact context row. Keep transfer/history/reduction visuals at
about 150 points and persistent Mix/Output.

- Shape: Dynamics (Threshold, Ratio, Knee, Ceiling); Timing (Attack, Release,
  Auto release); Character & gain (Character, Makeup, Auto makeup).
- Detector: Sidechain source, routed-source link, High-pass, Stereo link,
  Lookahead. The routing link remains usable when External has no source.
- Mode changes do not switch tasks. Retain effective character timing,
  Auto release, limiter ratio and actual makeup readouts. Disabled controls
  explain the relevant mode/character restriction. Keep threshold-plot editing
  and reduction-hold reset. Target below 540 points, never above 620.

### R1 Modulator

One workspace with a compact 160–176 point modulation/response visual.

- Type: Mode and Character.
- Motion: Rate or Division in one stable slot, Sync, Depth, Shape, Stereo.
- Tone: Center, Feedback, Voices or Stages in one stable slot, Low cut.
- Persistent Output: Mix, Output.
- Depth receives emphasis if it fits without crowding. Replace ambiguous Wet
  grouping with Tone. Retain mode-aware captions, swapped-slot values,
  rate-dragging and response details; dormant controls stay put.

### R1 Spacer

Two tasks: **Space** (default) and **Texture**. Both Echo and Reverb remain
visible on both tasks, preserving comparison and balance. Keep a shared
140–160 point delay/decay plot. Routing, Mix and Output stay visible.

- Both tasks have Echo and Reverb lanes, six columns each, with the existing
  enable and model choices in the same header locations.
- Space, Echo: Sync (two), Time or Division (two), Feedback (two); then Duck
  (three) and Level (three).
- Space, Reverb: Pre-delay, Size, Decay (two each); then Width, Duck, Level
  (two each).
- Texture, Echo: Taps (two), Stereo (four); then Color (two), Diffusion (two),
  Wobble (one), Low cut (one).
- Texture, Reverb: Seed, Diffusion, Movement (two each); then Color, Damping,
  Low cut (two each).
- Keep stable Time/Division replacement, variation stepper, routed topology,
  disabled-engine state and all plot gestures with numeric alternatives. No
  hero knob is required. Use dense controls if two rows plus context need them
  to fit; do not sacrifice the fixed-frame requirement.
- Routing retains its disabled explanation when either engine is off. Keep
  the existing Color/Wobble mode-specific hints. Modulator mode-change
  companion values remain one undo edit.

## Coding ownership and verification

Three coding agents implement the agreed result in separate ownership areas:

1. Shared task-navigation/state/layout rendering and persistent output;
   Filter and Equalizer. This owner establishes the small shared API and
   isolates device layouts so the other owners can edit concurrently.
2. Mini, Sampler and Drums layouts and instrument-specific interactions.
3. Saturator, Compressor, Modulator and Spacer layouts.

Use per-device modules or grouped `workspaces/{tone,instruments,effects}`
modules rather than concurrent edits throughout a monolithic layout file.
Layout state needs an explicit task index and an EQ selected node; selection
must stay independent of host parameter values. The root integrator owns
running-app checks, preview generation and documentation alignment.

Acceptance is behavioral, not merely a changed screenshot:

- Enumerate every task, every EQ band and every drum pad in both themes.
  Complete frames remain within declared width and 620-point height; output
  and keyboard coordinates stay stable across task changes.
- Verify complete descriptor-parameter coverage through task/persistent
  layouts, allowing graphic/numeric pairs and mode-specific swaps.
  Instrument expectations are 44 Mini parameters, 43 Sampler parameters,
  nineteen parameters for the selected drum pad plus global Kit volume.
- Switching tasks, EQ bands or drum pads does not mutate parameter values.
  Edit a value on one task, navigate away/back and verify it remains edited;
  preserve undo, reset, value typing and automation access.
- Empty/loaded sample state does not move output or performance controls.
  Held audition notes and wheels release normally across task navigation;
  navigation retains the selected drum pad and its muted indication.
- Exercise relevant mode/model/enable branches, synced/free timing and
  inactive controls. Every reachable control retains a usable hit target.
- Run the existing device UI tests and new meaningful interaction/geometry
  tests. Follow `AGENTS.md`: new user-facing navigation/inspector functionality
  needs executable E2E coverage through the running app.
- Capture every redesigned device from the running UI and refresh existing
  device-browser preview assets to match the shipped default task.

This is a new information architecture and task layout for each device, not a
change to audio processing or a promise of new synthesis/sample features.

## Delivery verification

- 77 device UI tests cover parameter reachability, editing, all task/band/pad
  combinations, loaded/empty sample geometry and stable output/performance rows.
- 18 shared layout tests cover control alignment, including the compact row.
- Running-app workflows cover all nine devices: audio behavior, presets,
  automation, sample import, task and band navigation, Undo/Redo and save/reopen.
- 15 plug-in editor tests protect native-editor lifecycle and history behavior.
  Built-in panels now retain their task and selected band/pad on same-device
  history or tool edits while reading restored parameters from the project.
- Native light/dark captures and browser previews reflect the final default
  task for every device, including newly supplied Sampler and Drums previews.

Integration retains the stable-Rust automation-clipboard fix already on main.
The sample-player and browser additions remain available alongside task
navigation, including composite players and the larger drum-pad targets.

</details>
