# Rill user guide

[Back to Rill](../README.md) · [Download builds](../README.md#download)

A reference for composing, recording, editing, mixing, and saving in Rill.

## First session

The initial session contains an original generated audio loop. Rill includes no instruments or insert effects; MIDI tracks remain unassigned until you choose a VST3 instrument. Use **File → New** to start empty. Press **Space** to play or pause.

An empty song offers three starting points: **Choose instrument…** opens your
installed sounds, **Import audio or MIDI…** brings in a recording or sequence,
and **Compose with AI** opens editable composition examples. Browsing sounds or
examples leaves the song untouched. **Add pattern** and dropping WAV files onto
the arranger remain available; the starting actions disappear once a track exists.

## Included

- Audio and MIDI tracks; VST3 instruments and effects hosted in isolated processes.
- Parallel processing of independent instrument/effect chains, with persistent workers, ordered buses, and a serial path for lightweight sessions.
- Non-destructive clip movement, trimming, splitting, duplication, cross-track movement, and undo/redo.
- Live volume, pan, note, and clip edits retain running plugins, sustained notes, effect tails, and transport position; new audio prepares in the background.
- Drag WAV files onto an existing audio track or empty arrangement space to create a track. Dropping on a MIDI/bus/folder track creates a new audio track.
- Non-destructive audio gain, normalization, reverse, curve fades, range clipboard editing, and transient split preview.
- Exclusive audio overlaps, optional spline crossfades, and audio/MIDI clip joining.
- Transport loop toggle and on-demand start/end beat controls, saved with the project.
- Piano roll with scale/chord highlighting, chord database, octave-stack drawing, both note edges, velocity painting, and held-note plugin audition; randomize and quantize previews, voice-aware legato, transpose, and note clipboard.
- Generate sustained chords, seven arpeggio patterns, and seeded melodies with optional counterpoint or tintinnabuli voices; preview notes in an editable whole-bar range before adding them in one undoable step.
- Large searchable instrument/effect browser with developer filters and cached native previews; drag-reorder insert chains and create send returns.
- Pre/post-fader sends, buses, nested bus routing, gain/pan/mute/solo, and automatic path delay compensation. Feedback cycles are rejected.
- Drag-and-drop track/channel ordering, nested folders, track colors, folder collapse, compact tracks, and safe track deletion.
- System-following light/dark themes, bundled Inter typography with Unicode fallbacks, shared text components, and vector toolbar icons.
- Audio input recording with mono/stereo selection, dry monitoring, loop passes retained as separate takes, and independent overdub layers.
- MIDI note recording with channel selection, plugin monitoring, loop overdubs, optional pre-count and an output-only metronome.
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

The **File**, **Edit**, **Track**, and **Window** menus hold application commands. Transport and contextual editing tools stay beside the canvas. Use **Track → Add audio track / Add MIDI track / Add bus / Add folder** to build the session and **File → Import…** to import media. **Add MIDI track** creates a four-beat pattern at the cursor and immediately opens the instrument chooser. Choose a VST3 to assign it and return to your caller; closing the chooser leaves the pattern unassigned.

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
| **Library** | Search, scan, and assign instruments. |
| **Assistant** | Open global AI for the current song; editor AI buttons target only their opened context. |
| **Setup** | Configure Audio, Storage, Plug-ins, Appearance, and AI on separate pages. |

Clip Editor and Mixer use the whole workspace, with **Back** to return to Arrangement. Routine utilities and parameter menus keep the surrounding workspace sharp and lightly dimmed, so notes and values remain readable while only the foreground view accepts edits. Explicit confirmation dialogs use a stronger dimmed, blurred background where the renderer supports it. Transport and application menus remain available. Close a utility, press Escape, or choose **Window → Arrangement** to return. Nested tools restore their caller and its editing position when dismissed.

Editing tools stay in a single shared strip. Arrangement, MIDI and audio retain their own selected tools when you move between them. Audio **Clip** and **Process** menus reveal trimming, copy/paste, normalization, reversal, and transient analysis; fade settings appear with the selected fade tool. MIDI **Notes** holds note transformations; **Draw settings** holds note length, grid scale and harmony. Secondary controls appear when needed, leaving the waveform or notes in focus.

The toolbar's available space shows the active tool's next mouse gesture. For
example, **Duplicate** selects a tool: click a clip or note to copy it. The
context-menu **Duplicate** command makes its copy immediately.
Choose **Tool help** in the toolbar for the active tool's gestures and shortcut.
Escape or a click outside closes the help. Hovering any tool also explains its
use, leaving the waveform and notes free of a permanent instruction strip.

Arrangement and both clip editors show **− / + / Fit / Fit selection** together.
Use **−/+** to zoom horizontally around the center of the current view, **Fit** to
show all content, or **Fit selection** to frame selected clips, notes or an audio
time range. **View → Previous view** returns to the previous framing. These
controls leave the music and selection unchanged.

In the piano roll, **Generate…** opens Chord, Arpeggio and Melody modes. Start bar and Length initially cover the completely visible bars; edit either field to change the highlighted range. Choices and range are remembered per clip for the app session, including after Cancel; **Next range** advances by the current length while retaining settings. Violet notes preview the result alongside existing notes. **New variation** rerolls random patterns, **Fit preview** reveals their full extent, and **Add notes** commits the exact preview and keeps the generator open. Use **Next range** to continue, then **Done** to close it. Before adding notes, **Cancel** discards only the preview; closing the generator after an addition keeps the committed notes. Parameter controls scroll independently so the range and confirmation buttons remain reachable.

Arrangement track headers separate the title, icon and color from the mix controls. Volume and pan sit beside **M**ute and **S**olo; **Routing** and **Instrument** or **Input** each have their own row. Stereo output meters occupy a dedicated column. Drag a volume or pan rail, or click its value for exact entry. Pan shows **C** at center and **L/R** percentages elsewhere. Cancel, Escape or clicking outside the value popup discards the draft; displaying a rounded caption never changes stored precision. −60 dB silences the channel.

Click **Routing** to choose Master or an eligible output bus. **Instrument** chooses or replaces the MIDI track's VST; **Input** opens audio recording and monitoring settings. The default header is 288 × 144 points. Compact tracks keep mix controls and expose the complete Channel view through the sliders icon beside the title. Very short rows use readable value controls instead of miniature rails. Folders and automation tracks have no audio meter or gain/pan controls; folder mute/solo acts on descendants.

Gain captions show one decimal place; hover or open the value editor to see its
full precision. Simply viewing or cancelling an edit preserves the stored value.
The Master meter follows the combined output, keeps short hits visible as the
peaks fall, and clears when playback pauses or stops.

In Arrangement, **Add pattern** creates a four-beat MIDI pattern at the cursor on the selected MIDI track, or creates a track when needed. Empty MIDI lanes also show a double-click hint and offer **Add pattern** in their context menu. Select **Window → Clip Editor**, or double-click a clip, to open it. Drag a clip to move it; drag its edges to trim its length. **Properties → Clip** provides editable **Start**, **Duration** and **Source offset** in beats. Visible **−/+** buttons step each value by one beat; drag the number for fine adjustment or click to type. Start is one-based, and each step supports Undo. Tempo has matching buttons that step by one BPM. While trimming an arrangement clip, a pointer readout shows its current end and duration in seconds and beats. Use **Edit → Split at cursor / Duplicate**, or the clip's context menu. Right-click a track header for properties, collapse/expand, or deletion; use **Window → Effects / Routing** for its signal path. Arrangement **Fit** keeps sparse track rows at a readable maximum of 144 points while fitting the song horizontally.

Right-click an audio or MIDI clip and choose **Properties** to edit that clip's
timing directly above Arrangement. In Clip Editor, **Properties** stays at the
same end of the header for both clip types. The header shows the track and the
clip's **Song** start/end bars and beats; hover for clock times and the ruler's
time basis. MIDI's **Local** ruler starts at bar 1 for the open phrase, even
when that phrase repeats later in the song. Audio's ruler shows song positions.

Clip Properties also shows **Starts at**, **Ends at** and **Elapsed** in clock time
and bars/beats, so you can check a clip's exact timing without converting units.

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

The piano roll separates note actions from the **View** menu. Note-tool controls keep their positions when switching tools. Hover a row or the piano keyboard to reveal its pitch names. **Highlight scale**, **Highlight chord**, and **Only scale keys** are independent view options; hidden notes remain intact. **Draw settings** shows the current length, root and scale, with root/scale/chord choices inside. Natural-key rows use a neutral base and black-key rows are slightly darker; scale and chord guides add a subtle green tint. Notes are blue, with stronger shades for higher velocities. Selected notes keep their shade and gain an outline. Notes outside the grid scale are red; turn off **View → Color out-of-scale notes** to keep every note blue. Folded dragging and arrow nudges follow visible scale steps, while Shift+Up/Down transposes by octaves.

The piano roll shows every tool directly: **Select**, **Pencil**, **Multi-draw**, **Cursor**, **Cut**, **Glue**, **Erase**, and **Duplicate**. Separators distinguish related tool families; the toolbar wraps when needed. The active tool keeps its glowing border, and all shortcuts remain available.

With **Pencil**, press at the note's starting time and pitch, then drag to choose
its length. The starting pitch stays fixed while drawing.

The select tool supports dragging a rectangle around notes. Drag note bodies to change time/pitch, and either edge to change length. **Draw settings → Length** chooses the initial drawing duration from **Last note**, fixed note values, or **1 bar**; Last note starts at 1/16 and remembers the last drawn or selected length. Double-click empty space to create a note or a note to delete it. Drawing past the marked **Clip end** extends the clip with the new note in one Undo step. The velocity lane supports individual and swept edits; selected chord/scale stacks fit within one octave. See [editor controls](EDITORS.md). The duplicate tool places a copy after a note; group duplication preserves the group's spacing. The cut tool splits at the snapped pointer position. Glue joins same-pitch notes from their earliest start to latest end, including gaps; use the tool on two notes or **Glue selected** on a selection. Leaving Clip Editor clears note selection so arrangement edit commands act on the clip.

Select notes with the mouse or **Edit → Select all notes**, then open **Notes** for Randomize, Quantize, and Legato. These actions are available with every tool:

- **Randomize…** previews velocity variation (initially ±16). Set **Timing ±** for up to ±0.25 beats of onset variation; zero keeps the rhythm fixed. Pitch and length stay fixed. **New variation** changes the seed; **Apply randomize** commits the preview and **Cancel randomize** discards it.
- **Quantize…** previews alignment to the chosen beat grid, with Strength, Swing, and optional Note ends. Apply or cancel before continuing to edit.
- **Notes → Legato top to bottom** connects the highest note at each onset to the next highest, then the second-highest, and so on. A missing lower voice bridges to its next appearance. Only selected note lengths change, and the last note in each voice retains its length. **Notes → Legato** instead connects every selected note to the next distinct onset, keeping simultaneous chords together.

Each applied action is one Undo step. Randomize and Quantize leave the song unchanged until Apply; a song edit or clip change discards an obsolete preview.

## Clips and looping

The arranger shows every tool directly, separated into related families: **Select**, **Pencil**, and **Cursor**; **Cut**, **Join**, and **Erase**; **Duplicate** and **Ghost copy**; **Stretch** and **Slip**. The toolbar wraps when needed. The active tool retains its glowing border; hover or **Tool help** shows its instructions and shortcut. The shortcuts remain **Q Select**, **W Pencil**, **E Cut**, **R Duplicate**, **T Join**, **G Ghost copy**, **S Stretch**, **L Slip**, **Z Erase**, and **P Cursor**.

Audio and MIDI clips share these tools; Pencil creates MIDI clips and Slip moves audio within fixed clip edges. Select drags move clips; either edge trims the visible source window. Shift-dragging the body creates one independent copy, while Shift at a trim/stretch edge temporarily bypasses snap. Join opens a focused chooser. Actions select their resulting clip consistently, whether invoked from tools, menus, or the context menu.

With several clips selected, click and release one member to select only that
clip. Drag a member directly to move the whole group. Use Shift-click or a clip's
Add/Remove selection context action to change group membership. **Erase** removes
only the clip you click, including one half of a cut when both halves are
selected. To remove the whole selection, use **Edit → Delete** or the selected
clips' **Delete** context action.

Choose **Snap → 1 bar** to align edits to the current meter: three beats in 3/4,
four in 4/4. Changing the meter later keeps the existing beat increment and shows
it explicitly, such as **Snap 3 beats**; choose **1 bar** again to use the new meter.

Clip Properties always shows **Stretch**, starting at **100.0%**, so its timing
controls stay in place when a duration edit changes between Trim and Stretch.

Choose **123** beside tempo, clip timing or loop boundaries to enter an exact
number using the mouse. The pad supports digits, decimal point, sign, Backspace
and Clear. **Apply value** makes one undoable edit; Cancel leaves the value
unchanged. Generate's Start bar and Length pads accept whole bars only.

Stretch changes duration without replacing the source. Audio keeps its pitch; MIDI note positions and lengths scale in playback and the editor. Stretching one ghost changes only that occurrence's timing. Source-rate factors range from 0.25× to 4×, and each drag remains one Undo step. In Properties, selecting **Stretch** makes the Duration field and **123** keypad show the supported range for that occurrence. For an unstretched four-beat clip it is 1–16 beats; trimmed or already-stretched clips use their current source window. Values outside the range stay as a draft until corrected. Preparing stretched audio happens in the background; the current sound continues until its replacement is ready. Extreme stretches can introduce artifacts, especially in transients and complex material.

In the audio Clip Editor or Properties, open **Pitch** to transpose by up to two octaves in either direction, including fine tuning to cents, without changing duration. Choose **Signalsmith** or **Bungee** for both pitch and time processing. Combined edits render once from the original audio, never from an earlier processed result. Reset pitch restores the original pitch while retaining stretch. Ghost copies share pitch and algorithm; independent duplicates can differ. Both algorithms are included; no external processing utility is required.

Choose **Edit → Ghost copy** (or the clip context menu) to place another instance after the selected clip. A ghost is a linked copy: a chain icon marks every member of the group. Notes, audio edits, fades, clip gain, and the name are shared: edit any member to update the group. Position, trimming, stretching, and overlap crossfades remain independent. **Properties** shows the shared name and instance count before track controls.

Hover an arrangement clip to see its full name, track and start/end timing. This distinguishes repeated occurrences even when their titles are shortened by the current zoom.

**Duplicate** creates an independent copy; in the MIDI editor with notes selected, it duplicates those notes instead. **Make independent** detaches the selected clip with its current name and edits intact. Deleting the original leaves the remaining ghosts connected. Splitting keeps linked parts in the group; joining produces an independent clip. Links survive undo/redo and portable project save/reopen.

Audio clips on one track are exclusive: the active clip with the latest start wins. If starts match, the later entry in the track wins; an earlier clip resumes when a shorter overlapping clip ends. Splitting creates a new start and can therefore change overlap priority. Overlaps switch immediately unless you enable **Edit → Crossfade…** (also in the clip context menu). Set its duration, choose Linear or Smooth, or drag the two spline handles. The complementary gain curves appear over the overlap in Arrangement. Crossfades are limited to the available overlap and next clip takeover; they apply at clip entry, not when an older clip resumes.

Use **Edit → Join clips…** or the clip context menu to choose clips on one track. MIDI joining preserves visible notes and timing. Audio joining renders source edits, overlaps, and crossfades to a new WAV in the scratch directory, including silent gaps; channel processing stays live. Include every audio clip intersecting the joined span. The result is an ordinary clip that can be split, trimmed, reversed, or joined again. Joining is undoable and its source travels inside saved `.rill` projects.

Select a clip or a group of clips, then choose **Range → Loop playback over selection** to repeat playback of that phrase. This sets playback bounds; use **Edit → Repeat to loop end** to create independent copies through a later loop endpoint. A time selection made with **Cursor** includes its leading and trailing rests. The action sets the range and enables looping together, without moving the playback cursor; one Undo restores the previous range and loop state. A selected clip's context menu also offers **Loop playback over selection**; right-clicking another clip offers **Loop playback over clip** without changing your selection. In Clip Editor, **Range → Loop playback over clip** repeats the open clip.

The transport's **Loop** toggle switches repetition on or off. **Range → Loop playback over arrangement** sets and enables a loop from the beginning through the last clip, preserving the selection and playback cursor. **Range** also opens **Start beat** and **End beat** for exact adjustment; use their **−/+** buttons for quarter-beat steps, drag the values, or double-click to type. Beats start at 1, and the end is exclusive: Start 1 / End 5 repeats the first four beats. The read-only **Starts at**, **Ends at** and **Elapsed** rows show clock time and bars/beats. The ruler's loop strip shows the same positions and times when space allows; hover it for the complete summary.

Drag either visible endpoint of the strip to adjust the loop directly. The drag follows **Snap**; turn Snap off for free adjustment. A pointer hint shows the endpoint and duration while dragging. Each completed drag is one Undo step. Only actual endpoints have handles, so a loop extending beyond the view must be brought into view before dragging its hidden end. Range changes are disabled while recording and saved with the project.

**Range → Duration** shows seconds and offers exact 5, 10, 15, 30, 60 and 120-second durations from the current loop start. Select a phrase and choose **Edit → Repeat to loop end** to fill that range, including a partial last copy. WAV export still renders the full arrangement once.

## Effects, sends, and folders

Mixer channels read from top to bottom: source, **Effects** and inserts, volume
and pan, **Mute**/**Solo**, stereo **Level**, then **Output** and sends. The fader
has a calibrated dB scale. Pan uses a compact rotary knob: drag up or right to
move toward R, down or left toward L; Shift makes adjustments finer. Pressing
the dial does not jump its value. Its readout shows **C** at center and **L/R**
percentages elsewhere. Right-click anywhere in the Pan row for exact percentage
entry and automation. The full-width Mute/Solo row shows each active state clearly. The
horizontal L/R meter measures the channel after volume and pan, while Master
shows the final stereo output.

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

Routing gives each send a larger level knob, a **1× unity** mark and a clickable
value beside **Level**. Drag up or right to increase the send, down or left to
decrease it; Shift makes adjustments finer. The taper gives quieter sends more
useful travel while retaining the 0–16× gain range. Click the value for named
exact entry and **123** mouse digits. The value readout follows the dial.

Selected clips and mixer channels have a subtle glow in their track color. Hover or select a mixer channel to trace its output and sends: continuous U-shaped arrows blend from that channel’s color to each destination’s color. Mixer reserves space for its output and first send route plus an ordinary notice; shorter windows and additional send lanes still scroll. Notices and added sends keep the existing fader, pan, and mute/solo controls in place.

Drag an Arrangement track's name or grip to reorder it. The insertion line shows
Before or After; drop in the middle of a folder header to move inside it. Folders
move with all their contents. Drag Mixer channel names the same way, including
buses, and both workspaces follow the new order. Dropping before or after a track
places it in that track's folder. Hold near a pane edge to scroll during a drag;
Escape cancels. Each completed move supports Undo/Redo and saves with the song.

Select a track and choose **Window → Effects**, or use **Channel → Effects** in Mixer. **+ Insert** opens the device browser; drag a slot handle to reorder it. **Open editor** opens the native effect controls and parameter automation. **Create send track** in the browser creates a return bus with the selected effect and a send from the source. Set the plugin wet/dry control as appropriate. To create a shared effect return:

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

The default header is 288 points wide and each normal row is 144 points high.
Resize the header gutter or track's lower edge, or use **View → Track layout**.
Compact tracks keep mix controls and expose the full channel through the small
channel-controls button; they do not squeeze routing and instrument names into
the title. Resizing the header or rows is view state and leaves the song and
its history intact.

## Automation

Right-click a channel fader, pan control, or send level to enter an exact value;
Routing also opens it directly from the send's value button. The menu identifies
the channel and parameter, plus the destination for a send, and shows a readable
current value and allowed range. Hover the field for its full stored value.
Opening the editor or choosing Apply without editing does not round the stored
number or create an automation override. Choose **123** beside the
field to enter digits with the mouse, then choose **Apply value** to
commit or **Cancel** to keep the original. This also works in compact track
headers. Volume entry uses dB (−60 dB means silence), pan uses −100% left through
0% center to 100% right, and send levels use a linear multiplier. Properties also
has a directly editable volume readout. Exact-entry fields retain fractional
values; concise displays do not change their stored precision.

Right-click a channel volume, pan, or send control and choose **Create automation** to start a lane at its current value. Track-header rails keep only a concise current value and pan center mark. Parameter sliders retain endpoint captions and adaptive tick scales, showing halves, fifths, or tenths. Mixer Pan uses a rotary position index with a current balance readout; its context menu accepts −100% left through 0% center to 100% right. Mixer volume keeps its vertical fader, dB scale and separate editable dB readout.

An orange **A** identifies a control following automation, a coral **!** identifies
a manual override, and a muted dash identifies bypassed automation. **Override
automation** holds the current value without deleting the curve; **Resume
automation** returns to the saved curve at the current playhead. Applying a new
value to an automated control creates the same visible override as dragging it.

The plug-in parameter browser uses a knob with a visible range, position index,
and editable number. Drag vertically or horizontally to adjust it; Shift makes
dragging finer. Right-click the knob for exact entry with **Apply value** and
**Cancel**, or open **123** there for mouse-only digit entry. Its value is
explicitly a normalized percentage, because plug-in
unit names alone do not provide a physical-unit conversion. Stepped parameters
use their declared steps when edited, and automation state remains visible.

Select a channel, then choose **Track → Add automation track…**. Pick Volume, Pan, a send, or a VST3 parameter. Each parameter gets its own arrangement track. Double-click that lane, or choose **Window → Automation**, to open its editor.

Click to add points, drag to move them, and use **Beat**, **Value**, or **Delete point** for precise edits. New lanes use **Spline**: drag the diamond between two points to bend that segment, or select it and choose **Log**, **Linear**, **Exp**, or adjust **Slope**. Each segment keeps its own shape without overshooting its endpoint values. Step, Linear and Smooth modes remain available.

Automation covers the entire song. The first point's value applies before that point and when playback starts; the last value is held indefinitely after the last point. Controls show the value at the playhead even while stopped. Orange curves and control markers identify automation consistently across the app. Automation itself does not extend the song's duration.

Adjust an automated control to override its curve. **Overridden** and coral-red markers identify the manual value, while the original curve stays intact. Choose **Resume automation** in the lane editor, the control's right-click menu, the channel status menu, or plug-in parameter details to follow the curve again at the current playhead. The lane editor also offers **Override automation** and an exact override value. Overrides save with the project and support Undo/Redo. Disable or mute a lane to use the channel's stored manual value instead; this is shown as **Bypassed**.

For an instrument, open **Window → Instrument**, choose **Open editor**, and adjust a native plug-in control. **Automate last touched** creates its parameter track. You can also search the parameter list, select a control, and choose **Automate parameter**. Effects provide the same native editor, searchable parameter list, and last-touched automation workflow from their insert chain; parameters are also available in the Automation view. Discovery and native editors run in separate helper processes. Mixer controls render sample-accurately; VST3 automation is evaluated once per 256-frame block. Automation lanes do not extend song duration.

## VST3 instruments

Open **Window → Setup → Plug-ins**, add folders, and apply the settings; Rill rescans them. Choose **Track → Add instrument…**, type a name or vendor, then press Enter or double-click a result. Rill creates a MIDI track and four-beat clip, assigns the VST, and opens its native editor. **Track → Add MIDI track** opens the same chooser after creating its empty pattern. Double-click the clip to write notes. To use an existing MIDI track, choose **Choose instrument…** in its arrangement or Clip Editor header, or open **Window → Instrument → Choose instrument**; assigning the plug-in preserves its notes. **Window → Library** also provides browsing and explicit add/assign actions. When no VST3 instruments/effects have been found, the library explains installation and links to **Plug-in folders**. When installed inventory is hidden by search or filters, it offers **Clear filters**.

Choosing an instrument in the initial creation flow gives an untouched **New
instrument** track the device's name; repeated names receive a numeric suffix.
That naming change belongs to the same Undo step as assignment. Names you set,
names loaded from projects, and names on tracks assigned through a reopened
chooser remain intact, even if the name is **New instrument**.

Rill keeps an index of installed plug-ins. Each launch shows the last index immediately, then rescans the plug-in folders in the background; **Refresh** in the library rescans on demand, for example after installing a plug-in. New, updated and removed plug-ins are picked up without restarting. After a scan, Rill opens each verified instrument or effect that has no preview in an isolated helper, screenshots its native editor, and adds the image to the index, one plug-in at a time. The library header shows how many previews remain. Cache entries are invalidated when the plug-in bundle changes. A failed editor or screenshot leaves the plug-in listed under **Without preview**, alongside plug-ins whose image has not been captured; it is not retried on every scan, and it does not close Rill. Plug-ins without classification metadata are labelled **Unverified VST3**; their preview is captured when you select one, and they are checked when loaded. **More → Capture preview** or **Refresh preview** captures one again. **Preview image / Unavailable** refers only to that optional image; details retain the capture error, and the instrument can still be assigned. Loading reports its own errors separately.

The library sidebar filters by collection (**All plug-ins**, **Favorites**, **With preview**, **Without preview**) and by developer, each with a count. Search, **All categories**, **Sort** (name, developer or recently added) and **View** (grid or list) sit above the catalog. Active filters appear as chips; click one to remove it, or use **Clear filters**. Selecting a device shows its preview, **Favorite**, install path with **Copy path**, categories and VST3 details (kind, version, architecture, preview status) beside the catalog. Categories come from the plug-in's VST3 subcategories until you type your own, separated by commas, and choose **Save**; saving an empty field restores the plug-in's own. **More** and a card's context menu also add or remove favorites. Favorites and categories are stored in `plugin-library.json` next to `settings.json` and are reported by the `list_plugins` tool.

Instrument and effect edits automatically persist normalized parameters and opaque plugin state. Native editors use a separate instance from audio rendering; parameter gestures update the playing instance and debounced state snapshots capture preset changes. Save, export, device changes, and document transitions flush the editor first. Full state changes rebuild playback and can briefly interrupt audio. The native editor and instrument parameter list provide access to the plug-in's controls; Rill does not supply instruments, effects, or a separate preset browser.

Thumbnails and the plug-in index live in `~/Library/Caches/Rill` on macOS, `%LOCALAPPDATA%/Rill/Cache` on Windows, and `$XDG_CACHE_HOME/rill` (default `~/.cache/rill`) on Linux. Set `RILL_CACHE_DIR` to override this location. Native editor capture needs a desktop session; Linux uses X11/XWayland with the XRes and Composite extensions.

## Recording and overdubs

Choose the input and output devices in **Window → Setup → Audio**, then select an audio track and open **Window → Recording**. Choose **Mono 1**, another input channel, or a stereo pair; enable **Arm recording** and press **Record** in the popup or transport. Track headers also have an arm control. Recording starts at the transport cursor and can continue beyond the current arrangement. The popup's meter shows the raw input level.

**Input monitor (dry)** offers **Off**, **Auto**, and **On**. Auto monitors armed tracks while stopped or recording and turns monitoring off during ordinary playback. On monitors even when the track is unarmed. Monitoring follows the channel and output buses' volume, pan, mute, and solo; it bypasses their inserts and sends. Recorded WAVs contain the selected raw input, independent of mixer levels, and monitoring is not included in export.

Enable the transport loop before recording to retain each pass as a separate take. The newest complete pass becomes audible; a partial final pass remains available without shortening that complete pass. Open **Takes…** to select whole passes or comp ranges with smooth or custom spline crossfades. **Record into → Overdub layer** creates a separate colored audio track with the source track's bus, folder, volume, and pan; inserts and sends are not copied. Existing clips and recorded sources remain intact. Each recording is one undoable edit.

**Stop recording** finalizes capture while backing playback continues. Pause, Stop, Save, New/Open, Quit, and audio-setup changes finish and attach captured audio before proceeding. Input choice is saved with the project; arm and monitoring choices are session-only and reset when a project opens. Sources are written under the scratch directory's `Recordings` folder and included in `.rill` archives. Input failures retain completed audio and report recovery details. Limits are 32 armed routes, 64 loop passes per session, and 512 MiB of audio per take. Driver timestamp alignment and failure recovery have device-free regression coverage; physical input timing and microphone permissions still require hardware verification.

### MIDI recording and pre-count

Select a MIDI track and open **Window → Recording**. Choose the shared **MIDI device** and **All channels** or one channel, arm the track, then press **Record**. Arming MIDI initially enables **Auto** monitoring through its assigned plugin instrument; **Off** and **On** remain available. You can record notes without an instrument and assign one later. MIDI device selection is also available under **Setup → Audio**. Opening a project never arms tracks or opens MIDI inputs automatically.

Each session adds a new MIDI clip on the existing instrument track. Existing clips remain intact and play alongside it, so recording over them overdubs their notes without creating another plugin instance. Loop passes accumulate in one clip, splitting held notes at the loop boundary. Recorded data includes note start/end, pitch, velocity, same-key retriggers, and sustain-pedal note lengths. Pitch bend, aftertouch, ordinary CC automation, and SysEx are not recorded. Audio and MIDI can be armed together and publish as one undoable edit.

**Pre-count** in the Recording popup offers **No pre-count**, **1 bar**, or **2 bars**. From stopped, the count uses project tempo/time signature while the cursor stays at the intended start. Notes and audio played before the cue are excluded; notes already held before it are not chased into the recording. Recording during existing playback starts without pre-count to keep backing audio uninterrupted. Stop during the count cancels without creating clips. MIDI audition pauses during pre-count while the existing instrument graph prepares the exact start; dry audio monitoring remains available, and MIDI monitoring resumes at the cue.

The transport's **Click** toggle enables the metronome during playback and recording, with a stronger first beat of each bar. The Recording popup also sets its level. Pre-count clicks play even when ordinary Click is off. Click audio goes only to the output device and is never written into recorded audio or a WAV export. Project archives save click, level, and pre-count choices. Dummy MIDI input and device-clock tests run without a connected keyboard or audio device; physical MIDI/hardware timing still requires verification.

## Setup and recovery

**Window → Setup → Audio** lists input/output devices and supported sample rates, buffer sizes, and High quality/Fast sample conversion. Input must support the selected project rate and channels. Rill defaults to 48 kHz, 32-bit float audio buffers, and 64-bit summing. High quality uses filtered source-rate conversion; Fast trades alias rejection for preparation speed. Changes finish recording, flush native editor state, stop and retire old device/plugin workers, then initialize replacements in the background. Failed device initialization restores the previous settings and reports the error; device errors offer **Audio setup…** directly in the notification. A restart includes a brief pause. **Storage** selects the scratch directory; **Plug-ins** adds VST3 scan roots. Unsupported device settings report an error rather than silently substituting a configuration. The render queue holds two requested buffers, with a minimum of 512 frames; hover the device buffer latency to see its contribution. Setup edits apply only after **Apply**; Cancel, Escape, and closing the popup discard them.

A recoverable audio-output dropout shows a warning with its count and **Audio setup…**; ordinary playback continues while the audio backend recovers. Try a larger buffer if dropouts recur. The warning stays in one notification instead of repeating for every dropout. If a dropout occurs while recording starts, during pre-count, or during capture, Rill stops capture to preserve timing and retains any recorded prefix; after capture finishes, choose **Retry playback** or press **Play** to reconnect before recording again.

If the output stream fails with a device error, Rill stops that stream and reports the device fault once, separately from plug-in failures. **Details** retains the backend message and **Audio setup…** opens the device settings. Check the output device, then choose **Retry playback** in the notice or press **Play** to reconnect at the current position, or **Stop** followed by **Play** to start from the beginning. Dismissing the notification does not restart the failed stream. Recording and note audition remain unavailable until it reconnects.

Setup persists at `~/Library/Application Support/Rill/settings.json` on macOS, `%APPDATA%/Rill/settings.json` on Windows, or `$XDG_CONFIG_HOME/rill/settings.json` (default `~/.config/rill`) on Linux. The scratch directory holds demo audio and `recovery.daw.json`. Unsaved edits are also saved to recovery on a normal close or before switching sessions. Use **Setup → Storage → Recover session**; recovery opens without an existing document save path.

New projects save as `.rill` ZIP archives containing `manifest.json` and source WAV files; edits, MIDI, routing, automation, and instrument/effect state live in the manifest. Extracted audio uses the configured scratch directory. Plugins and their own sample libraries must remain installed. Legacy `.daw.json` files stay supported. See [project format and limits](PROJECT_FORMAT.md).

After Save as, the project header shows the saved filename with a dirty marker for later edits; hover it for the full path. Save and WAV/MIDI export pickers reuse that directory and filename stem. Extensionless project saves receive `.rill`. **Export WAV…** warns when visible notes on contributing MIDI tracks have no instrument; Cancel to assign a VST3, or continue to export without those notes. The WAV dialog shows its sample rate, stereo layout and 32-bit float format. Its duration summary says **+ tails** when **Include effect tails** is enabled; turn it off to end exactly at the arrangement boundary. WAV and MIDI completion show the resulting filename and **Copy path**. MIDI export waits for a pending WAV export or session update to finish. Offline WAV rendering does not require an output device.

On Linux, if a document file picker reports a portal or Zenity helper failure,
Rill shows **Could not open the file picker** and retains your work. **Details**
includes the cause and, when the fallback fails, guidance to restore the desktop
portal or make `zenity` available on PATH; then retry Save, Open, Import or
export. A failed Save inside **Save changes?** leaves that decision open. Normal
picker cancellation stays silent. Other platforms retain their existing native
picker behavior.

**File → Recent exports** keeps the current document's last five successful WAV/MIDI destinations. Choose an entry to copy its full path. This lets you check an earlier WAV after the MIDI completion appears. Re-exporting the same destination moves it to the front; failed or cancelled exports add no success entry. New/Open clears the list.

## Plug-in stability

Each VST3 instrument/effect instance runs in a separate `daw-plugin-host` process. Scanning reads metadata without executing plug-in code in the DAW. A crash or timed-out processing command silences that path while the session stays open; **Stop**, then **Play**, creates fresh instances. Export failures preserve an existing destination file.

This is crash isolation. Helpers retain the user's filesystem and network permissions; they are not an OS permission sandbox. Plug-in IPC runs on the render worker, away from the lock-free device callback. The helper must be next to `rill`, or specified by `DAW_PLUGIN_HOST`.

VST3 audio, MIDI, latency, transport, effect inputs, native instrument/effect editors, parameter automation, and instrument/effect state recall are supported. Sidechains, a standalone preset browser, and CLAP/VST2/AU/AAX hosting are not included.

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

See [architecture and limitations](ARCHITECTURE.md) for the processing graph, real-time boundaries, file formats, and PDC model. Current media scope is PCM/float WAV, audio input recording, and constant-tempo note MIDI; MIDI controller/expressive recording, disk-streamed playback, tempo maps, automatic tempo-following warp markers, and surround are not implemented. Volume, pan, note, and clip edits update the existing playback graph; failed audio preparation keeps the last valid audio playing. Structural, plugin-state, and device changes still rebuild playback and may briefly interrupt it. Cosmetic changes do not rebuild the graph, including Undo/Redo.

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
