# Further mouse-only usability iteration: rounds 7–9

This cycle starts from latest fetched `origin/main` at
`6029e61dd7f0a168cd562fac573c183ef00ceabd`, following the merged composition and
mixer improvements. Five fresh simulated beginners establish the baseline;
two further groups of five fresh agents repeat the task after successive fixes.

## What changed

- **Mixer controls stay in place.** Audio/export notices and new sends previously
  shortened the faders and moved Pan, Mute and Solo beneath the pointer. Strip
  height now follows the unobscured window. Extra routing lanes scroll below
  the strips, with their full visible height reserved in the outer scroll pane.
  Real resizing still adapts the controls; normal scroll-boundary clamping remains.
- **Knobs retain the last movement.** An owned primary drag now consumes its final
  release position. The result no longer depends on whether the final movement
  arrived in a held frame or with release. Fine adjustment, automation override,
  secondary/middle-button ownership, click-without-jump and one-step Undo remain.
  A wholly batched gesture that never acquired a target is outside this claim.
- **Exact values work with the mouse.** The shared parameter context menu now has
  a **123** toggle for a numeric keypad. Mixer gain/pan/send, compact headers,
  Channel Properties and plug-in normalized parameters reuse it. Opening the
  keypad preserves the exact draft and precision. On short screens only the
  keys scroll; Apply/Cancel and automation actions remain reachable.
- **MIDI notes start where the mouse was pressed.** Round 8 exposed a second
  input issue: when press and motion arrived in one frame, the piano roll used
  the latest pointer position as the gesture origin. It now retains the primary
  press event's position for onset, pitch and the existing-note target, while
  later motion and release still set the endpoint. Pressing outside the canvas
  and moving into it cannot create a note. The existing canvas/modal ownership
  rules remain; capture outside the current canvas is not broadened.
- **Adding a channel send takes fewer steps.** After round 9, the empty channel's
  **Add send** opens destination choices directly in Mixer, reusing the toolbar's
  chooser. The menu names the clicked source and preserves selection/history
  until a destination is chosen. Existing send summaries still open Routing for
  level, Pre/Post, automation and removal. This final simplification receives
  regression and native visual validation; the round 9 ratings precede it.

The mixer correction responds to four independently reported layout jumps.
The knob release and shared keypad changes came from independent review; they
are not attributed to an unobserved participant gesture or credited with an
invented rating increase. The MIDI correction follows round 8's original ratings;
round 9 evaluates that corrected build. The existing toolbar grouping, signal-flow ordering,
pan knobs and mute/solo controls remain part of the evaluated application.

## Method and scores

Each participant receives the same musical persona and task as its counterpart:
compose exactly 30 seconds with audio plus MIDI through an actual native VST,
choose a name, save, export WAV/MIDI, balance mixer volume/pan, try mute/solo,
and create a bus/send if discoverable. The agents start independently without
conversation history, previous reports or source access. All composition input
uses the mouse driver; reports contain observable actions and brief reactions.

The viewport (1440×1000), stock audio, Rill Lab Sine VST3, 48 kHz sample rate,
1024-frame buffer request, 60 ms virtual Pulse buffering, build profile and task
wording are held constant. No builds or diagnostic sessions run alongside the
participants. Protocol, source, executable, plug-in, report and output hashes
are retained. Rounds 1–6 did not explicitly request mixer work and used different
build conditions; round 7 is the comparison baseline for this cycle.

| Category | Round 7 baseline | Round 8 | Round 9 | Net change |
| --- | ---: | ---: | ---: | ---: |
| Easy to use | 3.8 | 3.8 | 4.0 | +0.2 |
| How cool it looks | 4.0 | 4.0 | 4.0 | 0.0 |
| How fast it is | 3.4 | 3.2 | 3.2 | -0.2 |
| How confusing it is (lower is better) | 2.6 | 2.4 | 2.4 | -0.2 |
| Feature completeness | 4.0 | 4.0 | 4.0 | 0.0 |

Ease improved by **0.2 points** and confusion decreased by **0.2 points**.
Perceived speed declined by **0.2 points**; looks and completeness stayed at 4.0.
Round 8 improved confusion while speed fell; round 9 then improved ease. These
results do not establish a uniform or causal improvement across all categories.

