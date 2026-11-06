# Native composition study — 5 November 2026

This is a simulated usability study in which exactly ten distinct agents attempted to compose through the native Rill UI. It is not human user research. All ten used the frozen baseline commit `a5a9ce4b360ad9f303661d4e01039bcf3b7cda02`; the changes described below were developed from collected feedback while later attempts continued on that frozen executable. No second participant cohort rated the changed UI.

All ten attempts produced a saved project and exported WAV, confirmed by the artifact audit. The subsequent UI changes passed the relevant component, running-app, and native visual checks described below. The structured companion is [feedback.json](../artifacts/usability-2026-11-05/feedback.json).

## Method and evidence limits

Each participant received a private native application session, Xvfb display, virtual PulseAudio sink, settings/cache, and project directory at a 1440 × 1000 viewport. The task was to make a 30–60 second song with a beginning, development, and ending, using the supplied audio loop and an original MIDI part with the installed **Rill Lab Sine** study fixture, then edit, mix, undo a mistake, name, save a portable `.rill` project, and export WAV. The fixture is not a bundled Rill instrument and intentionally has no native editor. No provider requests were made.

Participants used logged native pointer events, typing, ordinary shortcuts, and screenshots. During composition they could not inspect application source/model data, inject projects, read peer reports, or use the headless driver. None of the ten reports used the product user guide. Post-study artifact inspection and diagnostic probes were separate phases; their findings do not retroactively become participant observations.

No participant heard the composition through a tool. Note placement, controls, meters, and transport support statements about visible behavior, not perceived musical quality, mix balance, or hardware latency. Post-study checks reopened and rendered the projects in the study environment. They do not establish cross-machine plug-in availability, successful relocation, or audible quality.

The intended 12–18 minute exploration period and three-minute limit on a stuck control were working guidelines. Fixture repairs, input changes, reduced concurrency, and coordinator-authorized extra time interrupted the schedule. P09 and P10 were paused after completed gestures while P06–P08 finished, then received extra time on resume. Five software-rendered desktops initially shared four CPUs; later scheduling limited active rendering to three. Wall-clock time and action counts are not human performance benchmarks or controlled comparisons.

The first group encountered omitted synth metadata and screenshots taken before a settled frame. Metadata was supplied, settling/press timing was increased, and the input adapter later changed from multiple `xdotool` processes to in-process XTest with 60 ms keyboard modifier delays. Apparent missed double-clicks, shortcuts, rapid numeric entry, and delayed screenshots are excluded as universal product defects unless separately reproduced. Ratings remain the agents' original descriptive self-reports; there is no causal before/after rating comparison.

## Songs and artifact audit

The ten audited attempts each saved a version-4 project containing two tracks and one embedded audio asset, and exported a finite stereo 48 kHz float WAV. The audit reports no errors for these ten. MIDI counts below count note events across **all saved clips**, including repeated phrases, rather than distinct notes in the original motif. Arrangement lengths are calculated from audited end beat and tempo; WAV lengths include the rendered tail.

| Participant | Chosen title / artifact stem | Tempo | Arrangement | WAV | MIDI notes | Clips |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| P01 | Neon Steps | 112 BPM | 34.286 s | 34.486 s | 45 | 7 |
| P02 | Windowlight¹ | 112 BPM | 34.286 s | 34.486 s | 75 | 7 |
| P03 | Neon Underpass | 112 BPM | 34.286 s | 34.486 s | 33 | 10 |
| P04 | Still Water, Returning | 80 BPM | 48.000 s | 48.200 s | 10 | 6 |
| P05 | Glass Current | 112 BPM | 34.286 s | 34.486 s | 51 | 7 |
| P06 | Lantern Steps¹ | 112 BPM | 34.286 s | 34.486 s | 30 | 6 |
| P07 | Copper Steps | 112 BPM | 34.286 s | 34.486 s | 36 | 10 |
| P08 | Glass Tides | 112 BPM | 34.286 s | 34.486 s | 99 | 4 |
| P09 | Low Tide, High Water | 112 BPM | 34.286 s | 34.486 s | 36 | 8 |
| P10 | Afterglow Steps | 112 BPM | 34.286 s | 34.486 s | 32 | 9 |

¹ The files are named `Windowlight.rill` and `Lantern Steps.rill`, but the audit reads their stored project name as `Midnight circuit`. P06 reports that Save As adopted the filename as the project title. The artifact metadata and that participant impression differ; this report retains both rather than claiming project-name synchronization was verified. P06's original saved file also predates later arrangement and mix changes; the table records that original file, not the recovery snapshot described below.

