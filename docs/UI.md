# Shared UI components

`daw-ui` is Rill's shared component library. It depends on `egui` 0.33.3 without depending on the application, audio engine, or native window runtime. The app uses it for controls, surfaces, popup windows, and design tokens. New reusable components belong here before they are used in an application view.

Arrangement, Clip Editor, and Mixer each own the central workspace. Clip Editor and Mixer have a Back action; returning restores the prior selection and viewport. Instrument, Effects, Routing, Automation, Library, Setup, and Properties open on demand above their caller. The previous workspace and any parent popup remain visible as dimmed, inactive context; only the foreground utility accepts editing input. Menus, dropdowns, and color pickers dim their caller too, while tooltips remain unobtrusive. Transport and the File, Edit, Track, and Window menus remain available. Persistent **Arranger** and **Mixer** choices in the transport switch workspaces directly, including while a utility is open. Window titles identify the current channel or clip, such as **Effects of Bass**. Components keep labels short; explanations belong in tooltips or the relevant utility rather than permanent interface text.

## Theme and tokens

Call `daw_ui::theme::apply(ctx)` when creating the application. It configures both native egui palettes. `theme::set_appearance(ctx, Appearance)` selects System, Light, or Dark; System is the default and follows operating-system changes. Read `theme::palette(ctx)` for the current canvas colors. Logical dimensions and typography come from `daw_ui::tokens`:

| Token family | Values |
| --- | --- |
| Spacing | `SPACE_XS` 4, `SPACE_SM` 8, `SPACE_MD` 12, `SPACE_LG` 16, `SPACE_XL` 24 |
| Controls | `CONTROL_HEIGHT` 34, `COMPACT_HEIGHT` 24, `SELECT_WIDTH` 160 |
| Corners | `RADIUS` 5, `SURFACE_RADIUS` 8 |
| Typography | Caption 13, note 14, button/body 15, subtitle 16, title 20; canvas 12, track title 14 |

Sizes are logical screen points and follow egui's display scaling. Surfaces use soft slate in Dark and warm mist in Light, with a muted sage accent and a restrained luminance hierarchy. Action buttons have a quiet fill; inputs retain visible boundaries. Accent color identifies active choices and keyboard focus. Cards and controls stay flat; utility windows use a clearer border and a soft shadow to distinguish them from the workspace. Sections use spacing rather than nested outlines. Body and button text remain 15 points, with 34-point action targets. Avoid introducing a second app-specific palette or copied spacing scale.

## Components

Standard controls are available at the crate root and through `daw_ui::widgets`. The curve editor is exported at the crate root.

| Purpose | API |
| --- | --- |
| Standard actions | `button`, `primary_button`, `small_button` |
| Search-result choice without native menu navigation | `picker_button` |
| Button builder for sized/disabled placement | `button_widget` |
| Application and contextual menus | `menu_bar`, `menu_button` |
| Focused workspace structure | `workspace_header`, `toolbar`, `toolbar_group`, `action_menu`, `action_row`, `context_section`, `bounded_columns`, `scroll_body` |
| Mixer channels and metering | `mixer_strip`, `stereo_meter`, `channel_controls_at`, `ChannelControlsResponse` |
| Boolean button | `toggle` |
| Boolean setting | `switch`, `checkbox` |
| Exclusive choices | `radio_value`, `selectable_value` |
| Selected navigation/item | `selectable` |
| Related-view navigation and inactive context | `view_links`, `ViewLink`, `ViewBackdrop`, `paint_popup_backdrop` |
| Arrangement overlay controls | `button_at`, `picker_at`, `selectable_at`, `number_at` |
| Text input | `text_field` |
| Aligned settings and paths | `form_grid`, `path_row` |
| Linked clip identity | `clip_title`, `ClipTitleLayout`, `clip_label`, `clip_choice`, `Icon::Link`, `Icon::Unlink` |
| Dropdown choices | `select`, `select_labeled` |
| Numeric widget builders | Generic `slider`, `number` |
| Track color | `color_edit`, `color_strip_at` |
| Timeline navigation and tools | `TimelineViewport`, `TimelineBounds`, `TimelinePane`, `EditorTool`, `editor_tools`, `editor_tool_shortcuts`, `timeline_fit_button`, `paint_timeline_ruler` |
| Interactive parameter curves | `curve_editor`, `curve_timeline_editor`, `CurvePoint`, `CurveView`, `CurveTimeline`, `CurveEdit` |
| Searchable plugin parameters | `parameter_browser`, `ParameterItem`, `ParameterBrowserState`, `ParameterScope` |
| Non-modal operation feedback | `notice`, `NoticeKind` |
| Complementary audio crossfades | `crossfade_editor`, `CrossfadeCurve`, `CrossfadeResponse`, `paint_crossfade` |
| Plug-in library entries and previews | `catalog_item`, `image_preview` |
| Bounded context titles | `text::bounded_block` |
| Inactive clip regions | `paint_clip_tail` |
| Surfaces | `panel`, `card`, `inset`, `soft_surface`, `paint_ambient` |
| Toolbar icons | `Icon`, `IconButton`; ordinary button builders resolve known action icons |
| On-demand windows | `popup`, `workspace_popup` |
| Compact section label | `theme::label` |

