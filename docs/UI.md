# Shared UI components

`daw-ui` is Rill's shared component library. It depends on `egui` 0.33.3 without depending on the application, audio engine, or native window runtime. The app uses it for controls, surfaces, popup windows, and design tokens. New reusable components belong here before they are used in an application view.

Arrangement, Clip Editor, and Mixer each own the central workspace. Clip Editor and Mixer have a Back action; returning restores the prior selection and viewport. Instrument, Effects, Routing, Automation, Library, Setup, and Properties open on demand as one centered utility at a time, with a quiet background instead of an obscured arranger. Transport and the File, Edit, Track, and Window menus remain available. Components keep labels short; explanations belong in tooltips or the relevant utility rather than permanent interface text.

## Theme and tokens

Call `daw_ui::theme::apply(ctx)` when creating the application. It configures both native egui palettes. `theme::set_appearance(ctx, Appearance)` selects System, Light, or Dark; System is the default and follows operating-system changes. Read `theme::palette(ctx)` for the current canvas colors. Logical dimensions and typography come from `daw_ui::tokens`:

| Token family | Values |
| --- | --- |
| Spacing | `SPACE_XS` 4, `SPACE_SM` 8, `SPACE_MD` 12, `SPACE_LG` 16, `SPACE_XL` 24 |
| Controls | `CONTROL_HEIGHT` 30, `COMPACT_HEIGHT` 24, `SELECT_WIDTH` 160 |
| Corners | `RADIUS` 3 |
| Typography | Caption 13, note 14, button/body 15, subtitle 16, title 20; canvas 12, track title 14 |

Sizes are logical screen points and follow egui's display scaling. Surfaces use neutral grays and a restrained luminance hierarchy. Action buttons have a quiet fill; inputs retain visible boundaries. Accent color identifies active choices and keyboard focus. Cards and utility windows have no shadows, and sections use spacing rather than nested outlines. Body and button text remain 15 points, with 30-point action targets. Avoid introducing a second app-specific palette or copied spacing scale.

## Components

Standard controls are available at the crate root and through `daw_ui::widgets`. The curve editor is exported at the crate root.

| Purpose | API |
| --- | --- |
| Standard actions | `button`, `primary_button`, `small_button` |
| Search-result choice without native menu navigation | `picker_button` |
| Button builder for sized/disabled placement | `button_widget` |
| Application and contextual menus | `menu_bar`, `menu_button` |
| Focused workspace structure | `workspace_header`, `toolbar`, `toolbar_group`, `action_menu`, `context_section` |
| Mixer channels and metering | `mixer_strip`, `stereo_meter` |
| Boolean button | `toggle` |
| Boolean setting | `switch`, `checkbox` |
| Exclusive choices | `radio_value`, `selectable_value` |
| Selected navigation/item | `selectable` |
| Arrangement overlay controls | `button_at`, `picker_at`, `selectable_at`, `number_at` |
| Text input | `text_field` |
| Dropdown choices | `select`, `select_labeled` |
| Numeric widget builders | Generic `slider`, `number` |
| Track color | `color_edit` |
| Timeline navigation and tools | `TimelineViewport`, `TimelineBounds`, `TimelinePane`, `EditorTool`, `editor_tools`, `editor_tool_shortcuts`, `timeline_fit_button`, `paint_timeline_ruler` |
| Interactive parameter curves | `curve_editor`, `CurvePoint`, `CurveView`, `CurveEdit` |
| Complementary audio crossfades | `crossfade_editor`, `CrossfadeCurve`, `CrossfadeResponse`, `paint_crossfade` |
| Plug-in library entries and previews | `catalog_item`, `image_preview` |
| Surfaces | `panel`, `card`, `inset` |
| Toolbar icons | `Icon`, `IconButton`; ordinary button builders resolve known action icons |
| On-demand windows | `popup`, `workspace_popup` |
| Compact section label | `theme::label` |

Use native response composition rather than adding a second event system:

```rust
use daw_ui as controls;

fn channel_controls(ui: &mut egui::Ui, enabled: &mut bool, name: &mut String, gain: &mut f32) {
    controls::card(ui.ctx()).show(ui, |ui| {
        controls::text_field(ui, name, "Channel name", 220.0);
        controls::switch(ui, enabled, "Enabled");
        ui.add(controls::slider(gain, 0.0..=2.0).text("Gain"));
        if controls::button(ui, "Reset").on_hover_text("Restore unity gain").clicked() {
            *gain = 1.0;
        }
    });
}
```

