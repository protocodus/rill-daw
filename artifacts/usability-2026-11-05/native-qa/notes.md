# Native visual and interaction QA — 2026-11-05

This is post-study diagnostic QA, not a new participant session. No participant originals or source files were modified. The private display was `:980`, using software rendering without a window manager, own config/cache/runtime, and a null ALSA output. Playback stayed stopped; audio quality was not assessed. No builds or cargo tests were run by this reviewer.

## Binary and fixture

Final formatter recheck binary: `/workspace/rill-daw/target/debug/rill`, SHA256 `00b0b84552f90caa289b6ff21b400bfd3b06afdf73fb521dcab31b4415df228b`. Screenshots prefixed `final-formatted-` use this build. The broader final native checks used SHA256 `6af63f55e5eb93e6a5f899cd6e166fa11c4df23bf910ce273d0ca2c5dff26e73`; all other `final-` screenshots use that earlier final-check build. Earlier screenshots document the intermediate builds and initial defects. `processes.json` records the app/Xvfb PID and Linux start ticks; `actions.jsonl` records native input. The copied QA adapter was lengthened to 350 ms presses after some 60 ms clicks were missed under shared CPU load; ambiguous click sequences are not counted as passes.

The fixture is a diagnostic copy (`fixture.rill`): a 64-beat song with a 16-beat MIDI phrase starting at 3.999, three notes including a 1/16-beat C#4, a second clip ending at beat 64, and source track/send gain automation. Headless use was limited to preparing and validating this fixture; subsequent checks used the actual native UI. The source instrument is None, deliberately avoiding plugin dependencies.

## Supported-layout observations

At 1100×720 and 1440×1000 the inspected Properties Bar/Beat popover, staged mouse keypad, gain popup/keypad, Apply/Cancel, Silence, Fit song, segment controls and automation close button were reachable. Dark Apply buttons now have a clearly visible blue fill and readable text; the initial black-on-charcoal Apply defect is fixed. Light theme uses a dark blue fill and white text. The pitch cue remains legible beside a very short C#4 note at dense row heights in both themes.

Native interactions verified:

- Properties: a staged bar/beat keypad Apply followed by Apply position changed the start to Bar 5, Beat 1; one Undo restored the original 3.999 start (intermediate build with the Apply fix). On the final binary, modifying the staged bar then Cancel returned to the original draft/model; Escape also returned from the staged keypad without a position change.
- Track gain: opening the dB editor showed the value in dB; Silence placed the selected point at zero/−∞ and one Undo restored the prior value (intermediate build). An earlier mouse-only −6 dB entry succeeded and was undone. A later final-build −6 attempt under load did not visibly apply and is not counted as a pass.
- Send gain: on the final binary, Fit song expanded the graph from its initial four-beat point span to the full 64-beat song (bar 17 at the right). The final minimum-size keypad Silence action succeeded, displaying −∞ and moving the selected point to zero. Undo restored the prior 15.56 dB value.
- Pitch cue: final dark/light wide captures show C#4 beside a short note at roughly eight-pixel row heights. Earlier minimum-size native testing also showed the cue, and clicking the previously painted cue position with Pencil created the underlying note: the cue did not intercept the pointer.

The coordinator noticed floating-point noise (`4.9990000000000006`) in the earlier idle Bar/Beat capture. The final formatter recheck shows clean idle `4.999` at 1100×720, while focusing the field or opening its keypad still shows the exact `4.9990000000000006` editable text. The Apply control remains fully visible. This presentation correction is verified in the `final-formatted-` screenshots; the coordinator separately ran its exact-value/no-op regression tests.

## Limits and transient behavior

The gain keypad can first expand downward, then relocate upward on the next pointer event. At 1100×720 one initial frame showed Silence near the bottom edge; a click at that stale position dismissed the popup after it had moved. Once settled, Silence is visible and works. This is a transient placement limitation, not a persistently blocked operation. No shared sizing change is proposed in this pass.

