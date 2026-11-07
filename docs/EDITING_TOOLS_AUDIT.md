# Editing tools: coverage and remaining priorities

Audit date: **7 November 2026**. This is a workflow audit of the current RILL
implementation and this development batch, extending the broader
[DAW comparison](PRODUCT_DIRECTION.md). It is not a feature-parity score.

RILL already covers substantial source-preserving audio editing and conventional
piano-roll work. The useful next steps are depth in timing, performed expression,
coordinated edits and finishing sessions. Another collection of unrelated tools
would provide less value than making these existing workflows dependable.

## Evidence and scope

RILL findings come from its source, current editor documentation and running-app
tests. The relevant implementation is linked below. “This batch” identifies new
behavior, not a claim that every external device or plug-in has been tested.
Supported plug-in ports and expression capabilities still depend on the plug-in.

This audit covers audio/MIDI music production. Video import/playback, scoring to
picture, picture synchronization, ADR, reconform and film/TV post-production
workflows are permanently outside RILL's product scope. They do not count as
missing tools, deficiencies or roadmap priorities. Competitor references to
those capabilities provide context only.

Competitor references are official manuals, release notes and product pages
researched on 6–7 November. The comparison covers Reason 14, Cubase Pro 16,
Nuendo 15, Live 12, Logic Pro 13, Pro Tools 2026.11 and REAPER 7.85. Some detailed
Steinberg reference pages remain the explicitly labelled Cubase **15** manual
baseline; Nuendo 15 must not be treated as “Cubase 16 plus post.” No commercial
DAW was installed or benchmarked for this audit. Vendor screenshots were located
but their pixels were not inspectable in the previous research environment;
there is no invented firsthand visual or task-speed ranking here. The broader
comparison links those references and distinguishes them from actual RILL
captures.

## Tool coverage

| Tool or workflow | RILL status and practical boundary |
| --- | --- |
| Select, move, trim and exact positioning | **Implemented.** Both clip edges, source slip, numeric timing, grid/millisecond/sample nudge, selection-driven arrangement edits and shared two-axis navigation. Source windows remain recoverable. |
| Split, crop, clipboard, duplicate and join | **Implemented.** Clip and range operations, ghost sources, grid/transient slicing, single-track ripple deletion and audio consolidation. Consolidation excludes plug-ins and channel routing; it is distinct from Freeze. |
| Clip gain, normalize, reverse and transitions | **Implemented.** Sample-peak normalization, source-window reversal, musical fades, occurrence-local boundary fades and shaped crossfades. Applied fades retain a subtle preview; handles reveal near the pointer. |
| Explicit stretch and pitch | **Implemented.** Source-preserving clip speed/stretch and pitch shifts, with prepared high-quality processing. This is not an editable warp-marker map, automatic beat conform or vocal-note editor. |
| Silence and transient editing | **Implemented.** Bounded background previews for strip silence and transient analysis, followed by explicit source-preserving edits. Detection is not a musical groove extractor. |
| Recording and alternative takes | **Implemented.** Audio/MIDI recording, loop takes, retained source alternatives, comp regions and boundary crossfades. Separate track takes do not supply phase-locked multitrack edit groups. |
| Ordinary note editing | **Implemented.** Both note edges, cut/glue, copy/paste, duplicate, legato, velocity painting, scale/chord guidance and scale-folded rows. Different pitches and partial overlaps remain separate; ordinary creation/movement resolves same-pitch containment. |
| Whole-clip timing and pitch transforms | **This batch.** Half/Double time changes instance timing while retaining source notes; semitone/octave operations transpose shared source notes, including trimmed material. Bounds are atomic, and linked occurrences retain independent placement and stretch. |
| Selected-note mirrors | **This batch.** Horizontal flip mirrors starts within the selected time span; vertical flip mirrors selected pitches. Unselected notes and source identity remain intact. These discrete mirrors preserve alternatives rather than invoking ordinary overlap cleanup. |
| Selected-note lengths and group gestures | **This batch.** Fixed/Grow/Shrink previews use audible beats at the clip's stretch rate. Additive and edge edits stop the whole selection at its first bound, retaining relative differences. Group move/copy/delete are single undoable edits; start trimming carries the latest per-note controller state and preserves later expression timing. |
| Quantize and Humanize | **Implemented and extended this batch.** Explicit preview, Apply and Cancel for selected notes or the audible clip window. Grid strength/swing and optional note ends are distinct from repeatable velocity/timing variation. Trimmed source material outside the chosen window remains retained. |
| Loop the object being edited | **This batch.** “Loop this clip” uses exact audio/MIDI region or automation-pattern bounds, independently of an unrelated arrangement selection. The loop change is undoable. |
| Multi-clip note editing | **Partial.** The arranger edits multiple clips; the piano roll edits one active clip/source. There is no simultaneous piano-roll editing of several independent clips across tracks. |
| Controller and note-expression editing | **This batch, with explicit transport boundaries.** The model adds bounded UMP containers, high-resolution MIDI 2.0 note data and per-note expression. Storing/editing that data is separate from a platform's hardware MIDI transport and a plug-in's supported event types. It must not be advertised as universal MIDI 2.0, MPE, controller or articulation-map compatibility. |
| Automation | **Implemented.** Gain, pan, sends and plug-in parameters, editable interpolation, clear manual override/resume and shared manual/tool/AI history. This batch adds input gain and the real Master path. Performed Touch/Latch/Write automation is a separate missing workflow. |
| Master, sidechain and instrument output handling | **This batch.** Real Master processing, additional plug-in input routes and independent instrument output channels. Collapsible groups organize the mixer; hiding a strip must not disable its DSP. This remains a stereo-channel graph, not an immersive speaker-layout system. |
| Freeze | **This batch.** Print source material and channel insert processing to new audio below the source, retain and mute originals, and restore them through the retained-source workflow. Volume/pan/sends remain live. This explicit print-to-a-new-track policy differs from hosts that temporarily replace an instrument track internally. |
| Level and phase diagnostics | **Implemented and extended this batch.** Calibrated sample-peak meters and holds, 300 ms exponential RMS, spectrum, stereo correlation/phase and K-weighted LUFS. The new Levels view separates sample peaks from oversampled true-peak estimates. These are engineering measurements with stated scope, not a certified broadcast-compliance claim. |
| Export and music-session hand-off | **Partial.** Exact-loop/arrangement WAV, PCM/float, independent rate and PCM dither are present. Freeze is not a batch stem-delivery dialog, and portable RILL projects do not provide cross-DAW music-session exchange such as DAWproject. Film/TV interchange is outside this audit's product scope. |

