# Expert review response — 1 November 2026

This matrix responds to the 14 findings in the frozen `REVIEW.md` and
`technical/findings-audit.json` under
`/workspace/scratch/rill-expert-review-2026-11-01/`. The audit supports 11 findings,
qualifies two, and partly contradicts one. Original reports, screenshots, logs,
music and ratings remain evidence, not editable implementation notes.
The [baseline provenance record][baseline] identifies six unmodified screenshot
selections: [send value][before-send], [pan entry][before-pan], [clip context][before-clip],
[menu context][before-menu], [mixer routing][before-mixer], and [existing trim tooltip][before-trim].

The three reviews were agent simulations using Jobs-, Ive- and Norman-inspired
design lenses, not participation or endorsement by those people. Their original
ratings remain **4 / 4 / 4 / 2 / 4** for ease, looks, perceived speed, confusion
(lower is better), and completeness. No rerating or score improvement is claimed.

| Finding | Audit | Implemented response and regression coverage |
| --- | --- | --- |
| R1:F1 — coarse send adjustment | Supported | [Routing][routing] uses a larger shared knob with a perceptual gain taper, unity mark, fine adjustment and clickable exact readout. [Send E2E][send-tests] checks small movements below unity; [knob tests][knob-tests] cover silence and the full range. |
| R1:F2 — stale send field | Supported | [Shared numeric state][values] refreshes when another control changes the value, without discarding a stable draft or caret. [Synchronization tests][sync-tests] cover stale blur commits and sibling controls; [send E2E][send-tests] compares live gain and readout. |
| R1:F3 — layered timing access and obscured context | Supported | [Arranger clip menus][arrangement] open Properties directly for the clicked audio/MIDI clip. Routine backdrops remain readable. [Navigation E2E][navigation-tests] covers target selection, mouse keypad, Cancel, return and Undo; [backdrop E2E][backdrop-tests] covers navigation and blocking. |
| R1:F4 — generic instrument/bus names | Qualified | The old destination menu **already had a divider**. [Initial instrument assignment][library] now adopts a unique plug-in name only with explicit creation intent; existing/user names are preserved. [Bus creation][mixer] uses `Bus 1`, `Bus 2`, skipping occupied names. [Instrument E2E][instrument-tests] and [bus E2E][navigation-tests] cover assignment, reassignment, saved names, collisions and Undo. |
| R2:F1 — coarse send adjustment | Supported | Same shared [send control][routing]; [send E2E][send-tests] exercises 4/5-point adjustments after the native drag threshold and exact-entry recovery. This addresses pointer sensitivity; it does not establish audible improvement. |
| R2:F2 — generic “Value” title | Supported | [Parameter editors][parameter-context] identify channel and parameter; send titles also name the destination. They open at their controls with readable background context. [Send/pan E2E][send-tests] checks titles and exact-entry behavior. |
| R2:F3 — trim feedback | Partly contradicted | The cited held-trim screenshots already show the local endpoint/duration tooltip at both 30.134 s and 30.000 s. No missing-tooltip fix is claimed. Existing [trim-feedback coverage][trim-tests] remains; the [direct Properties shortcut][navigation-tests] improves the valid exact-timing detour. |
| R2:F4 — pan decimal residue | Supported | [Parameter formatting][values] shows concise values while retaining the original value for an unchanged Apply. [Value-entry tests][value-tests] and [pan E2E][send-tests] cover readable display, stored precision and history. |
| R2:F5 — routing below the viewport | Supported | [Mixer layout][mixer] reserves space for the main output, one send and an ordinary notice without moving existing controls. [Navigation E2E][navigation-tests] checks visible captions/endpoints at 1440×1000, export notice, dismissal, fader use and Undo without scrolling. Additional routes and short windows retain scrolling. |
| R3:F1 — exact send discovery/numeric drag | Qualified | The recorded zeroing gesture used the numeric field, not the slider; exact entry already worked in R1/R2. The [new readout button][routing] opens exact entry directly, with a visible mouse keypad. [Send E2E][send-tests] covers entry, Cancel, unchanged Apply and automation preservation. |
| R3:F2 — local/global clip orientation | Supported | The [shared editor header][app] shows track and global song span; the [MIDI ruler][midi] explicitly says “Local.” [Clip-context E2E][clip-tests] covers repeated occurrences, media-specific ruler hints, narrow layouts and unchanged document/history. |
| R3:F3 — blur behind ordinary menus | Supported | [Routine backdrops][backdrop] use a light scrim and keep the source sharp. Decision modals retain stronger treatment and underlying input remains blocked. [Backdrop E2E][backdrop-tests] and the [opt-in native-render regression][blur-tests] distinguish routine and modal behavior. |
| R3:F4 — pan decimal residue | Supported | Same [concise parameter display][values] and unchanged-Apply preservation; [pan E2E][send-tests] checks the stored value and Undo history. This is a display correction, not evidence of an audio or parameter defect. |
| R3:F5 — Properties moves between editors | Supported | [The shared header][app] gives Properties a consistent command position for audio and MIDI. [Clip-context E2E][clip-tests] compares its bounds across media, widths and display densities while retaining useful editor space. |

