# Expert feedback implementation evidence

The [response matrix](../../docs/EXPERT_UI_REFINEMENTS.md) maps all 14 original
findings to changes, qualifications and executable coverage. These are agent
simulations, not reviews by the named designers. No new scores are inferred.

| Change | Native evidence |
| --- | --- |
| Send knob, unity landmark and direct exact entry | [Light exact editor](after/send-exact-light.png), [Dark keypad](after/send-keypad-dark.png) |
| Useful low-level adjustment and synchronized readout | [Four-pixel drag](after/send-small-drag-dark.png): 0.25× to about 0.291343×, then restored by mouse Undo |
| Concise parameter value and target identity | [Named Pan editor](after/pan-exact-dark.png): 15 rather than floating-point residue |
| Readable source behind routine menus | [Edit menu over MIDI](after/midi-menu-dark.png) |
| Output and first send visible together | [Mixer at 1440×1000](after/mixer-routing-light.png) |
| Instrument-derived names and numbered buses | [Fresh Rill Lab Sine and Bus 1](after/names-dark.png); existing loaded names remain preserved |
| Local MIDI time with global clip identity | [Dark 800×600](after/midi-context-dark-800.png), [Light 800×600](after/midi-context-light-800.png) |
| Consistent Properties and accurate audio time context | [Audio at 800×600](after/audio-context-light-800.png) |

These screenshots came from native mouse inspection of copies of the completed
study projects. Naming and drag edits were undone; the projects were not saved.
There was no playback or audio audition. The preview used virtual graphics/audio;
it does not establish physical-device reliability or listening quality.

[Native provenance](native-provenance.json) identifies each image, hash and
frozen executable. Earlier control/menu/mixer captures use `build.json`; final
header, naming and pan captures use `build-v3.json`. The subsequent application
change adds only opt-in rendered-glyph diagnostics for the executable tests.
[Baseline provenance](baseline-provenance.json) identifies the six untouched
before screenshots. [Preservation](original-evidence-preservation.json) verifies
all 214 frozen study evidence files remain unchanged. [Project checks](final-project-preservation.json)
also cover the original and copied projects; [cleanup](cleanup-verification.json)
verifies no owned app/audio/display processes or display sockets remained.

[Focused validation](focused-validation.json) records passing component, app,
native-rendering and picker checks, and identifies the superseded header-width
assertion rather than counting its failure as a pass. The replacement checks
the actual visible song-span glyphs. [Workspace validation](workspace-validation.json) records the full run and two
corrected assertion targets (1,492 passing tests, without double counting;
25 opt-in tests ignored). Five-platform CI results are reported on the pull request.

The recurring virtual-ALSA EIO was not reproduced or repaired by this UI work.
The existing explicit output recovery remains in place. The silent missing-file-picker
condition is addressed by distinguishing Linux portal/Zenity errors from Cancel;
the original trigger was a laboratory PATH setup error.
