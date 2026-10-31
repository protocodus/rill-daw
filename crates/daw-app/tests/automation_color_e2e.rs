//! Automation has its own visible color in every workspace and theme.
mod common;

use common::Harness;
use daw_core::{AutomationParameter, AutomationPoint, AutomationTarget, Project};
use daw_ui::theme::{self, Palette};
use eframe::egui::Color32;
use serde_json::{Value, json};

fn contains(rect: &Value, point: &Value) -> bool {
    (0..2).all(|axis| {
        let coordinate = point[axis].as_f64().unwrap();
        coordinate >= rect["min"][axis].as_f64().unwrap() - 0.5
            && coordinate <= rect["max"][axis].as_f64().unwrap() + 0.5
    })
}

fn menu(app: &mut Harness, parent: &str, child: &str) -> Value {
    app.click(parent);
    app.click(child)
}

fn paint(app: &mut Harness, baseline: &Value) -> Value {
    app.command(json!({"command":"pointer","phase":"move","x":1095,"y":845}));
    let state = app.command(json!({"command":"inspect","paint":true}));
    for field in ["project", "history", "revision", "modified"] {
        assert_eq!(state[field], baseline[field], "Viewing changed {field}");
    }
    state
}

fn contrast(a: Color32, b: Color32) -> f64 {
    let luminance = |color: Color32| {
        let linear = |byte: u8| {
            let value = f64::from(byte) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
    };
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn assert_outline(state: &Value, region: &Value, color: Color32) {
    assert!(
        state["painted_rects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|shape| {
                shape["stroke"] == json!(color.to_array())
                    && shape["stroke_width"].as_f64().unwrap_or(0.0) >= 1.0
                    && contains(region, &shape["rect"]["min"])
                    && contains(region, &shape["rect"]["max"])
            }),
        "Missing automation outline {color:?} inside {region}"
    );
}

fn assert_text(state: &Value, label: &str, color: Color32) {
    assert!(
        state["painted_text"]
            .as_array()
            .unwrap()
            .iter()
            .any(|text| {
                text["text"] == label
                    && text["colors"]
                        .as_array()
                        .unwrap()
                        .contains(&json!(color.to_array()))
            }),
        "Missing {label:?} painted in {color:?}"
    );
}

fn assert_curve(state: &Value, region: &Value, color: Color32) {
    let width = region["max"][0].as_f64().unwrap() - region["min"][0].as_f64().unwrap();
    assert!(
        state["painted_paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| {
                if path["stroke"] != json!(color.to_array()) || path["closed"] != false {
                    return false;
                }
                let points = path["points"].as_array().unwrap();
                points.len() > 20
                    && points.iter().all(|point| contains(region, point))
                    && points.last().unwrap()[0].as_f64().unwrap() - points[0][0].as_f64().unwrap()
                        > width * 0.9
            }),
        "Missing full-width automation curve {color:?} inside {region}"
    );
}

fn assert_editor(state: &Value, colors: Palette, overridden: bool) {
    let plot = &state["automation_editor"]["plot"];
    assert_curve(
        state,
        plot,
        if overridden {
            colors.muted
        } else {
            colors.automation
        },
    );
    assert_text(
        state,
        if overridden {
            "Overridden"
        } else {
            "Automated"
        },
        if overridden {
            colors.automation_override
        } else {
            colors.automation
        },
    );
    let nodes = state["painted_circles"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|circle| {
            circle["fill"] == json!(colors.automation.to_array())
                && contains(plot, &circle["center"])
        })
        .count();
    assert_eq!(nodes, 3, "All automation nodes keep the automation color");
    let handles = state["painted_paths"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|path| {
            path["closed"] == true
                && path["stroke"] == json!(colors.automation.to_array())
                && path["points"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|point| contains(plot, point))
        })
        .count();
    assert_eq!(handles, 2, "Both segment handles keep the automation color");
    if overridden {
        assert!(
            state["painted_lines"]
                .as_array()
                .unwrap()
                .iter()
                .any(|line| {
                    line["stroke"] == json!(colors.automation_override.to_array())
                        && line["points"][0][1] == line["points"][1][1]
                        && contains(plot, &line["points"][0])
                        && contains(plot, &line["points"][1])
                }),
            "Override needs a separate coral held-value line"
        );
    }
}