Implementation and executable workflow coverage:

- [Audio editor scope](AUDIO_EDITOR.md), [clip editors](EDITORS.md),
  [source-aware clip model](../crates/daw-core/src/model.rs),
  [whole-clip dispatch](../crates/daw-app/src/clip_actions.rs),
  [piano roll](../crates/daw-app/src/midi_editor.rs).
- [Musical clip-action E2Es](../crates/daw-app/tests/midi_clip_actions_e2e.rs),
  [batch duration and group gesture E2Es](../crates/daw-app/tests/midi_batch_edit_e2e.rs),
  [piano-roll refinements](../crates/daw-app/tests/piano_roll_refinement_e2e.rs),
  [precise clip edits](../crates/daw-app/tests/precise_clip_editing_e2e.rs),
  [recording](../crates/daw-app/tests/recording_e2e.rs),
  [takes](../crates/daw-app/tests/audio_takes_e2e.rs).
- [MIDI 2.0 data boundaries](../crates/daw-core/src/midi2.rs),
  [on-demand measurements](../crates/daw-engine/src/signal_monitor.rs),
  [reference measurement tests](../crates/daw-engine/src/signal_monitor_reference_tests.rs),
  [live analyzer E2Es](../crates/daw-app/tests/signal_analysis_e2e.rs).

## What to borrow from each reference

