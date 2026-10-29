# Make music with the assistant

[User guide](USER_GUIDE.md#ai-assistant-and-mcp) · [Composition tools](COMPOSER.md) · [Tool architecture](AI_TOOLS.md)

Open **AI** in the menu bar for the current song, or **AI for this context** in
an editor for that opened object only. Check the **Scope** line, then choose
**Examples**. Choose a musical task, adjust the
inserted prompt, and press **Send**. An example is a draft: browsing and inserting
one do not change the song or contact a provider. Existing draft text is kept.
The **Tips** page remains available when you want help making a prompt specific.

Use **AI settings** to choose OpenAI or Anthropic, a model, and your API key.
Apply the settings, then return to the assistant. Provider charges and usage
limits apply. Rill sends the prompt and bounded song context to the selected
provider; its audio analysis tools return local measurements rather than
sending audio for perceptual listening.

## Choose a starting point

Global AI includes these twelve prompts. Contextual AI supplies a smaller set
for its opened object, described below. Each can be edited before it is sent; the [source catalog](../crates/daw-ai/src/guidance.rs) contains their exact
wording.

| Category | Example | Starting point and result |
| --- | --- | --- |
| Compose | Start a song | Eight bars of D-minor chords, arpeggio, and bass at the cursor, preserving existing clips. |
| Compose | Write a melody | A four-bar A-minor motif with rests and an ending on A, placed on a new track. |
| Compose | Build a pulse | Four bars of a five-hit, sixteen-step Euclidean pulse, with explicit pitch and accents. |
| Develop | Create a variation | Select a MIDI clip; keep its original and develop the ending in an independent copy. |
| Develop | Humanize a phrase | Select MIDI notes or a clip; add small, reproducible onset and velocity changes. |
| Develop | Connect chord voices | Select a chord phrase; join note lengths by high-to-low voice rank while retaining pitches. |
| Mix | Check headroom | Inspect routing and measure a bounded mix excerpt; receive observations and suggestions without edits. |
| Mix | Make room in the mix | Reduce unmuted source-channel gains by 3 dB, retaining buses, sends and automated gains. |
| Mix | Automate a fade | Select an audio, MIDI or bus channel; create a four-bar spline fade after checking for existing automation. |
| Learn | Explain the harmony | Select a MIDI clip; discuss its actual pitches, likely tonal center and possible continuations. |
| Learn | Explore modal color | Compare C major and C Dorian using scale and chord queries, without editing the song. |
| Learn | Plan the arrangement | Inspect existing material and propose sections and part entrances before making edits. |

Composition uses the song's current meter and tempo. Newly created MIDI tracks
need VST3 instruments to be audible. Rill includes no bundled instruments or
effects; ask the assistant to reuse an existing instrument track or choose from
your installed plug-ins. A created MIDI clip alone is not proof that it will
make sound.

## Work inside one editor

Open a MIDI clip's piano roll and choose **AI for this context** to work only on
that clip. The **Scope** line names the pinned object, and the outgoing prompt
states the same boundary. The local **Examples** offer humanization, legato and
harmony explanation. Changing the selection elsewhere does not change the AI's
target. To create a separate variation or edit several clips together, open
menu-bar **AI** for the current song.

An audio clip's contextual AI can measure or normalize that region and adjust
its in-place properties such as fades. Splitting, stripping silence and joining
regions require global AI. Linked MIDI notes cannot be edited through local AI
if the edit would propagate to another clip; make the opened clip independent
first, then have AI inspect its fresh note IDs.

Other editors offer matching examples:

| Opened context | Example requests |
| --- | --- |
| Instrument | Explain its actual controls; suggest a parameter change before applying it. |
| Effect | Explain its parameters; bypass this insert while preserving its settings. |
| Effect chain | Review insert order; bypass this channel's chain in one edit. |
| Channel | Lower this channel by 3 dB; center its pan while respecting automation. |
| Routing | Explain this channel's routes; lower only its sends. |
| Automation lane | Explain the curve; use neutral spline segments; resume this lane's automation. |

Choose **Tools** to browse or search only the tools allowed in the displayed
scope. Select a tool to read its description and contexts, then choose **Use
tool** to add an editable prompt. It preserves an existing draft and does not
send anything. The same catalog is supplied to the model, and the host rejects
any call or resulting edit that would leave the stated scope.

Local AI cannot change global transport, recording, selection or history. Use
the normal transport to audition results and **Edit → Undo** to reverse an edit.
Changing AI scope starts a fresh conversation; switching songs also clears the
draft and cancels in-flight work.

## Build a song in small, audible steps

1. Place the cursor where the idea should begin. Choose **Examples → Compose →
   Start a song**.
2. Change the key, length, roles, or rhythm in the draft. Name existing tracks if
   you want to reuse their sounds, and state which material should remain intact.
3. Press **Send**. The activity rows show work in progress; the final response
   describes the results that the tools confirmed.
4. Play the result using the transport. Ask for one focused refinement, such as
   “Keep the chords and bass. Make only the arpeggio sparser, with a rest before
   each chord change.”
5. Use **Undo** if an edit does not work. Song edits share the ordinary history;
   a multi-step request can create more than one undo entry.

Keep your important constraints in the prompt when a conversation becomes long.
The assistant carries bounded recent conversation and refreshes song context for
each request. It does not retain unlimited memory of earlier briefs.

## Develop a phrase without losing it

Select a MIDI clip and choose **Develop → Create a variation**. The example asks
for an independent copy immediately afterward, leaving the original intact. It
asks for placement guidance if that space is occupied. You can make the brief
more precise:

> Keep the opening motif and the instrument. Make the last bar answer the first
> with fewer notes and a final A. Keep the variation separate from the original.

For timing and dynamics, **Humanize a phrase** uses explicit small amounts and a
fixed random seed. For connected chord parts, **Connect chord voices** changes
lengths while preserving pitches and onsets. The context line helps confirm the
selection; saying “selected notes only” makes the intended scope explicit.

## Ask for measurements before mixing

**Mix → Check headroom** inspects routing and renders a local excerpt for peak,
RMS, clipping, stereo and related measurements. Rendered analysis is limited to
30 seconds and must end within the song's first 120 seconds; the example handles
an out-of-range selection by explaining the limit and using a suitable earlier
excerpt. It requests suggestions without changes.

Listen yourself before applying a suggestion. A measurement can show clipping
or a level difference, but the assistant should not claim that it heard an
instrument or judged the emotional impact of a mix. A useful follow-up is:

> Lower the named source channels by 2 dB and keep the buses and sends unchanged.
> Leave existing automation intact. Explain which controls changed.

**Automate a fade** uses the current channel level, a four-bar spline, and held
endpoint values. If that channel already has volume automation, its example asks
the assistant to inspect and discuss it before replacing points or changing an
override. Orange curves and override indicators remain visible in the normal
automation and mixer views.

## Recover and start fresh

- **Stop** prevents further assistant work. Edits already committed remain in the
  song; reverse them with **Edit → Undo** if needed.
- **More → Edit last prompt** brings the last request back to an empty composer.
  Review the current song before sending again, especially if an earlier run
  stopped after making some changes.
- **More → New chat** clears the assistant's conversation and status, preserving
  the song and your unsent draft.
- Opening or creating another song clears the conversation and draft. In-flight
  work is cancelled, and stale tool requests cannot edit the replacement song.

Global AI can compose, inspect, edit and audition the current song. Contextual
AI is restricted to the object shown in its Scope line. Use the
normal **File** menu to open, save, import or export projects; the embedded
assistant does not perform those document operations.