Nine original saved projects reproduce their exported WAVs sample-for-sample.
P06's original project is the earlier Save As version: later clip placement,
another melody copy, the drum fade, and mix changes appear in its recovery
snapshot. Three subsequent Ctrl+S attempts were logged, but the original file
was not updated. The cause in that session was not recorded. A later native QA
reproduction showed that closing Zenity on bare Xvfb returned keyboard focus to
the root window: mouse edits worked, Ctrl+S did not save, File → Save worked, and
restoring application focus made Ctrl+S work again. This demonstrates an
infrastructure failure path without proving P06's original focus state. A separate copy of
the recovery snapshot, and a separately packaged **Lantern Steps (recovered
final).rill**, each reproduce P06's final WAV exactly. These are coordinator
recovery artifacts, not a retroactive participant save. The original project,
WAV, action log, and report remain unchanged.

Separate audio-only and MIDI-only renders of all ten original projects contain
finite, nonzero samples, confirming contributions from both the starter audio
and the actual native VST instrument. [Verification metadata](../artifacts/usability-2026-11-05/verification.json)
records sample comparisons, hashes, and the recovery qualification.

The songs include manually entered hooks, bass, chord voicings, and sparse sustained notes, plus P08's locally generated arpeggios and closing chord. P04 abandoned an attempted automation fade and saved a note-duration ending. Completion means a composed project and export exist; it does not mean every musical intent succeeded.

## What supported composition

| Observed benefit | Participants | Evidence location |
| --- | --- | --- |
| Pencil entry and note-length controls supported deliberate MIDI entry; extending notes beyond the initial clip automatically enlarged it. | P01, P02, P03, P05, P07, P10; generation also extended P08's clip | `great` findings in the referenced reports; P05 `17-four-bar-hook.png`, P07 `motif-drawn.png`, P10 `two-bar-melody.png` |
| Duplication appended adjacent copies and preserved an understandable song structure; undo and visible cut/trim/fade controls supported endings. | P01, P02, P03, P04, P05, P06, P08, P09, P10 | P03 `20-bass-arranged.png`, P05 `24-full-arrangement.png`, P06 `32-ending-and-level.png`, P09 `drum-fade.png` |
| Numerical level/pan feedback remained visible in the arranger; mixer routing and meters made state inspectable. | P01, P02, P04, P05, P06, P07, P08, P09, P10 | P05 `27-playback-meters.png`, P07 `final-stopped.png`, P10 `playback-mixer.png` |
| Clip Properties combined precise values with bar, beat, elapsed-time, and duration summaries. | P04, P06, P07; P03/P05 used summaries to verify corrections | P04 `ending-properties.png`, P06 `10-pattern-properties.png`, P07 `b-properties.png` |
| “Double-click for MIDI” and the instrument assignment action made track creation discoverable; context-menu Edit notes supplied an alternate route. | P06, P07, P09 | P06 `04-midi-created.png`, `08-clip-menu.png`; P07 `instrument-assigned.png`; P09 `assigned.png` |
| The generator exposed harmony, register, contour, articulation, and velocity, with a preview before committing notes. | P08 | `06-generate.png`, `07-arpeggio-options.png`, `11-added-arpeggio.png` |
| File commands and the export dialog stated the full arrangement duration and format; named success feedback resolved export uncertainty despite the shorter visible loop range. | P01–P10 | Each report's export screenshots; P05 `34-export-settled.png`, `37-export-completed.png` |

These are grouped editorial summaries. Exact selected participant wording, JSON pointers, and evidence paths are retained in the structured companion; the source reports are unchanged.

## Pain points and disposition

“Implemented” below means the change is present in the current worktree and covered by the validation below. It does not mean participants used or rated the changed UI.