Standard controls use comfortable horizontal/vertical padding; canvas-owned controls retain their compact geometry. Popups use 24-point horizontal and eight-point vertical inner margins and scroll within the available viewport. The shorter native title row keeps the complete close target clear of the titlebar's click region. Hover and keyboard focus remain visible. Utility windows have a prominent but restrained 1.5-point border and a broad, soft shadow; menus use a smaller shadow. Panels and controls remain flat.

`paint_ambient` adds a broad, faint radial tint to quiet utility backdrops. `soft_surface` places that same tint between a native frame and its contents, preserving layout and input behavior. Workspace headers and selected mixer channels use this treatment. Music canvases remain flat. The tint uses one bounded mesh (at most 425 vertices), transparent edges, and no textures, blur, animation, or extra repaint requests. Use it on large surfaces rather than individual controls.

`view_links` presents a wrapping row of related destinations with the active one marked. The caller supplies destinations and handles the returned selection. `ViewBackdrop` retains the caller's paint for a utility background without creating widgets, hit targets, or keyboard focus; cached context cannot edit the project or duplicate active accessibility controls. Nested utilities keep their parent context and restore the prior selection and inspector page when dismissed.

Call `paint_popup_backdrop(ctx, utility_open)` once after drawing the workspace and utilities. It paints one theme-aware dim layer below the active window or native menu/picker, including submenus. Retained backgrounds exclude this layer so nested navigation cannot compound the dimming. The shared `modal` uses the same palette color through its native input-blocking backdrop. Confirmation dialogs retain the previous view. Dimming adds one solid rectangle, no blur passes, animation, hit targets, or additional repaint requests.

`scroll_body` inside a window with `.vscroll(false)` lets short utility content fit naturally while long chains remain bounded and scrollable. Use a stable height for tabbed forms and reserve footer space before sizing inner lists.

`notice` keeps readable feedback and trailing actions within one bounded row. It supports progress, success, warning, and error states; long text truncates with a full-message tooltip. Export completion stays visible until dismissed, shows the resulting filename, and offers **Copy path**. Errors retain priority over completion, and a new export clears the previous result. Notifications never add project history or block editing.

Long names and paths must stay within their window. `action_row` reserves trailing controls before flexible content; `path_row` adds path truncation and a full-path tooltip. `form_grid` aligns label/control columns; associate a separate label with its control using `.labelled_by(label_response.id)`. `bounded_columns` gives each library pane a fixed viewport so catalog and preview scrolling leave search and primary actions visible. `text::bounded_block` wraps within a row limit and exposes the full source on hover. Task names belong in native window titles; long track and device names belong in bounded content headings. `clip_choice` includes visible instance metadata to distinguish linked copies with shared names.

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

`mixer_strip(ui, id, size, color, selected, contents)` creates a vertical channel with a stable ID, optional thin track-color marker, and a subtle divider. Size includes its margins; callers choose a common width and height. Mixer strips fill the available workspace height, and faders and meters stretch with the window. Names and footer controls retain their natural size; short windows scroll when the faders reach their usable minimum. `stereo_meter(ui, [left, right], size)` displays −60 to 0 dBFS peaks, marks clipping with the warning color, and sanitizes non-finite input. It does not own audio state or smoothing. Catalog rows reserve the same preview space while thumbnails are loading so names and actions remain aligned.

**Setup → Appearance** applies and persists System, Light, or Dark. Cancel discards the draft. Changing only appearance leaves playback and native editors running. Piano-roll black-key rows remain darker than white-key rows in both modes; blue scale and chord tints preserve that distinction. Note labels select a contrasting foreground for their velocity-dependent fill.

`selectable_at(ui, rect, id_salt, selected, label)` is for controls drawn inside an existing arrangement canvas. It supplies a compact native button in the given rectangle without advancing or enlarging the parent's layout. It requires a stable semantic ID such as `(track.id, "mute")` and retains native hover, focus, keyboard, and accessibility behavior. It does not own arrangement geometry.

