//! Mixer strips separate their signal-flow sections by space alone: no
//! section frames, and outlines only on the gain field, the pan and fader
//! troughs and the meter troughs. The controls stay directly editable with the mouse.
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
            for pair in sections.windows(2) {
                let gap = pair[1]["rect"]["min"][1].as_f64().unwrap()
                    - pair[0]["rect"]["max"][1].as_f64().unwrap();
                assert!(
                    (gap - 13.0).abs() < 0.6,
                    "sections are separated by one 13 pt gap: {gap} in {sections:?}"
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
                    ["gain", "pan", "fader", "meter"]
                        .iter()
                        .any(|area| within(&shape["rect"], &channel[*area])),
                    "only the gain field, pan and fader troughs and meter are outlined: {shape}"
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
