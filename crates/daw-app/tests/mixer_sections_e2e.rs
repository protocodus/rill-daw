//! Mixer sections use spacing instead of enclosing frames. Outlines belong to
//! individual input fields, processing actions and troughs, which remain mouse-editable.
mod common;
use common::Harness;
use daw_core::Project;
use serde_json::{Value, json};

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) / 2.0,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) / 2.0,
    )
}
fn click(app: &mut Harness, rect: &Value) -> Value {
    let (x, y) = center(rect);
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}))
}
#[test]
fn signal_flow_panels_are_ordered_painted_and_keep_controls_editable() {
    let mut app = Harness::new();
    let mut project = Project::new();
    project.add_audio_track("Drums");
    project.add_bus("Room");
    let path = app.directory.path().join("mixer-sections.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(
            json!({"command":"viewport","width":1440,"height":1100,"pixels_per_point":density}),
        );
        app.click("Mixer");
        let state = app.command(json!({"command":"inspect","paint":true}));
        for channel in state["mixer"]["channels"].as_array().unwrap() {
            let sections = channel["sections"].as_array().unwrap();
            assert_eq!(
                sections
                    .iter()
                    .map(|section| section["name"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                ["Source", "Effects", "Volume & pan", "Output & sends"]
            );
            // Input trim sits before the inserts; post-fader RMS statistics
            // sit before routing. Neither belongs to an empty section gap.
            let source_role_space = if channel["bus"] == true {
                0.0
            } else {
                state["mixer"]["role_height"].as_f64().unwrap()
            };
            for (previous, next, expected_gap) in [
                (
                    &sections[0]["rect"],
                    &channel["input_gain"],
                    source_role_space + f64::from(daw_ui::MIXER_SECTION_GAP),
                ),
                (
                    &channel["input_gain"],
                    &sections[1]["rect"],
                    f64::from(daw_ui::MIXER_ROW_GAP),
                ),
                (
                    &sections[1]["rect"],
                    &sections[2]["rect"],
                    f64::from(daw_ui::MIXER_SECTION_GAP),
                ),
                (&sections[2]["rect"], &channel["statistics"], 0.0),
                (
                    &channel["statistics"],
                    &sections[3]["rect"],
                    f64::from(daw_ui::MIXER_SECTION_GAP + daw_ui::MIXER_ROW_GAP),
                ),
            ] {
                let gap = next["min"][1].as_f64().unwrap() - previous["max"][1].as_f64().unwrap();
                assert!(
                    (gap - expected_gap).abs() < 0.6,
                    "Expected {expected_gap} pt gap, got {gap} between {previous} and {next}"
                );
            }
            let within = |rect: &Value, area: &Value| {
                let (x, y) = center(rect);
                x >= area["min"][0].as_f64().unwrap() - 1.0
                    && x <= area["max"][0].as_f64().unwrap() + 1.0
                    && y >= area["min"][1].as_f64().unwrap() - 1.0
                    && y <= area["max"][1].as_f64().unwrap() + 1.0
            };
            let strip = &channel["strip"];
            let outlined: Vec<_> = state["painted_rects"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|shape| {
                    shape["stroke_width"].as_f64().unwrap() > 0.0
                        && shape["stroke"][3].as_u64().unwrap() > 0
                        && within(&shape["rect"], strip)
                        // The selected strip's own outline.
                        && (shape["rect"]["max"][0].as_f64().unwrap()
                            - shape["rect"]["min"][0].as_f64().unwrap())
                            < (strip["max"][0].as_f64().unwrap()
                                - strip["min"][0].as_f64().unwrap())
                                - 2.0
                })
                .collect();
            for shape in &outlined {
                assert!(
                    [
                        "input_gain",
                        "effects",
                        "analyze",
                        "gain",
                        "pan",
                        "fader",
                        "meter"
                    ]
                    .iter()
                    .any(|area| within(&shape["rect"], &channel[*area])),
                    "outlines must belong to input/volume fields, processing actions or troughs: {shape}"
                );
            }
            assert!(
                !outlined.is_empty(),
                "the gain field and troughs keep their outline"
            );
        }
        let channel = &state["mixer"]["channels"][0];
        let muted = click(&mut app, &channel["mute"]);
        assert_eq!(muted["project"]["tracks"][0]["mute"], true);
        app.click("Edit");
        let restored = app.click("Undo");
        assert_eq!(restored["project"], state["project"]);
        let opened = click(&mut app, &restored["mixer"]["channels"][0]["effects"]);
        assert_eq!(opened["popups"]["effects"], true);
        app.key("Escape");
    }
    app.quit();
}