Forced 800×600 screenshots show the automation window’s right edge/close button and part of the bottom controls clipped. This is below the application’s supported 1100×720 minimum and was forced through X11; it is not evidence of a supported-size regression. No native baseline comparison was performed. Main actions are reachable at the supported sizes, sometimes after ordinary vertical scrolling. The native check is not exhaustive across all OS/window-manager/DPI combinations. Final-build screenshots cover Properties/keypad in light minimum and dark wide, gain/keypad in light minimum and dark minimum/wide, and pitch cues in both themes at wide size. The final formatter recheck adds dark minimum Properties/keypad; earlier intermediate-build captures cover minimum-size pitch. This is not a complete cross-product of every control, theme, and size.

## Selected screenshots

All paths below are under `screenshots/`.

| Evidence | Screenshot |
|---|---|
| Final formatter idle/focused/keypad at dark minimum | `final-formatted-position-dark-minimum.png`, `final-formatted-position-focused-dark-minimum.png`, `final-formatted-position-keypad-dark-minimum.png` |
| Final light minimum Properties | `final-properties-light-minimum.png` |
| Final light minimum Bar/Beat Apply | `final-position-light-minimum.png` |
| Final light minimum nested keypad | `final-position-keypad-light-minimum.png` |
| Final staged Cancel result | `final-position-cancel-light-minimum.png` |
| Final dark wide Bar/Beat Apply | `final-position-dark-wide.png` |
| Final dark wide nested keypad | `final-position-keypad-dark-wide.png` |
| Final staged Escape result | `final-position-escape-dark-wide.png` |
| Final light minimum gain popup/keypad | `final-gain-light-minimum.png`, `final-gain-keypad-light-minimum.png` |
| Final light wide automation | `final-automation-light-wide.png` |
| Final dark wide Fit song/send gain | `final-fit-song-send-dark-wide.png`, `final-gain-dark-wide.png` |
| Final dark wide keypad | `final-gain-keypad-dark-wide.png` |
| Final dark minimum gain/keypad | `final-gain-dark-minimum.png`, `final-gain-keypad-dark-minimum.png` |
| Final minimum Silence/Undo result | `final-silence-dark-minimum.png`, `final-silence-undo-dark-minimum.png` |
| Final light short/dense pitch cue | `final-pitch-short-light-wide.png`, `final-pitch-dense-light-wide.png` |
| Final dark dense pitch cue | `final-pitch-dense-dark-wide.png` |
| Earlier minimum cue and click-through | `pitch-dense-dark-minimum.png`, `pitch-hint-click-through.png` |
| Unsupported forced 800 layout | `automation-dark-compact-settled.png`, `automation-dark-compact-scrolled.png` |
| Original dark Apply defect | `position-dark-wide.png`, `gain-dark-wide.png` |

## Separate post-study focus reproduction

The QA session reproduced a keyboard-focus loss after native Save As in Xvfb without a window manager. This is environment evidence, not proof of the historical focus state in participant 06.

`focus.jsonl` contains exact focus IDs, names, timestamps, saved note counts and file mtimes. Before Save As, focus was Rill window 4194307. After the Zenity dialog closed, focus was unnamed root window 543. The saved copy contained four notes. Pointer editing added a fifth note, but Ctrl+S with focus still at root left both the saved four-note count and file mtime unchanged. File > Save by pointer saved five notes while focus remained root. Explicit `xdotool windowfocus` returned focus to Rill; adding a sixth note followed by Ctrl+S saved six notes and updated the mtime. The copy is `focus-base.rill`.

Screenshots: `focus-save-as-dialog.png`, `focus-after-save-as.png`, `focus-ctrl-s-unfocused.png`, `focus-file-save.png`, `focus-recovered-ctrl-s.png`. The participant reports/projects were not touched. This supports an infrastructure mitigation: explicitly restore app keyboard focus after closing external native dialogs when running without a window manager.

## Cleanup

Completed after the formatter capture. `cleanup.json` records successful SIGTERM of only verified QA app PID 82361 (start ticks 647072) and Xvfb PID 59696 (start ticks 471313). Both are stopped (zombie state awaiting their launcher’s reap), as are the three prior QA app PIDs. No participant process was touched. Playback was stopped before termination. The private X11 socket is absent.
