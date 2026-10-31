# Shared UI components

`daw-ui` is Rill's shared component library. It depends on `egui` 0.33.3 without depending on the application, audio engine, or native window runtime. The app uses it for controls, surfaces, popup windows, and design tokens. New reusable components belong here before they are used in an application view.

## Component gallery

These previews render the actual `daw-ui` components with Rill's theme and fonts.
They use fixed example values for waveforms, meters, device metadata, and the
assistant transcript. Each preview follows your page's light or dark theme;
the links below it open either version at full size.

[Actions and control roles](#actions-and-control-roles) · [Inputs and forms](#inputs-and-forms) · [Navigation and menus](#navigation-and-menus) · [Musical canvas primitives](#musical-canvas-primitives) · [Mixer and recording](#mixer-and-recording) · [Curves and crossfades](#curves-and-crossfades) · [Devices and parameters](#devices-and-parameters) · [Feedback and assistant](#feedback-and-assistant)

### Actions and control roles

Filled commands, selected tools, outlined view controls, icons, and disabled states.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/ui-gallery/actions-dark.png">
  <img alt="Filled action buttons, an active editing tool, outlined view controls, vector icons, and disabled controls." src="assets/ui-gallery/actions-light.png" width="1120">
</picture>

[Light preview](assets/ui-gallery/actions-light.png) · [Dark preview](assets/ui-gallery/actions-dark.png)

[`button`, `primary_button`, `tool_value`, `view_button`](../crates/daw-ui/src/widgets.rs) · [`IconButton`](../crates/daw-ui/src/icons.rs)

### Inputs and forms

Text, precise numbers, continuous sliders, choices, and boolean settings share the same spacing and focus treatment.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/ui-gallery/inputs-dark.png">
  <img alt="Text and masked inputs, numeric fields, sliders, dropdown choices, switches, and aligned form labels." src="assets/ui-gallery/inputs-light.png" width="1120">
</picture>

[Light preview](assets/ui-gallery/inputs-light.png) · [Dark preview](assets/ui-gallery/inputs-dark.png)

[`text_field`, `number`, `slider`, `select`, `checkbox`](../crates/daw-ui/src/widgets.rs) · [`secret_field`](../crates/daw-ui/src/assistant.rs) · [`form_grid`, `path_row`](../crates/daw-ui/src/form.rs)

### Navigation and menus

Workspace headers, grouped toolbars, concise tool help, empty-workspace actions,
related-view links, a properties popup, and an open choice menu.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/ui-gallery/navigation-dark.png">
  <img alt="Workspace navigation, grouped controls, compact tool help, empty-workspace actions, a properties popup, and an open choice menu." src="assets/ui-gallery/navigation-light.png" width="1120">
</picture>

[Light preview](assets/ui-gallery/navigation-light.png) · [Dark preview](assets/ui-gallery/navigation-dark.png)

[`workspace_header`, `toolbar`](../crates/daw-ui/src/layout.rs) · [`view_links`](../crates/daw-ui/src/navigation.rs) · [`choice_menu`](../crates/daw-ui/src/widgets.rs) · [`empty_workspace`](../crates/daw-ui/src/empty_workspace.rs)

### Musical canvas primitives

Musical time, piano-key states, velocities, waveforms, and clip regions use the shared canvas palette.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/ui-gallery/timeline-dark.png">
  <img alt="A musical ruler, piano-key states, velocity markers, and an audio waveform." src="assets/ui-gallery/timeline-light.png" width="1120">
</picture>

[Light preview](assets/ui-gallery/timeline-light.png) · [Dark preview](assets/ui-gallery/timeline-dark.png)

[`paint_timeline_ruler`](../crates/daw-ui/src/timeline.rs) · [`piano_key`, `velocity_marker`](../crates/daw-ui/src/piano.rs) · [`paint_audio_waveform`](../crates/daw-ui/src/waveform.rs)

### Mixer and recording

Channel strips, meters, gain/pan controls, and recording controls with fixed demonstration levels.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/ui-gallery/mixer-dark.png">
  <img alt="Mixer strips with stereo meters, gain and pan controls, recording buttons, and input metering." src="assets/ui-gallery/mixer-light.png" width="1120">
</picture>

[Light preview](assets/ui-gallery/mixer-light.png) · [Dark preview](assets/ui-gallery/mixer-dark.png)

[`mixer_strip`, `stereo_meter`](../crates/daw-ui/src/layout.rs) · [`channel_controls_at`](../crates/daw-ui/src/channel_controls.rs) · [`record_button`, `input_meter`](../crates/daw-ui/src/recording.rs)

### Curves and crossfades

Breakpoint editing and complementary crossfade curves keep their data and evaluation callbacks with the caller.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/ui-gallery/curves-dark.png">
  <img alt="An editable breakpoint curve and the shared crossfade editor with complementary fade curves." src="assets/ui-gallery/curves-light.png" width="1120">
</picture>

[Light preview](assets/ui-gallery/curves-light.png) · [Dark preview](assets/ui-gallery/curves-dark.png)

[`curve_editor`](../crates/daw-ui/src/curve.rs) · [`crossfade_editor`](../crates/daw-ui/src/crossfade.rs)

### Devices and parameters

Selectable device cards and a searchable parameter browser with example catalogue data.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/ui-gallery/devices-dark.png">
  <img alt="Device cards with selected states and a parameter browser showing example values and automation indicators." src="assets/ui-gallery/devices-light.png" width="1120">
</picture>

[Light preview](assets/ui-gallery/devices-light.png) · [Dark preview](assets/ui-gallery/devices-dark.png)

[`device_card`](../crates/daw-ui/src/device_cards.rs) · [`parameter_browser`](../crates/daw-ui/src/parameter_browser.rs)

### Feedback and assistant

Progress and result notices sit alongside the bounded assistant transcript and prompt composer.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/ui-gallery/feedback-dark.png">
  <img alt="Progress, success, warning, and error notices alongside the Assistant component with a sample conversation." src="assets/ui-gallery/feedback-light.png" width="1120">
</picture>

[Light preview](assets/ui-gallery/feedback-light.png) · [Dark preview](assets/ui-gallery/feedback-dark.png)

[`notice`](../crates/daw-ui/src/notice.rs) · [`assistant`](../crates/daw-ui/src/assistant.rs)

### Regenerate the previews

Run the interactive component example to explore the same widgets locally:

```sh
cargo run --locked -p daw-ui --example component_gallery
```

Capture every gallery page in both themes:

```sh
cargo run --locked -p daw-ui --example component_gallery -- --capture docs/assets/ui-gallery
```

Capturing requires a desktop display. On Linux, a private Xvfb display can render
the previews with software OpenGL:

```sh
LIBGL_ALWAYS_SOFTWARE=1 xvfb-run -a cargo run --locked -p daw-ui --example component_gallery -- --capture docs/assets/ui-gallery
```

The example renders through a development-only `eframe` host; the shared library
retains its runtime independence from application state, audio, and windowing.
After changing a component or its theme, regenerate the previews and inspect both
variants. The API reference and interaction rules continue below.

## Workspace conventions

Arrangement, Clip Editor, and Mixer each own the central workspace. Clip Editor and Mixer have a Back action; returning restores the prior selection and viewport. Instrument, Effects, Routing, Recording, Automation, Library, Setup, and Properties open on demand above their caller. The previous workspace and any parent popup remain visible as softly blurred, dimmed context; only the foreground utility accepts editing input. Menus, dropdowns, and color pickers dim their caller too, while tooltips remain unobtrusive. Transport and the File, Edit, Track, and Window menus remain available. Persistent **Arranger** and **Mixer** choices in the transport switch workspaces directly, including while a utility is open. Window titles identify the current channel or clip, such as **Effects of Bass**. Components keep labels short; explanations belong in tooltips or the relevant utility rather than permanent interface text.

`empty_workspace(ui, title, description, actions)` offers the next step when a
workspace has no content. `EmptyWorkspaceAction` supplies a stable `id`, concise
`label`, hover `description`, and optional `primary` emphasis. The first primary
action appears above the secondary choices; those share a row when space allows
and stack on narrow canvases. The component uses at most 560 logical points,
limits heading and description height, and displays at most four actions.
`EmptyWorkspaceResponse` returns the chosen action ID, its occupied rectangle,
and the real button responses. The caller decides where to place it and routes
commands through the normal navigation or edit service. It does not intercept
the surrounding canvas or change focus, selection, or song state while drawing.

## Theme and tokens

Call `daw_ui::theme::apply(ctx)` when creating the application. It configures both native egui palettes. `theme::set_appearance(ctx, Appearance)` selects System, Light, or Dark; System is the default and follows operating-system changes. Read `theme::palette(ctx)` for the current canvas colors. Logical dimensions and typography come from `daw_ui::tokens`:

| Token family | Values |
| --- | --- |
| Spacing | `SPACE_XS` 4, `SPACE_SM` 8, `SPACE_MD` 12, `SPACE_LG` 16, `SPACE_XL` 24 |
| Controls | `CONTROL_HEIGHT` 34, `COMPACT_HEIGHT` 24, `SELECT_WIDTH` 160 |
| Corners | `RADIUS` 5, `SURFACE_RADIUS` 8 |
| Typography | Caption 13, note 14, button/body 15, subtitle 16, title 20; canvas 12, track title 14, dense scale 10 |

Sizes are logical screen points and follow egui's display scaling. Surfaces use neutral graphite in Dark and cool porcelain in Light, with a muted sage accent and a restrained luminance hierarchy. Action buttons have a quiet fill; inputs retain visible boundaries. Accent color identifies active choices and keyboard focus. Cards and controls stay flat; utility windows use a clearer border and a soft shadow to distinguish them from the workspace. Sections use spacing rather than nested outlines. Body and button text remain 15 points, with 34-point action targets. Avoid introducing a second app-specific palette or copied spacing scale.

Choose controls by what they change:

| Role | Behavior | Shared treatment |
| --- | --- | --- |
| Action | Execute a command on content, such as Duplicate or Generate | Filled `button` or `action_menu`; no persistent active state |
| Tool | Change how the next canvas interaction behaves, such as Pencil or Cut | Exclusive `tool_value`; active tint and persistent underline, radio selection semantics |
| View | Change representation or framing, such as highlighting, Show all, or Fit | Outlined `view_button` or eye-marked `view_menu`; no document or tool change |
| Parameter | Set a stored value, such as gain, duration, or interpolation | Labelled numeric field, slider, checkbox, or current-value choice |

Keep related labels, values, units, and choices together with the shared wrapping
toolbar groups or form grid. Use a numeric field when exact entry matters, a
slider for continuous adjustment, a checkbox for an independent boolean, and a
current-value chooser for an exclusive setting. Make precision and drag steps
fit the parameter: clip gain displays tenths of a dB; fade duration accepts
thousandths of a beat. Viewing a value must retain its stored precision.

## Components

Standard controls are available at the crate root and through `daw_ui::widgets`. The curve editor is exported at the crate root.

| Purpose | API |
| --- | --- |
| Standard actions | `button`, `primary_button`, `small_button` |
| Presentation commands | `view_button`, `view_button_widget` |
| Exclusive interaction modes | `tool_value`, `IconButton::tool` |
| Search-result choice without native menu navigation | `picker_button` |
| Button builder for sized/disabled placement | `button_widget` |
| Application and contextual menus | `menu_bar`, `menu_button` |
| Toolbar command, choice, and presentation menus | `action_menu`, `choice_menu`, `view_menu` |
| Isolated plugin context menus | `NativeContextMenu`, `NativeMenuItem`, `native_context_menu_ui`, `show_native_context_menu` (macOS/Windows) |
| Focused workspace structure | `workspace_header`, `toolbar`, `toolbar_group`, `toolbar_group_sized`, `action_menu`, `action_row`, `context_section`, `bounded_columns`, `bounded_panes`, `scroll_body` |
| Empty workspace next steps | `empty_workspace`, `EmptyWorkspaceAction`, `EmptyWorkspaceResponse` |
| Mixer channels and metering | `mixer_strip`, `mixer_channel_button`, `stereo_meter`, `channel_controls_at`, `ChannelControlsResponse` |
| Track and channel ordering | `track_reorder_source`, `track_reorder_grip_at`, `track_reorder_target`, `track_reorder_target_with_after_edge`, `track_reorder_preview_label`, `track_reorder_scroll_delta`, `TrackDropIntent` |
| Mixer identity, inserts and routing | `mixer_master_title`, `mixer_effects_button`, `mixer_insert_summary`, `mixer_bus_input_summary`, `paint_mixer_fader_scale`, `mixer_output_button`, `mixer_sends_button`, `MixerSend` |
| Insert-chain structure | `effect_chain_header`, `effect_slot`, `empty_effect_chain` |
| Audio recording and raw input level | `record_button`, `record_arm_at`, `input_meter`, `paint_recording_region`, `recording_color` |
| Boolean button | `toggle` |
| Boolean setting | `switch`, `checkbox` |
| Exclusive choices | `radio_value`, `selectable_value` |
| Selected navigation/item | `selectable` |
| Related-view navigation and inactive context | `view_links`, `ViewLink`, `ViewBackdrop`, `paint_popup_backdrop` |
| Native pane middle-drag navigation | `scroll_area`, `scroll_area_is_panning` |
| Arrangement overlay controls | `button_at`, `picker_at`, `selectable_at`, `number_at` |
| Text input | `text_field` |
| Assistant workspace, examples, tools and tips | `assistant`, `AssistantState`, `AssistantPage`, `AssistantOptions`, `AssistantExample`, `AssistantTool`, `AssistantTip`, `AssistantMessage`, `AssistantResponse`, `contextual_ai_button` |
| Masked credential input | `secret_field` |
| Aligned settings and paths | `form_grid`, `path_row` |
| Ghost clip identity | `clip_title`, `ClipTitleLayout`, `clip_label`, `clip_choice`, `Icon::Link`, `Icon::Unlink` |
| Arrangement clip surfaces and previews | `clip_surface`, `ClipSurfaceLayout`, `clip_note_preview` |
| Dropdown choices | `select`, `select_labeled` |
| Numeric widget builders | `slider`, `Slider`, `SliderScaleGeometry`, `slider_scale_geometry`, `number`, `Number` |
| Rotary parameter controls | `knob`, `Knob`, `KnobGeometry`, `knob_geometry` |
| Track color | `color_edit`, `color_strip_at` |
| Timeline navigation and tools | `TimelineViewport`, `TimelineBounds`, `TimelinePane`, `EditorTool`, `editor_tools`, `compact_editor_tools`, `editor_tool_shortcuts`, `timeline_fit_button`, `paint_timeline_ruler` |
| Piano keys, notes, and velocity | `PianoRows`, `piano_key`, `piano_velocity_marker`; `Palette::piano_row`, `piano_note_fill`, `piano_note_stroke` |
| MIDI drawing duration and boundary | `DrawNoteLength`, `drawing_note_length`, `paint_clip_end_label` |
| Interactive parameter curves | `curve_editor`, `curve_timeline_editor`, `CurvePoint`, `CurveView`, `CurveTimeline`, `CurveEdit` |
| Automation state and segment shape | `AutomationState`, `AutomationControl`, `automation_badge`, `paint_automation_control`, `automatable_parameter_context`, `automatable_menu_contents`, `automation_parameter_context`, `automation_channel_menu`, `curve_timeline_editor_with_segments`, `CurveSegment`, `curve_segment_controls`, `curve_toolbar`, `curve_endpoint_summary`, `paint_curve_override` |
| Searchable plugin parameters | `parameter_browser`, `ParameterItem`, `ParameterBrowserState`, `ParameterScope` |
| Non-modal operation feedback | `notice`, `NoticeKind` |
| On-demand error diagnostics | `error_details`, `ErrorDetailsResponse`, `text::selectable_block` |
| Complementary audio crossfades | `crossfade_editor`, `CrossfadeCurve`, `CrossfadeResponse`, `paint_crossfade` |
| Plug-in library cards, rows and previews | `device_card`, `device_grid`, `device_grid_layout`, `device_row`, `device_list`, `device_list_layout`, `device_preview`, `favorite_toggle`, `image_preview` |
| Library navigation and active filters | `count_item`, `filter_chip`, `Icon::Star`, `Icon::StarFilled` |
| Bounded context titles | `text::bounded_block` |
| Inactive clip regions | `paint_clip_tail` |
| Surfaces | `panel`, `card`, `inset`, `soft_surface`, `paint_ambient`, `paint_selection_glow` |
| Toolbar icons | `Icon`, `IconButton`; ordinary button builders resolve known action icons |
| On-demand windows | `popup`, `workspace_popup` |
| Compact section label | `theme::label` |

`hover_text(response, text)` adds a delayed, read-only tooltip using the shared
native popup styling. Repeated hints for one response stack without intercepting
pointer input: even a mouse move and press in the same frame reaches the control
beneath a disappearing hint. Sliders and automation hints use this helper. Keep
native `on_hover_ui` for tooltips that intentionally contain interactive content.

Standard controls use comfortable horizontal/vertical padding; canvas-owned controls retain their compact geometry. Popups use 24-point horizontal and eight-point vertical inner margins and scroll within the available viewport. The shorter native title row keeps the complete close target clear of the titlebar's click region. Hover and keyboard focus remain visible. Utility windows have a prominent but restrained 1.5-point border and a broad, soft shadow; menus use a smaller shadow. Panels and controls remain flat.

`slider` retains native dragging, keyboard focus and exact numeric entry, with
minimum/maximum captions and a center line on its handle. Scale positions use
the handle's actual travel, so a tick and the handle center identify the same
value. Short rails show halves; at 100 points of travel they add fifths, and at
200 points they show tenths. The midpoint remains visible at every density.
Ticks are paint-only and never quantize values beyond the caller's configured
precision. Secondary and middle clicks do not change parameters.

Use `.compact()` for canvas-owned rows, `.endpoint_labels(...)` for units such
as L/R, and `.current_label(...)` for a concise readout inside the caption row.
The whole rail and caption remain clickable. `.show_value(false)` is for
controls with a current caption and a separate field or exact-value context editor. Only suppress
the built-in endpoints with `.scale_labels(false)` when providing a calibrated
external scale, as Mixer does for dB faders. `slider_scale_geometry` exposes the
same native rail and handle positions for such scales and diagnostics.

`value_entry(ui, id, current, range, unit)` provides a deliberate exact editor
inside a parameter menu. It displays the current value and range, selects the
initial text, and returns a changed value only on **Apply value**. Invalid or
non-finite text keeps the editor open; **Cancel**, Escape, and outside dismissal
leave the value untouched. Use `automatable_parameter_value_context` for channel
controls so exact entry and Create/Edit/Override/Resume automation share one
menu. Such menus close only on outside clicks or explicit actions, allowing
normal text selection and correction. `value_entry_rect` exposes the active
field for app interaction diagnostics.

Endpoint formatting retains small ranges instead of rounding both ends to zero.
Stored values outside an editing range are preserved, visibly marked as outside
the range, and reported exactly in the tooltip and context editor. A slider's
automation glyph sits beside its handle: **A** for Read, **!** for Overridden,
and a muted dash for Bypassed, without covering the center index or captions.

`knob` / `Knob::new` provides a calibrated 270-degree rotary control with
minimum/maximum captions, a visible value index and an always-present shared
numeric editor. Dragging is relative: up or right increases, down or left
decreases, and pressing a different part of the dial never jumps the value.
Shift makes a drag ten times finer; the numeric field provides the same precision
without a modifier. Click the number to type and click elsewhere to commit.
`step_by` quantizes intentional edits, while `.clamping(SliderClamping::Edits)`
(the default) preserves existing values when rendering. Values outside the
editing range remain readable and show **Outside range**. The arc uses halves,
fifths or tenths according to diameter, always retaining its midpoint.

Use `.text`, `.suffix`, `.endpoint_labels`, and decimal builders to state the
parameter's actual units. `.automation(Some(state))` colors the arc and displays
**Automated**, **Overridden**, or **Bypassed**. A caller that already displays the
same state beside its automation actions can use `.show_automation_label(false)`
to avoid duplicate wording while keeping the colored arc. `knob_geometry` exposes the actual
dial, numeric input, arc endpoints and value index for host diagnostics. The
component caches its arc directions and paints bounded geometry without timers.
Native plug-in values are normalized; a 0–100% knob must be labelled as such
unless the host supplies an actual physical-unit conversion.

`assistant(ui, prompt, state, options)` provides a bounded, selectable conversation,
captured-context label, optional progress, multiline **AI prompt**, and fixed
**Send**, **Stop**, and **Undo** controls. Enter adds a newline; Ctrl/Cmd+Enter
submits only while the prompt owns focus. The application owns execution,
conversation retention, cancellation, permission checks, and undo; closing its
window does not imply cancellation. Returned responses expose interaction bounds
for app-level tests. Prompt input is capped at 16,384 characters; rendering uses
the latest 64 messages and at most 8,192 characters per message so provider output
cannot cause unbounded transcript layout. The composer scrolls independently,
keeping actions visible even for a long draft. `secret_field` masks a credential
using ordinary shared input spacing; callers still protect its backing value and
must never include it in diagnostic or conversation text.

`paint_ambient` adds a broad, faint radial tint to quiet utility backdrops. `soft_surface` places that same tint between a native frame and its contents, preserving layout and input behavior. Workspace headers and selected mixer channels use this treatment. Music canvases retain flat fills; selected clips and channels add a soft track-colored outline glow. The tint uses one bounded mesh (at most 425 vertices), transparent edges, and no textures, blur, animation, or extra repaint requests. Use it on large surfaces rather than individual controls.

`paint_selection_glow` adds a six-point track-colored halo using one fixed mesh (128 vertices, 576 indices). It respects the parent clip and culls hidden items. Clip and mixer selection use this shared treatment without altering layout, input, or playback.

`view_links` presents a wrapping row of related destinations with the active one marked. The caller supplies destinations and handles the returned selection. `ViewBackdrop` retains the caller's paint for a utility background without creating widgets, hit targets, or keyboard focus; cached context cannot edit the project or duplicate active accessibility controls. Nested utilities keep their parent context and restore the prior selection and inspector page when dismissed.

For a modal opened above a menu or another modal, `ViewBackdrop::capture_dialog_context` also retains foreground content. Call it before drawing the new dialog. It omits existing shared popup and modal scrims so the new modal dims the complete retained caller once; replay still creates no controls or accessibility targets.

Call `paint_popup_backdrop(ctx, utility_open)` once after drawing the workspace and utilities. It paints one theme-aware dim layer below the active window or native menu/picker, including submenus. Retained backgrounds exclude this layer so nested navigation cannot compound the dimming. The shared `modal` uses the same palette color through its native input-blocking backdrop. Confirmation dialogs retain the previous view. The native app enables shared `glow-backdrop` rendering: two low-resolution Gaussian passes soften the inactive context before the dim rectangle. Each of two GPU targets is capped at 512 × 512 pixels (2 MiB combined). Frozen utility backgrounds reuse the blurred result; menus refresh only during existing redraws. There is no CPU pixel readback, animation, extra hit target, or repaint request. Unsupported graphics contexts retain dimming. Call `enable_backdrop_blur` after renderer creation and `destroy_backdrop_blur` while its context is current at shutdown.

`scroll_body` inside a window with `.vscroll(false)` lets short utility content fit naturally while long chains remain bounded and scrollable. Use a stable height for tabbed forms and reserve footer space before sizing inner lists.

`notice` keeps readable feedback and trailing actions within one bounded row. It supports progress, success, warning, and error states; long text truncates with a full-message tooltip. Export completion stays visible until dismissed, shows the resulting filename, and offers **Copy path**. Errors retain priority; ongoing work and later export completions remain visible alongside a retained failure. A new export clears the previous result. Notifications never add project history or block editing.

The project header uses the saved filename stem, with the full path on hover and a dirty marker after edits. Unsaved projects retain their musical project name. Save and WAV/MIDI destination pickers reuse the saved directory and filename stem. New projects also remember the last successful project-save directory across restarts, ignoring deleted folders; extensionless project saves receive `.rill`, while explicit `.rill` and legacy `.json` extensions are accepted case-insensitively. MIDI export uses the same persistent completion notice as WAV.

Errors offer **Details** from the notification, plugin view, or unsaved-project dialog.
Audio-device errors also offer **Audio setup…** in the notification, restoring the caller when Setup closes.
The diagnostic view shows the full available cause chain and contextual plugin,
channel, application, and audio configuration data. **Copy details** copies the
report exactly; **Close** or Escape returns to the unchanged caller. The report is
a snapshot, so background completion cannot replace it while it is being read.
Underlying editor shortcuts and file drops are blocked; playback continues.

`error_details(ctx, title, summary, details)` shows a diagnostic snapshot in the shared modal above the current workspace. Contextual titles stay within one row with their full text on hover; put operation, device/path and cause information in `details`. The summary is bounded, while the complete selectable diagnostic text wraps inside a scrollable inset. **Copy details** copies every supplied line, including offscreen content; **Close**, Escape or the backdrop dismisses. The footer stays outside the scrolling area. The application owns opening, snapshot lifetime and raw keyboard/drop gating, as for other modals.

Long names and paths must stay within their window. `action_row` reserves trailing controls before flexible content; `path_row` adds path truncation and a full-path tooltip. `form_grid` aligns label/control columns; associate a separate label with its control using `.labelled_by(label_response.id)`. `bounded_columns` gives each library pane a fixed viewport so catalog and preview scrolling leave search and primary actions visible. `bounded_panes` places a flexible center pane between optional fixed-width side panes; a zero width hides a side, so a narrow window can drop a detail pane while keeping the catalog usable. `text::bounded_block` wraps within a row limit and exposes the full source on hover. Task names belong in native window titles; long track and device names belong in bounded content headings. `clip_choice` includes visible instance metadata to distinguish ghost copies with shared names.

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

When actions must stay visible, use `.vscroll(false)` and `popup_scroll_body(ui, id, reserved_footer_height, contents)`. Setup keeps Apply/Cancel below this scrolling body; Recording keeps timing and Record controls below its input choices. Reserve spacing as well as the footer controls so narrow windows remain usable. `popup_body_size(ctx)` supplies the available utility content bounds when a view needs an explicit minimum height. Routing uses this bound to keep a useful send viewport below its navigation and AI actions as display density changes; long send lists and output choices scroll together.

`workspace_header(ui, title, actions)` returns `WorkspaceHeaderResponse { back, inner, response }`; handle `back.clicked()` in the application's navigation logic. The action closure is laid out from right to left. The title truncates before trailing controls, and the header occupies one bounded row. `toolbar(ui, ...)` wraps whole native controls when needed; both helpers include an eight-point inset. `toolbar_group` keeps related controls together. `context_section` groups a quiet label and controls without another panel outline.

`toolbar_group_sized(ui, width, contents)` reserves a known width before drawing
related controls from left to right. It stays on the current row when space
permits and wraps as a complete group otherwise. Use it for fixed-width choice
groups such as MIDI multi-draw's two selectors; keep `toolbar_trailing` for
existing right-aligned controls.

`editor_tools` uses shared `tool_group` surfaces to keep selection/drawing,
cut/join/erase, copy, and timing tools together. Groups wrap as whole units with
every applicable tool still visible. Toolbars have a quiet panel fill and bottom
divider; active tools retain their underline and keyboard semantics.

Use distinct control roles consistently. Direct actions use filled buttons. `action_menu` keeps that filled treatment and adds a disclosure chevron for commands available on demand. `choice_menu` is a bordered, inset field labelled with its current choice; ordinary single choices use `select`. `view_menu` is a quiet outlined eye button for presentation options such as scale highlighting and row visibility. Keep view controls grouped separately from editing actions. Both settings menus remain open while their controls change; Escape or an outside click dismisses them, and `ui.close()` accepts a choice. Use nested `choice_menu` or `action_menu` for submenus rather than a ComboBox inside a menu, because those share egui's popup state. All three roles retain native focus, accessible labels, disabled behavior, and whole-control toolbar wrapping; they never own project history.

The main bar groups workspace views, transport commands, musical position/tempo,
and playback guides separately. Arranger/Mixer and Click/Loop use quiet outlined
choices; transport commands retain their filled treatment. Guide controls retain
their playback and project semantics. Reserve each group's measured width before
painting it so narrow windows wrap the complete group without clipping controls.

`mixer_strip(ui, id, size, color, selected, contents)` creates a vertical channel with a stable ID, optional thin track-color marker, and a subtle divider. Size includes its margins; callers choose a common width and height. Mixer strips fill the available workspace height, and faders and meters stretch with the window. Names and footer controls retain their natural size; short windows scroll when the faders reach their usable minimum. `stereo_meter(ui, [left, right], size)` displays −60 to 0 dBFS peaks, marks clipping with the warning color, and sanitizes non-finite input. It does not own audio state or smoothing. Catalog rows reserve the same preview space while thumbnails are loading so names and actions remain aligned.

`device_row` is the list counterpart of `device_card`: one whole-row target with a small preview, name, producer and an optional trailing detail such as categories, sharing the card's double-click activation, focus and selected state. `device_list` virtualizes rows with the same `scroll_to` contract as `device_grid`; `device_list_layout` gives keyboard steps a single column. Cards and rows mark favorites with a star badge that adds no hit target; `favorite_toggle` changes the mark and reports checkbox semantics. `device_preview` shows a selected device's preview at a fixed size. `count_item` is a full-width selectable navigation row with a trailing count, for collections and producers. `filter_chip` shows an active filter and removes it when clicked; pair it with a **Clear filters** action.

**Setup → Appearance** applies and persists System, Light, or Dark. Cancel discards the draft. Changing only appearance leaves playback and native editors running. Piano-roll black-key rows remain slightly darker than natural-key rows in both modes. Scale and chord guides use subtle green tints. `Palette::piano_note_fill(velocity, out_of_scale)` keeps notes blue, or red for an optional scale warning, with velocity-dependent strength. `piano_note_stroke_in_rect(selected, fill, rect)` adds a neutral contrasting selection outline without changing the fill, with thinner strokes at dense zoom levels. `piano_velocity_marker` shares these colors with the velocity lane. Note labels select a contrasting foreground for their fill.

`selectable_at(ui, rect, id_salt, selected, label)` is for controls drawn inside an existing arrangement canvas. It supplies a compact native button in the given rectangle without advancing or enlarging the parent's layout. It requires a stable semantic ID such as `(track.id, "mute")` and retains native hover, focus, keyboard, and accessibility behavior. It does not own arrangement geometry.

`number_at(ui, rect, id_salt, number(&mut value))` places the shared `Number` widget inside a canvas rectangle without advancing the parent layout. Its builder preserves native `DragValue` range, speed, precision, prefix, suffix and deferred-commit behavior. Drag to adjust; enter text mode and double/triple-click to select the entire numeric value, including its sign and decimal portion. Ordinary single-click caret editing stays native. Use a stable ID such as `(track.id, "gain")`, and commit when the returned response reports `changed()`.

Expanded track headers show concise gain and pan readouts beside their sliders,
leaving room for the meter. Clicking a readout opens the shared exact-value
editor with the precise current value. Enter or **Apply value** commits an edit;
Cancel, Escape or clicking outside leaves the channel unchanged. The sliders
retain direct dragging, and right-click still opens parameter automation actions.

`button_at` places a compact action in a canvas rectangle, truncating long labels without changing the row geometry. `picker_at` uses the same action with an anchored choice popup; it returns the action response and optional popup result. Both require stable IDs and retain native keyboard/focus behavior. Add a tooltip when the full label may be clipped, and use `ui.close()` when a choice is accepted.

Arrangement owns its 248-point header geometry. Audio, MIDI, and bus headers show the same channel number and destination as Mixer (`Ch N > Master` or a bus name). Click this label to choose Master or an eligible output bus; self-routing and feedback through outputs or sends are excluded. MIDI headers also show their assigned instrument (or `Choose instrument…`); clicking it opens the VST chooser bound to that track. Assigning returns to the composition caller; open **Window → Instrument** explicitly to edit its sound. Both pickers remain available in compact rows. Rows reuse `selectable_at` for mute/solo and `channel_controls_at` for volume and pan slider rails with a stereo level meter. Expanded rows provide dB volume and −100..100 pan readouts with exact entry on click; compact rows keep both rails and expose their precise values in tooltips. Meters read each channel's post-insert, post-fader, post-pan engine output, including buses and routed audio. Peaks attack immediately, release at 24 dB/s, and clear when stopped; channel meters do not copy the master level. Folders and automation lanes have no audio channel number, gain/pan rails, or meter; folder mute/solo acts on descendants.

**Track → Add MIDI track** creates a four-beat pattern at the cursor and opens the library in assignment mode. The chooser identifies its target by name. Selecting an instrument assigns that existing track and returns to its caller without opening another panel; **Window → Instrument** remains available explicitly. Closing the chooser leaves the pattern unassigned. **Add instrument…** starts with the chooser and creates the track and pattern after selection.

Arrangement **Add pattern** creates a four-beat pattern at the cursor on the selected MIDI track, or creates a MIDI track when necessary. Empty MIDI lanes show **Double-click to add a pattern** and expose the same action in their context menu. An unassigned MIDI clip's Clip Editor header offers **Choose instrument…** without changing its notes.

New tracks advance through a shared palette of distinct colors, preferring unused colors before reusing one. This applies to menu additions, imported tracks, and instrument or send creation. Existing and manually chosen colors remain unchanged; undo/redo and saved projects retain each assignment.

Click the color strip at the left of an arranger track header to open its color picker. Edits update the shared track color used by its clips and mixer channel; other tracks retain their colors. Mixer channel names use the exact track color with contrasting text, and the full strip carries a quiet tint of that color in both themes. Master stays neutral. The strip remains usable on compact tracks and folders, and opening the picker does not change selection or collapse a folder. Color changes support Undo/Redo and project recall.

Arrangement and clip editors share `TimelineViewport` and `TimelinePane`. Call `navigate` before domain hit testing and suppress edits while its `navigating` flag is true. Middle drag or Alt+left drag pans; Ctrl+wheel zooms horizontally around the pointer, Ctrl+Shift+wheel vertically; Ctrl+right or Ctrl+middle drag zooms around the initial pointer position, with Shift selecting the vertical axis. Plain wheel scrolls vertically and Shift+wheel horizontally. Plus/minus zoom horizontally; Shift+plus/minus vertically. `Fit` or F fits the earliest-to-latest event interval and every relevant row on both axes; Arrangement caps fitted row height at 96 points so sparse projects retain ordinary track proportions. The shared ruler shows musical bars and elapsed clock time using the same mapping. Arrangement reserves a separate loop-indicator strip below the two label rows with eight points of clearance. Text entry, disabled background canvases, and open menus do not consume editor shortcuts.

The arranger reserves a separate loop strip below bar/time labels. Drag the label
gutter to resize all track headers, or a header's lower edge to resize every track
row together. View → Track layout provides precise dimensions. These dimensions
are session view state and leave content, selection and history intact. Folder
headings use disclosure controls, folder icons, depth guides and spacing; Tracks
and row context menus offer Add subfolder and Move to folder. Nesting choices
exclude a folder's descendants, with validation in the existing model.

Drag a track's title or visible grip to reorder it, including folders, buses and
automation lanes. Header halves show Before/After insertion lines; folder headers
have an explicit middle **Into folder** zone. A folder carries its entire subtree,
and After points to the end of that subtree. Before/After makes the source a
sibling of its destination; Into folder appends it as a child. Invalid descendant
targets are rejected. Hover cues identify the destination and folder, and edge
dragging scrolls long sessions. Escape or releasing outside a target cancels.
Only a valid drop commits one Undo step; previews leave the document unchanged.

Arrangement and Mixer share the project's complete hierarchy order. Collapsed
folders keep their children in that order while hiding their arranger rows.
Reordering preserves track/clip IDs, selection, routing, insert state and
automation. Changes in order alone preserve playback and loaded plug-ins;
moving into another folder retains the usual folder mute/solo behavior.

Each folder also encloses its header and descendant lanes in its own colored
border. Nested containers add a small inset and separate closing edges, with
matching left padding for child headers. Every track retains the same beat-grid
origin and scale. Scrolling clips the real edges rather than drawing false caps
at viewport boundaries; a collapsed folder encloses just its visible header.

The foreground workspace opts in with `TimelinePane::keyboard_active()` so Fit
and zoom keys also work while the pointer remains over its toolbar. Enable it
for only the active editor; reusable panes default to hover/click focus.
`CurveTimeline.keyboard_active` makes the same choice for automation. The ruler
adds bar.beat labels at 80 points per beat, half-beat divisions at 160, and
sixteenth divisions at 320. Labels retain bar and time context, avoid collisions,
and stay within the visible ruler.

Navigation must preserve the selected content and project history. Horizontal-only
parameter curves retain their complete vertical value range and pass ordinary
vertical wheel scrolling to the containing popup or list. Shift-wheel still pans
time; modifier zoom belongs to the canvas and must not also scroll its parent.
Library and parameter-result lists use unmodified arrows and Enter for selection
and activation; modified keys remain available for text editing and other
shortcuts.

Mixer reserves middle-drag for native pane scrolling, including gestures that
start over gain or pan. `scroll_area` claims that gesture before descendants;
callers use `scroll_area_is_panning` to suppress value publication through the
release frame. It retains native wheel and scrollbar behavior. This helper does
not reserve Alt-click or block arbitrary descendant actions; musical canvases
continue to use `TimelinePane` for their full navigation model.

Mixer, Channel Properties, and expanded track headers reuse `channel_gain_number` for precise volume
entry. Double-click the dB value to type, or drag in 0.1 dB steps. The editing
range is −60 dB (silence) through the channel maximum; quieter stored positive
gains remain intact until edited. Readouts retain up to six decimal places as
needed. Compact headers and Mixer pan expose exact entry through their context
menus; pan entry uses −100..100 percent consistently, with zero at center.
Master peak is a readout rather than a gain
field.

Mixer follows signal order from top to bottom: source/channel settings, Effects
and insert summary, volume and pan, Mute/Solo, then **Target** and auxiliary sends.
The output control carries the destination's color. A separate send action names
a single destination or shows the send count; hover lists every destination,
pre/post position and level. **Add send…** opens Routing for an empty send list.
Selecting or hovering a channel previews its output and
pre/post sends in separate routing lanes below the controls. `mixer_route_lane` connects actual `MixerRouteEndpoint` positions with a continuous three-point-wide U-shaped ribbon, including the arrowhead touching the destination. Vertex colors blend smoothly from source to destination along the path; Master uses the theme accent. Each route uses eleven vertices, with no textures or repaint requests. Offscreen endpoints retain their true coordinates and remain named. **+ Bus** creates a bus, and Routing can add several sends or create
a bus and send together. Output and send choices exclude feedback and duplicate
sends; the model validates routing changes too.

Mixer separates contiguous Tracks and Buses with group headings and spacing,
followed by Master. Drag a channel name to place it before or after another
channel, including across track/bus boundaries. Bus headers share a routing icon,
BUS badge and quiet tint with the arranger. Grouping follows the saved order;
channel numbers match the arranger. Master remains the terminal output readout.
Narrow source
rows use `compact_action_menu`, which retains the normal action target height
while removing the optional leading icon and reducing horizontal padding.

Channel strips reserve four points of canvas between their bordered surfaces;
their allocated width and control alignment remain unchanged. Selection uses a
clear border, while track colors stay strongest in names and top markers. Bus
input counts expose their named output/send sources on hover. Effects show an
insert-count badge and bounded first-name summary; the full ordered chain and
bypass states appear on hover. Fader marks show unity, −6, −12 and silence using
the native fader's existing gain law. Master has a neutral identity header and
aligned meters. These readouts add no timers or animated paint.

Effect slots use the shared `effect_slot` card: bounded identity and enable switch,
Active/Bypassed status, then a separate wrapping action row. The chain overview
shows processing order; **+ Insert** stays outside the scrolling body. Utilities
derive their top limit from the actual transport height, including wrapped rows.
Library card previews shrink in short windows while names, producer labels and
the original image aspect ratio remain intact.

Tool keys retain the same meaning wherever supported: Q Select, W Pencil, E Cut, R Duplicate, T Glue, Z Erase, U Multi-draw, I Fade in, O Fade out, P Cursor. Each editor exposes only its applicable subset. The arranger retains fixed track headers and a fixed ruler while the musical canvas pans; its native vertical scrollbar remains available.

`clip_title` paints an elided clip name and optional chain icon inside a timeline rectangle, returning the icon/text bounds for diagnostics. It adds no hit target. `clip_label` shows the same link identity in editor headers and Properties. Names remain ordinary Unicode text in the project; the link indicator is a vector icon, not a name prefix. Properties separates Clip, Track, and Channel pages, opening Clip for a valid selected clip and Track otherwise. **Start** and **Duration** are editable through the ordinary move/trim paths with Undo; Start uses one-based beats, and duration and the read-only source offset remain distances in beats. Only audio clips expose **Clip gain**; MIDI clips have no gain or pan. Track holds naming, color, hierarchy, and display options. Channel holds mixer gain, pan, mute, solo, and latency for audio, MIDI, and bus channels. Folders and automation tracks have no Channel page. Every ghost group member displays the shared name and exposes **Make independent**.

`clip_surface` gives audio and MIDI clips the same opaque, track-tinted body and
quiet title band, keeping the beat grid behind musical content. Its returned title
and content rectangles remain separate even on compact tracks. `clip_note_preview`
fits thumbnail notes within that content area using the clip's visible pitch range;
very high and low notes remain distinct and never cover the name. These helpers
only paint; selection, dragging, trimming and ghost identity retain the clip's
existing interaction target.

`DrawNoteLength` supplies **Last note**, 1/16, 1/8, 1/4, 1/2 and one-bar drawing choices through `drawing_note_length`. Last note begins at 1/16 and remembers the last drawn or selected duration; the application converts audible beats to source time. `paint_clip_end_label` marks the visible **Clip end** without adding hit targets. Drawing beyond it extends the MIDI occurrence in the same Undo step as the note.

## State, focus, and accessibility

Every operation has a mouse-only path. Shortcuts accelerate the same visible
controls or context-menu commands. Arbitrary selection membership is available
through Add/Remove context actions as well as Shift-click. Long command menus
use `command_menu_contents` to retain full-size targets inside short windows.

Plain arrows navigate selection: left/right choose neighbouring times and
up/down choose neighbouring visual rows. Shift extends selection. Ctrl/Cmd plus
arrows edits position or pitch; numeric fields and mouse tools provide the same
edits. Text fields, dropdowns and modals retain their native navigation.

`selection_navigation` supplies the shared deterministic spatial policy.
`timeline_selection_view_actions`, `remember_timeline_view` and
`restore_previous_timeline_view` provide Fit selection and Previous view without
changing project data. Previous view coalesces a wheel burst or a complete pan,
including a pause while dragging, and toggles back to the newer view.

Undo/Redo restores the affected clip/note selection and explicit phrase span.
Presentation snapshots follow the retained project history IDs and use a
separate 8 MiB budget; they contain no audio samples.

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

Automation defaults to **Spline**. `curve_timeline_editor_with_segments` adds a
diamond between each sufficiently separated pair of points; drag it vertically
to bend only that segment. Select its starting point or diamond to use **Log**,
**Linear**, **Exp**, and the continuous **Slope** control. The shared component
returns an outgoing-point ID and shape intent; the core owns bounded cubic Bézier
evaluation. Step, Linear and Smooth remain available for existing projects.
Curve painting submits one bounded polyline rather than a shape per sample.
The compact toolbar uses an interpolation picker and keeps the point and slope
controls visible at 800 × 600. `curve_endpoint_summary` identifies the held
values; `paint_curve_override` retains the muted curve beneath its manual line.

Curves cover the complete musical timeline: the first value applies before the
first point (including stopped controls and playback startup), and the final
value holds indefinitely after the last. Curves do not extend song duration.
Native gain, pan and send values are evaluated per sample; VST values retain the
existing block-rate scheduling.

An automated control displays its effective playhead value without changing the
stored manual value. A real manual edit enters **Overridden**, with a coral-red
marker and explicit text; the saved curve remains visible and editable. The
Automation editor, native control context menus, channel status menus, and
plug-in parameter details offer **Resume automation**. Resume follows the curve
at the current playhead. **Override automation** explicitly holds the current
value, and its numeric field adjusts that held value. Overrides persist with the
project and participate in Undo/Redo. Lane/folder bypass instead uses the stored
manual value and is labelled **Bypassed**. Changing a curve or override on an
existing enabled lane uses the live edit path, preserving loaded plug-ins.
Automation has a reserved orange identity (`Palette::automation`) across curves,
nodes, handles, parameter-browser markers, control outlines and status menus,
independent of track colors. Manual takeover uses `automation_override` coral
red; bypassed status remains gray. State wording supplements color in both
themes, and the original curve stays muted beneath an override's held line.
Tool mix edits use the same takeover behavior when a stored value changes.
Explicit override or resume commands take precedence within an atomic batch.
Channel volume, pan and send controls offer **Create automation** directly in
their context menus before a lane exists, then **Edit automation** and **Resume
automation** as appropriate. Properties, Routing and Mixer channel menus expose
the same operations. Creation targets the control's owner, independently of the
current selection, and starts at its current value. Send sliders cover their
supported 0–16 gain range and retain precise numeric entry.

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
The same popup warns by track name when visible MIDI notes on otherwise audible
tracks have no assigned instrument. Tracks excluded by mute, folder mute or solo,
empty clips and source notes outside trimmed bounds do not trigger it.
**Export…** explicitly continues without those notes; healthy audio exports add
no confirmation. Offline rendering does not depend on the output device. VST3 mixdown and baked audio use explicit offline processing with a bounded five-second processing-response budget; live playback retains its 100 ms processing deadline.

A running export keeps its filename and elapsed time visible independently of editor status. Completion remains distinct from rendering. **Retry export** repeats the failed mix, destination and options, retaining its original project snapshot even if later edits were made; hover the action for that scope. New/Open/Quit wait for active export completion. If export fails while switching, **Cancel** returns to its retry action and **Continue** resumes the normal document-save decision without claiming a WAV was completed.

## Device library and effect chains

**Window → Library** opens a large card grid with Instruments and Effects tabs. Each card shows a cached native editor preview, the device name, and its developer. Names wrap to two lines; full text remains in the tooltip. Missing previews use a quiet instrument/effect icon. The grid adapts its columns to the available width and scrolls independently of the selected device's fixed action footer. Click to select, double-click or press Enter to insert/assign, and use arrow keys to move between cards. Left/Right remain text-editing keys while search has focus.

Empty inventory explains that instruments/effects must be installed as VST3,
offers **Plug-in folders**, and points to **Scan**. When instruments are absent it also
states that unassigned MIDI notes are silent. Inventory hidden by search or
developer filters instead shows **No matching instruments/effects** with
**Clear filters**.

Search matches names, developers, and category metadata; the developer selector narrows results independently. **More** holds alternate actions, available metadata, and preview capture/refresh. **Scan** stays visible; **Manage → Folders** opens scanning setup. The footer keeps one primary action appropriate to the insertion context. Background preview completion cannot move its target or change the selection. Only visible card rows are built; a bounded asynchronous cache decodes previews off the UI thread, memoizes failures, and invalidates refreshed images without clearing unrelated previews. The shared `device_card` and `device_grid` components own card styling, stable input identities, focus, and virtualized layout.

The instrument tab creates a MIDI track and pattern or assigns the selected MIDI track, then opens its native editor. **Effects → + Insert** opens this same browser pinned to that channel. **Add to effect chain** returns to the channel's inserts. **Create send track** creates a bus containing the effect and a post-fader send from the source at 25%; configure the plugin's wet/dry control for the desired return. Routing also offers **Create send track**. Escape returns to the caller without inserting anything.

Plugin discovery stays metadata-only. After discovery, one isolated helper at a time initializes verified instruments and effects and captures native previews into Rill's fingerprinted cache. A selected device is prioritized. Failures are retained and retried only with **Capture preview**; **Refresh preview** explicitly updates a cached image. Existing cache entries display immediately. The browser limits resident small textures to 128 and scales the selected preview to at most 1000×600 pixels.

Effect rows use shared `reorder_handle` and `reorder_drop_target` components. Drag the handle to another row to change processing order; Up/Down remains available for direct activation. Reordering moves the complete slot, preserving its ID, opaque state, normalized overrides, bypass state, and automation targets. The normal graph rebuild recomputes processing order and delay compensation.

Mixer uses equal-width shared strips with aligned names, destinations, faders and pan controls. Click a channel name to open its Channel page or its destination to open Routing. The **Channel** menu also provides **Open channel**, Instrument, Effects, and Routing actions where applicable. Channel, Effects, Instrument, and Routing share navigation links that keep the same channel selected. The links omit the current destination, whose context is already shown in the title; closing a utility returns to its caller. Each channel shows its own post-fader stereo meter beside the fader, and Master shows the stereo output meter. Readings come from the running engine.

Instrument opens a concise device overview with the native **Open editor** action. **Automate…** opens a large parameter browser with automatic loading and focused search. Effect rows expose **Automate…** directly, without opening a native editor first. The last-touched parameter remains a direct shortcut in the overview. Native effect editor wrappers identify both the effect and its channel. Parameter editing and automation retain their isolated plugin state and undo behavior.

If a plug-in explicitly cannot create a native editor before startup completes,
the overview explains that saving and exporting remain available. Pending
document actions continue from the stored project state. Failed state capture
from an editor that became active still cancels the operation and retains
recovery, protecting unsaved native changes.

In a native VST3 editor, right-click a parameter and choose **Automate in Rill** to create its automation track from the current value, or open the existing curve. Rill brings the automation editor forward and keeps the plugin editor running. The action follows that instrument or effect instance, even if another channel is selected. Creating a lane has its own Undo step; reopening one preserves its points and adds no history. This uses the optional VST3 `IComponentHandler3` context-menu hook; editors that do not expose host menu entries still use **Automate…** and the parameter browser. Read-only and non-automatable parameters do not receive the native action.

`NativeContextMenu` in the shared component library preserves action IDs, checked/disabled entries, separators, and nested submenus. The editor sandbox either handles the plugin's copied host action or sends its popup model to a short-lived menu process. macOS and Windows use native OS menus; Linux reuses the shared egui menu renderer. Holding a host-rendered menu open does not suspend editor processing, state synchronization, or heartbeats. Closing/replacing the editor or menu kills the menu worker and releases retained plugin targets. Plugin-rendered menus remain under the plugin's own event-loop behavior and the editor sandbox's timeout protection.

The menu worker announces readiness before receiving its model. On macOS this lets the editor yield activation to the registered worker before it opens the menu; cancellation returns activation only when the menu still owns focus. Request serialization and pipe writes run outside the editor loop. The parent keeps stdin open as a lifetime signal, and a crashed parent closes the worker automatically. Linux converts physical X11 coordinates using the actual display scale before revealing the menu.

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

## Common arranger tools

Select supports cross-track marquee selection and per-clip Add/Remove context
actions. Cursor time-block selection spans tracks and retains trailing rests.
Move, copy, cut, duplicate, delete and nudge operate on the complete selection as
one edit. Slip moves source data inside fixed clip edges; selected audio also
shows shared fade handles. Previous/Next clip lives in each Clip Editor View
menu and retains the current tool and scale.

`clip_editing` supplies item-anchored Rename with visible Apply/Cancel, and
precise Start, Duration and Source offset fields. Duration explicitly chooses
Trim or Stretch. F2 accelerates the same Edit/context Rename command.

`batch_fades` supplies millisecond edge settings. Edit → Smooth edit boundaries
applies the last accepted settings; Boundary fade settings offers Apply,
Remove boundary fades and Cancel. A local boundary envelope preserves existing
musical fade parameters and other ghost occurrences. The default is 3 ms in and
5 ms out; already protected edges retain their musical fade unchanged.

`batch_actions` supplies independent Repeat to Loop End and atomic routing of
selected channels. Mixer context menus provide Select/Add/Remove/All/Clear;
Shift-click and arrows accelerate those selection changes. Re-export uses the
last successful destination and tail setting through the normal export path.

`EditorTool` owns tool identities, shortcuts and hover hints. `editor_tools` and `editor_tools_with_labels` use shared exclusive tool controls with a persistent active underline and separate them from subsequent controls. `toolbar_trailing` reserves and wraps trailing controls before right alignment so their hit areas cannot overlap preceding tools. `editor_tools_with_labels` allows contextual wording (Join in the arranger, Glue in the MIDI editor) without changing the tool or key. Unsupported tools reset to Select when changing editors. Ghost copy and Stretch use the shared chain/stretch icons. Fit, Fit preview, and Show all use presentation controls.

`compact_editor_tools(ui, tool, supported, primary, context)` keeps the supplied
primary modes visible and places the remaining modes in a shared tool chooser.
Arrangement uses Select, Pencil, Cursor and Cut as its primary modes. The chooser
reads **More tools** until a secondary mode is selected, then shows that mode's
name and active underline. Its width remains stable as modes change; the
disclosure arrow stays visible. Choices include their existing shortcut and
contextual hover hint. `IconButton::tool_choice` supplies the shared styling;
callers continue routing shortcuts through `editor_tool_shortcuts`. Tool changes
leave selection and project history unchanged. The other editors retain their
existing tool strips.

Audio and MIDI clips share Select/move, either-edge trim, Cut, Join, Duplicate, Ghost copy, Stretch, Erase and Cursor. Pencil creates MIDI clips. Shift-body-drag duplicates once; Shift-edge-drag bypasses snap. Stretch is distinct from trim and preserves the source window. During playback, prepared edits use the existing live-edit path and remain one Undo step per gesture. Menu/context/tool commands share result selection and history labels. Joining retains its focused chooser for selecting clips. Arrangement supports groups of selected clips and time selections that retain rests.

Transport **Range → Loop selection** and the clip context menu share one range
action. A selected time span takes precedence over the enclosing bounds of
selected clips. Setting the bounds and enabling looping is one undoable edit;
the cursor, selection and viewport stay in place. Empty selections and recording
cannot change the loop through this action. Exact start/end fields remain in
Range for manual adjustment. In Clip Editor the action is named **Loop clip**
and targets the open clip, independent of note or waveform selections. An
unselected arranger clip's context menu also uses **Loop clip** and retains the
existing selection.

`editor_tool_hint` keeps the active tool's name, shortcut and a concise gesture
summary on one quiet line below the toolbar. **Tool help** expands the complete
instructions and shortcuts with a mouse click; click it again to reclaim canvas
space. Arrangement, MIDI and audio use `ToolHintContext` for their own semantics
and retain separate disclosure preferences when changing tools. The summary does
not claim focus or consume editing shortcuts; only the help button is a normal
keyboard focus target. Automation uses `interaction_hint` for its fixed point
interactions. Opening help never changes selection, song state or undo history.

Arranger **View → Waveform amplitude** uses the shared
`WaveformAmplitudeScale` choice: **Default** retains the ordinary level scale,
and **Fit** fills each clip's available waveform height. A single reference for
both stereo channels preserves their balance. The reference spans the complete
trimmed source window, so panning and zooming do not change the scale. Fades
remain visible; silent audio stays flat. This session view preference changes
neither project history nor playback. Peak and geometry caches keep it bounded.

## Application identity

`brand_lockup(ui, BrandSize::Compact | BrandSize::Hero)` pairs the shared RILL icon with live Inter SemiBold lettering. The wordmark follows the active palette; image textures are retained per context instead of uploaded every frame. `app_icon()` returns cached native RGBA data, and `APP_ID` is shared with Linux desktop metadata. The menu bar uses Compact; Help → About RILL uses Hero inside the standard contextual popup/backdrop/navigation flow. Artwork, outlined light/dark logo exports and generation prompts live in `crates/daw-ui/assets/brand/`.

## Musical generation previews

The piano-roll **Generate…** action opens a movable dialog over the current clip.
Start bar and Length initially cover only complete visible bars, in clip-local
time. Choices, harmony, preview state and range are remembered per clip for the
app session after apply or cancel; New/Open clears that memory. **Next range**
advances by the current whole-bar length while retaining settings and is disabled
at an empty range or project bounds. Generator harmony stays independent of the
editor's visual Grid scale. A view containing no full bar starts with zero length
and requires an explicit range. The range stays fixed while changing settings or using **Fit
preview**. Range and confirmation controls remain visible when parameters scroll.

**Chord** sustains the selected chord or scale across the range, with octave and
voicing-shift controls. **Arpeggio** offers seven orders and an eighth-note-based
speed multiplier. **Melody** offers random walk, motif variation and Euclidean
rhythm, with algorithm-specific controls and up to three companion voices.
Counterpoint favors consonance and contrary motion; it is not a strict species
solver. Tintinnabuli pairs the melody with nearby tonic-triad tones, following the
[Arvo Pärt Centre's description](https://www.arvopart.ee/en/arvo-part/article/formal-algorithms-of-tintinnabuli-in-arvo-parts-music/).

**New variation** changes the seed; changing Preview or Fit does not regenerate
notes. **Add notes** inserts the exact draft alongside existing notes as one Undo
step, using the existing live playback edit path. The selected occurrence extends through the complete requested bar range, including trailing rests; individual note durations and ghost peers' placement bounds remain independent. The generator stays open after adding, allowing **Next range** and another **Add notes**; the unchanged applied draft cannot be added twice and its purple preview is hidden. **Done** returns to the editor. Trimmed and stretched ghosts retain source-time conversion and shared editing. Closing, Escape or navigation discards only the current draft and restores the prior viewport and scale-only view; previously committed additions remain in history. The Root picker displays all twelve pitch classes together. For Scale harmony, **Use scale for grid** explicitly copies the chosen root/scale into the visual guide without editing notes. Previews
temporarily show all pitches so off-scale choices remain visible. Draft generation
is cached by settings, bounded to 8,192 notes and never allocates project IDs.

`generation_root_selector` exposes all twelve pitch classes in a three-column menu without scrolling. `generation_bar_range` edits a one-based **Start bar** and **Length** in whole bars
using the shared native numeric controls, and displays the inclusive active bar
range. Zero length represents a view with no complete bars; the caller disables
confirmation until a valid range is chosen. Returned input responses expose
change state, focus and precise interaction rectangles.

Generation previews stay in the live piano roll. Draw the ordinary canvas, then
`paint_generation_backdrop`, `paint_generation_range` and
`paint_generation_note`, followed by the shared movable utility window. The light
scrim preserves musical context; the ruler and roll share the active range, and
violet outlined draft notes differ from existing blue notes in both themes.
Additional voices use nearby distinct preview shades. These painters do not
create input targets, note IDs or project edits. The application owns generation,
validation, cancellation and the single undoable confirmation.

### Audio selection, takes, and comping

The audio editor shares Q Select, E Cut, R Duplicate, G Ghost, S Stretch,
I/O fades, P Cursor, middle/Alt panning, modifier zoom, and Fit with the other
musical canvases. Select drags the body to move and either edge to trim;
Shift-body drag makes an independent copy. Cursor drag selects a time range.
Snap separates grid, clip boundaries, and detected transients; Shift bypasses
snapping. View options control neighbouring waveforms and transient markers
without changing audio. Transient analysis remains asynchronous and is restricted
to the selected range when splitting. Markers survive closing the analysis panel.

**Clip → Takes…** opens retained alternatives over the editor's dimmed context.
**Add takes…** imports WAVs aligned to the group's original start, as one Undo
step. **Group overlapping clips** keeps the selected clip as the initial comp.
Choose a waveform row and use its whole take or the selected time range. Boundary
crossfades default to 10 ms with a smooth complementary curve, limited to actual
source handles; Crossfade exposes the editable spline. Every resulting comp piece
can still be cut, trimmed, moved, stretched, duplicated, or joined.

**Layer as overdub** creates an independent audio track with a distinct color and
the same output bus/folder. Imported alternatives and newly recorded loop passes
share this take editor. **Track → Audio takes…** recovers alternatives even after every
comp piece was deleted or joined, and a group picker appears for multiple groups.
**Takes → Keep comp only** discards the retained project alternatives without
modifying source files; Undo restores them.

`paint_timeline_selection` and `paint_audio_clip_edges` provide shared selection
painting and visible edge targets. Offscreen boundaries never become phantom
trim handles. `audio_take_lane` is a reusable, keyboard-focusable waveform choice
with a name, short detail, selection tint, and a common normalized time range.
The app uses `audio_take_lane_with_waveform` to render the same progressive source
preview as the arranger and editor. Its callback is clipped to the waveform plot;
source span mapping and selection remain with the caller.

`paint_audio_waveform_raw` provides an immediate bounded fallback.
`prepare_audio_waveform` creates a reusable `PreparedAudioWaveform` with a single
stereo envelope mesh and physical-pixel coverage fringes; `paint` reuses its mesh
storage and respects the parent clip. The app caches geometry until source/edit,
viewport, density, or theme inputs change. Background peak publication precedes
FFT publication, so refinement does not wait for the complete spectrum. Faint
spectral texture quads remain behind the foreground waveform in every view.

Long uncached stretch/transpose edits can first use a band-limited proxy rendered
by the chosen engine. **Refining audio…** appears while full-rate processing
replaces it in the background. Swaps use a short preallocated correction ramp,
retain running plug-ins and transport, and coalesce obsolete edits. Export always
uses the full-rate render, even while a live preview is being refined.

### Audio recording

**Window → Recording** opens **Recording · Track name** above its caller. **Audio
setup…** connects to input/output-device selection; the focused view contains the
track's **Input**, **Arm recording**, **Input monitor (dry)**, and **Record into**
choices. Inputs use one-based names such as **Mono 1** and **Stereo 1–2**, plus
**No input**. Record placement chooses **New take** or **Overdub layer**. The raw
input meter appears below these controls; recording and monitoring do not require
opening the mixer. **Edit recording** and **Takes…** connect directly to editing.

Monitoring defaults to **Off**. **Auto** monitors an armed track while stopped or
recording, and switches off during ordinary playback; **On** does not require
arming. Dry monitoring follows channel/output-bus level, pan, mute, and solo,
bypasses inserts/sends, and does not alter captured audio. The focused popup
states this distinction. Arming and monitoring are session-only; opening a
project resets them. Input selection is a normal project edit with Undo/Redo.

Transport and the popup reuse `record_button`: **Record** becomes **Stop
recording** during capture. Audio track headers reuse `record_arm_at`, which
exposes an accessible checkbox role, focus/hover feedback, and disabled state.
`paint_recording_region` paints the active arrangement span without creating an
editable clip before finalization. `input_meter` shows the peak of the selected
raw input channels on a −60 to 0 dBFS scale. All recording indicators share
`recording_color` with light/dark variants; ordinary select/checkbox/form controls
come from the existing shared library.

Input, arm, monitor, and placement choices are disabled during capture. Tempo,
loop boundaries, seeking, and Undo/Redo cannot invalidate the active recording.
**Stop recording** preserves backing playback; Pause and Stop also stop playback.
**Finishing recording…** covers writer/tail completion, and Record remains
disabled until publication finishes. Save, New/Open, Quit, and audio-setup changes
wait for that publication. Errors retain captured files, show recovery details,
and offer **Retry input** rather than repeatedly reopening a failed device.

Loop recording keeps the newest complete pass audible while preserving a partial
last pass in **Takes…**. Overdub recording creates a separate colored track with
the original bus/folder and volume/pan, leaving inserts/sends unassigned. Both
paths enter normal clip editing and commit one named history entry.

### MIDI recording and timing controls

The same **Recording · Track name** popup supports MIDI tracks. **MIDI device** selects the input shared by MIDI tracks, with **Refresh MIDI inputs** beside its channel choices. **All channels**, **Channel 1–16**, and **No input** are per-track project edits. MIDI arming defaults to session-only **Auto** monitoring; audio arming retains **Off**. MIDI monitoring uses the assigned plugin and its effects. An unassigned track offers **Choose instrument…** while still allowing note recording. Every recording creates a new clip; loop passes accumulate without replacing existing notes.

Both MIDI and audio track headers reuse `record_arm_at`. Its direct child UI preserves the arranger's parent row geometry. The MIDI instrument selector sits alongside the arm control. **Window → Recording** also opens with no selected track so global timing controls remain available.

`recording_timing_controls` is a shared component composed from the form, choice, toggle and slider components. **Pre-count** is a choice among **No pre-count**, **1 bar** and **2 bars**, disabled during capture. **Click** is a distinct toggle, also available on the transport, with the shared `Metronome` icon. **Click level** remains adjustable during playback. The count shows remaining beats while preserving the intended cursor position. Narrow transport layouts keep loop-range details in the existing Range popup/tooltip.

MIDI audition pauses during pre-count so the prepared timeline can start exactly at the cue; dry audio monitoring remains available. Pre-count starts only from stopped transport; recording over running backing audio starts immediately. Click and level can change without replacing plugin processes. Recorded audio and exported WAVs exclude the click. MIDI/audio publication, error recovery, Save/Open/New/Quit and Undo use the common recording lifecycle.

### Assistant workspace

`assistant(ui, prompt, state, options)` keeps Conversation, Examples, Tools, and Tips in
one bounded body above a fixed prompt and Send/Stop/Undo row. Hosts supply
provider readiness, a pinned scope, current musical context, example categories,
available tools, and tips; the
component does not depend on a provider, project, or tool registry. Tool transcript
rows are labeled **Activity**, since starting a tool is not proof that an edit
succeeded. Context and long status rows retain their full bounded text on hover.

`AssistantOptions::scope` appears persistently as **Scope · …**, separate from
the changing musical selection. Global scope is explicitly labeled as the entire
song. Editor toolbars reuse `contextual_ai_button(ui, scope)`, labeled **AI for
this context**, with the target on hover. The host captures the target identity
when opening the assistant, states that boundary in the provider prompt, and
enforces it for every tool call.

The **Tools** page searches only the host-supplied `AssistantTool` catalog. Each
entry supplies a stable machine `name`, readable `title`, concise `description`,
available `contexts`, and an editable suggested `prompt`. Selecting a title shows
its details; **Use tool** appends its complete prompt to the draft, focuses the
composer, and returns to Conversation without sending. **Clear** resets the
search. The host filters this catalog to the pinned scope; hiding a tool is not
an authorization check. Tool selection and search live in `AssistantState`.

An example title inserts its complete prompt at the end of the current draft,
separated by a blank line, focuses the composer, and returns to Conversation. It
never sends or overwrites a draft. An example that cannot fit the 16,384-character
limit is disabled. Browsing examples and tips remains available before setup.
**AI settings** requests host navigation; **More → Edit last prompt** requests
recovery into an empty draft, and **More → New chat** requests conversation reset
while preserving the song and unsent draft. Settings and conversation actions
are disabled during a run; cancellation remains available through **Stop**.

The host owns `AssistantState`, conversation reset/reuse, and all execution.
`AssistantResponse::inserted_example` and `inserted_tool` identify draft insertions without
turning it into a tool call. Transcripts lay out at most 64 recent messages,
message bodies are capped at 8,192 characters, and guidance catalogs are bounded
to 64 items. Tool catalogs are bounded to 128 items, searches to 256 characters,
and selected tool descriptions to 4,096 characters. Browsing pages and short
popups use a one-line composer that scrolls the complete draft; Conversation
uses a taller composer when space allows. Scope, browsing actions, and the
Send/Stop/Undo row remain reachable at compact window sizes.