| Reference | Useful established idea and the appropriate RILL interpretation |
| --- | --- |
| **Ableton Live 12** | Pitch/time utilities keep stretching, half/double timing, reverse/invert and legato near the note selection. RILL's new clip and note actions adopt these ordinary musical operations, while retaining an explicit distinction between the whole clip and selected notes. Live's groove/warp and multi-clip workflows remain substantially deeper. [MIDI editing](https://www.ableton.com/en/live-manual/12/editing-midi/), [audio timing](https://www.ableton.com/en/live-manual/12/audio-clips-tempo-and-warping/). |
| **Reason 14** | The sequencer connects clip selection, note editing, precise inspector values and quantization; the track-centric layout keeps device context nearby. Borrow that continuity, and make the scope of each command visible. RILL need not reproduce the virtual rack. [Sequencer functions](https://docs.reasonstudios.com/reason14/sequencer-functions), [Reason 14 workflow](https://www.reasonstudios.com/press/reason-studios-introduces-reason-14). |
| **Cubase Pro 16** | Rich quantization, musical context and structured parameter editing reduce repetitive manual corrections. RILL should extend its existing preview and scale tools to deepen musical editing. The detailed reference remains the 15 manual; newer 16 additions come from release notes. [Cubase Pro 15 manual](https://www.steinberg.help/r/cubase-pro/15.0/en), [Cubase 16 changes](https://www.steinberg.net/cubase/release-notes/16/). |
| **Nuendo 15** | Retained alternatives, precise range editing and mature edit/automation history are useful in a music application. Its video, ADR, picture reconform and film/TV interchange workflows serve a different audience and are outside RILL's product scope, not missing editor requirements. [Nuendo manual](https://www.steinberg.help/r/nuendo/15.0/en), [post features](https://www.steinberg.net/nuendo/features/). |
| **Logic Pro 13** | The MIDI Transform workflow makes batch musical edits explicit and reusable, and inspectors expose note/region values without moving between unrelated windows. RILL's previews and precise fields follow this principle. A mature transform/preset system remains broader than a few flip and transpose commands. [MIDI Transform overview](https://support.apple.com/en-mz/guide/logicpro/lgcp2158341a/mac), [Logic guide](https://support.apple.com/guide/logicpro/welcome/mac). |
| **Pro Tools** | Predictable body/edge/corner editing and retained playlist alternatives matter more than the number of toolbar buttons. RILL can refine its current gestures, take recall and print workflow using these ideas. Music-session exchange and studio hardware are separate capabilities; video and film/TV post tools are outside RILL's scope. [Smart Tool](https://www.avid.com/pro-tools/user-guide/smart-tool), [edition/workflow boundaries](https://www.avid.com/pro-tools/comparison). |
| **REAPER 7.85** | Precise item properties, several MIDI event views, a large action system and deep render choices illustrate the value of extending existing operations consistently. RILL should preserve discoverability and one authoritative operation behind menu, context and tool actions rather than reproducing every customizable action. [Current user guide](https://dlx.reaper.fm/userguide/html/), [official feature overview](https://www.reaper.fm/about.php). |

These references establish available workflows. They do not prove that one DAW
is faster, easier or more reliable for a matched RILL task.

## Remaining gaps, in practical order

1. **Performed automation and expressive capture.** Finish dependable
   Read/Touch behavior and round-trip controller/note-expression preservation
   across recording, editing, playback, Undo and export. New data containers or
   a native parameter list alone do not complete that workflow.
2. **Coordinated editing.** Add clear edit groups and phase-coherent multitrack
   operations when recording multiple microphones. Folders, ghosts, bus groups
   and instrument-output groups solve different problems and are not substitutes.
3. **Timing depth.** Develop groove extraction/templates, audio quantization and
   explicit warp-marker editing around the existing transient and stretch tools.
   Tempo/meter maps require a deliberate project time-model change; a global
   tempo value plus clip stretching cannot provide equivalent behavior.
4. **Finishing and media continuity.** Extend Freeze with understandable source
   ownership and asset lifetime, missing-media relink and measured larger-session
   behavior. Add aligned stem/range batch delivery before specialist interchange.
   Build on bounded disk streaming and portable Save; measure long-session cache
   preparation, storage performance and source lifetime before adding cleanup.
5. **Musical context and repeatability.** Reusable transform/groove presets,
   clear section markers and better simultaneous clip inspection can reduce
   repeated edits. Validate these through complete song-editing tasks before
   adding another permanent panel.

Integrated vocal-note correction, spectral repair, score notation and immersive
music delivery are separate specialist capabilities. They should be driven
by an explicit audience need rather than a feature-count comparison. The next
release should demonstrate one coherent song workflow, correct recall and
recoverable edits on declared hardware and plug-ins.