The missing native-picker helper was a confirmed **lab PATH fault**; restoring
`zenity` let the original sessions save/export. The additional [RFD/app error
handling][picker-patch] makes missing or failed Linux/BSD helpers visible through
the existing error UI and preserves unsaved work. Genuine cancellation stays
silent. [Adapter tests][picker-results] and [running-app failure tests][picker-tests]
cover missing/broken helpers, cancellation, restoration and guarded Save retry.
Scripted native-adapter tests do not by themselves verify a desktop chooser's
appearance or broaden the patch to other platforms.

The recurring ALSA EIO/output-stream failure remains unresolved. The review and
separate diagnosis do not establish its cause, a verified backend repair, or
physical-device reliability. No audio-backend fix or listening-quality improvement
is claimed; the reviewers did not audition the audio.

The [focused validation record][validation] records 41 precision/control tests,
25 context/layout component tests, passing application regressions, the native
routine-versus-modal rendering test, and portal-cancellation classification.
The narrow-header regression checks actual rendered glyphs at both display
densities. Workspace build and strict all-target Clippy passed. Final full-suite
and cross-platform results are recorded with the pull request.

[Native inspection evidence][native] includes both appearances at 1440×1000 and
800×600, exact send entry/keypad, concise named Pan entry, readable clip context,
visible mixer routes and fresh instrument/bus names. A controlled four-pixel
send drag changed 0.25× to approximately 0.291343×; mouse Undo restored 0.25×.
This is an interaction check, not a new expert rating or listening test.
All 214 frozen original study evidence files remain unchanged, and all owned
preview processes were stopped. Eighteen component-gallery captures were
inspected; the four changed input/navigation images are updated in the shared component guide.

[routing]: ../crates/daw-app/src/routing.rs
[values]: ../crates/daw-ui/src/value_entry.rs
[arrangement]: ../crates/daw-app/src/arrangement.rs
[library]: ../crates/daw-app/src/library.rs
[mixer]: ../crates/daw-app/src/editor.rs
[parameter-context]: ../crates/daw-ui/src/automation_navigation.rs
[app]: ../crates/daw-app/src/app.rs
[midi]: ../crates/daw-app/src/midi_editor.rs
[backdrop]: ../crates/daw-ui/src/popup_backdrop.rs
[send-tests]: ../crates/daw-app/tests/expert_send_controls_e2e.rs
[knob-tests]: ../crates/daw-ui/tests/knob.rs
[sync-tests]: ../crates/daw-ui/tests/parameter_value_sync.rs
[navigation-tests]: ../crates/daw-app/tests/expert_mixer_navigation_e2e.rs
[backdrop-tests]: ../crates/daw-app/tests/popup_backdrop_e2e.rs
[instrument-tests]: ../crates/daw-app/tests/instrument_naming_e2e.rs
[trim-tests]: ../crates/daw-app/tests/trim_feedback_e2e.rs
[value-tests]: ../crates/daw-ui/tests/value_entry.rs
[clip-tests]: ../crates/daw-app/tests/clip_editor_context_e2e.rs
[blur-tests]: ../crates/daw-app/tests/visual_polish_e2e.rs
[picker-patch]: ../vendor/rfd/RILL-PATCH.md
[picker-results]: ../crates/daw-app/tests/native_picker_results.rs
[picker-tests]: ../crates/daw-app/tests/native_picker_failure_e2e.rs
[baseline]: ../artifacts/expert-ui-refinements-2026-11-01/baseline-provenance.json
[before-send]: ../artifacts/expert-ui-refinements-2026-11-01/before/send-stale-value.png
[before-pan]: ../artifacts/expert-ui-refinements-2026-11-01/before/pan-exact-value.png
[before-clip]: ../artifacts/expert-ui-refinements-2026-11-01/before/midi-editor-context.png
[before-menu]: ../artifacts/expert-ui-refinements-2026-11-01/before/menu-context.png
[before-mixer]: ../artifacts/expert-ui-refinements-2026-11-01/before/mixer-routing.png
[before-trim]: ../artifacts/expert-ui-refinements-2026-11-01/before/trim-tooltip.png
[validation]: ../artifacts/expert-ui-refinements-2026-11-01/focused-validation.json
[native]: ../artifacts/expert-ui-refinements-2026-11-01/README.md