`number_at(ui, rect, id_salt, number(&mut value))` places a shared `DragValue` inside a canvas rectangle without advancing the parent layout. Configure its range, speed, precision, prefix, and suffix through the `number` builder. Drag to adjust or double-click to type; use a stable ID such as `(track.id, "gain")`, and commit when the returned response reports `changed()`.

`button_at` places a compact action in a canvas rectangle, truncating long labels without changing the row geometry. `picker_at` uses the same action with an anchored choice popup; it returns the action response and optional popup result. Both require stable IDs and retain native keyboard/focus behavior. Add a tooltip when the full label may be clipped, and use `ui.close()` when a choice is accepted.

Arrangement owns its 248-point header geometry. Audio, MIDI, and bus headers show the same channel number and destination as Mixer (`Ch N > Master` or a bus name). Click this label to choose Master or an eligible output bus; self-routing and feedback through outputs or sends are excluded. MIDI headers also show their assigned instrument (or `Choose instrument…`); clicking it opens the VST chooser for that track and choosing a plugin opens its native editor. Both pickers remain available in compact rows. Rows reuse `selectable_at` for mute/solo and `channel_controls_at` for volume and pan slider rails with a stereo level meter. Expanded rows retain editable dB volume and −100..100 pan readouts; compact rows keep both rails and expose their precise values in tooltips. Meters read each channel's post-insert, post-fader, post-pan engine output, including buses and routed audio. Peaks attack immediately, release at 24 dB/s, and clear when stopped; channel meters do not copy the master level. Folders and automation lanes have no audio channel number, gain/pan rails, or meter; folder mute/solo acts on descendants.

**Track → Add MIDI track** creates a four-beat pattern at the cursor and opens the library in assignment mode. Selecting an instrument assigns that existing track and opens its native editor. Closing the chooser leaves the pattern unassigned. **Add instrument…** starts with the chooser and creates the track and pattern after selection.

New tracks advance through a shared palette of distinct colors, preferring unused colors before reusing one. This applies to menu additions, imported tracks, and instrument or send creation. Existing and manually chosen colors remain unchanged; undo/redo and saved projects retain each assignment.

Click the color strip at the left of an arranger track header to open its color picker. Edits update the shared track color used by its clips and mixer channel; other tracks retain their colors. The strip remains usable on compact tracks and folders, and opening the picker does not change selection or collapse a folder. Color changes support Undo/Redo and project recall.

Arrangement and clip editors share `TimelineViewport` and `TimelinePane`. Call `navigate` before domain hit testing and suppress edits while its `navigating` flag is true. Middle drag or Alt+left drag pans; Ctrl+wheel zooms horizontally around the pointer, Ctrl+Shift+wheel vertically; Ctrl+right or Ctrl+middle drag zooms around the initial pointer position, with Shift selecting the vertical axis. Plain wheel scrolls vertically and Shift+wheel horizontally. Plus/minus zoom horizontally; Shift+plus/minus vertically. `Fit` or F fits the earliest-to-latest event interval and every relevant row on both axes. The shared ruler shows musical bars and elapsed clock time using the same mapping. Text entry, disabled background canvases, and open menus do not consume editor shortcuts.

Tool keys retain the same meaning wherever supported: Q Select, W Pencil, E Cut, R Duplicate, T Glue, Z Erase, U Multi-draw, I Fade in, O Fade out, P Cursor. Each editor exposes only its applicable subset. The arranger retains fixed track headers and a fixed ruler while the musical canvas pans; its native vertical scrollbar remains available.

`clip_title` paints an elided clip name and optional chain icon inside a timeline rectangle, returning the icon/text bounds for diagnostics. It adds no hit target. `clip_label` shows the same link identity in editor headers and Properties. Names remain ordinary Unicode text in the project; the link indicator is a vector icon, not a name prefix. Properties separates Clip, Track, and Channel pages, opening Clip for a valid selected clip and Track otherwise. Start uses one-based beats; duration and source offset remain distances in beats. Only audio clips expose **Clip gain**; MIDI clips have no gain or pan. Track holds naming, color, hierarchy, and display options. Channel holds mixer gain, pan, mute, solo, and latency for audio, MIDI, and bus channels. Folders and automation tracks have no Channel page. Every linked group member displays the shared name and exposes **Make independent**.

## State, focus, and accessibility