| Observation | Participants / diagnostic | Disposition |
| --- | --- | --- |
| Precise clip placement required translating a desired bar into an absolute beat number. | P03, P05, P07, P10 | **Implemented:** a direct Bar / Beat in bar Start popup alongside the existing exact Beat control. Apply/Cancel preserve deliberate commit behavior. Default snap and Duration entry are unchanged. |
| At whole-song zoom, 1/16 snap allowed subtle phrase offsets such as bar 5 beat 1.25 or bar 8 beat 4.75. | P03, P06, P07; P10's initial offset is timing-confounded | **Deferred:** default/adaptive snap choice and placement-preview policy. P06 and P10 found the existing 1 bar setting. The new Start popup addresses arithmetic, not this snap behavior. |
| Most pitches were not immediately apparent near the pointer before drawing, and short notes could lack in-note pitch labels. | P02, P05; diagnostic 02 | **Implemented:** an immediate pointer pitch cue using the visible piano-row mapping. Existing gutter hover labels were already present; the change supplements their distance/density limitations. |
| Automation opened around its default one-bar points, while gain inspection used linear multipliers against a dB axis. A separate diagnostic found send-gain axis labels used track-gain scaling. | P04; diagnostic 04 | **Implemented:** target-correct dB fields for point/override gain, explicit Silence, correct track/send dB axis scaling, and Fit song while retaining Fit points. The send-axis mismatch was a diagnostic discovery, not something P04 reported directly. |
| P04 could not complete a final fade through drag and numeric edits. | P04; diagnostic 04 | **Deferred timing hardening:** ordinary multi-frame drag and numeric input succeeded in probes; fully coalesced drags and focus-plus-text in one frame reproduced lost edits. The native log cannot prove those frame boundaries caused every attempt. The dB/framing patch does not fix generic numeric/coalesced gestures. |
| Double-clicking empty arranger space near bar 3 created the initial MIDI clip at bar 1. | P07 | **Implemented:** the first clip of a new MIDI track uses the clicked, snapped beat. |
| Duplicate initially looked like an immediate command on the selection; it was a mode requiring a subsequent clip click. | P01, P06 | **Implemented:** clearer Duplicate-tool hover text. The established click-to-duplicate behavior remains. |
| A small arranger level drag overshot the intended reduction. | P03; diagnostic 03 | **Deferred:** gain-slider sensitivity/discoverability. Exact entry already exists on the dB readout/context menu and was independently verified, including precise values and undo. No new mapping or redundant exact-entry control is claimed. |
| Slowing tempo enlarged the musical audio-clip duration while the visible source waveform ended earlier. | P04 | **Deferred:** explain the relationship among source duration, tempo, clip extent, and stretching. No audio-follow-tempo change is claimed. |
| Generator Next range could leave the active preview offscreen; the initial major triad did not follow the displayed minor key. | P08 | **Deferred:** preview following and generator default quality. Existing Fit preview provided a recovery route. |
| Sharp-only root choices or pitch labels required enharmonic translation in a minor-key context. | P08, P09, P10 | **Deferred:** key-aware/enharmonic spelling in generator and piano-roll labels. The pointer pitch cue does not change spelling policy. |
| Transposing a selected phrase an octave moved its upper notes outside the visible piano roll, requiring scroll correction. | P09 | **Deferred:** following the selection after transpose and discoverability of the existing Fit selection route. No missing fit capability is claimed. |
| Extending a one-bar phrase to four bars required entering 16 beats through a separate duration keypad. | P09 | **Deferred:** common bar-length Duration presets. The new Bar/Beat popup changes Start only. |
| Exact-volume and duration range text exposed excessive precision or technical limits. | P04, P09 | **Deferred:** verbose legacy range labels. Consistent automation gain units do not imply all existing numeric labels were revised. |
| The numeric Start popover's Apply value looked very dark; Return was the successful commit route. | P10; subsequent native visual QA | **Implemented:** explicit primary button fills remain visible inside menus. Native QA reproduced dark text on a dark surface for valid Apply actions; the shared menu styling suppressed their fill. Ordinary quiet menu rows retain their appearance. |
| Shortcut and rapid-input outcomes were inconsistent in some native attempts. | P02, plus environmental notes in P06–P08 | **Excluded as general defect claims:** adapter/load changes confound these observations. Successful visible menu/tool alternatives remain documented; narrowly reproduced automation timing weaknesses are tracked separately above. |

P01's high-severity playback complaint remains verbatim in its source report. It is classified below as a study-backend limitation, not counted as a confirmed Rill UI defect. Severity labels elsewhere are the original participants' assessments, not a measured priority ranking.

## Environment and separate diagnostics

The initial virtual-audio settings were 1024 frames / 60 ms for P01–P05. A standalone CPAL probe outside Rill reproduced 1,371 ALSA `snd_pcm_avail_delay` I/O errors over eight seconds. A 4096-frame / 200 ms probe completed eight seconds with 110 callbacks and zero errors, and that configuration was used for P06–P10. Some later participants still reported output warnings, so the larger setting is not claimed to eliminate every session failure. These probes establish that the original failure could occur in the study backend independently of Rill's UI/engine. They do not measure production hardware latency or perceived audio.

Post-study diagnostic 04 used the frozen baseline executable (`SHA-256 ac0b1ad9679de34ac142197c7848acd3c2ac7441e502c0b5b6421c7e923ee137`) and separate probes. Ordinary automation drag and typed position/gain changes worked; complete gestures coalesced into one frame failed, and the shared Number component could lose text delivered with initial focus acquisition. The native study lacks raw frame/event boundaries, so this is a reproducible failure mode, not proof of the cause of each participant gesture. The same diagnostic confirmed a send-gain axis maximum of roughly +3.5 dB where 16× requires about +24.08 dB, and default view framing around beats 0–4 rather than the 64-beat song.

