# Shared UI components

`daw-ui` is Rill's shared component library. It depends on `egui` 0.33.3 without depending on the application, audio engine, or native window runtime. The app uses it for controls, surfaces, popup windows, and design tokens. New reusable components belong here before they are used in an application view.

Arrangement remains the base canvas. Clip Editor, Instrument, Effects, Mixer, Routing, Automation, Library, Setup, and Properties open on demand as one focused popup at a time. Application commands live in File, Edit, Track, and Window menus. Components keep their labels short; explanations belong in tooltips or the relevant popup rather than permanent interface text.

## Theme and tokens

Call `daw_ui::theme::apply(ctx)` when creating the application. Palette constants—including arrangement and piano-roll colors—come from `daw_ui::theme`. Logical dimensions and typography come from `daw_ui::tokens`:

| Token family | Values |
| --- | --- |
| Spacing | `SPACE_XS` 4, `SPACE_SM` 8, `SPACE_MD` 12, `SPACE_LG` 16, `SPACE_XL` 24 |
| Controls | `CONTROL_HEIGHT` 28, `COMPACT_HEIGHT` 22, `SELECT_WIDTH` 160 |
| Corners | `RADIUS` 4 |
| Typography | Caption 10, small 11, button 12, body 13, heading 22 |

Sizes are logical screen points and follow egui's display scaling. The theme also supplies hover, active, selection, and focus styling. Avoid introducing a second app-specific palette or copied spacing scale.

## Components

Standard controls are available at the crate root and through `daw_ui::widgets`. The curve editor is exported at the crate root.

| Purpose | API |
| --- | --- |
| Standard actions | `button`, `primary_button`, `small_button` |
| Button builder for sized/disabled placement | `button_widget` |
| Application and contextual menus | `menu_bar`, `menu_button` |
| Boolean button | `toggle` |
| Boolean setting | `switch`, `checkbox` |
| Exclusive choices | `radio_value`, `selectable_value` |
| Selected navigation/item | `selectable` |
| Arrangement overlay control | `selectable_at` |
| Text input | `text_field` |
| Dropdown choices | `select`, `select_labeled` |
| Numeric widget builders | Generic `slider`, `number` |
| Track color | `color_edit` |
| Interactive parameter curves | `curve_editor`, `CurvePoint`, `CurveView`, `CurveEdit` |
| Surfaces | `panel`, `card`, `inset` |
| On-demand windows | `popup`, `workspace_popup` |
| Compact section label | `theme::label` |

Use native response composition rather than adding a second event system:

```rust
use daw_ui as controls;

fn channel_controls(ui: &mut egui::Ui, enabled: &mut bool, name: &mut String, gain: &mut f32) {
    controls::card().show(ui, |ui| {
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
fn waveform(ui: &mut egui::Ui, selected: &mut usize) {
    daw_ui::select("waveform", ["Sine", "Triangle", "Saw"][*selected])
        .width(120.0)
        .show_ui(ui, |ui| {
            for (value, label) in ["Sine", "Triangle", "Saw"].into_iter().enumerate() {
                if daw_ui::selectable_value(ui, selected, value, label).clicked() {
                    ui.close();
                }
            }
        });
}
```

Use `popup(title).open(&mut open).show(ctx, ...)` for utility windows. The builder supplies a common frame, spacing, and initial width. It defaults to fixed sizing and no collapse triangle; an individual tool may explicitly override `.resizable(true)` or its width when its content needs it. An `open = false` popup renders no content.

`selectable_at(ui, rect, id_salt, selected, label)` is for controls drawn inside an existing arrangement canvas. It supplies a compact native button in the given rectangle without advancing or enlarging the parent's layout. It requires a stable semantic ID such as `(track.id, "mute")` and retains native hover, focus, keyboard, and accessibility behavior. It does not own arrangement geometry.

Arrangement and piano-roll canvases reserve pointer drags for editing. Their scroll areas disable background drag scrolling with `ScrollSource { drag: false, ..Default::default() }`, retaining wheel and scrollbar navigation. This prevents a canvas from scrolling a target away while a clip, note, or track header is being pressed.

## State, focus, and accessibility

State stays with the caller. Boolean and choice components mark `Response::changed()` only when their value changes. Use `.clicked()` for actions and `.changed()` for committing edits. Shared components do not create undo history, mutate project state indirectly, or rebuild audio themselves.

Disable a group with `ui.add_enabled_ui(false, |ui| ...)`, or use `ui.add_enabled(enabled, button_widget(...))`. Disabled components do not change values from pointer or keyboard input. Scope repeated rows using `ui.push_id(model_id, ...)`; repeated labels alone are not sufficient persistent identity for every control.

Native buttons, checkboxes, radio choices, text fields, dropdowns, and numeric controls retain egui's keyboard behavior. The custom visual switch exposes a labelled checkbox role and selected state through `WidgetInfo`; its whole label/track area accepts clicks, and Space or Enter activates it when focused. It paints a visible focus ring. Supply a descriptive label, including for icon-only controls, through their response's accessibility/widget metadata where the visual label cannot describe the action.

The library's headless interaction tests exercise pointer activation on a switch label, focus and keyboard activation, disabled input, choice change reporting, Unicode text entry, dropdown selection, compact-control layout isolation, and hidden popups. These test behavior without coupling application views to widget internals.

Run the component checks with:

```sh
cargo test -p daw-ui
cargo clippy -p daw-ui --all-targets -- -D warnings
```

`workspace_popup(title, ctx)` sizes a canvas tool below the transport within the current viewport. The app disables the background arrangement while a tool is open. Menu commands, Escape, and window close all use the same state transitions. Every added user-facing feature requires a subprocess E2E test; component interaction tests complement those scenarios.

`curve_editor` receives normalized points and a sampling callback, and returns add/move/delete intents. It does not depend on the project model or audio engine. The application owns interpolation, validation, and undo. Canvas points support click, drag, and deletion; precise numeric controls use the same edit path.
