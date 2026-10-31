# Latest-main rerun — five fresh mouse-only beginners

See the [three additional rounds and final score comparison](USABILITY_2026-10-31_ROUNDS4-6.md) for the subsequent study.

**All five participants saved a named project and exported an exactly 30.000-second
WAV and MIDI file.** Independent sample checks establish both audio and actual
native VST contributions in every WAV. All 30 project/export files from the two
previous studies remain unchanged.

These are five new, independent **simulated beginners**, not human participants.
Their ratings do not show a general ease-of-use improvement. Feature completeness
rose to 4.0/5; playback failures still affected three sessions. After preserving
their reports, the recoverable-underrun defect and the additional editing problems
below received targeted corrections. Those subsequent changes have not been
assigned new beginner ratings.

## Rebase and build provenance

The feedback work was backed up and stashed, the branch rebased onto fetched
`origin/main` at **`b6910155f17e3dfbf1cbb84dcafeb0366e6e7253`**, then the changes
reapplied. Conflict resolution retained main's contextual toolbars, Notes and
Draw settings menus, typography, grouped transport, and track/header polish.
All 25 original findings have responses in the
[resolution matrix](../artifacts/usability-2026-10-31-round2/feedback-resolution.json).
The [integration record](../artifacts/usability-2026-10-31-round3/feedback-integration.json)
links the additional corrections and regression targets.

The rebased study executable was frozen before dispatch:
`35a1c098fcc2bf79c11e59724df2cf24cf8e4913b221f44a33a83a0f8e5a1933`.
[Setup](../artifacts/usability-2026-10-31-round3/setup-manifest.json),
[source hashes](../artifacts/usability-2026-10-31-round3/source-hashes.json),
[tracked patch](../artifacts/usability-2026-10-31-round3/source-diff.patch), and the
[untracked source archive](../artifacts/usability-2026-10-31-round3/untracked-frozen-source.tar.gz)
reconstruct that evaluated build. Later implementation fixes are identified
separately from these frozen results.

## Method and comparison limits

Five agents started with `fork_turns=none`: no prior discussion, previous findings,
source code or peer records. Each received only a persona, the task and neutral
mouse-driver instructions. They used real 1440×1000 native windows, screenshots,
clicks, double-clicks, drags, scrolling and pointer movement. No keyboard events,
shortcuts, composition-state injection or project-file editing were allowed.
The logs contain observable intentions, actions and reactions, not private
reasoning transcripts.

Each session had fresh configuration/cache and its own paced Pulse/ALSA virtual
audio endpoint: 48 kHz, 1024 requested frames, 60 ms backend buffering. The stock
starter audio was unmodified. The same actual **Rill Lab Sine VST3** was available;
its simple laboratory interface does not test third-party plugin compatibility.
The host uses software graphics and four shared CPU cores. No builds or diagnostic
app sessions ran alongside the five participants in this round.

Naming was explicitly encouraged in rounds 2 and 3. This round also added held
mouse-button screenshots, allowing users to inspect feedback during a drag;
those down/move/up events can increase gesture counts. Elapsed time includes
screenshots and reporting pauses. These timings and subjective speed scores are
not hardware benchmarks. No participant auditioned audio, so audible quality and
musical success beyond the saved composition structure remain unassessed.

## Frozen ratings and outcomes

Scores are 1–5; higher means more of the named quality. **Higher confusion is worse.**

| Category | Original | Round 2 | Rebased fresh test |
| --- | ---: | ---: | ---: |
| Easy to use | 3.6 | 3.4 | 3.4 |
| How cool it looks | 4.0 | 4.0 | 4.0 |
| How fast it is | 4.2 | 3.2 | 3.4 |
| How confusing it is | 2.8 | 2.8 | 2.8 |
| Feature completeness | 3.4 | 3.6 | 4.0 |

| Participant / piece | Ease | Looks | Speed | Confusion | Completeness | WAV duration | Session / input events |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| 1 — night drift | 3 | 4 | 3 | 4 | 4 | 30.000 s | 7m57s / 131 |
| 2 — Neon Pocket | 4 | 4 | 4 | 2 | 4 | 30.000 s | 6m54s / 88 |
| 3 — moon glass | 4 | 4 | 4 | 2 | 4 | 30.000 s | 6m28s / 79 |
| 4 — ember rise | 3 | 4 | 3 | 3 | 4 | 30.000 s | 7m15s / 92 |
| 5 — velvet steps | 3 | 4 | 3 | 3 | 4 | 30.000 s | 7m30s / 92 |

There are **482 logged mouse input events and 182 screenshots**, averaging
**7m12.8s** between first and last input. All five WAVs are exactly 1,440,000 stereo
frames at 48 kHz. All projects contain embedded audio, notes and the native VST
assignment; MIDI exports match notes overlapping the saved clip windows.