Diagnostic 02 verified that baseline pitch labels existed in the gutter on hover but were distant from pointer work and disappeared at dense row heights. Diagnostic 03 verified existing gain exact entry and the compact slider's broad dB travel. These diagnostics refined the changes and prevented treating discoverability problems as missing functionality.

## Validation

The final relevant results total **543 passing tests and zero failures**: 307 shared UI tests and 236 application tests, including the explicitly invoked native screenshot scenario. This is the complete `daw-ui` suite plus affected application unit and E2E targets, not every application integration target in the repository. Two cases were ignored in regular runs; the native screenshot case was subsequently run successfully, leaving one optional installed-VST automation case unrun. [Validation metadata](../artifacts/usability-2026-11-05/validation.json) records the latest result and source log for each target.

The running-app tests cover musical position entry, staged keypads and Escape/Cancel, exact untouched values, different meters, one-step Undo, duplicate placement, pitch cues on short and dense notes, Multi-draw pitch, track/send gain units, Silence, automation framing, snapped empty-space placement, and popup button contrast. Existing relevant clip, MIDI, automation, numeric entry, toolbar, and popup tests also passed. The empty-arranger placement regression was first demonstrated against the frozen baseline, where it created a clip at beat 0 instead of the clicked beat 8.

The workspace build, strict Clippy checks for all shared UI targets and the modified application targets, `cargo fmt --all -- --check`, and `git diff --check` passed. An unchanged vendored `vst3-host` dead-code warning remains. The native screenshot test exercised seven application views; the component gallery generated 22 captures, with the four affected light/dark examples refreshed in the repository.

Separate native visual review exercised both themes at the supported 1100 × 720 minimum and 1440 × 1000. The new position and gain controls were reachable, Apply actions had visible fills, dense and short-note pitch cues remained legible, and gain Silence could be undone. Exact checks, build hashes, and capture provenance are retained in the [native QA notes](../artifacts/usability-2026-11-05/native-qa/notes.md). This was not an exhaustive cross-product of every control, theme, and size. A forced 800 × 600 native window, below the advertised minimum, could clip a utility window; that is not evidence of a supported-size regression. Keypads can expand/reposition before settling, and some short synthetic clicks were missed under load. These remain qualifications, rather than a claim that every interaction is now smooth.

Selected captures show the [concise position editor](../artifacts/usability-2026-11-05/native-qa/screenshots/final-formatted-position-dark-minimum.png), [dense-note pitch cue](../artifacts/usability-2026-11-05/native-qa/screenshots/final-pitch-dense-dark-wide.png), and [gain entry in the light theme](../artifacts/usability-2026-11-05/native-qa/screenshots/final-gain-light-minimum.png).

Development runs exposed and corrected nested-keypad closing, Escape ownership, an end-of-timeline placement bound, position-caption rounding, excessive digits in the idle position field, and the actual pointer pitch during Multi-draw. Exact focused/keypad values and untouched focus/blur cycles remain lossless. Earlier failed logs remain in the evidence; the validation metadata selects the final complete result for each target. Paint assertions were also corrected to request fresh snapshots and inspect rendered text colors, retaining their behavioral and contrast checks. No follow-up participant study or improvement in ratings is claimed.

## Provenance and retained evidence

The source root is `/workspace/rill-study-20261105`. Inputs are `protocol.json`, `artifact-audit.json`, `participant-01/report.json` through `participant-10/report.json`, and the post-study `diagnostics-02/pitch-hint-findings.md`, `diagnostics-03/notes.md`, and `diagnostics-04/diagnosis.md`. Each report links its own screenshots and logged native interactions remain in the corresponding participant directory. The companion JSON records exact source paths, SHA-256 fingerprints, participant IDs, feedback JSON pointers, artifact hashes, and selected unmodified finding strings.

Song/project binaries and the original participant screenshot collections remain outside the repository. The downloadable study archive contains all ten original projects, WAVs, reports, native input logs and screenshots, the fixture instrument, verification evidence, and the separately named P06 recovery project. Compact metadata, validation logs, and selected post-change QA captures accompany this report in the repository. Original participant reports are preserved word-for-word, including statements later qualified by the audit or diagnostics; all 35 recorded source/report/project/WAV hashes and 32 selected verbatim feedback excerpts were rechecked before packaging.
