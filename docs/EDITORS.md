# Clip editors

Double-click a clip, or select it and choose **Window > Clip Editor**. Each editor uses the same timeline gestures: middle-drag or Alt-drag to pan, Ctrl/Cmd-wheel to zoom time, and Ctrl/Cmd-Shift-wheel to zoom vertically. **Fit** or **F** restores the complete clip and its vertical content. Bars and elapsed time appear together in the ruler. Opening a utility popup preserves the editor context; Escape returns to it after dismissing an open menu first.

## Audio

The audio editor changes source references and playback parameters. Source WAV files remain unchanged. The arrangement overview and exported audio use the same reverse and envelope settings.

The **Clip** menu contains cursor jumps, trimming, splitting and clipboard actions. **Process** contains reversal, normalization and transient analysis. Gain stays beside the tools; fade duration and curve controls appear only with the corresponding fade tool. **Close transients** dismisses the analysis settings and markers, including a preview still being calculated.

- **Cursor (P)** seeks by clicking or dragging the waveform. **Clip start** and **Clip end** jump to its bounds. **Set start** and **Set end** trim to the cursor; **Snap end** moves the end to the nearest selected grid division. Trimming cannot expose samples before the source begins.
- **Cut (E)** splits at the clicked grid position. **Split at cursor** uses the current playback position. Splits and trims preserve the original fade envelope and source samples, including reversed regions.
- **Fade in (I)** and **Fade out (O)** set the duration by clicking or dragging the waveform. Durations can also be entered in beats. Each fade offers **Linear**, **Equal power**, and **S curve** shapes. Editing a fade explicitly anchors a new envelope to the current clip bounds.
- **Gain** sets the clip level in dB. **Normalize** analyzes the selected source window across its channels and sets its sample peak to −1 dBFS, before fades. Repeating it has no cumulative effect. Silent clips report an error without changing gain.
- **Reverse** reverses the selected source window while retaining stereo channel order. It does not rewrite the source or reverse the entire underlying recording.
- **Copy**, **Paste**, and **Duplicate clip** preserve all clip parameters. Paste creates a new clip at the cursor on the current audio track; duplicate places a new clip immediately after the selected one.
- **Transients** opens onset detection. **Energy rise** finds short increases in RMS energy; **Amplitude gate** finds threshold crossings with hysteresis. Sensitivity controls the threshold, and the minimum gap suppresses closely spaced detections. **Analyze** shows markers without editing. **Split transients** cuts at those unsnapped onset positions as one undoable edit. Changing the clip or analysis settings clears an obsolete preview.

Normalization and transient analysis run in background workers. Their results apply only while the source clip, tempo, and analysis settings still match. Fit covers the entire selected clip horizontally and scales both audio channels to its visible peak, including gain and fades. That amplitude reference stays fixed during edits until the next Fit; silence uses a unity reference. Navigation never changes clip data. Undo and Redo cover audio editing operations; portable `.rill` projects preserve their parameters and bundle the referenced source audio.

The waveform is a bounded display preview; normalization scans every source sample. Onset detection uses 5 ms analysis windows and a maximum of 2,048 markers per clip. It is an onset detector rather than a musical beat tracker.

## MIDI

The piano roll uses **Select (Q)**, **Pencil (W)**, **Cut (E)**, **Duplicate (R)**, **Glue (T)**, **Erase (Z)**, **Multi-draw (U)**, and **Cursor (P)**. Pencil creates a note with a click or sets its length by dragging. Select moves note bodies and resizes either edge. Shift adds to the selection; Cmd/Ctrl+A selects all notes, Delete removes the selection, and Cmd/Ctrl+D duplicates it. Double-clicking empty space creates a note; double-clicking an existing note deletes it in Select, Pencil, or Multi-draw mode.

**Notes** opens selection operations without leaving the editor:

- **Quantize...** previews a grid, strength, and swing against the arrangement timeline. Strength moves each selected start partway toward its nearest grid point. Swing delays alternate grid points; 50% is straight. Durations stay unchanged unless **Note ends** is enabled. **Apply quantize** commits one undoable edit; **Cancel quantize** or Escape discards the preview. Editing controls are suspended during the preview, and changing the clip or undoing cancels it.
- **Legato** ends selected notes at the next distinct note onset. Simultaneous chord notes share that endpoint; notes with no later onset keep their existing duration.
- **+1/-1 semitone** and **+1/-1 octave** transpose the selection. Up/Down arrows transpose by a semitone, or an octave with Shift. Left/Right arrows nudge by the current grid, or a bar with Shift. A group stops together at pitch and time limits so its intervals and spacing stay intact.
- **Copy notes** or Cmd/Ctrl+C stores the selection with its relative timing and velocities. **Paste notes at cursor** or Cmd/Ctrl+V creates new note IDs on the selected MIDI clip, extending it when needed.

Shift-dragging a selected note body makes one copy of the selection and moves the copies. Shift-dragging an edge temporarily bypasses the resize grid; ordinary resizing stays snapped. Alt and middle-drag remain reserved for panning.

**Pitch** opens root, scale and chord choices. The current root and scale remain visible beside it. Scale members tint the keyboard keys and pitch rows blue; fourteen chord types highlight their members more brightly. Multi-draw reveals its stack and chord/scale controls, then places that stack in the clicked octave. **Notes** also provides selected-note velocity and glue controls. Fit and F reveal the earliest note start, latest note end, and full pitch range on both axes.

The velocity lane shares note timing and selection. Pencil paints velocities across notes. Select sets a velocity with a click; dragging selected stems applies a relative change to the selection. Holding a keyboard key or note auditions the assigned VST instrument. Releasing the pointer, losing focus, or leaving the editor stops the audition. An unassigned MIDI track remains silent.

These choices adapt established editing patterns to Rill's focused editor. Ableton documents adjustable quantization strength, separate start/end quantization, and legato; Logic exposes quantize strength and swing. Rill combines those controls in a temporary preview with explicit Apply/Cancel, while keeping the final legato notes unchanged. FL Studio's middle-drag panning informs the shared two-axis navigation. The explicit preview and reduced persistent toolbar are Rill design decisions. [Ableton MIDI editing](https://www.ableton.com/en/live-manual/12/editing-midi/), [Logic quantization](https://support.apple.com/en-asia/guide/logicpro/lgcpfa6e7f80/10.7/mac/11.0), [FL Studio piano roll](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/pianoroll.htm).