Ratings are 1–5; lower confusion is better. These are simulated ordinal
judgments, not human research, a statistical effect estimate, or a hardware
performance benchmark. Session timings include tool and screenshot overhead.
Successful exports do not establish continuous listening quality.

## Observed outcomes

All **15 participants** saved a project and exported exactly **30.000-second**
WAVs plus MIDI. Every full saved graph reproduces its original WAV sample-for-sample,
and source-mute ablations prove both audio and actual native-VST contributions.
All requested mixer operations were reported as completed through the visible UI.

| Round | Verified pieces | Mouse input events | Screenshot captures | Mean session seconds |
| --- | ---: | ---: | ---: | ---: |
| 7 | 5/5 | 468 | 203 | 362.8 |
| 8 | 5/5 | 438 | 192 | 363.4 |
| 9 | 5/5 | 424 | 174 | 341.8 |

The three rounds total 1,330 mouse input events and 569 screenshot captures.
There are 568 retained PNGs: round 9 participant 5 captured `final.png` twice before
closeout, retaining the last image; both actions remain logged. These durations
include tool and observation overhead. Round 9 verification preserved all 120
earlier project/WAV/MIDI outputs and all 658 frozen evidence files across these
three rounds, including the current originals.

Round 7's full saved-graph renders exactly reproduce the original WAVs, with
maximum absolute sample difference zero. Muting audio and MIDI sources in
separate diagnostic copies proves that both audio and the native VST contribute
to each exported mix. Saved projects and MIDI source windows are checked without
modifying the participant files. A diagnostic metadata field in the first
round-7 verification helper was corrected from `volume` to the actual `gain`;
the original helper, amendment and audited rerun remain preserved.

## Feedback disposition and remaining limitations

The matrices respond to **145 explicit observations and 75 rating explanations**:
49 observations in round 7, 47 in round 8 and 49 in round 9. Positive findings,
recovery reports, confirmed defects and remaining limitations retain their original
wording and evidence. Corrections never revise a participant's original score.

Four round-7 participants independently reported mixer controls shifting under
the pointer. Round 8 exposed the MIDI press-origin issue. Round 9 repeated the
extra Add-send step, prompting the final destination chooser. That final change
also addresses routing-step reservations retained in the earlier matrices, but
has no additional beginner rating assigned to it.

Playback failures remain unresolved in this laboratory. An independent CPAL
probe, with no RILL, UI or VST code, reproduced the same ALSA
`snd_pcm_avail_delay` I/O error after controlled callback stalls. A 20-second
unstalled run had no errors; the same stalled run failed on both default and
named devices, which resolve to the same ALSA PCM. A larger buffer request also
failed and is not a verified remedy. ALSA negotiated different actual callback
sizes from the requested sizes. This isolates one backend failure path, but
does not establish what caused each participant's stall or predict physical
device reliability. Fatal error handling and explicit recovery remain intact.

Exact edge trimming can still need a small snap correction; duration entry and
Repeat to loop end provide other mouse-accessible routes. Duplicate remains a
persistent tool with an inline next-gesture hint; immediate context/Edit commands
remain available. GTK dialogs sometimes paint late in the lab without a window
manager. These observations are retained with explicit responses, not marked as
newly fixed. Broader plug-in compatibility and untested production features are
outside this small task.

Round 9 participant 4 correctly encountered the 1–16-beat Stretch range for an
original four-beat source: repeated stretching cannot exceed the engine's
25%–400% source-rate limits. The participant completed the piece by duplicating
and trimming. Participant 5 perceived a late first note, but the pre-correction
screenshot, recorded mouse coordinates and saved notes already place C4 at
beat zero; the later Select drag starts outside that note. The original finding
and rating remain unchanged. A separate native trace audit records this
reconciliation and participant 1's correctly aligned note starts, without
claiming access to uncaptured intermediate frames.

## Verification

Before round 9, the complete Rust 1.99.1 workspace suite passed **1,447 tests,
zero failures and 25 ignored tests across 261 targets**, including documentation
targets. After the final send simplification, **160 tests across ten affected
targets** passed, counting the latest complete result for each target once.
This includes all 141 app-unit tests and the new four-case send chooser target.
The full-workspace run precedes that final polish; the affected-target results
validate the subsequent change. Final workspace build, strict all-target Clippy,
formatting and diff checks passed.