`modal(ctx, id, title, contents)` creates a bounded, centered decision dialog using the shared typography, border and shadow. The caller owns the decision and cancellation policy; backdrop clicks never approve an action. Keep decisions short and block background shortcuts and dropped files while they are pending.

New, Open, recovery and Quit check native plugin state before deciding whether the document has unsaved changes. **Save changes?** offers Save, Don't save and Cancel; Escape cancels and retains the current workspace. Canceling the native save picker keeps the decision open. Save waits for the final native snapshot and successful file write before continuing; failures keep the project and show an error in the dialog. Ctrl/Cmd+S, Ctrl/Cmd+Shift+S and Ctrl/Cmd+O remain available from focused utility views.

State stays with the caller. Boolean and choice components mark `Response::changed()` only when their value changes. Use `.clicked()` for actions and `.changed()` for committing edits. Shared components do not create undo history, mutate project state indirectly, or rebuild audio themselves.

Viewing a control must not normalize valid stored values to a narrower editing range. Use `SliderClamping::Edits` or `.clamp_existing_to_range(false)` where the project model allows values beyond the control range. Arrangement, Mixer, Routing, Clip Editor, and Properties preserve those values until an intentional edit.

The original Rill outline icon pack uses a 24-unit vector grid and a consistent stroke weight. Icons render through egui at the current display scale, independently of Unicode symbol fonts. Buttons keep their text, native accessible name, keyboard behavior, disabled state, and tooltip. Shared `IconButton` uses native `Button`/`AtomLayout` to reserve icon space; `IconButton::icon_only` requires an accessible action label. Reorder handles use the vector `Grip`, and native editor actions use `Editor`. Call `Icon::paint` for canvas-owned controls. Add new icons here instead of inserting ad hoc emoji into toolbar labels.

**Edit → History** shows named undo and redo actions. Clicking an entry moves through the intervening steps in one engine update. Ctrl/Cmd+Z and Ctrl/Cmd+Shift+Z work in editing views and utility popups; text fields retain their native text undo. Ctrl/Cmd+Y also redoes. Continuous drags are one step, and text/native parameter bursts on the same control coalesce for up to 600 ms. Navigation, zooming, and tool selection never enter history. Saving establishes a checkpoint; returning to it removes the unsaved marker. A new edit discards redo. History is session-local and limited to 128 steps or an estimated 128 MiB of snapshots, including opaque plugin state. An oversized snapshot becomes a history boundary; allocator overhead and the active project are outside this budget.

Disable a group with `ui.add_enabled_ui(false, |ui| ...)`, or use `ui.add_enabled(enabled, button_widget(...))`. Disabled components do not change values from pointer or keyboard input. Scope repeated rows using `ui.push_id(model_id, ...)`; repeated labels alone are not sufficient persistent identity for every control.

Native buttons, checkboxes, radio choices, text fields, dropdowns, and numeric controls retain egui's keyboard behavior. The custom visual switch exposes a labelled checkbox role and selected state through `WidgetInfo`; its whole label/track area accepts clicks, and Space or Enter activates it when focused. It paints a visible focus ring. Supply a descriptive label, including for icon-only controls, through their response's accessibility/widget metadata where the visual label cannot describe the action.

The library's headless interaction tests exercise pointer activation on a switch label, focus and keyboard activation, disabled input, choice change reporting, Unicode text entry, dropdown selection, compact button/numeric-control layout isolation, and hidden popups. These test behavior without coupling application views to widget internals.

Run the component checks with:

```sh
cargo test -p daw-ui
cargo clippy -p daw-ui --all-targets -- -D warnings
```

`workspace_popup(title, ctx)` remains available for a large utility canvas such as automation, bounded below the transport. Clip Editor and Mixer use the central workspace and `workspace_header` instead. Utilities retain the calling screen as inactive visual context. Menu commands, Back, Escape, and window close use the same navigation state transitions. Every added user-facing feature requires a subprocess E2E test; component interaction tests complement those scenarios.

`curve_editor` receives normalized points and a sampling callback, and returns add/move/delete intents. It does not depend on the project model or audio engine. The application owns interpolation, validation, and undo. Canvas points support click, drag, and deletion; precise numeric controls use the same edit path.

`curve_timeline_editor` adds the shared musical ruler and caller-owned `TimelineViewport`. Automation uses the same middle/Alt-drag pan, Ctrl-wheel/Ctrl-right-drag zoom, plus/minus shortcuts, and Fit/F as the other editors. `TimelinePane::horizontal_only` keeps the complete parameter range visible; `keyboard_focus` allows canvas shortcuts without stealing text input. Fit spans every point, with a full bar for a single-point lane. Visible Beat input is one-based. Navigating cannot create, move, or delete points, and New/Open resets the automation viewport and selection.