Dropdown builders preserve egui's composable API. Give unlabelled dropdowns a stable ID, and use shared choice controls inside them:

```rust
fn interpolation(ui: &mut egui::Ui, selected: &mut usize) {
    daw_ui::select("interpolation", ["Step", "Linear", "Smooth"][*selected])
        .width(120.0)
        .show_ui(ui, |ui| {
            for (value, label) in ["Step", "Linear", "Smooth"].into_iter().enumerate() {
                if daw_ui::selectable_value(ui, selected, value, label).clicked() {
                    ui.close();
                }
            }
        });
}
```

Use `popup(title, ctx).open(&mut open).show(ctx, ...)` for utility windows. The builder centers them below the transport, bounds their size to the viewport, and scrolls long content vertically. The native close control remains visible. It supplies a common frame, spacing, and initial width, with fixed sizing and no collapse triangle. Callers may override width, `.resizable(true)`, or `.fixed_pos(...)`; the default pivot is the center. Surfaces `panel(ctx)`, `card(ctx)`, and `inset(ctx)` use the same resolved palette. An `open = false` popup renders no content.

`workspace_header(ui, title, actions)` returns `WorkspaceHeaderResponse { back, inner, response }`; handle `back.clicked()` in the application's navigation logic. The action closure is laid out from right to left. The title truncates before trailing controls, and the header occupies one bounded row. `toolbar(ui, ...)` wraps whole native controls when needed; both helpers include an eight-point inset. `toolbar_group` keeps related controls together. `action_menu` is a native menu for secondary commands; use nested action menus for submenus rather than a ComboBox inside a menu, because those share egui's popup state. `context_section` groups a quiet label and controls without another panel outline.

`mixer_strip(ui, id, width, color, selected, contents)` creates a vertical channel with a stable ID, optional thin track-color marker, and a subtle divider. Width includes its side margins; callers choose a common height. `stereo_meter(ui, [left, right], size)` displays −60 to 0 dBFS peaks, marks clipping with the warning color, and sanitizes non-finite input. It does not own audio state or smoothing. Catalog rows reserve the same preview space while thumbnails are loading so names and actions remain aligned.

**Setup → Appearance** applies and persists System, Light, or Dark. Cancel discards the draft. Changing only appearance leaves playback and native editors running. Piano-roll black-key rows remain darker than white-key rows in both modes; blue scale and chord tints preserve that distinction. Note labels select a contrasting foreground for their velocity-dependent fill.

`selectable_at(ui, rect, id_salt, selected, label)` is for controls drawn inside an existing arrangement canvas. It supplies a compact native button in the given rectangle without advancing or enlarging the parent's layout. It requires a stable semantic ID such as `(track.id, "mute")` and retains native hover, focus, keyboard, and accessibility behavior. It does not own arrangement geometry.

`number_at(ui, rect, id_salt, number(&mut value))` places a shared `DragValue` inside a canvas rectangle without advancing the parent layout. Configure its range, speed, precision, prefix, and suffix through the `number` builder. Drag to adjust or double-click to type; use a stable ID such as `(track.id, "gain")`, and commit when the returned response reports `changed()`.

`button_at` places a compact action in a canvas rectangle, truncating long labels without changing the row geometry. `picker_at` uses the same action with an anchored choice popup; it returns the action response and optional popup result. Both require stable IDs and retain native keyboard/focus behavior. Add a tooltip when the full label may be clipped, and use `ui.close()` when a choice is accepted.

Arrangement owns its 248-point header geometry. Audio, MIDI, and bus headers show the same channel number and destination as Mixer (`Ch N > Master` or a bus name). Click this label to choose Master or an eligible output bus; self-routing and feedback through outputs or sends are excluded. MIDI headers also show their assigned instrument (or `Choose instrument…`); clicking it opens the VST chooser for that track and choosing a plugin opens its native editor. Both pickers remain available in compact rows. Expanded rows reuse `selectable_at` for mute/solo and `number_at` for dB volume and −100..100 pan. Folders and automation lanes have no audio channel number or fader/pan; folder mute/solo acts on descendants.

