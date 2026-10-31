# Three additional mouse-only usability cycles

**All three additional rounds are complete: 15 fresh independent simulated
beginners saved projects and exported exactly 30-second pieces containing both
audio and native VST instruments.** Final corrections follow their frozen reports.
The work builds on the requested rebase to fetched `origin/main` at
`b6910155f17e3dfbf1cbb84dcafeb0366e6e7253`.

## Method

Each cycle uses five newly spawned, independent simulated beginners with no
inherited conversation, previous findings, source code or peer records. They
receive the same five musical personas and task: compose a 30-second piece with
audio and MIDI through a native VST, choose a name if possible, save the project,
and export WAV and MIDI using only the mouse. The records contain observable
actions and brief reactions, not private reasoning transcripts.

The native viewport, stock audio, laboratory sine VST, mouse driver and virtual
audio settings remain fixed: 1440×1000, 48 kHz, 1024 requested frames and 60 ms
Pulse buffering. Each participant starts with fresh configuration and cache.
No builds or diagnostic app sessions run alongside the five participants. Old
idle display/audio services were removed before round 4; this laboratory change
is recorded as a comparison limitation against earlier rounds. Round 5 and 6
receive the same task text through individually assigned prompt files; the
dispatched messages, full instructions and file hashes are preserved.

Each round freezes executable, helper, plugin, complete source hashes and
reconstruction files before launch. Reports and ratings are preserved before
corrections. Independent checks inspect saved project/MIDI structure and compare
each original WAV against isolated audio and native-VST renders. Original files
are never rewritten for verification. Nonlinear processing requires a separate
full-routing comparison; a failed linear check is not treated as an app defect.

These are agent simulations, not human research. No participant is credited with
audible quality judgments without listening. Session durations include tool and
reporting overhead; software graphics, five concurrent apps and a virtual backend
prevent interpreting these timings as a desktop performance benchmark.

## Ratings

Scores are 1–5. Higher means more of the named quality; **higher confusion is
worse**. The comparison baseline is the preceding fresh study, round 3.

| Category | Round 3 baseline | Round 4 | Round 5 | Round 6 |
| --- | ---: | ---: | ---: | ---: |
| Easy to use | 3.4 | 3.8 | 4.0 | 4.0 |
| How cool it looks | 4.0 | 4.0 | 4.0 | 4.0 |
| How fast it is | 3.4 | 3.6 | 4.0 | 4.0 |
| How confusing it is | 2.8 | 2.4 | 2.0 | 2.2 |
| Feature completeness | 4.0 | 4.0 | 4.0 | 4.2 |

## Round 4: observe, correct, validate

All five saved a named project and exported exactly 30.000 seconds with verified
audio and native-VST contributions. Their 411 mouse input events and 176 screenshots
span an average of 356.4 seconds per session. The maximum sample reconstruction
error was 1.49×10⁻⁸. All 45 earlier project/WAV/MIDI files and current originals
were unchanged. [Aggregate and individual records](../artifacts/usability-2026-10-31-round4/aggregate.json)
retain the evidence; the [feedback matrix](../artifacts/usability-2026-10-31-round4/feedback-resolution.json)
responds to all 48 explicit findings and additional rating reservations.

The five corrections are:

- **Active-tool guidance.** All five needed to discover that toolbar Duplicate
  selects a persistent tool. The shared toolbar now shows its next gesture,
  such as “Click a clip to duplicate,” in available space. Immediate context-menu
  commands retain their behavior. Captions do not move controls or the canvas.
- **Mouse number entry.** Timing and tempo fields now offer **123** for exact
  values. The shared keypad keeps a draft until Apply, rejects invalid values,
  preserves stored precision and creates one Undo entry. Cancel and unchanged
  Apply are no-ops. Generator Start/Length use an integer variant for whole bars.
- **Stable generation actions.** The first successful addition had moved Add
  notes and Next range. A shared footer reserves feedback space from the start;
  the options panel uses the available viewport while keeping actions reachable.
- **Explicit playback recovery.** Fatal output notices offer **Retry playback**,
  which replaces a failed stream at its current position. It is guarded during
  loading, document work, export and capture finalization. Stale or unrelated
  notices cannot pause or restart a healthy stream. Raw backend errors remain.
- **Recent export confirmations.** **File → Recent exports** retains the current
  document's five most recent successful WAV/MIDI destinations and their paths.
  Later MIDI feedback no longer removes the ability to inspect an earlier WAV
  confirmation. Failures and cancellations add no success receipt; the primary
  result and last-WAV re-export destination retain their existing semantics.

Automated checks caught and corrected a nested keypad menu closing its owner,
small-window generator overflow and a content-dependent sizing feedback loop.
A generator test's coarse scroll steps also skipped a reachable control; recorded
geometry justified changing the test to approach the visible target in bounded
steps while retaining reachability, geometry, note and Undo assertions. The
component gallery was regenerated and its examples checked for clipping.