Automation's **Add parameter** opens a chooser with the shared parameter browser for VST instruments and effects. Back, Escape and its close button restore the original curve, selected point and zoom. Choosing a parameter that already has a lane offers **Edit automation** instead of failing on duplicate creation; switching lanes does not add undo history. Creating a new lane consumes the chooser and preserves the outer workspace return path. Parameter metadata reads restore the selected instance's saved state and overrides in its sandbox, so a new lane starts from that instance's value rather than a plugin default. Switching between instances invalidates the selection and metadata.

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

File → **Export WAV…** shows the arrangement duration and **Include effect
tails**, initially enabled. Turn it off for a WAV that ends exactly at the last
clip, rounded to the nearest sample. **Export…** then opens the destination
picker; **Cancel**, Escape, and the close button return to the previous view.
The tail choice is retained for the app session and captured with each export
request. Existing command-line exports continue to include tails.

## Device library and effect chains

**Window → Library** opens a large browser with Instruments and Effects tabs. Search matches names, developers, and category metadata; the developer selector narrows results independently. The selected device shows its cached native editor image and one primary action appropriate to the insertion context. **More** holds alternate actions, available metadata, and preview capture/refresh. **Scan** stays visible; **Manage → Folders** opens scanning setup. A tooltip includes the plugin location and class identity. Actions remain above preview content, so a background capture cannot move them under the pointer.

The instrument tab creates a MIDI track and pattern or assigns the selected MIDI track, then opens its native editor. **Effects → + Insert** opens this same browser pinned to that channel. **Add to effect chain** returns to the channel's inserts. **Create send track** creates a bus containing the effect and a post-fader send from the source at 25%; configure the plugin's wet/dry control for the desired return. Routing also offers **Create send track**. Escape returns to the caller without inserting anything.

Plugin discovery stays metadata-only. After discovery, one isolated helper at a time initializes verified instruments and effects and captures native previews into Rill's fingerprinted cache. A selected device is prioritized. Failures are retained and retried only with **Capture preview**; **Refresh preview** explicitly updates a cached image. Existing cache entries display immediately. The browser limits resident small textures to 128 and scales the selected preview to at most 1000×600 pixels.

Effect rows use shared `reorder_handle` and `reorder_drop_target` components. Drag the handle to another row to change processing order; Up/Down remains available for direct activation. Reordering moves the complete slot, preserving its ID, opaque state, normalized overrides, bypass state, and automation targets. The normal graph rebuild recomputes processing order and delay compensation.

Mixer uses equal-width shared strips with aligned names, destinations, faders and pan controls. Click a channel name to open its Channel page or its destination to open Routing. The **Channel** menu also provides **Open channel**, Instrument, Effects, and Routing actions where applicable. Channel, Effects, Instrument, and Routing share navigation links that keep the same channel selected. The links omit the current destination, whose context is already shown in the title; closing a utility returns to its caller. Each channel shows its own post-fader stereo meter beside the fader, and Master shows the stereo output meter. Readings come from the running engine.

Instrument opens a concise device overview with the native **Open editor** action. **Automate…** opens a large parameter browser with automatic loading and focused search. Effect rows expose **Automate…** directly, without opening a native editor first. The last-touched parameter remains a direct shortcut in the overview. Native effect editor wrappers identify both the effect and its channel. Parameter editing and automation retain their isolated plugin state and undo behavior.

`parameter_browser(ui, id, state, items, height, details)` shares the device library's list-and-details layout. Search matches words across names and units, ranks exact/prefix matches first, and handles Unicode. **All**, **Automated**, and **Last touched** narrow the results. Cards show normalized percentages, unit metadata, automation/touch markers, and compact value bars. Up/Down chooses a result; Enter or double-click creates its lane or reopens the existing one. Cmd/Ctrl+F returns focus to search; Clear restores results. An empty result list clears selection, so Enter cannot automate a stale parameter. Browsing does not alter plugin values. The caller supplies detail controls; the list virtualizes large parameter sets and both panes stay bounded and scroll independently.


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

Native file and folder pickers use `daw_ui::with_native_dialog_menu`. On macOS
the helper temporarily adds responder-chain Cut, Copy, Paste and Select All
menu actions, enabling standard shortcuts in native filename/path fields. It
removes its menu on return, cancellation or unwinding so egui retains its own
shortcuts. Other platforms execute the picker directly; injected headless picker
results bypass native UI entirely.