**Track → Add MIDI track** creates a four-beat pattern at the cursor and opens the library in assignment mode. Selecting an instrument assigns that existing track and opens its native editor. Closing the chooser leaves the pattern unassigned. **Add instrument…** starts with the chooser and creates the track and pattern after selection.

Arrangement and clip editors share `TimelineViewport` and `TimelinePane`. Call `navigate` before domain hit testing and suppress edits while its `navigating` flag is true. Middle drag or Alt+left drag pans; Ctrl+wheel zooms horizontally around the pointer, Ctrl+Shift+wheel vertically; Ctrl+right or Ctrl+middle drag zooms around the initial pointer position, with Shift selecting the vertical axis. Plain wheel scrolls vertically and Shift+wheel horizontally. Plus/minus zoom horizontally; Shift+plus/minus vertically. `Fit` or F fits the earliest-to-latest event interval and every relevant row on both axes. The shared ruler shows musical bars and elapsed clock time using the same mapping. Text entry, disabled background canvases, and open menus do not consume editor shortcuts.

Tool keys retain the same meaning wherever supported: Q Select, W Pencil, E Cut, R Duplicate, T Glue, Z Erase, U Multi-draw, I Fade in, O Fade out, P Cursor. Each editor exposes only its applicable subset. The arranger retains fixed track headers and a fixed ruler while the musical canvas pans; its native vertical scrollbar remains available.

## State, focus, and accessibility

State stays with the caller. Boolean and choice components mark `Response::changed()` only when their value changes. Use `.clicked()` for actions and `.changed()` for committing edits. Shared components do not create undo history, mutate project state indirectly, or rebuild audio themselves.

The original Rill outline icon pack uses a 24-unit vector grid and a consistent stroke weight. Icons render through egui at the current display scale, independently of Unicode symbol fonts. Buttons keep their text, native accessible name, keyboard behavior, disabled state, and tooltip. Shared `IconButton` uses native `Button`/`AtomLayout` to reserve icon space; call `Icon::paint` for canvas-owned controls. Add new icons here instead of inserting ad hoc emoji into toolbar labels.

**Edit → History** shows named undo and redo actions. Clicking an entry moves through the intervening steps in one engine update. Ctrl/Cmd+Z and Ctrl/Cmd+Shift+Z work in editing views and utility popups; text fields retain their native text undo. Ctrl/Cmd+Y also redoes. Continuous drags are one step, and text/native parameter bursts on the same control coalesce for up to 600 ms. Navigation, zooming, and tool selection never enter history. Saving establishes a checkpoint; returning to it removes the unsaved marker. A new edit discards redo. History is session-local and limited to 128 steps or an estimated 128 MiB of snapshots, including opaque plugin state. An oversized snapshot becomes a history boundary; allocator overhead and the active project are outside this budget.

Disable a group with `ui.add_enabled_ui(false, |ui| ...)`, or use `ui.add_enabled(enabled, button_widget(...))`. Disabled components do not change values from pointer or keyboard input. Scope repeated rows using `ui.push_id(model_id, ...)`; repeated labels alone are not sufficient persistent identity for every control.

Native buttons, checkboxes, radio choices, text fields, dropdowns, and numeric controls retain egui's keyboard behavior. The custom visual switch exposes a labelled checkbox role and selected state through `WidgetInfo`; its whole label/track area accepts clicks, and Space or Enter activates it when focused. It paints a visible focus ring. Supply a descriptive label, including for icon-only controls, through their response's accessibility/widget metadata where the visual label cannot describe the action.

The library's headless interaction tests exercise pointer activation on a switch label, focus and keyboard activation, disabled input, choice change reporting, Unicode text entry, dropdown selection, compact button/numeric-control layout isolation, and hidden popups. These test behavior without coupling application views to widget internals.

Run the component checks with:

```sh
cargo test -p daw-ui
cargo clippy -p daw-ui --all-targets -- -D warnings
```

`workspace_popup(title, ctx)` remains available for a large utility canvas such as automation, bounded below the transport. Clip Editor and Mixer use the central workspace and `workspace_header` instead. Utilities do not paint an inactive arranger behind them. Menu commands, Back, Escape, and window close use the same navigation state transitions. Every added user-facing feature requires a subprocess E2E test; component interaction tests complement those scenarios.