[Validation](../artifacts/usability-2026-10-31-round4/validation/verified-results.json)
combines the previous full baseline with the latest affected-target results,
counting each target once: **1,077 passed, 0 remaining failures, 18 ignored across
212 targets**. Workspace build, formatting and diff checks passed. Failed attempts
and corrected command-selection errors remain in the logs. The round 4 ratings
precede these corrections; round 5 evaluates the resulting build.

Remaining observations are explicit: two sessions encountered ALSA EIO, and one
recovered through transport actions; this backend failure has not been eliminated.
Native GTK dialogs briefly painted incompletely in three reports. Some trims
needed correction against snap, and short clips abbreviate long names at wider
timeline views. Exact numeric entry addresses one precision route, but does not
establish that every drag is intuitive. Export elapsed time remains available;
there is no estimated completion time. A report of ineffective File-menu clicks
during export remains unconfirmed rather than declared fixed by export receipts.

## Round 5

All five again saved projects and exported exactly 30.000-second WAVs plus MIDI.
All audio/VST contributions were independently verified, including stretched
audio in participant 4's piece. There are 370 mouse events and 150 screenshots;
mean session duration is 317.6 seconds, including observation overhead. All 60
earlier output files and the current originals remained unchanged.
[Aggregate and session evidence](../artifacts/usability-2026-10-31-round5/aggregate.json)
and the [47-finding matrix](../artifacts/usability-2026-10-31-round5/feedback-resolution.json)
retain the unchanged reports.

Every participant independently rated ease, looks, speed and completeness 4/5,
and confusion 2/5. The inline Duplicate gesture was explicitly useful; participant
4 used the duration keypad. This round did not directly exercise the generator
or Recent exports. The participant who recovered playback used Setup and Play,
so that success is not credited to the new Retry button. Those controls retain
their automated validation without invented participant coverage.

Three correction groups follow from the reports:

- **Short edge gestures.** Participants reported four- and five-pixel corrections
  doing nothing. The arranger's combined click/drag response waits for more than
  six pixels or a long hold before beginning a drag. Dedicated edge gestures now
  retain body selection behavior while making these small trims work. The
  correction preserves snap, source bounds, fade priority, group selection and Undo.
  A stationary press does not round an existing off-grid endpoint.
- **MIDI release position.** Participant 4 saw matching Pencil drags produce
  different note lengths. Source inspection found that active gestures updated
  only while the button remained down, then cleared on release. A final movement
  delivered with release was omitted. Applying that final position before cleanup
  addresses a defect consistent with the report; no raw native frame trace proves
  it caused that particular session's discrepancy.
- **Playback-loop scope and clip identity.** “Loop clip” was mistaken for a copy
  command. Visible playback-specific wording clarifies its scope; Repeat to loop
  end remains the independent-copy command. Full names already appear on hover,
  so repeated occurrences need track and timing context rather than automatic
  renaming or more permanent timeline labels.

The frozen round 5 executable failed all three new MIDI-release regressions and
five of seven precise-trim cases; the corrected executable passed those same
targets. The failures include a short edge drag being treated as a double-click
and a stationary hold rounding an off-grid boundary. Additional validation caught
a complete fast gesture arriving in one frame and a menu-dismissal click reaching
the custom trim handler. A tooltip assertion was corrected only after recorded
geometry proved a linked icon occupied the entire narrow title width; full hover
identity and text whenever there is room remain required. Failed attempts and
previous-build regression proofs are retained.

One input limit remains explicit: if the entire press/move/release arrives in a
single frame and ends outside the original edge hit area, egui supplies no
accepted edge target. The app leaves the document unchanged rather than inferring
ownership through another control. Ordinary outward drags and batched corrections
that retain an accepted edge target are covered; universal coalesced-input support
is not claimed.

[Integrated validation](../artifacts/usability-2026-10-31-round5/validation/verified-results.json)
combines the previous full baseline with the latest complete affected targets:
**1,091 passed, 0 remaining failures and 18 ignored across 217 targets** (including
a zero-test documentation target). Workspace build, formatting and diff checks
passed. The round 5 ratings above precede these changes; round 6 evaluates the
resulting frozen build.

One participant encountered a fatal output error and recovered through Audio
setup; its backend cause was not established. Another reported a nonfatal dropout
warning while the playhead and meters continued. The original report calls that
a failure; the coordinator distinguishes continued playback without changing the
report. GTK paint delays remain observed. Clicking Stop outside project rename
dismissed its popup; reopening restored access. That conventional outside-click
behavior is retained, with no evidence of lost committed data.

## Round 6: final observations and correction

