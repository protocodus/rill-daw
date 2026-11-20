# Rill user guide

[Back to Rill](../README.md) · [Download builds](../README.md#download)

A reference for composing, recording, editing, mixing, and saving in Rill.

## First session

The initial session contains an original generated audio loop. Rill includes a built-in synth, **R1 Mini**, a sample player, **R1 Sampler**, a drum sampler, **R1 Drums**, and the **R1 Filter**, **R1 Equalizer**, **R1 Saturator**, **R1 Compressor**, **R1 Modulator** and **R1 Spacer** effects. A new MIDI track starts with R1 Mini, so its notes are audible at once; choose a plug-in instrument to replace it. Use **File → New** to start empty. Press **Space** to play or pause.

An empty song offers three starting points: **Choose instrument…** opens your
installed sounds, **Import audio or MIDI…** brings in a recording or sequence,
and **Compose with AI** opens editable composition examples. Browsing sounds or
examples leaves the song untouched. **Add pattern** and dropping audio files onto
the arranger remain available; the starting actions disappear once a track exists.

## Included

- Audio and MIDI tracks; VST3 instruments and effects hosted in isolated processes.
- Parallel processing of independent instrument/effect chains, with persistent workers, ordered buses, and a serial path for lightweight sessions.
- Non-destructive clip movement, trimming, splitting, duplication, cross-track movement, and undo/redo.
- Live volume, pan, note, and clip edits retain running plugins, sustained notes, effect tails, and transport position; new audio prepares in the background.
- Drag WAV, AIFF, FLAC, MP3, Ogg/Vorbis, AAC, M4A or CAF audio files onto an existing audio track to place clips there. Dropping elsewhere inside the app creates a new audio track for each file. Dropping on a MIDI/bus/folder track also creates a new audio track.
- Non-destructive audio gain, normalization, reverse, curve fades, range clipboard editing, and transient split preview.
- Exclusive audio overlaps, optional spline crossfades, and audio/MIDI clip joining.
- Transport loop toggle and on-demand start/end beat controls, saved with the project.
- Meter changes at bar lines and named, colored song markers on the arranger ruler, exported to MIDI.
- Piano roll with scale/chord highlighting, chord database, octave-stack drawing, both note edges, velocity painting, and held-note plugin audition; randomize and quantize previews, voice-aware legato, transpose, and note clipboard.
- Generate sustained chords, seven arpeggio patterns, and seeded melodies with optional counterpoint or tintinnabuli voices; preview notes in an editable whole-bar range before adding them in one undoable step.
- Large searchable instrument/effect browser with developer filters and cached native previews; drag-reorder insert chains and create send returns.
- Pre/post-fader sends, buses, sidechain/multi-input routing, grouped instrument outputs and automatic path delay compensation. Every audio channel, including Master, has automatable input gain, volume and pan, plus an invert polarity switch (**Ø**) and a stereo width rail under the input gain. Polarity flips both channels before the inserts. Width works after the inserts and before volume and pan: 0% folds to mono, 100% leaves the signal unchanged and 200% doubles the side signal. It snaps to 100%, and right-click enters an exact value. Feedback cycles are rejected.
- Reversible clip/track Freeze creates managed audio below the source, preserves originals and supports Return to originals.
- Drag-and-drop track/channel ordering, nested folders, track colors, folder collapse, compact tracks, and safe track deletion.
- System-following light/dark themes, bundled Inter typography with Unicode fallbacks, shared text components, and vector toolbar icons.
- Audio input recording with mono/stereo selection, dry monitoring, loop passes retained as separate takes, and independent overdub layers.
- MIDI performance recording with channel/group selection, plugin monitoring, loop overdubs, optional pre-count and an output-only metronome; high-resolution note/expression data remains in `.rill` projects.
- Audio take selection and non-destructive range comping, smooth boundary crossfades, imported alternatives, and retained sources in portable projects.
- Responsive audio move/trim/stretch gestures, time selections, transient/edge snapping, and quick live previews that refine in the background; exports retain full quality.
- Source slip, sample/millisecond/grid nudging, zero-crossing snap, track-local ripple deletion, and previewed Strip silence.
- Progressive audio waveforms: immediate coarse previews refine into transient-preserving, anti-aliased envelopes, with subtle spectral backdrops. Shared background analysis and cached geometry keep redraws light.
- Named undo/redo history with grouped gestures, saved checkpoints, and bounded memory.
- One automation track per parameter, with adjustable spline segments, clear manual overrides, undo/redo, and playback/export integration.
- Input/output-device, sample-rate, and buffer-size setup; writable scratch directory and custom VST3 scan directories.
- Portable `.rill` ZIP projects with a JSON manifest and source WAV assets, automatic plugin-state recall, scratch recovery, MIDI import/export, and background stereo WAV mixdown.
- An integrated OpenAI/Anthropic assistant and shared MCP tools for current-song composition, instrument assignment, mixing, audio processing/analysis, and recording.

See the [audio editor coverage and shortcuts](AUDIO_EDITOR.md) for common workflows, verification, and remaining advanced tools.

## CPU load

The menu-bar **CPU** meter shows total system usage. Click it to see the percentage for each logical core. A warning icon and **Overload** appear when total usage or any core reaches 95% in a one-second sample. The popup highlights each affected meter. The warning clears when usage drops below the threshold.

## AI assistant and MCP

Choose **Window → Setup → AI**, select OpenAI or Anthropic, enter your model and
API key, and apply. The assistant's **AI settings** button opens that page
directly. Before setup, **Connect AI** opens the same page and keeps your draft
and selected AI context when you Apply or Cancel. Open **AI** in the menu bar or
**Window → Assistant**. An empty Conversation offers up to three starting points for
the current scope. Choose **Examples** to browse more under **Compose**, **Develop**,
**Mix**, and **Learn**. Choosing an example adds it to your draft; it does not
run it or replace text you have already written. Review the prompt and choose
**Send**, or use Ctrl/Cmd+Enter while the prompt is focused. **Tips** explains
how to specify scope, musical constraints, instruments, and follow-up requests.
If recording, an unfinished edit, or a document operation prevents sending, the
composer explains what needs to finish. An unavailable local target stays scoped
to that object; reopen AI from the intended editor to choose a different context.

The menu-bar **AI** button and **Window → Assistant** open global AI for the
current song. For a local edit, open the piano roll, audio clip, instrument,
effect, effect chain, channel, routing or automation editor and choose **AI for
this context**. The **Scope** line names the pinned target, and that boundary is
also included in the AI prompt. Selecting something else does not retarget the
conversation. Local AI cannot edit another clip or channel, even indirectly
through a linked source; make a linked clip independent before changing its notes.

**Tools** lists only tools available in the displayed scope. Search, select a
tool to inspect it, and choose **Use tool** to append an editable prompt without
sending it. Local examples also match the opened object: “Humanize this phrase”
in the piano roll or “Normalize this clip” for audio. Global AI handles requests
that create tracks or span multiple objects. The host validates local tool calls
and resulting edits before committing them through the same undoable service
used by MCP. The assistant cannot open or save projects. Keys are encrypted in `settings.json` with the built-in application key, which protects
against casual inspection. Provider charges and usage limits apply.

After a result, audition it with the transport and ask for a specific refinement.
**Undo** in global AI reverses the latest song edit; use **Edit → Undo** for
local results. A request that makes several edits can require several undo
steps. **Undo** appears when an edit is available to undo. During a request,
**Stop** cancels further assistant work but keeps edits already committed.
After a failed or stopped request, **Edit last prompt** appears beside the primary
action when the draft is empty. It restores the request for editing without
sending or undoing anything. Failure details remain readable in Conversation;
review any edits already made before sending again. **More → Edit last prompt**
also makes a completed request available for editing. **More → New chat** clears
the conversation while preserving the song and any unsent draft. Opening a
different AI scope stops the old run and starts a fresh conversation. Changing
songs also clears the draft so old requests do not carry over.
See [AI examples and composing workflows](AI_EXAMPLES.md) for a practical start.

Launch `rill --mcp` to expose the running native app's current song over MCP
stdio. `rill --mcp-application` explicitly adds project I/O and global Setup
tools for external clients; the embedded assistant remains song-only.
The initial tool surface covers composition, VST3 assignment/parameters,
mixing, clip processing, transport, and recording; complete GUI tool parity is
ongoing. Audio analysis uses local measurements; models do not yet receive audio.
See [AI architecture, tool inventory, and verification](AI_TOOLS.md).

![Rill piano roll in dark mode](../artifacts/rill-piano-dark.png)

[View the light theme](../artifacts/rill-piano-light.png). Appearance follows the system by default; change it in **Setup → Appearance**.

## Working with the editors

The **File**, **Edit**, **Track**, and **Window** menus hold application commands. Transport and contextual editing tools stay beside the canvas. Use **Track → Add audio track / Add MIDI track / Add bus / Add folder** to build the session and **File → Import…** to import media. **Add MIDI track** creates a pattern at the cursor that matches the loop length (up to one bar) and immediately opens the instrument chooser. Choose an instrument to replace R1 Mini and return to your caller; closing the chooser keeps R1 Mini.

**File → Import audio…** always creates a new audio track and clip at the cursor.
WAV, AIFF/AIFF-C, FLAC, MP3, Ogg/Vorbis, AAC, M4A (AAC/ALAC) and CAF
(PCM/ALAC) decoding runs in an isolated background worker;
the progress notice offers **Cancel import**. A drop captures its track and
position before decoding, so later selection changes cannot redirect it. A
deleted destination fails clearly. New/Open/Quit cancels unfinished imports.
The existing **Import…** command also retains MIDI-file import.

Up to eight files form one atomic import and one Undo action. If any file is
corrupt, unsupported, too large or cannot finish, the batch creates no tracks or
clips. Mono/stereo audio at 8–384 kHz is supported, up to just below 4 GiB
of canonical decoded audio per file and 8 GiB per batch. This is bounded RIFF
WAV media, not RF64 or unlimited-length audio. Successful imports use unique managed WAV copies in the scratch folder;
original files are untouched. Undo retains those copies for Redo and recovery,
and saved `.rill` projects include them for portable reopening.

The MIDI Clip Editor keeps **Instrument** beside its editing actions after a sound
is assigned. Open it to inspect or change the sound, then close it or press Escape
to return to the same phrase, selected notes, and editing position. An unassigned
track shows **Choose instrument…** in that position.

Arrangement is the main canvas. **Window** opens one focused tool at a time:

| View | Use |
| --- | --- |
| **Clip Editor** | Edit MIDI notes or inspect the selected audio waveform. |
| **Instrument** | Choose, edit, and automate a VST3 instrument for the selected MIDI track. |
| **Effects** | Add, reorder, bypass, and edit the selected channel's inserts. |
| **Mixer** | Balance channels, with shortcuts to their instruments, effects, and routing. |
| **Routing** | Choose the output bus and configure pre/post-fader sends. |
| **Recording** | Select the audio track's input, arm capture, choose dry monitoring, and record takes or an overdub layer. |
| **Automation** | Create parameter tracks and edit curves for channel, send, insert, and VST3 controls. |
| **Properties** | Edit names, levels, audio fades, colors, folder membership, and compact/collapsed state; inspect clip timing. |
| **Device browser** | Search, scan, and assign instruments and effects. |
| **Assistant** | Open global AI for the current song; editor AI buttons target only their opened context. |
| **Setup** | Configure Audio, Storage, Plug-ins, Appearance, and AI on separate pages. |

Clip Editor and Mixer use the whole workspace, with **Back** to return to Arrangement. Routine utilities and parameter menus keep the surrounding workspace sharp and lightly dimmed, so notes and values remain readable while only the foreground view accepts edits. Explicit confirmation dialogs use a stronger dimmed, blurred background where the renderer supports it. Transport and application menus remain available. Close a utility, press Escape, or choose **Window → Arrangement** to return. Nested tools restore their caller and its editing position when dismissed.

Editing tools stay in a single shared strip. Arrangement, MIDI and audio retain their own selected tools when you move between them. Audio **Clip** and **Process** menus reveal trimming, copy/paste, normalization, reversal, and transient analysis; fade length and curve appear in the tool row with the selected fade tool. **Snap** follows the tools in both editors. MIDI **Notes** holds note transformations; **Length** and **Key** sit beside Snap. Secondary controls appear when needed, leaving the waveform or notes in focus.

The toolbar's available space shows the active tool's next mouse gesture. For
example, **Duplicate** selects a tool: click a clip or note to copy it. The
context-menu **Duplicate after** command makes one independent copy immediately after the clip or selected phrase.
Choose **Tool help** in the toolbar for the active tool's gestures and shortcut.
Escape or a click outside closes the help. Hovering any tool also explains its
use, leaving the waveform and notes free of a permanent instruction strip.

Arrangement and both clip editors show **− / + / Fit / Fit selection** together.
Use **−/+** to zoom horizontally around the center of the current view, **Fit** to
show all content, or **Fit selection** to frame selected clips, notes or an audio
time range. **View → Previous view** returns to the previous framing. These
controls leave the music and selection unchanged.

In the piano roll, **Generate…** opens Chord, Arpeggio and Melody modes. Start bar and Length initially cover the completely visible bars; edit either field to change the highlighted range. Choices and range are remembered per clip for the app session, including after Cancel; **Next range** advances by the current length while retaining settings. Violet notes preview the result alongside existing notes. **New variation** rerolls random patterns, **Fit preview** reveals their full extent, and **Add notes** commits the exact preview and keeps the generator open. Use **Next range** to continue, then **Done** to close it. Before adding notes, **Cancel** discards only the preview; closing the generator after an addition keeps the committed notes. Parameter controls scroll independently so the range and confirmation buttons remain reachable.

Double-click a track name or icon in Arrangement to open Track Properties. Type to replace the selected name, click **Color** to pick a tint, or choose an **Icon**. **Track type** restores the default icon. A folder’s chevron opens and closes its children. Volume and pan sit beside **M**ute and **S**olo; **Routing** and **Instrument** or **Input** each have their own row. Stereo output meters occupy a dedicated column. Drag a volume or pan rail, or click its value for exact entry. Pan shows **C** at center and **L/R** percentages elsewhere. Cancel, Escape or clicking outside the value popup discards the draft; displaying a rounded caption never changes stored precision. −60 dB silences the channel.

Choose **View → Track density → Overview** to see more of the arrangement. Its 34-point rows keep clip previews visible and put M/S beside each channel name. The sliders icon opens the full channel controls. **Compact** is the default and includes volume, pan, arm and meters in a 64-point row. **Standard** adds room for the volume and pan rails. **Detailed** shows routing and source controls. Rill remembers your density across projects and restarts. Fit and manual row resizing are temporary view changes.

In Detailed headers, click **Routing** to choose Master or an eligible output bus. **Instrument** chooses or replaces the MIDI track's VST; **Input** opens audio recording and monitoring settings. The default label width is 288 points. Very short rows use readable value controls instead of miniature rails. Folders and automation tracks have no audio meter or gain/pan controls; folder mute/solo acts on descendants.

Clip names are optional. Use the clip's context menu **Rename…**, clear its name and choose **Apply name** to hide the title band. A thin top border remains while the waveform or notes gain the space. Double-click the clip’s title strip to rename it, including the thin strip on an unnamed clip. Double-click its body to edit the notes, audio, or automation points. The context menu also offers the corresponding edit action. Clearing/restoring a name supports Undo/Redo and saved projects; linked copies share the name.

Gain captions show one decimal place; hover or open the value editor to see its
full precision. Simply viewing or cancelling an edit preserves the stored value.
The Master strip meters its processed signal before device clipping, dry input
monitoring and metronome. Live bars fall to silence when paused/stopped; peak
holds remain available until reset. The transport device meter is separate.

In Arrangement, **Add pattern** creates a MIDI pattern at the cursor on the selected MIDI track, or creates a track when needed. On an existing track, new clips inherit the last selected clip’s length. A new instrument track starts with a pattern the length of the loop, up to one bar in the meter at its insertion point. Right-click an audio track and choose **Insert sample…** to pick an audio file. On a MIDI or automation track, choose **Insert clip**. A right-click on empty lane space uses that spot on the grid. A right-click on the track header inserts at the snapped playhead. Double-clicking empty MIDI or automation lane space also creates a clip. Double-clicking below the tracks creates a new MIDI track with its first clip at the clicked, snapped position. **Add track…** below the last track, and **Add track** in a track's context menu, add instrument, MIDI, audio, bus or folder tracks (automation tracks come from right-clicking a control and choosing **Automate**); from a track's menu the new track follows that track. Select **Window → Clip Editor**, or the clip's **Edit notes** / **Edit audio**, to open it. Drag a clip to move it; drag its edges to trim its length. **Properties → Clip** provides editable **Start**, **Duration** and **Source offset** in beats. Visible **−/+** buttons step each value by one beat; drag the number for fine adjustment or click to type. Start is one-based, and each step supports Undo. Tempo has matching buttons that step by one BPM. While trimming an arrangement clip, a pointer readout shows its current end and duration in seconds and beats. Use **Edit → Split at cursor / Duplicate**, or the clip's context menu. Right-click a track header for properties, collapse/expand, or deletion; use **Window → Effects / Routing** for its signal path. Arrangement **Fit** keeps sparse track rows at a readable maximum of 144 points while fitting the song horizontally.

Right-click an audio or MIDI clip and choose **Properties** to edit that clip's
timing directly above Arrangement. In Clip Editor, **Properties** stays at the
same end of the header for both clip types. The header shows the track and the
clip's **Song** start/end bars and beats; hover for clock times and the ruler's
time basis. MIDI's **Local** ruler starts at bar 1 for the open phrase, even
when that phrase repeats later in the song. Audio's ruler shows song positions.

Clip Properties also shows **Starts at**, **Ends at** and **Elapsed** in clock time
and bars/beats, so you can check a clip's exact timing without converting units.
Beside **Start**, click the **Bar · Beat** position to enter a bar and beat
directly. For example, Bar 5, Beat 1 places the clip after four bars. Type in either field, drag it, or use its −/+ buttons. **Apply position** commits
the move in one Undo step. The beat field accepts fractional beats and follows the song's meter.

| Shortcut | Action |
| --- | --- |
| Space / Home | Play-pause / return to start |
| Cmd/Ctrl + S / O | Save / open session |
| Cmd/Ctrl + Z / Shift + Z | Undo / redo; named steps under Edit → History |
| Cmd/Ctrl + A | Select all notes in Clip Editor |
| Cmd/Ctrl + D | Duplicate selected notes, or the selected clip |
| Delete / Backspace | Delete selected notes or clip |
| Q / W / E / R / T / Z / U / I / O / P | Select / pencil / split / duplicate / glue / erase / multi-draw / fade in / fade out / cursor (where applicable) |
| F / Fit | Fit content on both axes |
| Middle drag / Alt + left drag | Pan |
| Ctrl + wheel / Ctrl + Shift + wheel | Horizontal / vertical zoom around the pointer |
| + / −, with Shift | Horizontal zoom, or vertical zoom with Shift |
| Shift + click | Add/remove a note from selection |
| Right-click a note | Open the selection menu to add or remove the note |

The piano roll separates note actions from the **View** menu. Note-tool controls keep their positions when switching tools. Hover a row or the piano keyboard to reveal its pitch names. Inside the grid, a pitch cue follows the pointer, so short notes and dense rows stay identifiable. While drawing, it shows the starting pitch. **Highlight scale**, **Highlight chord**, and **Only scale keys** are independent view options; hidden notes remain intact. **Scale guide C · Major** shows the current root and scale, with root/scale/chord choices inside. A fresh editor starts in C major with a major chord; choosing another scale or chord does not retune existing notes. The keys column is drawn as a keyboard; natural-key rows use a neutral base and black-key rows are slightly darker; scale and chord guides add a subtle green tint. Notes have a neutral fill, with stronger shades for higher velocities. Selected notes keep their shade and gain a contrasting outline. Notes outside the scale guide have a small blue mark, with the tooltip **Outside the selected scale guide**. Turn off **View → Mark out-of-scale notes** to hide those marks. Folded dragging and arrow nudges follow visible scale steps, while Shift+Up/Down transposes by octaves.

Instruments that provide note names open in **Named notes** view. R1 Drums shows its sixteen pads with loaded sample names, or **Pad N · Empty** when no sample is loaded. CLAP and VST3 instruments can supply their own preset note names. Click a row label to audition it and draw on that row to enter its exact MIDI note. Scale guides and chord drawing stay in the piano view. Use the **Named notes / Piano roll** picker to switch views; **Other MIDI notes (N)** reveals notes outside the map without changing them. Instruments without note names use the full piano keyboard.

The piano roll shows every tool directly: **Select**, **Pencil**, **Multi-draw**, **Cursor**, **Cut**, **Glue**, **Erase**, and **Duplicate**. Separators distinguish related tool families; the toolbar wraps when needed. The active tool keeps its glowing border, and all shortcuts remain available.

With **Pencil**, press at the note's starting time and pitch, then drag to choose
its length. The starting pitch stays fixed while drawing.

The select tool supports dragging a rectangle around notes. Drag note bodies to change time/pitch, and either edge to change length. **Length** chooses the initial drawing duration from **Last note**, fixed note values, or **1 bar**; Last note starts at 1/16 and remembers the last drawn or selected length. In Select or Cursor, double-click empty space to create a note as long as the last one you clicked. Before you click a note, the new note spans one grid step. Double-click a note with Select to delete it. Drawing past the marked **Clip end** extends the clip with the new note in one Undo step. The velocity lane supports individual and swept edits; selected chord/scale stacks fit within one octave. See [editor controls](EDITORS.md). The duplicate tool places a copy after a note; group duplication preserves the group's spacing. The cut tool splits at the snapped pointer position. Glue joins compatible notes from their earliest start to latest end, including gaps; use the tool on two notes or **Glue selected** on a selection. Expressive notes remain independent during automatic containment cleanup so channel, attributes and per-note data cannot be discarded by a coarse pitch match. Leaving Clip Editor clears note selection so arrangement edit commands act on the clip.

Select notes with the mouse or **Edit → Select all notes**, then open **Notes** for Humanize, Quantize, lengths, flips and Legato. These actions are available with every tool:

- **Humanize…** previews repeatable velocity and timing variation. **New variation** changes the seed; **Apply humanize** commits the preview and **Cancel humanize** discards it. Pitch and length stay fixed.
- **Quantize…** previews alignment to the chosen beat grid, with Strength, Swing, and optional Note ends. Apply or cancel before continuing to edit.
- **Notes → Legato top to bottom** connects the highest note at each onset to the next highest, then the second-highest, and so on. A missing lower voice bridges to its next appearance. Only selected note lengths change, and the last note in each voice retains its length. **Notes → Legato** instead connects every selected note to the next distinct onset, keeping simultaneous chords together.

**Notes → Note lengths…** previews **Fixed**, **Grow** or **Shrink** in audible
beats. Fixed uses one length; Grow/Shrink apply one group delta, limited by the
shortest note, preserving length differences. **Apply lengths** commits one Undo
step and **Cancel lengths** restores the preview. Starts and clip bounds stay
fixed, and hidden expression data is retained. **Flip horizontally** mirrors
selected note timing within its span; **Flip vertically** inverts selected
pitches within their range. Shift-drag or Cmd/Ctrl-drag duplicates selected
notes without changing the originals.

Resizing selected note edges uses one group delta, limited by the shortest note
or source zero. Shortening a note keeps later expression data available if it is
extended again. Moving its start retains the latest expression state at the new
edge and keeps later points at their original absolute time. Modifier-dragging
an edge still resizes; duplication uses the note body.

The MIDI editor's **Clip** menu and a clip's **MIDI timing and pitch** context
submenu offer **Half time**, **Double time**, **Semitone up/down** and **Octave
up/down**. Half time doubles duration, Double time halves it; both keep the
start and source notes, affecting only that occurrence's stretch. Transpose
changes the complete shared note source, including trimmed notes and ghosts.
Bounds are checked before an edit. Clip **Quantize…**/**Humanize…** preview notes
intersecting its audible window; the Notes versions use the selection.

Each applied action is one Undo step. Humanize, Quantize and length previews
leave the song unchanged until Apply. Cancel restores temporary folding/view
changes; a song edit or clip change discards an obsolete preview. **Loop this
clip** uses exact audio/MIDI/automation clip bounds without changing workspace.

### Note expression

Select one note and choose **Notes → Expression…**. The popup edits **Attack
velocity** and **Release velocity** at 16-bit precision, and **Note pitch** at
1/512 semitone resolution. Original MIDI 1.0 attack precision remains intact
when editing another property. Use the steppers to adjust a value or right-click its field to type an exact value.

Per-note **Pressure**, **Pitch bend**, **Registered controller** and **Assignable
controller** entries use **After note** time in audible beats, an exact 32-bit
value and a normalized slider. Controller entries also choose a controller
number from 0–127. **Add point**, **Remove point** and **Center
pitch** edit a local draft. These are timestamped packets, not interpolated
automation segments. Unsupported raw event types remain retained/read-only
unless explicitly removed. **Apply** commits once; Cancel, Escape or closing
discards the draft. A stale source/session cannot apply over newer edits.
The destination VST3 must support the relevant musical mapping to sound it.
Per-note pitch-bend packets remain saved but are currently unsupported in
playback; Rill does not assume an arbitrary range. Broader
unsupported packet families and program changes are retained. A saved performance
using multiple groups needs separate tracks for complete playback through the
current VST3 event-bus mapping; an advisory identifies skipped data.

## Clips and looping

The arranger shows every tool directly, separated into related families: **Select**, **Pencil**, and **Cursor**; **Cut**, **Join**, and **Erase**; **Duplicate** and **Ghost copy**; **Stretch** and **Slip**. The toolbar wraps when needed. The active tool retains its glowing border; hover or **Tool help** shows its instructions and shortcut. The shortcuts remain **Q Select**, **W Pencil**, **E Cut**, **R Duplicate**, **T Join**, **G Ghost copy**, **S Stretch**, **L Slip**, **Z Erase**, and **P Cursor**.

Audio and MIDI clips share these tools; Pencil creates MIDI clips and Slip moves audio within fixed clip edges. Select drags move clips; either edge trims the visible source window. Shift-dragging the body creates one independent copy, while Shift at a trim/stretch edge temporarily bypasses snap. Join opens a focused chooser. Actions select their resulting clip consistently, whether invoked from tools, menus, or the context menu.

With several clips selected, click and release one member to select only that
clip. Drag a member directly to move the whole group. Use Shift-click or a clip's
Add/Remove selection context action to change group membership. **Erase** removes
only the clip you click, including one half of a cut when both halves are
selected. To remove the whole selection, use **Edit → Delete** or the selected
clips' **Delete** context action.

**Snap → Grid**, the default, snaps to the grid lines currently shown, so zooming in gives finer steps and zooming out coarser ones, from 1/32 notes up to several bars. Choose **Snap → 1 bar** to align edits to the current meter: three beats in 3/4,
four in 4/4. Changing the meter later keeps the existing beat increment and shows
it explicitly, such as **Snap 3 beats**; choose **1 bar** again to use the new meter.

### Meter changes and markers

The transport meter chooser sets the meter of bar 1. To change the meter later in
the song, right-click the arranger ruler and choose **Insert meter change…**, or use
**Edit → Markers → Insert meter change at playhead…**. Pick the starting bar and
the meter, then **Apply meter change**. Clips keep their beat positions; only the
bar lines around them move. A flag on the ruler marks each change: click it to
edit or move it, or right-click it to **Remove meter change**. In the arranger,
**Snap → 1 bar** follows the real bar lines; the clip editors use the bar length
at the clip's start.

Markers name song positions. Choose **Edit → Markers → Add marker at playhead**
(**M**), or right-click the ruler and choose **Add marker here**. Click a marker
flag to move the playhead to it, drag it to move it with snap (Shift bypasses
snap), and double-click it or choose **Rename marker…** from its context menu to
rename it. **Delete marker** removes it. **Previous marker** (**,**) and **Next
marker** (**.**) step the playhead between markers. Every change is one Undo step.
MIDI export writes meter changes as time signatures and markers as marker events;
MIDI import reads time signatures that fall on bar lines when importing into an
empty song.

Clip Properties always shows **Stretch**, starting at **100.0%**, so its timing
controls stay in place when a duration edit changes between Trim and Stretch.

Click a tempo, clip timing or loop boundary field to type a number, or drag
it to adjust the value. Right-click the field to see its exact value and allowed
range. **Apply value** commits one edit. Generate's Start bar and Length
accept whole bars.

Stretch changes duration without replacing the source. Audio keeps its pitch; MIDI note positions and lengths scale in playback and the editor. Stretching one ghost changes only that occurrence's timing. Source-rate factors range from 0.25× to 4×, and each drag remains one Undo step. In Properties, select **Stretch** and right-click Duration to see the supported range for that occurrence. For an unstretched four-beat clip it is 1–16 beats; trimmed or already-stretched clips use their current source window. Values outside the range stay as a draft until corrected. Preparing stretched audio happens in the background; the current sound continues until its replacement is ready. Extreme stretches can introduce artifacts, especially in transients and complex material.

### Warp markers and audio quantize

The audio Clip Editor's **Warp** tool (**K**) moves individual attacks without
cutting the clip. Detected attacks show as faint ticks. Click the waveform to add
a warp marker; near an attack it snaps onto it, and Shift places it exactly where
you click. The first marker also pins the clip's start and end. While Warp is
active the loop lane shows numbered marker flags: drag a flag to move that
moment of the audio (the grid applies; Shift bypasses it), click it to seek, or
right-click and choose **Delete warp marker**. The audio between markers
stretches smoothly with its pitch unchanged. **Process → Clear warp markers**
returns to ordinary whole-clip stretch. Each add, drag or delete is one Undo step,
and markers are saved with the project.

**Process → Quantize audio…** (also in the waveform's context menu) moves attacks
toward the grid. Choose the grid, **Strength** (100% lands exactly on the grid)
and **Max move**; attacks further from the grid stay put, and the clip's start and
end never move. Arrows preview each move and the drawer reports how many attacks
move and the largest move in milliseconds. **Apply quantize** is one Undo step.
**Markers only** places markers on the attacks without moving anything, ready for
manual adjustment. Sensitivity and minimum gap are the same transient settings
used by Process → Transients.

In the audio Clip Editor or Properties, open **Pitch** to transpose by up to two octaves in either direction, including fine tuning to cents, without changing duration. Choose **Signalsmith** or **Bungee** for both pitch and time processing. Combined edits render once from the original audio, never from an earlier processed result. Reset pitch restores the original pitch while retaining stretch. Ghost copies share pitch and algorithm; independent duplicates can differ. Both algorithms are included; no external processing utility is required.

Choose **Edit → Ghost copy** (or the clip context menu) to place another instance after the selected clip. A ghost is a linked copy: a chain icon marks every member of the group. Notes, audio edits, fades, clip gain, and the name are shared: edit any member to update the group. Position, trimming, stretching, and overlap crossfades remain independent. **Properties** shows the shared name and instance count before track controls.

Hover an arrangement clip to see its full name, track and start/end timing. This distinguishes repeated occurrences even when their titles are shortened by the current zoom.

Hover near an audio clip's upper corners or fade endpoints to reveal its orange
fade handles, in Arrangement or Clip Editor. Drag to adjust; double-click to
remove that fade. Handles hide when you move away, while applied fades retain a
subtle shaded envelope over the waveform. The lower incoming corner reveals an
overlap's crossfade diamond; drag its duration or double-click to shape its curve.
Each drag remains one Undo step.

Right-click a track header to **Collapse all automation tracks** or **Expand all automation tracks** belonging to that channel. A folder applies the action to its channels, and the **Tracks** menu applies it to the whole arrangement. Collapsing a lane changes only its display. Its automation keeps playing.

Edits that break a ghost link open **Linked copies** before changing the song. **Cancel** keeps the song as it is. **Propagate change** applies the edit across linked occurrences. **Apply and disconnect** edits just the selection. **Apply and delete all copies** keeps the edited selection and removes its other linked occurrences. One Undo restores the whole operation. Stretching an automation pattern rescales its point times, so it asks for this choice. Audio and MIDI occurrence stretching already preserves the link.

**Duplicate** creates an independent copy; in the MIDI editor with notes selected, it duplicates those notes instead. **Make independent** detaches the selected clip with its current name and edits intact. Deleting the original leaves the remaining ghosts connected. Splitting keeps linked parts in the group. Links survive undo/redo and portable project save/reopen.

Audio clips on one track are exclusive: the active clip with the latest start wins. If starts match, the later entry in the track wins; an earlier clip resumes when a shorter overlapping clip ends. Splitting creates a new start and can therefore change overlap priority. Overlaps switch immediately unless you enable **Edit → Crossfade…** (also in the clip context menu). Set its duration, choose Linear or Smooth, or drag the two spline handles. The complementary gain curves appear over the overlap in Arrangement. Crossfades are limited to the available overlap and next clip takeover; they apply at clip entry, not when an older clip resumes.

Use **Edit → Join clips…** or the clip context menu to choose clips on one track. MIDI joining preserves visible notes and timing. Audio joining renders source edits, overlaps, and crossfades to a new WAV in the scratch directory, including silent gaps; channel processing stays live. Include every audio clip intersecting the joined span. The result is an ordinary clip that can be split, trimmed, reversed, or joined again. Joining is undoable and its source travels inside saved `.rill` projects.

Select a clip or a group of clips, then choose **Range → Loop playback over selection** to repeat playback of that phrase. This sets playback bounds; use **Edit → Repeat to loop end** to create independent copies through a later loop endpoint. A time selection made with **Cursor** includes its leading and trailing rests. The action sets the range and enables looping together, without moving the playback cursor; one Undo restores the previous range and loop state. A clip's context menu offers one playback action: **Loop playback over clip** for a single clip, or **Loop playback over selection** for a selected group or time range. Right-clicking another clip loops that clip without changing your selection. Beside it, **Duplicate after** creates one independent copy and **Repeat to loop end** creates copies through the loop endpoint. When the endpoint is too early, the disabled repeat action explains **Extend the loop beyond the phrase**. In Clip Editor, **Range → Loop playback over clip** repeats the open clip.

Right-click the loop strip and choose **Start loop here** or **End loop here** to set that boundary at the clicked grid position. The transport's **Loop** toggle switches repetition on or off. **Range → Loop playback over arrangement** sets and enables a loop from the beginning through the last clip, preserving the selection and playback cursor. **Range** also opens **Start beat** and **End beat** for exact adjustment; use their **−/+** buttons for quarter-beat steps, drag the values, or double-click to type. Beats start at 1, and the end is exclusive: Start 1 / End 5 repeats the first four beats. The read-only **Starts at**, **Ends at** and **Elapsed** rows show clock time and bars/beats. The ruler's loop strip shows the same positions and times when space allows; hover it for the complete summary.

Drag either visible endpoint of the strip to adjust the loop directly. The drag follows **Snap**; turn Snap off for free adjustment. A pointer hint shows the endpoint and duration while dragging. Each completed drag is one Undo step. Only actual endpoints have handles, so a loop extending beyond the view must be brought into view before dragging its hidden end. Range changes are disabled while recording and saved with the project.

**Range → Duration** shows seconds and offers exact 5, 10, 15, 30, 60 and 120-second durations from the current loop start. Select a phrase and choose **Edit → Repeat to loop end** to fill that range, including a partial last copy. WAV export still renders the full arrangement once.

## Freeze and return to originals

Right-click an audio or MIDI clip and choose **Freeze clip**, or right-click its
track name and choose **Freeze track**. Rill renders into new managed audio,
places an audio track immediately below the source (below its output group for
a multi-output instrument), and mutes the printed original clips in one Undo
action. Track Freeze creates one clip from the first unmuted clip's position
through the last clip and the channel's reported effect tail. A multi-output
instrument prints all configured output channels together from one instance.
Reported tails are capped at 30 seconds; a Freeze batch is limited to 512 MiB
of decoded stereo float audio.

The print includes clip processing, input gain, polarity and inserts. Width, volume, pan, sends
and their automation remain editable on the new channels; Master processing
stays live. Sidechain consumers retain their signal through parallel routes to
the printed channels. When every original clip is frozen, the original channel
and its instrument outputs are muted too. Other clips on a partly frozen track
remain active. **Cancel Freeze**, a rendering error or a changed song discards
the pending print without muting any originals.

Use **Return to originals** from an original or printed track/clip context menu
to restore the original clips and the channel mutes changed by Freeze. Printed
tracks, audio files and edits stay in the project, with their printed clips
muted. Unchanged sidechain route copies are removed; edited copies remain.
Freeze and Return both support Undo/Redo and portable save/reopen. Splitting an
original while frozen preserves its return references. Removing or joining away
an original, or missing original media/plug-ins, can make Return unavailable;
Rill keeps the printed audio intact and reports the missing original instead
of discarding work. Frozen audio remains usable without the source instrument.

Freeze clip prints that part in isolation through its channel. Use Freeze track
for shared processing across overlapping MIDI parts. Audio clips overlapping
unselected clips must be frozen together with Freeze track, so muting a clip
cannot reveal an older clip beneath its printed audio.

## Effects, sends, and folders

Mixer channels read from source and **Gain** through **Effects**/inserts, pan,
**Mute**/**Solo**, volume fader and level readouts, then **Output** and sends.
Gain is pre-insert trim; Volume is after inserts. Pan is a balance rail with a
small center point and **C** or **L/R** readout. Drag to adjust, Shift for finer
movement, or right-click for exact entry and automation. Faders and stereo
meters share a calibrated dB scale; amber Mute and green Solo make states clear.

Master has the same Gain, inserts, pan and volume automation, plus Mute, with no
Solo or sends. Its Effects/Channel controls expose the real processing chain.
Master strips and Analyze measure after those controls, before device clipping,
dry monitoring and click; the transport meter shows device output separately.

Instrument outputs share a group header, **name · N outputs**. The primary
channel starts expanded; auxiliary outputs start folded with their names,
meters, M/S and routing available. The header expands/collapses all auxiliaries;
each chevron controls one output. Revealing an output expands it. Folding is
view state and never changes the sound. Expanded outputs have independent Gain,
inserts, volume, pan, sends and output routing. Reordering keeps the instrument
group together, while Shift left/right on an output moves among its siblings.

**Analyze** offers Spectrum, Levels, Loudness, Correlation and Phase. Levels
shows sample peak/hold, L/R RMS, an oversampled true-peak estimate/hold and overs
counts. RMS uses a 300 ms exponential mean-square time constant, not a VU or
boxcar measurement. True peak estimates inter-sample levels to at least 192 kHz
(4× at 48 kHz, 8× at 44.1 kHz); it is not a certified compliance meter. Loudness
uses K-weighted 400 ms/3 s windows and an approximate gated integrated LUFS
reading. Reset, seek/restart or a target change starts a fresh measurement;
Stop holds the last analyzer reading. Opening or switching views never edits
the song.

### Plug-in inputs and outputs

Open **Window → Routing**, select the instrument/insert under **Plug-in inputs
and outputs**, then choose **Inspect I/O**. Inspection first saves the native
editor state and runs in a separate process. **Refresh I/O** checks a changed
preset/configuration. Available ports use the plug-in's bus names and native
primary mapping rather than assuming its main bus is number one.

For an extra input pair, choose **Add source…**, its level and **Pre volume**
when needed. Pre volume taps follow the source's inserts but precede volume/pan;
otherwise the tap follows volume/pan. Several sources can sum into one pair.
This connects sidechains and audio-input instruments; a processor must actually
support that input. The built-in R1 Compressor has one, named **Sidechain**;
it is listed without **Inspect I/O**. Feedback-producing choices are excluded. Ordinary insert
input stays connected to its channel.

Use **Enable output…** or **Enable all output channels** for an instrument.
Wide buses appear as stereo pairs, with a final odd channel treated as mono.
The instrument runs once; its outputs feed independent grouped mixer channels.
**Show in Mixer** reveals one and **Remove output** removes its channel with
Undo. These are stereo/mono channel pairs, not surround or Atmos routing.

The Mixer toolbar places **Add send…** beside **Add bus**. Select a channel and
choose **Add send…**, then an available bus, or **New bus and send** to create
both together. Opening the chooser changes nothing; each completed action
supports Undo, including bus and send together. Existing sends and destinations
that would cause feedback are excluded. New sends are post-fader; open the
channel's sends control to adjust their level or choose Pre/Post in Routing.
Pre-fader sends follow inserts but bypass volume and pan; both types respect
Mute. **Add bus** creates and selects a bus without adding a send.
**Add bus** and **New bus and send** choose **Bus 1**, **Bus 2**, and so on,
skipping names already in use. The destination list keeps existing buses
separate from the creation command.

An empty channel's own **Add send…** opens the same destination choices directly.
The menu names its source, so you can add a send without changing the selected
channel. Click an existing send summary to edit its settings in Routing.

Routing gives each send a text-free level ring and a clickable value beside
**Level**. Drag up or right to increase the send, down or left to
decrease it; Shift makes adjustments finer. The taper gives quieter sends more
useful travel while retaining the 0–16× gain range. Click the value for named
exact entry. The value readout follows the dial.

Selected clips and mixer channels have a subtle glow in their track color. Hover or select a mixer channel to trace its output and sends: continuous U-shaped arrows blend from that channel’s color to each destination’s color. Mixer reserves space for its output and first send route plus an ordinary notice; shorter windows and additional send lanes still scroll. Notices and added sends keep the existing fader, pan, and mute/solo controls in place.

Drag an Arrangement track's name or grip to reorder it. The insertion line shows
Before or After; drop in the middle of a folder header to move inside it. Folders
move with all their contents. Mixer defaults to **Song order**, following Arrangement. Drag a channel's
small grip or name the same way, including buses; both workspaces share this
saved order. **Signal flow** sorts channels automatically: right-click a channel
and choose **Use Song order** to enable manual ordering. The same context menu
offers **Shift Left / Shift Right**; Arrangement offers **Move Up / Move Down**.
These one-step moves stay in the current folder and move folders with their contents.
Dropping before or after a track
places it in that track's folder. Hold near a pane edge to scroll during a drag;
Escape cancels. Each completed move supports Undo/Redo and saves with the song.

Click the effects button beside a track's name in Arrangement: it opens the track's only effect, the effects list when there are several, or the device browser when there are none. In Mixer, click an insert row or the instrument row to open that device. You can also select a track and choose **Window → Effects**, or use **Channel → Effects** in Mixer. **Insert before…** and **Insert after…** open the device browser at that slot. **+ Insert** appends to the chain. Drag a slot handle to reorder it, or right-click the card to insert, reorder, bypass or remove an effect. **Open editor** opens the native effect controls and parameter automation. **Create send track** in the browser creates a return bus with the selected effect and a send from the source. Set the plugin wet/dry control as appropriate. To create a shared effect return:

1. Choose **Add bus** in Mixer or **Track → Add bus**, then rename it in **Window → Properties**.
2. Open **Window → Effects** and add a VST3 effect to that bus; use 100% wet when appropriate.
3. Select each source channel and choose the bus from Mixer's **Add send…**, or add it in **Window → Routing**.

To group channels for shared processing, choose their output bus in **Window → Routing**. A bus can feed another bus. Folder membership is independent of audio routing: set **Folder** in Properties to organize tracks. Folder mute/solo applies to descendants. Track color is available in Properties; track-header context menus provide collapse/expand.

## Track headers

Each track has a clear order: its color, icon and name; volume, pan and **M/S**;
then **Routing** and **Instrument** or **Input**. A separate stereo meter shows
the channel's output level. Bus tracks offer **Channel** for effects and sends.
Click a routing destination to choose its output bus, an instrument name to
choose its sound, or the input row for recording and monitoring settings.

Drag the volume or pan rail to adjust it. Click a displayed value for precise
entry; pan shows **C** at center and **L/R** percentages to either side. **M**
mutes the track and **S** solos it. Clicking the color strip opens the color
picker, while the name and grip retain track selection and reordering.

The default header is 288 points wide. **View → Track density** chooses Overview
(34 points), Compact (64, the default), Standard (89) or Detailed (144).
Resize the header gutter or a track's lower edge for temporary custom sizing.
The Compact density keeps mix controls and a channel-controls button. To collapse
one track, click the chevron beside its icon. The 21-point header keeps the color
strip, chevron, icon and name. Click the chevron again to restore the controls.
An audio, MIDI or bus header also offers **Channel** in its right-click menu.
A folder's chevron hides or shows its children. Each track's collapsed state is saved with the song.

On a collapsed audio, MIDI or automation track, colored bars show where clips
start and end. Double-click a bar to open its editor. A closed folder shows its
children's activity in the folder color. Expand it to open a child's clip.

## Automation

Right-click a channel Gain, volume fader, pan control, or send level to enter an exact value;
Routing also opens it directly from the send's value button. The menu identifies
the channel and parameter, plus the destination for a send, and shows a readable
current value and allowed range. Hover the field for its full stored value.
Opening the editor or choosing Apply without editing does not round the stored
number or create an automation override. Type in the field, then choose **Apply value** to
commit or **Cancel** to keep the original. This also works in compact track
headers. Volume entry uses dB (−60 dB means silence), pan uses −100% left through
0% center to 100% right, and send levels use a linear multiplier. Properties also
has a directly editable volume readout. Exact-entry fields retain fractional
values; concise displays do not change their stored precision.

Right-click a channel Gain, volume, pan, or send control and choose **Create automation** to start a lane at its current value. This includes Master and instrument-output channels. Master lanes appear as global automation rows above ordinary tracks; selecting one and returning to Mixer reveals Master. Gain is pre-insert trim (up to +24.08 dB); volume remains post-insert. Track-header rails keep concise values and a pan center point. Sliders retain endpoint captions and adaptive ticks; Mixer volume keeps its vertical fader, aligned dB scale and separate editable readout.

An orange envelope/zigzag icon and border identify automation, while a coral-red
highlight identifies a manual override; bypassed automation is muted. **Override
automation** holds the current value without deleting the curve; **Resume
automation** returns to the saved curve at the current playhead. Applying a new
value to an automated control creates the same visible override as dragging it.

The plug-in parameter browser keeps the parameter name and editable number
outside the dial. Small dials use simple rings; larger dials retain a detailed
scale and position index. Drag vertically or horizontally to adjust it; Shift makes
dragging finer. Right-click the knob for exact entry with **Apply value** and
**Cancel**. Its value is
explicitly a normalized percentage, because plug-in
unit names alone do not provide a physical-unit conversion. Stepped parameters
use their declared steps when edited, and automation state remains visible.

Right-click any control (volume, pan, a send, input gain, or a knob, menu or switch on a device panel) and choose **Automate**. Rill adds an automation track for that parameter, creates a one-bar clip at the playhead's bar and takes you to it in Arrangement. Lane and clip are one Undo step. Every automatable control carries a small corner indicator: a faint ring while it has no automation, orange when automated, coral red when overridden and gray when bypassed. Right-click an automated control for **Edit automation**, or choose **Window → Automation**, to open its editor. Double-clicking an empty part of the lane creates a clip there.

New automation tracks start collapsed. Colored bars show automation clips, and
short marks show individual lane points. Click the chevron to see and edit the
curve in place, or double-click a clip’s bar to open its Automation editor.

With **Pencil**, click to add points and drag to move them; other tools select, move, trim, stretch, split and copy automation patterns like audio and MIDI clips. Use **Beat**, **Value**, or **Delete point** for precise edits. New lanes start with straight **Linear** segments. Drag a line up or down, or drag its midpoint diamond, to bend only that segment toward a logarithmic or exponential shape. This works in Arrangement lanes, automation patterns and the focused editor. Select its starting point or diamond for **Log**, **Linear**, **Exp**, or precise **Slope** controls. Both endpoints and neighboring segments stay fixed, with no overshoot. Clicking a line still adds a point; each drag is one undoable edit. Existing Step, Smooth and Spline projects keep their saved shapes.

Input-gain, volume and send-gain points show **Gain** in dB, matching the graph. Click the
readout for exact entry. **Silence** sets zero gain.
Manual gain overrides use the same units. **Fit** frames the existing points;
**Fit song** shows the whole arrangement so you can place an ending fade even
when a new lane has points only in the first bar.

Automation covers the entire song. The first point's value applies before that point and when playback starts; the last value is held indefinitely after the last point. Controls show the value at the playhead even while stopped. Orange curves and control markers identify automation consistently across the app. Automation itself does not extend the song's duration.

Adjust an automated control to override its curve. **Overridden** and coral-red markers identify the manual value, while the original curve stays intact. An override belongs to that one control, not to its lane or clip: choose **Resume automation** or **Override automation** in the control's right-click menu, the channel status menu, or plug-in parameter details. Resume follows the curve again at the current playhead. While at least one control is overridden, a coral warning beside AI in the menu bar says how many (for example **2 automations overridden**); click it to clear every override and resume enabled curves together in one undoable action. **Edit → Cancel all automation overrides** provides the same action. Curve points, bypass states and selection stay unchanged. Overrides save with the project and support Undo/Redo. Disable or mute a lane to use the channel's stored manual value instead; this is shown as **Bypassed**.

For an instrument, open **Window → Instrument**, choose **Open editor**, and adjust a native plug-in control. **Automate last touched** creates its parameter track. In the native editor, right-click a control and choose **Automate in Rill**, or choose **Automate** and then either click the control in the plug-in editor or search for it by name in the list that opens. **Parameters…** lists a plug-in's parameters for editing, with **Automate parameter** per entry. Plug-in effects offer the same from their insert chain. Built-in devices such as R1 Mini need no list: right-click a control on the panel. Discovery and native editors run in separate helper processes. Mixer controls render sample-accurately; plug-in automation (VST3, CLAP, LV2, AU) is evaluated once per 256-frame block. Automation lanes do not extend song duration.

Built-in panels keep their output controls together at the bottom. **R1 Mini** opens on **Oscillators**, with both oscillator models and their source levels side by side. **Filter & amp** holds the filter and both envelopes. **Modulation** sets the LFO and its destinations. **Voices** controls voice allocation, glide, bend and drift. **R1 Filter** groups its circuit and cutoff controls together and pairs the LFO with its envelope follower. **R1 Spacer** has a tab for each engine. Choose **Echo** or **Reverb** to edit that engine while seeing both in the time display.

**R1 Equalizer** has a high-pass, four bands and a low-pass. Add it from **Window → Effects → + Insert** under **Built-in (R1)**. In the plot, drag a band's node to set its frequency and gain (hold Shift for fine steps), turn the wheel over it for Q or the slope, double-click it to switch the band on or off, and right-click it for its type or **Reset band**. Choose **HP**, **1–4** or **LP** below the graph to inspect that band. Selecting a band does not enable or alter it. The focused controls hold its frequency, gain and Q or slope, with a separate enable switch and type choice. Disabled bands remain editable so you can prepare them before enabling. The spectrum behind the curve shows the track before or after the equalizer (**Analyzer**) while it plays. **Mode** chooses the exact Digital curves or the broader, slightly coloured Analog ones, and **Auto gain** offsets the level change of the bands so a comparison with bypass is fair.

**R1 Saturator** adds harmonics and drive. Choose a **Model**: Transistor is an edgy clipper, Tube a warmer curve whose second harmonic you set, Tape a soft stage that also compresses. **Drive** is how hard the signal meets the curve; the next knob is named for what it does in the chosen model (**Hardness**, **Bias** or **Compress**). **Tone** tilts the result darker or brighter, and **Low bypass** keeps everything below its frequency clean, so bass stays solid under heavy drive. The picture shows the curve, lit where the signal reaches it while the song plays, and the harmonics it adds at that level. **Auto gain** takes back the loudness Drive adds, so you compare colour and not level; the number under it is what it applies. **Quality** trades processor time for cleaner highs and never changes the timing of the track. **Mix** blends the untouched signal back in.

**R1 Compressor** evens out levels, limits peaks or closes gaps, by **Mode**:

- **Compressor** turns down what is louder than **Threshold** by **Ratio**; **Knee** rounds the corner. **Attack** is how fast it reacts and **Release** how fast it lets go; **Auto** release adapts to the material. **Character** chooses how it behaves: Digital is exact and can look ahead, Tube is slow and smooth, Opto lets go fast at first and then slowly, FET grabs fastest and adds edge. **Makeup** restores loudness. **Auto makeup** adds a fixed amount for the set curve, shown beside the switch.
- **Limiter** lets nothing pass the **Ceiling**. Makeup becomes **Gain**, the level that drives it. It looks ahead 5 ms unless you set **Lookahead**; the track's timing is compensated. The ceiling holds with **Mix** at 100 % and **Output** at or below 0 dB.
- **Expander** turns down what is quieter than Threshold; with a high Ratio it is a gate. Attack opens it and Release closes it.

The picture shows the curve with its threshold as a handle (**T**, or **C** for the ceiling): drag it, or the line in the history beside it, to set the level; turn the wheel over the handle for Ratio; double-click it to reset. The history shows the last eight seconds of input and output with the gain reduction hanging from the top, and the meter at the right holds the largest reduction until you click it. Controls that do nothing in the current mode stay in place, dimmed.

The compressor groups **Sidechain**, **Dynamics** and **Timing** below the plot. To duck a pad under a kick, add R1 Compressor to the pad. Under **Sidechain**, set **Source** to **External**. The **Sidechain Duck** preset also selects this source. Click **No source** below the selector to open **Routing**, choose **Add source…** under **Sidechain** and pick the kick's track. The name of the kick track appears below the selector and its level drives the compressor. **High-pass** keeps bass out of the detector. **Stereo link** sets how closely the left and right detectors follow one another.

**R1 Modulator** is a chorus, a flanger and a phaser. Choose a **Mode**; on the panel this also sets a fitting **Feedback** and **Center** for it, and one Undo takes all of it back. Chorus thickens with one to four slowly moving **Voices**, Flanger sweeps a comb of notches (try negative Feedback for a hollow tone), and Phaser sweeps two to twelve **Stages**. **Rate** and **Depth** set how fast and how far the sweep moves around **Center**; **Sync** ties it to the song with a **Division** from eight bars to a sixteenth, so the sweep is at the same point in every bar. **Shape** is a sine, a triangle or a smooth random wander, and **Stereo** moves the right channel's sweep away from the left's for width. **Character** switches between the exact Clean sweep and Analog, which is darker and adds a little grit. **Low cut** keeps bass out of the effect, and **Mix** at 100 % in Chorus is a vibrato. The picture shows the LFO at the left (drag it sideways to change Rate) and the response it sweeps at the right: drag the handle **C** to move Center and turn the wheel over it for Depth; while the song plays, **L** and **R** mark where each channel is.

**R1 Spacer** has an **Echo** tab for repeat timing, delay-line colour and stereo return, and a **Reverb** tab for room, tail and stereo return. Above the graph, **Echo on** and **Reverb on** switch each engine independently. **Routing** feeds the echo into the reverb (**Echo → Reverb**) or runs both engines side by side (**Parallel**).

- **Echo.** Under **Repeats**, choose a **Model**. The choices are a clean **Digital** delay, a bucket-brigade **BBD** line that darkens and roughens as it slows down, or a **Tape** that saturates and wavers. With **Sync** on, **Division** sets the time in note values (D is dotted, T a triplet); with it off, **Time** in milliseconds. **Feedback** is how many repeats follow; at 100 % Digital repeats forever, and BBD and Tape build up into self-oscillation. **Taps** adds earlier heads on the same line for rhythms (Double, Triple, Quad, Dotted, Swing, Gallop, Golden); more heads are louder, so trim **Level**. **Stereo** places the repeats: Mono, Ping (alternating left and right) or Wide. **Diffusion** smears each repeat a little more than the last, until the echoes melt into a wash. **Color** is the tone of the model at hand, darker to the left: the high cut of the Digital repeats (the value shows its corner), the clock of the BBD line (left is slower, darker and noisier) or the condition of the tape (left is worn and driven hot). **Wobble** makes the time drift, most audibly on Tape, and **Low cut** thins each repeat a little more than the last. Changing the time repitches what is already in a BBD line or on the tape, as on the hardware.
- **Reverb.** Under **Room**, choose a **Model**. The choices are a **Studio** (small and dense at once), a **Room** or a **Hall** (large, building up slowly). Each is its own algorithm; no knob moves when you switch, and the old tail rings out. **Size** scales the space within its kind. **Decay** is how long it rings and **Damping** how much faster the highs die away. **Pre-delay** separates the sound from its reverb, **Diffusion** smooths the onset, **Movement** keeps long tails from ringing, and **Width** narrows or widens it. **Seed** gives another space of the same size and decay: step through the 32 with the arrows, or roll the die. **Color** tilts the reverb darker or brighter, and **Low cut** trims its low end.
- **Duck**, one in each section, turns that section down while the track itself is loud, so echoes and reverb fill the gaps instead of covering the sound. **Mix** blends the effect with the dry signal; on a send, use the **Send Echo** or **Send Hall** preset, which are effect only.

The picture shows what you will hear over time: every repeat as a bar (left above the line, right below), blurred as Diffusion smears it, and the reverb as a wedge that narrows to nothing where it has died away, with a darker wedge inside for the highs, which Damping and Color shorten. While a Duck is in use, **E** and **R** at the right edge show how far the echo and the reverb are turned down. Drag **T** to set the echo's time and level, **F** for Feedback, **P** for Pre-delay and the reverb's level, and **D** for Decay; double-click a handle to reset it.

## Sample browser

Open **Window → Device browser → Samples**. The browser separates **Sources**, a searchable results list, and the selected sample’s preview and actions. Drag either divider to resize a sidebar. Choose **All samples**, global **Favorites**, **Project samples**, or a folder in Sources. Selecting a folder includes its descendants; the separate chevron expands its subfolders.

**Add folder…** opens the system folder picker. In **Folders…**, add another root or remove one. A parent folder covers its descendants. Adding a parent replaces any listed children. Click **Scan folders** to index the roots. Scanning is always explicit: opening the browser, searching, changing sources and restarting Rill reuse the saved index. **Cancel scan** keeps the last completed results available. Scan again after changing files or folders; partial-scan details remain available beside the source controls.

**Search samples** matches names, folder paths and favorite tags, case-insensitively. Combine words in any order. Search applies to the selected source and stays in place when you change sources. **Clear search** keeps the source; **Search all samples** broadens a search without erasing it. Source navigation stays visible during searching. Results show each sample’s name, location, known length and favorite star. File details are read only for selected samples, never by decoding the entire library.

Select a sample, then use **Preview** or **Stop preview** in its inspector. **Auto-play** previews each selection automatically; it is saved across restarts and starts off. Double-click and Enter audition a result, without importing it. When Results owns keyboard focus, Up/Down selects another sample and Space toggles preview. Preview does not edit the song or move its transport. Click the search or tag field to type.

The inspector shows length, format/codec, sample rate, channels, available bit depth and file size. The waveform and spectrum appear below the metadata after background analysis; changing selection cancels obsolete analysis. Preview and the main use action stay visible while the details scroll. Missing or corrupt files stay selected with a readable error; metadata Retry only inspects that sample, without rescanning its folder.

**Add to Project Samples** copies the selected recording into managed project media without creating a track. You can also drag a result onto **Project samples** in Sources, even while browsing another collection. The operation preserves the original, creates one Undo step, and travels in the saved project. **In Project Samples** indicates known membership. Choose the Project samples source to find **Bounce…**, which opens the shared bounce dialog.

Use **Use… → Load into instrument…** to choose a sampler or drum-player destination, or **Add audio track** to place the recording in the arrangement. Ordinary browsing stays open after loading. When you enter the browser from an instrument’s **Browse…**, **Load sample** targets that captured player; **Change…** changes the destination. Successful loading returns to the caller. Failed or canceled loading leaves the selected source available to retry. **Load file…** uses the system picker for a captured destination; the ordinary browser’s file alternative adds to Project Samples.

Click a result’s star to add or remove a global favorite. The star always controls membership; its separate colored tag icon and label identify the category. **Edit tag…** changes the optional label, color and icon. Favorites remain available across projects. External favorites reference their original files; favoriting a project source retains a durable library copy. Removing a favorite preserves the audio.

The Player editor’s **Samples** sidebar uses the same source, query and selection with a compact layout. Preview and project-add actions stay visible below the results. Drag a result onto a sample leaf or waveform. **Details…** or **Browse…** expands the same browsing context into the full browser.

## R1 Sampler

Use **Sample** to load a recording and set its playback region and loop. **Pitch & voices** sets key mapping, tuning and voice behavior. **Filter & amp** holds the filter and envelopes, and **Modulation** sets the LFO and its destinations. The recording, output controls and keyboard stay visible across these tabs.

**R1 Sampler** plays a sample player tree across the keyboard. It starts with one Sample leaf. Open **Window → Instrument** and choose **Browse…** for Project samples or indexed sample folders; **Sample actions → Load file…** uses the system file picker. Dropping an audio file on the waveform loads it too. Imported recordings are stored with the project, and loading or clearing a recording is one Undo step. **Use as sampler instrument** on an arranger audio clip creates a new MIDI track holding its source recording; use Bounce when you want the clip's processing included.

**Edit player…** opens the shared Player editor. The left side shows an indented tree; the right side edits the selected node. **Add sample**, **Add group**, and **Wrap in** build the hierarchy. Groups can play **All at once**, choose one **Random** child, advance **Round robin**, or select children by **Velocity**. Velocity ranges are inclusive, from 1 to 127: overlapping ranges play together and gaps are silent. **Distribute velocities** divides the range evenly. Each nested selector has independent state; playback resets reproduce the same sequence. **Duplicate** copies a subtree while sharing its recordings. **Move up**, **Move down**, **Remove**, and **Rename…** are available with the mouse.

For example, wrap a Sample in Random, wrap that group in Round robin, then wrap the result in Velocity. Add sibling groups to create three velocity bands, each containing its own round-robin sequence of random variations. The Samples sidebar uses the same searchable browser; drag a recording onto a Sample row or its waveform, or select the leaf and use Browse/Load file.

Each leaf has its own gain, Start/End and Reverse. **Trim silence** moves the boundaries inside the current crop; **Reverse audible region** trims and reverses that region. Neither changes the source file. The waveform marks the detected audible span (−60 dBFS with 1 ms padding). Instrument-wide Start/End and loop settings address the chosen leaf's region. **Sample actions → Export sample…** opens the shared Bounce dialog.

Keys play the recording at their interval from **Root key** (C4 unless you change it; turn **Key track** off to play it at its own pitch everywhere). Drag the Start, End and loop markers on the waveform or turn their knobs. **Loop** is Off, Forward (with a seam **Crossfade**), Ping-pong or One-shot, which plays to End however short the note. The filter is off until you switch it on in its card's title.

To play a drum loop or phrase one piece per key, switch on **Slice** in its card's title. Start to End splits into equal **Slices** (eight by default), and the waveform shows where each begins. **Root** plays the first slice and each key above it the next, at the recording's own pitch; a Forward or Ping-pong loop repeats the whole slice for stutters. **Character** adds the sound of early samplers ahead of the filter: **Rate** lowers the sample rate (its maximum leaves it untouched), **Bits** quantizes to 12 or 8 bits, and **Drive** saturates. On **Modulation**, **Fade in** lets vibrato or tremolo grow over each note, **Mod wheel** puts the LFO depth on your controller's wheel, and **Retrigger** starts the LFO afresh with each new phrase. Recordings larger than 32 MiB once decoded (about three minutes of stereo at 48 kHz) are refused with a message; a missing sample file stops playback like a missing audio clip until it is found or cleared.

## R1 Drums

Select a pad to see its recording and signal path together. Trigger, region and tuning sit directly below the waveform. The next row contains the pitch envelope, amp envelope and tone controls. The output row holds **Pad volume**, Pan and Mute, followed by **Kit volume**.

For a snare with lower-pitched ghost notes, go to **Tuning** and set **Soft hit pitch** to −4 semitones. Click near the bottom of the pad to hear the lower pitch, then near the top to hear its original tuning. A positive setting raises softer hits. The default is 0 semitones.

For a grainier tail, choose **Tone → Bit depth** on that pad. **12-bit** adds subtle grit as the hit fades. **8-bit** makes the steps easier to hear. **Clean** keeps full precision and is the default for every pad.

**R1 Drums** has sixteen larger pads, each with its own sample player tree, on MIDI notes 36 (C1, pad 1) to 51 (D#2, pad 16). Give a MIDI track R1 Drums from the device browser and open **Window → Instrument**. Pads are laid out bottom row first, pad 1 at the bottom left. Clicking a pad selects and plays it, louder the higher you hit it. **Rename…** sets a pad name independently of its source filenames.

Select a pad and choose **Edit player…** for the same hierarchy editor used by R1 Sampler. All, Random, Round robin and Velocity groups can nest freely; pad pitch, envelopes, filter, drive, choke group and level shape the resulting hit together. A layered hit consumes one overlapping-hit position. A single-sample pad accepts files directly; dropping onto a composite pad adds a Sample child without replacing the existing tree.

Set the pad's **Start** and **End** with the knobs or waveform markers. **Reverse** changes playback direction. In **Trigger**, One-shot plays to the end and Gate releases over Decay when the note ends. Assign closed and open hi-hats to the same **Choke** group so each stops the other. **Tuning** sets the pad pitch. For kicks and toms, set the pitch envelope’s **Amount** in semitones and **Decay** to choose how quickly the pitch returns to its tuned value. In **Amp envelope**, **Hold** at **Full** plays the whole recording. A shorter hold fades over **Decay**. Velocity sets how much softer a soft hit sounds.

In **Tone**, Bit depth feeds the filter. Turn **Filter** left for low-pass or right for high-pass, then adjust resonance and drive. Up to four hits of a pad ring together; a pad in a choke group, or on Gate, cuts its own previous hit instead. **Randomize** varies how pads sound but never which recording plays, its markers, its mute or its choke group. Each Randomize click is one independent Undo step for all changed values; Redo restores that same result.

## Plug-in instruments and effects

Rill hosts **VST3** and **CLAP** plug-ins on macOS, Linux and Windows, **LV2** plug-ins on Linux, and **Audio Units** (AU) on macOS. Each library card carries a format badge (VST3, CLAP, LV2 or AU), device details name the format, and typing a format name in the library search filters by it. Rill looks in each format's standard folders and in the `VST3_PATH`, `CLAP_PATH` and `LV2_PATH` variables, plus folders you add. LV2 needs the system lilv library (`liblilv-0-0` on Debian/Ubuntu, `lilv` elsewhere); without it LV2 bundles are listed with an error and other formats are unaffected. On macOS Rill also lists every installed Audio Unit instrument, effect and MIDI-controlled effect from the system registry (Setup → Plug-ins says “plus installed Audio Units”); there is no AU folder to add, and an AU's install path shows `/System/Library/Frameworks/AudioToolbox.framework`. AUv3 extensions appear once their app has been opened. CLAP and AU editors open like VST3 editors; an AU without its own editor shows Apple's generic controls. MIDI-controlled AU effects work as effects but receive no MIDI from an insert. LV2 plug-ins have no embedded editor in Rill: **Open editor** reports that, and **Parameters…** lists their controls for editing and automation.


Open **Window → Setup → Plug-ins**, add folders, and apply the settings; Rill rescans them. Choose **Track → Add instrument…**, type a name or developer, then use the primary button, Enter, or double-click a result. Rill creates a MIDI track with a loop-length clip capped at one bar and opens its instrument panel. **Track → Add MIDI track** opens the chooser after creating its empty pattern. Double-click the clip to write notes. To replace an existing MIDI track's sound, choose **Choose instrument…** in its arrangement or Clip Editor header, or open **Window → Instrument → Choose instrument**; replacing the instrument preserves its notes and returns to the caller. **Window → Device browser** opens the same full workspace with **Back**. It starts on Instruments when choosing a sound and Effects when adding an insert or send.

The bottom action names what will happen: **Add instrument track**, **Set instrument** for a newly created track, **Replace instrument**, **Add to effect chain**, or **Create send track**. Its destination stays fixed while you browse. Master accepts inserts, but cannot be a send source. **Back** or Escape cancels the chooser without loading a device; a new MIDI track and pattern created before opening it remain available.

Choosing an instrument in the initial creation flow gives an untouched **New
instrument** track the device's name; repeated names receive a numeric suffix.
That naming change belongs to the same Undo step as assignment. Names you set,
names loaded from projects, and names on tracks assigned through a reopened
chooser remain intact, even if the name is **New instrument**.

Rill keeps an index of installed plug-ins. Each launch shows the last index immediately, then rescans in the background; **Manage plug-ins → Refresh** rescans on demand. The same menu provides **Plug-in folders…**. New, updated and removed plug-ins are picked up without restarting. Built-ins and discovered results remain usable during scans; Enter never queues a later load. After a scan, Rill captures missing editor images in isolated helpers, one plug-in at a time. Cache entries are invalidated when a bundle changes. A failed screenshot leaves the device available and is not retried on every scan. Devices without classification metadata appear under **Unverified**, with explicit **Try as instrument** and **Try as effect** actions where compatible. **More → Capture preview** or **Refresh preview** captures an image again. An unavailable image does not imply the device cannot load; loading reports its own errors separately.

Search matches words in any order across device name, developer, category and format. **Clear** removes only the query. Category and developer choices narrow the results; their active chips remove individual filters, while **Reset filters** clears the query and all filters. Switching between Instruments, Effects and Unverified keeps the query. The result count reflects the current filters.

The four collections are **All devices**, **Favorites**, **Recently used**, and **Built-in (R1)**. Their counts reflect the selected device type. A narrow window replaces the collection rail with a visible **Collection** chooser. The browser opens in **List** view; choose **Grid** for image cards. Sort by name, developer or recently added; **Recently used** always puts the last added or assigned device first. Clicking a row selects it; arrows move the selection, and Enter or a double-click loads the selected device using the visible primary action.

The inspector shows the selected device's identity and categories. **Details** makes the same controls available in a small window. Use **Favorite** to save a device, **Organize** to edit comma-separated categories, and **Save** to keep them; **Reset categories** restores the plug-in's defaults. **Device details** opens technical information and the parameter catalogue. **More** and a device's context menu also provide favorite actions. Favorites, categories and recently used devices persist in `plugin-library.json` next to `settings.json`; favorites and categories are also reported by the `list_plugins` tool.

Instrument and effect edits automatically persist normalized parameters and opaque plugin state. Native editors use a separate instance from audio rendering; parameter gestures update the playing instance and debounced state snapshots capture preset changes. Save, export, device changes, and document transitions flush the editor first. Full state changes rebuild playback and can briefly interrupt audio. The native editor and instrument parameter list provide access to hosted plug-in controls; built-in R1 devices also include their own panels and preset browsers.

Thumbnails and the plug-in index live in `~/Library/Caches/Rill` on macOS, `%LOCALAPPDATA%/Rill/Cache` on Windows, and `$XDG_CACHE_HOME/rill` (default `~/.cache/rill`) on Linux. Set `RILL_CACHE_DIR` to override this location. Native editor capture needs a desktop session; Linux uses X11/XWayland with the XRes and Composite extensions.

## Recording and overdubs

Choose the input and output devices in **Window → Setup → Audio**, then select an audio track and open **Window → Recording**. Choose **Mono 1**, another input channel, or a stereo pair; enable **Arm recording** and press **Record** in the popup or transport. Track headers also have an arm control. Recording starts at the transport cursor and can continue beyond the current arrangement. The popup's meter shows the raw input level.

**Input monitor (dry)** offers **Off**, **Auto**, and **On**. Auto monitors armed tracks while stopped or recording and turns monitoring off during ordinary playback. On monitors even when the track is unarmed. Monitoring follows channel/output-bus input gain, polarity, width, volume, pan, mute and solo, plus the same Master controls except solo. It bypasses inserts and sends, including Master inserts, and joins device output after the Master meter/Analyze tap. Recorded WAVs contain the selected raw input, independent of mixer levels, and monitoring is not included in export.

Enable the transport loop before recording to retain each pass as a separate take. The newest complete pass becomes audible; a partial final pass remains available without shortening that complete pass. Open **Takes…** to select whole passes or comp ranges with smooth or custom spline crossfades. **Record into → Overdub layer** creates a separate colored audio track with the source track's bus, folder, volume, and pan; inserts and sends are not copied. Existing clips and recorded sources remain intact. Each recording is one undoable edit.

**Stop recording** finalizes capture while backing playback continues. Pause, Stop, Save, New/Open, Quit, and audio-setup changes finish and attach captured audio before proceeding. Input choice is saved with the project; arm and monitoring choices are session-only and reset when a project opens. Sources are written under the scratch directory's `Recordings` folder and included in `.rill` archives. Input failures retain completed audio and report recovery details. Limits are 32 armed routes, 64 loop passes per session, and 512 MiB of audio per take. Driver timestamp alignment and failure recovery have device-free regression coverage; physical input timing and microphone permissions still require hardware verification.

### MIDI recording and pre-count

Select a MIDI track and open **Window → Recording**. Choose the shared **MIDI device** and **All channels** or one channel, arm the track, then press **Record**. **Input format** reports MIDI 1.0 or UMP; **UMP group** selects one of sixteen groups for the track. Legacy inputs use Group 1. Use separate tracks for other incoming groups; changing this selector does not rewrite existing clips. Arming MIDI initially enables **Auto** monitoring through its assigned plugin instrument; **Off** and **On** remain available. You can record notes without an instrument and assign one later. MIDI device selection is also available under **Setup → Audio**. Opening a project never arms tracks or opens MIDI inputs automatically.

Each session adds a new MIDI clip on the existing instrument track. Existing clips remain intact and play alongside it, so recording over them overdubs their notes without creating another plugin instance. Loop passes accumulate in one clip, splitting held notes at the loop boundary. The project can retain high-resolution attack/release velocity, pitch attributes and controller/per-note UMP data alongside note timing and retriggers. MIDI 1.0 sustain extends note lengths; MIDI 2.0 preserves the actual key-release edge and sustain controller for the instrument. Original MIDI 1.0 attack velocity keeps its seven-bit precision. Audio and MIDI can be armed together and publish as one undoable edit.

Native UMP input targets macOS 11 and newer. Linux and Windows currently use MIDI 1.0 input. VST3 does not carry raw UMP, so preserved project data and audible plug-in expression are different capabilities; the assigned instrument must support the relevant mapping. The macOS module has been compiler-checked, but physical UMP hardware and driver timing have not been verified here. MIDI-CI/profile negotiation and universal SysEx8 support are outside this implementation.

**Pre-count** in the Recording popup offers **No pre-count**, **1 bar**, or **2 bars**. From stopped, the count uses project tempo/time signature while the cursor stays at the intended start. Notes and audio played before the cue are excluded; notes already held before it are not chased into the recording. Recording during existing playback starts without pre-count to keep backing audio uninterrupted. Stop during the count cancels without creating clips. MIDI audition pauses during pre-count while the existing instrument graph prepares the exact start; dry audio monitoring remains available, and MIDI monitoring resumes at the cue.

The transport's **Click** toggle enables the metronome during playback and recording, with a stronger first beat of each bar. The Recording popup also sets its level. Pre-count clicks play even when ordinary Click is off. Click audio goes only to the output device and is never written into recorded audio or a WAV export. Project archives save click, level, and pre-count choices. Dummy MIDI input and device-clock tests run without a connected keyboard or audio device; physical MIDI/hardware timing still requires verification.

## Setup and recovery

Large audio uses disk-backed playback automatically at 32 MiB of decoded PCM,
or when its processed variant needs a large buffer. A small disk icon identifies
streamed clips; Properties shows **Disk-streamed audio**. Small clips can stay in
memory. The indicator describes source storage, independently of whether a page
or waveform preview is currently cached.

Rate conversion, pitch changes and Signalsmith/Bungee stretching can prepare a
private PCM disk cache before playback. That preparation takes time and disk
space; the original remains intact. Reads run on audio graph workers, while the
device callback consumes its queue. Slow or cold storage can still cause the
existing dropout warning. Changed, truncated or unreadable media reports a
failure while retaining the song; offline export keeps its prior destination.
Normalization, transient analysis and Strip silence also scan large sources in
bounded chunks instead of loading the full file into memory.

**Window → Setup → Audio** starts with **Driver** when this computer has more than one audio system (for example ALSA and JACK on Linux, WASAPI and ASIO on Windows; macOS has one, so the control is hidden). Devices, sample rates and buffer sizes come from the chosen driver, and input and output always use the same one. JACK (including PipeWire's JACK) and ASIO dictate their own settings: with JACK the sample rate and buffer size are the server's and those controls are disabled; ASIO limits the buffer to the driver's range and drives one device for both input and output, so Input follows Output; pick input channels in the Recording panel as usual. If JACK reports no devices, start the JACK server and press **Refresh devices**. A saved driver that is missing at launch (uninstalled, or an ASIO build not in use) falls back to the platform default with a notice, and returns when the driver does. The page lists input/output devices and supported sample rates, buffer sizes, and High quality/Fast sample conversion. Input must support the selected project rate and channels. Rill defaults to 48 kHz, 32-bit float audio buffers, and 64-bit summing. High quality uses filtered source-rate conversion; Fast trades alias rejection for preparation speed. Changes finish recording, flush native editor state, stop and retire old device/plugin workers, then initialize replacements in the background. Failed device initialization restores the previous settings and reports the error; device errors offer **Audio setup…** directly in the notification. A restart includes a brief pause. **Storage** selects the scratch directory; **Plug-ins** adds VST3 scan roots. Unsupported device settings report an error rather than silently substituting a configuration. The render queue holds two requested buffers, with a minimum of 512 frames; hover the device buffer latency to see its contribution. Setup edits apply only after **Apply**; Cancel, Escape, and closing the popup discard them.

A recoverable audio-output dropout shows a warning with its count and **Audio setup…**; ordinary playback continues while the audio backend recovers. Try a larger buffer if dropouts recur. The warning stays in one notification instead of repeating for every dropout. If a dropout occurs while recording starts, during pre-count, or during capture, Rill stops capture to preserve timing and retains any recorded prefix; after capture finishes, choose **Retry playback** or press **Play** to reconnect before recording again.

If the output stream fails with a device error, Rill stops that stream and reports the device fault once, separately from plug-in failures. **Details** retains the backend message and **Audio setup…** opens the device settings. Check the output device, then choose **Retry playback** in the notice or press **Play** to reconnect at the current position, or **Stop** followed by **Play** to start from the beginning. Dismissing the notification does not restart the failed stream. Recording and note audition remain unavailable until it reconnects.

Setup persists at `~/Library/Application Support/Rill/settings.json` on macOS, `%APPDATA%/Rill/settings.json` on Windows, or `$XDG_CONFIG_HOME/rill/settings.json` (default `~/.config/rill`) on Linux. The scratch directory holds demo audio and `recovery.daw.json`. Unsaved edits are also saved to recovery on a normal close or before switching sessions. Use **Setup → Storage → Recover session**; recovery opens without an existing document save path.

Projects save as self-contained `.rill` archives containing every referenced source WAV, including muted clips, retained takes, frozen originals and prints. Edits, MIDI, routing, automation and instrument/effect state live in the manifest. Opening a legacy `.daw.json` remains supported; its next GUI Save offers a new `.rill` filename and keeps the legacy file intact. Explicit application/tool save paths also contain portable audio regardless of their extension. Extracted audio uses the configured scratch directory. Plugins and their own sample libraries must remain installed. Recovery checkpoints still reference managed media, so keep the scratch folder while recovery or unsaved work may need it. See [project format and limits](PROJECT_FORMAT.md).

**File → Save and optimize** rewrites the saved project with only the audio still
referenced by the song, excluding sources used only by deleted clips. Normal Save
also omits unused assets; optimization additionally combines byte-identical
source files and applies lossless audio compression to reduce the archive size.
It can take longer than Save. Muted clips, retained takes, Freeze originals and
the full source ranges remain available. Optimization preserves Undo/Redo and
leaves original and scratch files in place for recovery; it does not clean the
global scratch folder.

After Save as, the project header shows the saved filename with a dirty marker for later edits; hover it for the full path. Save and audio/MIDI export pickers reuse that directory and filename stem. Extensionless project saves receive `.rill`. **Export audio…** warns when visible notes on contributing MIDI tracks have no instrument; Cancel to assign a VST3, or continue without those notes.

In **File → Export audio…**, **Bit depth** offers **16-bit PCM**, **24-bit PCM**, and **32-bit float** for WAV, or 16/24-bit PCM for FLAC. Lossy formats show their quality or bitrate controls instead. The export sample rate is independent of playback. **Follow audio setup** uses the current audio rate when Export is pressed. Choose the entire **Arrangement**, or **Loop range** to export between the loop markers at an exact length. The duration summary says **+ tails** when arrangement tails are enabled; loop-range exports always end at the end marker. These choices do not move the transport or change the song.

**File type** chooses WAV, FLAC (16 or 24-bit, lossless), Ogg Vorbis (quality 0-10) or MP3 (128, 192, 256 or 320 kbit/s CBR, up to 48 kHz). Lossy and FLAC files limit samples to 0 dBFS and report the limited count; lossy files can decode slightly above full scale, so leave headroom.

**What: Stems** writes one file per chosen channel into a folder you pick, named `<song> - <NN> <channel>.<ext>` (names are cleaned for every file system, numbered in project order). Tick tracks, buses and folders in the list (**Select all** / **Select none**); **Include full mix** adds `00 Mix` with Master processing. Each stem is the song with only that channel audible, so its inserts, fader, sends and downstream buses sound as in the mix. Stems are taken before Master unless **Through Master processing** is on. Every file has the same start, length and format. Existing files are never replaced unless you confirm in the folder picker; the files are staged together and a failure or **Cancel** leaves none behind. Each instrument output channel is listed as its own "(output)" stem.

**Dither (TPDF)** adds low-level noise when converting to integer PCM. It is ignored for float output. PCM samples above 0 dBFS are clipped; completion reports the clipped sample count. Float output preserves headroom. The completion summary's **Rendered sample peak** describes the mix before PCM conversion, rather than a true-peak measurement of the encoded file.

WAV and MIDI completion show the filename and **Copy path**. Hover the completed WAV notice to see its format, sample rate, duration, and captured loop bounds. **Retry export** retains the original song snapshot and all export choices. **Re-export last WAV (replace)** renders the current song with the last completed WAV's choices, including its captured loop bounds. MIDI export waits for a pending WAV export or session update to finish. Offline WAV rendering does not require an output device.

On Linux, if a document file picker reports a portal or Zenity helper failure,
Rill shows **Could not open the file picker** and retains your work. **Details**
includes the cause and, when the fallback fails, guidance to restore the desktop
portal or make `zenity` available on PATH; then retry Save, Open, Import or
export. A failed Save inside **Save changes?** leaves that decision open. Normal
picker cancellation stays silent. Other platforms retain their existing native
picker behavior.

Standard MIDI `.mid` export creates a constant-tempo MIDI 1.0 note file. If the
project contains richer performance data, **Export MIDI 1.0 copy** first explains
that high-resolution pitch/velocity will be reduced and UMP/per-note expression
omitted. Choose **Export MIDI 1.0 copy…** to continue to the file picker or
**Cancel**. The original `.rill` retains the full data. This is not a raw UMP or
standard MIDI Clip File export.

**File → Recent exports** keeps the current document's last five successful WAV/MIDI destinations. Choose an entry to copy its full path. This lets you check an earlier WAV after the MIDI completion appears. Re-exporting the same destination moves it to the front; failed or cancelled exports add no success entry. New/Open clears the list.

## Plug-in stability

Each instrument/effect instance, of any format, runs in a separate `daw-plugin-host` process. Scanning reads metadata without executing plug-in code in the DAW: VST3 metadata is read from `moduleinfo.json`, CLAP/LV2 bundles are read by a short-lived helper per bundle, and macOS Audio Units by one short-lived helper that reads the component registry. A crash or timed-out processing command silences that path while the session stays open; **Stop**, then **Play**, creates fresh instances. Export failures preserve an existing destination file.

This is crash isolation. Helpers retain the user's filesystem and network permissions; they are not an OS permission sandbox. Plug-in IPC runs on the render worker, away from the lock-free device callback. The helper must be next to `rill`, or specified by `DAW_PLUGIN_HOST`.

VST3 and CLAP audio, supported MIDI/expression mappings, latency, transport, auxiliary inputs, grouped instrument outputs, native instrument/effect editors, parameter automation and state recall are supported; LV2 (Linux) supports the same except native editors, MIDI output and transport position. Audio Units (macOS) support the same as VST3 except note expressions, MIDI output and MIDI into MIDI-controlled effects. A standalone preset browser and VST2/AAX hosting are not included. ARA requires a separate document/random-access integration; see the [format assessment](PRODUCT_DIRECTION.md#additional-plug-in-formats).

## Rendering and verification

Run the subprocess end-to-end suite with `./scripts/test-e2e.sh`. It launches the actual application in headless mode, drives UI events, imports media, and validates rendered audio. The same suite runs without a display or audio device in a Linux container; see [E2E instructions](E2E.md). Every new user-facing feature must include an E2E test.

```sh
./target/release/rill --render-demo /tmp/rill-demo.wav
./target/release/rill --render session.rill /tmp/mixdown.wav
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run --release -p daw-engine --example render_bench
```

Real native plug-in tests are opt-in; commands and environment variables are in the test sources. Local validation exercised Surge XT and CHOWTapeModel, including forcibly terminated plug-in children, recovery, sample-offset MIDI, effect input, state roundtrips, and a routed chain with compensated bus latency.

See [architecture and limitations](ARCHITECTURE.md) for the graph, real-time boundaries, file formats and delay compensation. Current media scope includes WAV, AIFF/AIFF-C, FLAC, MP3, Ogg/Vorbis, AAC, M4A and CAF import, disk-backed large-audio playback, recording and constant-tempo MIDI with rich project performance data. Tempo maps, automatic tempo-following warp and surround remain absent; manual warp markers and audio quantize are available in the audio Clip Editor. Input gain, polarity, width, volume, pan, note and clip edits update the existing graph; failed audio preparation keeps the last valid audio playing. Structural, plug-in-state and device changes still rebuild playback and may briefly interrupt it. Cosmetic changes do not rebuild the graph, including Undo/Redo.

## Build from source

Requires Rust 1.92 or newer. Build the **whole workspace** so the isolated plug-in helper is installed alongside the app:

```sh
cargo build --release --workspace
./target/release/rill
```

Or run `./scripts/run.sh`. On macOS, `./scripts/package-macos.sh` creates `dist/Rill.app`. On Windows, launch `target\release\rill.exe`. Linux needs ALSA development headers, a desktop display, and OpenGL; see the [CI workflow](../.github/workflows/ci.yml) for build dependencies.

Build the same distribution locally with `./scripts/package-release.sh` on macOS/Linux or `./scripts/package-windows.ps1` on Windows. See [distribution and release instructions](RELEASING.md) for prerequisites and the test gates before publication.

## Implementation references

The UI uses [eframe/egui](https://github.com/emilk/egui) through the shared `daw-ui` component library. New controls are added to that library before being used in a view; see [UI components and conventions](UI.md). Audio devices use [CPAL](https://github.com/RustAudio/cpal), and the isolated VST3 adapter uses [vst3-host](https://github.com/HelgeSverre/rust-vst3-host). Crate versions are pinned by `Cargo.lock`.