`curve_editor` receives normalized points and a sampling callback, and returns add/move/delete intents. It does not depend on the project model or audio engine. The application owns interpolation, validation, and undo. Canvas points support click, drag, and deletion; precise numeric controls use the same edit path.

`crossfade_editor` edits two normalized cubic Bézier control points and displays
the paired incoming and outgoing gains. Their sum is one. Drag a handle or use
arrow keys (Shift for fine adjustments); Home restores a linear fade. Handle
times stay ordered and gains remain bounded. The response includes change/drag
state and handle rectangles for application undo and interaction tests.
`paint_crossfade` reuses the same curve drawing in a timeline region without
adding layout or input targets.

The transport's **Loop** toggle enables repetition. **Range** opens Start beat and
End beat controls; both use one-based beat numbers, and the end is exclusive.
The current range remains visible beside the picker. Range edits and enabling
Loop are undoable and saved with the project. WAV export renders the complete
song regardless of the transport loop.

## Device library and effect chains

**Window → Library** opens a large browser with Instruments and Effects tabs. Search matches names, developers, and category metadata; the developer selector narrows results independently. The selected device shows its cached native editor image and one primary action appropriate to the insertion context. **More** holds alternate actions, available metadata, and preview capture/refresh. **Scan** stays visible; **Manage → Folders** opens scanning setup. A tooltip includes the plugin location and class identity. Actions remain above preview content, so a background capture cannot move them under the pointer.

The instrument tab creates a MIDI track and pattern or assigns the selected MIDI track, then opens its native editor. **Effects → + Insert** opens this same browser pinned to that channel. **Add to effect chain** returns to the channel's inserts. **Create send track** creates a bus containing the effect and a post-fader send from the source at 25%; configure the plugin's wet/dry control for the desired return. Routing also offers **Create send track**. Escape returns to the caller without inserting anything.

Plugin discovery stays metadata-only. After discovery, one isolated helper at a time initializes verified instruments and effects and captures native previews into Rill's fingerprinted cache. A selected device is prioritized. Failures are retained and retried only with **Capture preview**; **Refresh preview** explicitly updates a cached image. Existing cache entries display immediately. The browser limits resident small textures to 128 and scales the selected preview to at most 1000×600 pixels.

Effect rows use shared `reorder_handle` and `reorder_drop_target` components. Drag the handle to another row to change processing order; Up/Down remains available for direct activation. Reordering moves the complete slot, preserving its ID, opaque state, normalized overrides, bypass state, and automation targets. The normal graph rebuild recomputes processing order and delay compensation.

Mixer uses equal-width shared strips with aligned names, destinations, faders and pan controls. Click a channel name to select it or its destination to open Routing. **Channel** reveals Instrument, Effects and Routing actions. Master shows the stereo output meter; its readings come from the running engine.


## Typography and international names

The shared `daw_ui::text` module defines Title, Subtitle, Body, Label and Note
roles. `title`, `subtitle`, `body`, `label`, and `note` wrap to available width and
expand vertically. Use `truncated` only for intentional single-row layouts; it
retains the complete name in a tooltip. Canvas-owned catalog and track labels use
`paint_row` / `paint_single_line` with ellipsis and a caller-owned hover target.
Keep layout spacing in the shared tokens instead of padding text with spaces.

Inter 4.1 Regular and Semibold are embedded for interface text, with Noto Sans CJK
2.004 for Chinese, Japanese and Korean names. Numeric readouts retain Hack;
egui's Noto Emoji fallback renders monochrome emoji. No runtime font downloads or
system-font lookup is needed. Fonts rasterize at the current display density;
canvas labels use at least 12 logical points and hide when a zoomed row cannot fit
them. Interface text is 15 points, notes 14, subtitles 16, and titles 20.

Font coverage does not imply full Unicode shaping: colored emoji, joined emoji
sequences, bidirectional layout and complex-script shaping are not guaranteed by
the current renderer. Stored project names remain Unicode strings regardless of
glyph availability. Font versions, exact hashes, OFL notices and sources live in
[`crates/daw-ui/assets/fonts`](../crates/daw-ui/assets/fonts/README.md), and notices
are included in release archives and the standalone macOS application bundle.
