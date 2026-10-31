Always reply to all PR comments.
Do not merge a PR without an explicit request from the user.
Only merge when checks are green and feedback is addressed.

Add every new UI component to `crates/daw-ui` first, then reuse it from application views. Extend the shared component library when a control is missing; do not introduce one-off application controls or duplicate component styling. Use the shared theme and design tokens. See `docs/UI.md` for the component API and interaction conventions.

Center horizontal control rows vertically by default: labels, icons, fields and actions share a common center line. Reserve row height before painting mixed controls. Use shared `parameter_row` or `parameter_toolbar` for rows containing captioned sliders so scale captions do not offset their rails from adjacent controls.

Keep the interface simple and focused on one workspace at a time. Clip Editor, Instrument, Effects, Mixer, Routing, Library, Setup, and Properties open on demand from the menu bar. Keep persistent labels and explanatory text concise.

Every operation must be possible using only the mouse. Keyboard shortcuts accelerate the same operations; provide a visible control or context-menu action for each operation, including selection changes.


Every new user-facing functionality must include an executable end-to-end test that exercises it through the running app. Keep the headless subprocess suite container-compatible; run the relevant E2E tests before considering the feature complete.