Instrument assignment, note drawing, mouse naming and named save/export defaults
worked in all five sessions. Held-drag timing feedback was now observable and
helped participants reach precise endpoints. No participant reported the pattern double-click failure from
round 2. Two participants completed with no reported playback
failure; three saw output-device failures, one of whom recovered after choosing
the named laboratory output.

## Feedback responses after the frozen study

| Finding | Evidence | Response |
| --- | --- | --- |
| Recoverable output underrun treated as permanent stream loss | Participant 5 captured “Backend: A buffer underrun or overrun occurred”; participants 1 and 4 also reported output failures without establishing the same cause. | CPAL's ALSA output worker automatically prepares and restarts after typed `Xrun`. Ordinary playback now keeps the same stream and reports a coalesced dropout warning/count. Fatal EIO, unclassified errors and disconnection remain terminal. Recording and count-in still stop conservatively to protect capture integrity. |
| Play did nothing after unchanged Audio setup Apply | Participant 4 repeatedly clicked Play after Refresh/Apply. | Explicit Play now retires the failed stream and opens a fresh one at the current cursor. It preserves project, history and selection; the engine's terminal-fault latch remains intact. |
| Dragging loop endpoints had no effect | Participants 2 and 5 tried the visible strip; one found Range → Loop arrangement. | Visible start/end grips now support mouse dragging, snap/off, live timing, bounded positive ranges and one Undo per gesture. Clipped edges never masquerade as actual endpoints; editing is disabled during recording. |
| Erase after Cut unexpectedly deleted both selected pieces | Participants 1, 4 and 5 reported whole-region removal; participant 4 demonstrated the retained split-pair selection and recovered with Undo. | Arranger Erase now deletes only the pointed clip. Explicit context-menu Delete retains bulk selection behavior. Audio and MIDI regressions verify survivors, selection and Undo/Redo. |
| First Stretch shifted Properties and subsequent clicks missed | Participant 1's screenshots show the newly appearing Stretch row. | The readout is always present, initially 100.0%, keeping timing controls stable through repeated mouse edits. |

Other observations were assessed rather than converted into speculative changes:

- **Duplicate discoverability:** all five needed to distinguish a persistent tool
  from an immediate command. Existing Tool help explained clicking a clip and
  participants then succeeded. Group duplication remains intact; direct mouse
  menu duplication remains available. The report does not treat an unsuccessful
  Duplicate-tool drag as a successful copy.
- **Tempo and waveform length:** changing tempo does not automatically warp audio.
  Participant 1 completed the manual Trim/Stretch workflow. Automatic tempo-follow
  remains a separate product capability; the app's existing stretch operation and
  explicit 100% readout are retained.
- **Precise trimming and MIDI miniatures:** snap can move a cursor-derived cut away
  from its intended clock time. Snap/off, explicit loop duration and live timing
  offer precise mouse alternatives. MIDI trimming preserves source note timing;
  regression tests distinguish it from stretching.
- **Native file-dialog paint delay:** two participants saw a temporarily black GTK
  picker before controls appeared. Their files saved successfully. This remains
  an observed platform/lab rendering delay, not a claim about normal desktops.
- **Export wait:** some WAV exports took tens of seconds while five software-rendered
  apps shared the host. Progress/completion and serialization were visible; MIDI
  export waits for an active WAV export. Independent isolated renders completed
  quickly afterward, but that is a different workload and does not establish a
  user-facing performance improvement.

The earlier captured ALSA EIO and the separate historical plugin failure are not
reclassified as recoverable underruns. No claim is made that all backend or
third-party plugin failures are solved.

## Verified pieces and evidence

Analysis-only copies were rendered with the frozen executable and native plugin
helper: one retained only audio, another only MIDI/VST. Their sum matches every
sample of each participant's original WAV within **1.5×10⁻⁸**. All samples are
finite, both components are nonzero, and participant files were never rewritten.
See [component proof](../artifacts/usability-2026-10-31-round3/technical/isolated-midi/mix-component-verification.json)
and [aggregate evidence](../artifacts/usability-2026-10-31-round3/aggregate.json).