All four new send-choice tests failed on the frozen round-9 build before the
change. Two initial cancellation tests later used blind screen coordinates and
navigated to Arrangement. Their final versions derive an empty mixer-body point
from rendered geometry and verify it lies outside foreground menus, retaining
all project, history, selection and view assertions. No particular original hit
target is claimed. Existing send-summary Routing tests remain intact.

Before the fixes, three new mixer tests reproduced 50-pixel notice jumps and a
40-pixel send-induced shift; two shared and two app knob tests reproduced lost
release displacement; three initial parameter-entry scenarios failed without
mouse digits. The subsequent MIDI correction has a failing app-unit scenario
for partially coalesced events and three failing executable-app regressions for
pressed pitch/onset, existing-note movement and outside-canvas presses. Final
tests cover actual original-coordinate clicks, routing
reachability, short-window scrolling, exact drafts, invalid/cancel/no-op handling,
automation and Undo. Failed attempts remain in the evidence. A fourth mixer
test's original Undo/dismiss ordering was a test-assumption error and is not
counted as a demonstrated baseline defect. Existing route tests now scroll to
the controls while retaining their geometry, audio and history assertions.

Independent source review and native Dark/Light previews found no blockers.
The expanded keypad and scrolled send lanes are captured in both themes.
Before round 8, gallery regeneration produced 16 identical images; the remaining
two differ in their animated busy indicator. No gallery images needed replacement.
The final send chooser also passed a separate native Dark/Light review: captions
and destinations fit above their buttons, an unselected source stays correctly
named, and cancellation preserves the selected channel and Mixer. No project
was edited or saved during these visual checks.

## Evidence and publication scope

- [Round 7 reports and aggregate](../artifacts/usability-2026-11-01-round7/aggregate.json)
  and [feedback matrix](../artifacts/usability-2026-11-01-round7/feedback-resolution.json).
- [Round 8 reports and aggregate](../artifacts/usability-2026-11-01-round8/aggregate.json)
  and [feedback matrix](../artifacts/usability-2026-11-01-round8/feedback-resolution.json).
- [Round 9 reports and aggregate](../artifacts/usability-2026-11-01-round9/aggregate.json)
  and [feedback matrix](../artifacts/usability-2026-11-01-round9/feedback-resolution.json).
- [Score comparison](../artifacts/usability-2026-11-01-rounds7-9/comparison.json),
  [workspace validation](../artifacts/usability-2026-11-01-rounds7-9/validation/workspace-final-summary.json),
  [native previews](../artifacts/usability-2026-11-01-rounds7-9/native-preview/observations.json)
  and [audio diagnosis](../artifacts/usability-2026-11-01-rounds7-9/audio-diagnostics/results.json).
- [MIDI press-origin trace review](../artifacts/usability-2026-11-01-rounds7-9/press-origin/press-origin-review.json)
  and [its final validation](../artifacts/usability-2026-11-01-rounds7-9/press-origin/finalpress-origin-validation.json).
- [Round 9 native note audit](../artifacts/usability-2026-11-01-rounds7-9/press-origin/round9-note-origin-audit.json).
- [Final send validation](../artifacts/usability-2026-11-01-rounds7-9/validation/send-choice-validation-summary.json),
  [native send previews](../artifacts/usability-2026-11-01-rounds7-9/send-preview/observations.json),
  [cross-round response](../artifacts/usability-2026-11-01-rounds7-9/cross-round-send-followup.json)
  and [separate final implementation](../artifacts/usability-2026-11-01-rounds7-9/final-implementation/manifest.json).

Published evidence is copied byte-for-byte with SHA-256 manifests. It includes
unchanged reports, mouse logs, ratings, source reconstruction, verification,
validation logs and selected screenshots. Executables, plug-in binaries, original
music and analysis renders, and bulk screenshots remain in the complete local
archive. Absolute `/workspace/...` paths in the records refer to that archive,
not downloadable repository attachments. Small source tar archives contain
only untracked source needed to reconstruct the evaluated build.