All five again saved named projects and exported exactly 30.000-second WAVs plus
MIDI. Independent renders verified audio and actual native-VST contributions,
including participant 4's stretched MIDI occurrence. This round recorded 353
mouse events and 145 screenshots, with a mean of 263.2 seconds per session
including observation overhead. All 75 preceding output files and the current
originals were preserved. The [aggregate](../artifacts/usability-2026-10-31-round6/aggregate.json)
and [feedback matrix](../artifacts/usability-2026-10-31-round6/feedback-resolution.json)
retain the original reports, rating reservations and responses.

The final fresh sessions directly exercised the Duplicate gesture caption,
small trim corrections, mouse naming, duration entry and finishing workflow.
Participant 4 recovered visible playback with **Retry playback** after a fatal
output failure; the later playhead and both meters progressed. Participant 5
inspected **Recent exports**, providing fresh observable coverage beyond the
automated checks. No audible quality judgment follows from that evidence.

One concrete inconsistency remained: participant 4 selected Stretch for a
four-beat pattern, entered 56 beats in a keypad advertising a broad general range,
and received the engine's 25%–400% source-speed rejection only after Apply.
The final correction makes the visible Duration range depend on Trim/Stretch,
the occurrence's current source window and the remaining timeline space. The
same bounds govern the number, steppers and keypad; the engine remains the final
validator. This correction follows the final ratings and receives automated
validation, not an invented additional beginner score.

The other findings have explicit responses. Fine trims can still require a
second snapped adjustment; all final pieces reached the requested duration.
Long names remain abbreviated at narrow zoom, with full names, track and timing
on hover. Piano pitches already reveal on hovering a row or the keyboard before
drawing. Naming retains individual character entry and explicit Apply; Apply is
available while the keyboard is open. Tempo retains visible steppers, dragging
and exact entry; a wheel-only attempt was not successful. The export panel shows
the current sample rate and stereo 32-bit-float format; sample rate comes from
Audio setup, and no additional PCM format options are claimed. Native GTK paint
delays and the intermittent virtual-backend output failure remain limitations.
For the File/MIDI export complaint, both relevant sessions clicked File and then
the expected MIDI row before taking a screenshot. Neither captured File alone.
The existing regression verifies File opens while WAV runs, whereas its disabled
MIDI entry dismisses the menu when clicked. That sequence explains the captures
without establishing a whole-menu failure or claiming a fresh native reproduction.
The [read-only evidence review](../artifacts/usability-2026-10-31-round6/validation/export-menu-evidence-review.json)
preserves that distinction; the regression is included in final validation.

[Final validation](../artifacts/usability-2026-10-31-round6/validation/verified-results.json)
combines the prior full baseline with latest complete affected-target results:
**1,096 passed, 0 remaining failures and 18 ignored across 218 targets** (including
a zero-test documentation target). The five new Stretch regressions pass; the
frozen round 6 build fails the invalid-draft case as expected. The final app
regressions include the pending-WAV/File/MIDI sequence and all passed, as did
shared UI targets, workspace build, formatting and diff checks. Independent
read-only review found no further issue. An initial test-only out-of-viewport
drag was corrected; its failed attempt remains in the validation record.

The [final implementation snapshot](../artifacts/usability-2026-10-31-round6/final-implementation/manifest.json)
records the corrected source and binaries separately from the evaluated build.
All [142 explicit observations](../artifacts/usability-2026-10-31-rounds4-6/feedback-matrix.json)
across the three rounds, including positive findings and recovery reports, have
responses in the three matrices; additional
rating reservations and unconfirmed/environmental limitations are retained.

## Measured score changes

Against round 3, ease and perceived speed each increased **0.6 points**, confusion
improved by **0.6 points** (2.8 → 2.2), and completeness increased **0.2 points**.
Appearance stayed at 4.0. Progress was not monotonic: round 6 confusion was 0.2
higher than round 5, while completeness was 0.2 higher.

An optional equal-weight descriptive composite reverses confusion before
averaging: `(ease + looks + speed + completeness + (6 − confusion)) / 5`.
It moved **3.6 → 3.8 → 4.0 → 4.0**, a net **+0.4 points on a five-point scale**.
This is a transparent summary of the requested ratings, not a validated usability
index, percentage improvement or statistical causal estimate.

The [versioned comparison](../artifacts/usability-2026-10-31-rounds4-6/comparison.json)
recomputes scores from all 15 unchanged reports and verifies all **90 original
project/WAV/MIDI files across rounds 1–6**. The three added rounds contain 1,134
mouse input events and 471 screenshots. Their durations are retained as laboratory
observations, not app-performance claims. Each frozen source snapshot, executable,
prompt, report, output hash, correction and validation record remains linked for
review. No fourth additional study or post-correction rerating is implied.

## Publication scope

The repository contains the frozen participant reports, mouse actions, ratings,
aggregate checks, validation summaries and selected screenshots. Raw captures,
additional iteration logs, executables and generated music remain intact in the
complete local archive. Absolute `/workspace/...` links and historical paths in
JSON refer to that archive; they are not downloadable repository attachments.