fn assert_arranger(state: &Value, id: u64, colors: Palette, overridden: bool) {
    let header = &state["timeline"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["track"] == id)
        .unwrap()["header"];
    let curve = json!({"min":[header["max"][0],header["min"][1]],
        "max":[state["timeline"]["canvas"]["max"][0],header["max"][1]]});
    assert_curve(
        state,
        &curve,
        if overridden {
            colors.muted
        } else {
            colors.automation
        },
    );
    assert_outline(
        state,
        header,
        if overridden {
            colors.automation_override
        } else {
            colors.automation
        },
    );
    if overridden {
        assert_curve(state, &curve, colors.automation_override);
    }
}

fn open_editor(app: &mut Harness, name: &str) {
    app.click("Arranger");
    app.click("Fit");
    app.click(name);
    menu(app, "Window", "Automation");
}

#[test]
fn automation_and_override_colors_follow_state_across_workspaces_and_themes() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let source = project.add_audio_track("Source");
    let bus = project.add_bus("Room");
    project.add_send(source, bus, 0.5, false).unwrap();
    let mut lanes = Vec::new();
    for (name, parameter) in [
        ("Volume envelope", AutomationParameter::TrackGain),
        ("Send envelope", AutomationParameter::SendGain { bus }),
    ] {
        let id = project
            .add_automation_track(
                name,
                AutomationTarget {
                    track: source,
                    parameter,
                },
            )
            .unwrap();
        let lane = project.automation_lane_mut(id).unwrap();
        lane.points = vec![
            AutomationPoint::new(0.0, 0.2),
            AutomationPoint::new(4.0, 0.8),
            AutomationPoint::new(8.0, 0.4),
        ];
        // Saved user colors must not make an automation curve look like a
        // normal channel or change when its theme/state changes.
        project.track_mut(id).unwrap().color = [70, 155, 205];
        lanes.push(id);
    }
    let path = app.directory.path().join("automation-colors.rill");
    project.save(&path).unwrap();
    let original = app.command(json!({"command":"open","path":path}));
    let mut baseline = original.clone();

    for (step, overridden) in [false, true, false].into_iter().enumerate() {
        for (dark, density) in [(false, 1.0), (true, 2.0)] {
            app.command(json!({"command":"system_theme","dark":dark}));
            app.command(
                json!({"command":"viewport","width":1100,"height":850,"pixels_per_point":density}),
            );
            let colors = if dark { theme::DARK } else { theme::LIGHT };
            let active = if overridden {
                colors.automation_override
            } else {
                colors.automation
            };
            assert_ne!(
                active, colors.accent,
                "Automation must stand apart from general UI selection"
            );
            assert_ne!(colors.automation, colors.automation_override);
            for background in [colors.bg, colors.panel, colors.raised, colors.control_fill] {
                assert!(
                    contrast(active, background) >= 3.0,
                    "Automation graphics need visible contrast"
                );
            }

            app.click("Arranger");
            // Keep every lane visible when wrapped mouse controls reduce canvas height.
            app.click("Fit");
            let state = paint(&mut app, &baseline);
            for id in &lanes {
                assert_arranger(&state, *id, colors, overridden);
            }

            open_editor(&mut app, "Volume envelope");
            let state = paint(&mut app, &baseline);
            assert_editor(&state, colors, overridden);

            app.click("Mixer");
            let state = paint(&mut app, &baseline);
            let channel = state["mixer"]["channels"]
                .as_array()
                .unwrap()
                .iter()
                .find(|channel| channel["track"] == source)
                .unwrap();
            for control in ["gain", "fader"] {
                assert_outline(&state, &channel[control], active);
            }
            assert_text(
                &state,
                if overridden {
                    "Overridden"
                } else {
                    "Automated"
                },
                active,
            );

            app.click("Source");
            let state = paint(&mut app, &baseline);
            assert_eq!(state["popups"]["properties"], true);
            assert_outline(&state, &state["window_rects"][0], active);

            app.click("Routing");
            let state = paint(&mut app, &baseline);
            assert_eq!(state["popups"]["routing"], true);
            assert_outline(&state, &state["window_rects"][0], active);
        }
        if step == 2 {
            break;
        }
        // Existing mouse commands change only override state; all later view
        // and appearance checks compare against this exact history checkpoint.
        for name in ["Volume envelope", "Send envelope"] {
            open_editor(&mut app, name);
            baseline = app.click(if overridden {
                "Resume automation"
            } else {
                "Override automation"
            });
        }
        if overridden {
            assert_eq!(baseline["project"], original["project"]);
        }
    }
    app.quit();
}