| Piece | Local archive outputs | Session record |
| --- | --- | --- |
| night drift | [WAV](</workspace/scratch/rill-usability-round3/session-1/night drift.wav>) · [Project](</workspace/scratch/rill-usability-round3/session-1/night drift.rill>) · [MIDI](</workspace/scratch/rill-usability-round3/session-1/night drift.mid>) | [Report](../artifacts/usability-2026-10-31-round3/participant-1/report.json) · [Reactions](../artifacts/usability-2026-10-31-round3/participant-1/reactions.txt) · [Mouse log](../artifacts/usability-2026-10-31-round3/participant-1/mouse-actions.jsonl) |
| Neon Pocket | [WAV](</workspace/scratch/rill-usability-round3/session-2/Neon Pocket.wav>) · [Project](</workspace/scratch/rill-usability-round3/session-2/Neon Pocket.rill>) · [MIDI](</workspace/scratch/rill-usability-round3/session-2/Neon Pocket.mid>) | [Report](../artifacts/usability-2026-10-31-round3/participant-2/report.json) · [Reactions](../artifacts/usability-2026-10-31-round3/participant-2/reactions.txt) · [Mouse log](../artifacts/usability-2026-10-31-round3/participant-2/mouse-actions.jsonl) |
| moon glass | [WAV](</workspace/scratch/rill-usability-round3/session-3/moon glass.wav>) · [Project](</workspace/scratch/rill-usability-round3/session-3/moon glass.rill>) · [MIDI](</workspace/scratch/rill-usability-round3/session-3/moon glass.mid>) | [Report](../artifacts/usability-2026-10-31-round3/participant-3/report.json) · [Reactions](../artifacts/usability-2026-10-31-round3/participant-3/reactions.txt) · [Mouse log](../artifacts/usability-2026-10-31-round3/participant-3/mouse-actions.jsonl) |
| ember rise | [WAV](</workspace/scratch/rill-usability-round3/session-4/ember rise.wav>) · [Project](</workspace/scratch/rill-usability-round3/session-4/ember rise.rill>) · [MIDI](</workspace/scratch/rill-usability-round3/session-4/ember rise.mid>) | [Report](../artifacts/usability-2026-10-31-round3/participant-4/report.json) · [Reactions](../artifacts/usability-2026-10-31-round3/participant-4/reactions.txt) · [Mouse log](../artifacts/usability-2026-10-31-round3/participant-4/mouse-actions.jsonl) |
| velvet steps | [WAV](</workspace/scratch/rill-usability-round3/session-5/velvet steps.wav>) · [Project](</workspace/scratch/rill-usability-round3/session-5/velvet steps.rill>) · [MIDI](</workspace/scratch/rill-usability-round3/session-5/velvet steps.mid>) | [Report](../artifacts/usability-2026-10-31-round3/participant-5/report.json) · [Reactions](../artifacts/usability-2026-10-31-round3/participant-5/reactions.txt) · [Mouse log](../artifacts/usability-2026-10-31-round3/participant-5/mouse-actions.jsonl) |

In the complete local archive, every cited screenshot is preserved beside its participant report. Full screenshot
sequences remain in the original session directories. WAVs are self-contained;
reproducing a native project requires the supplied lab VST.

## Validation

Before the final fresh test, the complete app suite plus shared UI/engine suites
and corrected/affected reruns produced **1,049 passing tests, no remaining failures,
and 18 optional tests ignored across 209 targets**. This combines full-suite
coverage with focused reruns; historical failed attempts remain in their logs.
[Rebased validation](../artifacts/usability-2026-10-31-round2/validation/rebased-verified-results.json)
records each target's final result and the corrected command-name typo.

After the study's corrections, the combined full-suite baseline and latest
affected-target reruns contain **1,061 passing tests, no remaining failures, and
18 optional tests ignored across 211 targets**. Each target is counted once in
the [final validation record](../artifacts/usability-2026-10-31-round3/validation/verified-final-results.json).
The final workspace build, formatting and diff checks passed; all 18 light/dark
component-gallery captures were regenerated. A linker attempt exhausted disk
space before running tests; removing generated compiler intermediates allowed
the affected tests and final build to complete. That environment failure remains
recorded beside the successful logs.

A separate [native backend replay](../artifacts/usability-2026-10-31-round3/technical/xrun-replay/final-comparison.json)
verified the underrun correction using actual ALSA/Pulse output and the native
VST, without synthetic error injection. After a controlled 250 ms process stall,
the backend dropout count increased from 0 to 1 and the same engine continued
from 60,016 to 310,200 consumed frames, with no terminal output fault or render
errors. A 4.116-second post-resume recording contained finite, nonzero PCM. The
frozen pre-fix build had latched a real underrun as fatal and emitted silence.

Three earlier revised-build replay attempts encountered startup ALSA EIO before
consuming frames. Those attempts remain preserved; the successful underrun replay
does not establish recovery from EIO. Its initial playback was healthy, so it did
not exercise explicit Play retry. Running-app tests separately verify Play after
a terminal fault, including Refresh → unchanged Apply → Play, with a fresh stream
and preserved cursor, project, history and selection.

The [final build manifest](../artifacts/usability-2026-10-31-round3/final-build/manifest.json)
identifies the resulting executable, complete source hashes and reconstruction
files. These post-study fixes have regression and native-backend evidence, but
the five participants did not rate this final build. Their ratings above remain
unchanged.

## Publication scope

The repository contains the frozen participant reports, mouse actions, ratings,
aggregate checks, validation summaries and selected screenshots. Raw captures,
additional iteration logs, executables and generated music remain intact in the
complete local archive. Absolute `/workspace/...` links and historical paths in
JSON refer to that archive; they are not downloadable repository attachments.
