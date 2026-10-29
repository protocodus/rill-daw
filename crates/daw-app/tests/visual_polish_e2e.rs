//! Focus feedback and routing paint are exercised in the running application.
mod common;

use common::Harness;
use daw_core::{Clip, Note, Project};
use serde_json::{Value, json};

fn bounds(rect: &Value) -> [f64; 4] {
    [
        rect["min"][0].as_f64().unwrap(),
        rect["min"][1].as_f64().unwrap(),
        rect["max"][0].as_f64().unwrap(),
        rect["max"][1].as_f64().unwrap(),
    ]
}

fn same_bounds(actual: &Value, expected: [f64; 4]) -> bool {
    actual.as_array().is_some_and(|actual| {
        actual.len() == 4
            && actual
                .iter()
                .zip(expected)
                .all(|(a, e)| a.as_f64().is_some_and(|a| (a - e).abs() <= 0.15))
    })
}

fn unchanged(state: &Value, baseline: &Value) {
    for field in ["project", "history", "revision", "modified"] {
        assert_eq!(
            state[field], baseline[field],
            "Presentation changed {field}"
        );
    }
}

fn painted(app: &mut Harness) -> Value {
    app.command(json!({"command":"pointer", "phase":"move", "x":1095,"y":895}));
    app.command(json!({"command":"inspect", "paint":true}))
}

fn glow_meshes(state: &Value, rect: [f64; 4]) -> Vec<&Value> {
    let extent = f64::from(daw_ui::tokens::SELECTION_GLOW_EXTENT);
    let expected = [
        rect[0] - extent,
        rect[1] - extent,
        rect[2] + extent,
        rect[3] + extent,
    ];
    state["painted_meshes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|mesh| {
            same_bounds(&mesh["bounds"], expected) && mesh["max_alpha"].as_u64().unwrap() > 14
        })
        .collect()
}

fn assert_glow(state: &Value, rect: [f64; 4], selected: bool) {
    let meshes = glow_meshes(state, rect);
    assert_eq!(
        meshes.len(),
        usize::from(selected),
        "Selection must add one halo around {rect:?}"
    );
    if let Some(mesh) = meshes.first() {
        assert_eq!(mesh["valid"], true);
        assert!(mesh["vertices"].as_u64().unwrap() <= 200);
        assert!(mesh["indices"].as_u64().unwrap() <= 700);
        assert!(
            mesh["transparent_vertices"].as_u64().unwrap() > 0,
            "Halo must fade out"
        );
        assert!(
            mesh["max_alpha"].as_u64().unwrap() <= 100,
            "Halo must leave content legible"
        );
    }
}

#[test]
fn selected_clips_and_channels_gain_a_bounded_halo_without_editing_content() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let mut project = Project::new();
        let source = project.add_midi_track("Indigo synth");
        project.track_mut(source).unwrap().color = [100, 125, 215];
        for (name, start) in [("First phrase", 1.0), ("Second phrase", 5.0)] {
            project.track_mut(source).unwrap().clips.push(Clip::midi(
                name,
                start,
                3.0,
                vec![Note::new(60, 0.0, 1.0, 100)],
            ));
        }
        let bus = project.add_bus("Amber bus");
        project.track_mut(bus).unwrap().color = [225, 160, 65];
        project.set_output(source, Some(bus)).unwrap();
        let path = app.directory.path().join("focus-polish.rill");
        project.save(&path).unwrap();
        let baseline = app.command(json!({"command":"open", "path":path}));
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(
            json!({"command":"viewport", "width":1100, "height":900, "pixels_per_point":density}),
        );

        for name in ["First phrase", "Second phrase", "First phrase"] {
            app.click(name);
            let state = painted(&mut app);
            unchanged(&state, &baseline);
            for clip in state["timeline"]["clips"].as_array().unwrap() {
                assert_glow(
                    &state,
                    bounds(&clip["rect"]),
                    clip["id"] == state["selected_clip"],
                );
            }
        }

        app.click("Mixer");
        for selected in [bus, source] {
            let state = app.inspect();
            let [left, top, right, bottom] = bounds(&channel(&state, selected)["name"]);
            app.command(
                json!({"command":"pointer","phase":"click","button":"secondary",
                "x":(left+right)*0.5,"y":(top+bottom)*0.5}),
            );
            app.click("Select channel");
            let state = painted(&mut app);
            assert_eq!(state["selected_track"], selected);
            unchanged(&state, &baseline);
            for channel in state["mixer"]["channels"].as_array().unwrap() {
                let mut rect = bounds(&channel["strip"]);
                // Shared strips reserve a two-point gutter inside each side.
                rect[0] += 2.0;
                rect[2] -= 2.0;
                assert_glow(&state, rect, channel["track"] == selected);
            }
            let repeated = painted(&mut app);
            assert_eq!(
                repeated["painted_meshes"], state["painted_meshes"],
                "Idle paint must not animate or accumulate halos"
            );
            unchanged(&repeated, &baseline);
        }
        app.quit();
    }
}

fn channel(state: &Value, id: u64) -> &Value {
    state["mixer"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["track"] == id)
        .unwrap()
}

fn point(value: &Value) -> [f64; 2] {
    [value[0].as_f64().unwrap(), value[1].as_f64().unwrap()]
}

fn bottom_center(rect: &Value) -> [f64; 2] {
    let [left, _, right, bottom] = bounds(rect);
    [(left + right) * 0.5, bottom]
}

fn assert_route(state: &Value, route: &Value, project: &Project) {
    let source = route["source"].as_u64().unwrap();
    let target = route["target"].as_u64();
    let from = point(&route["from"]);
    let to = point(&route["to"]);
    assert_eq!(from, bottom_center(&channel(state, source)["strip"]));
    let target_strip = target.map_or(
        &state["mixer"]["master_strip"],
        |id| &channel(state, id)["strip"],
    );
    assert_eq!(to, bottom_center(target_strip));
    assert_eq!(route["source_clipped"], false);
    assert_eq!(route["target_clipped"], false);
    let path: Vec<_> = route["path"]
        .as_array()
        .unwrap()
        .iter()
        .map(point)
        .collect();
    assert_eq!(path.len(), 4);
    assert_eq!(path[0], from);
    assert_eq!(path[3], to);
    assert_eq!(path[1][0], from[0]);
    assert_eq!(path[2][0], to[0]);
    assert_eq!(path[1][1], path[2][1]);
    assert!(
        path[1][1] > from[1] && path[2][1] > to[1],
        "Routing must descend and return to its destination"
    );
    assert!(route["stroke_width"].as_f64().unwrap() >= 3.0);
    let source_color = project.track(source).unwrap().color;
    let source_color = json!([source_color[0], source_color[1], source_color[2], 255]);
    assert_eq!(route["source_color"], source_color);
    if let Some(target) = target {
        let color = project.track(target).unwrap().color;
        assert_eq!(
            route["target_color"],
            json!([color[0], color[1], color[2], 255])
        );
    }
    let meshes = state["painted_meshes"].as_array().unwrap();
    let shaft = meshes
        .iter()
        .find(|mesh| {
            mesh["vertices"] == 8
                && mesh["indices"] == 18
                && same_bounds(&mesh["bounds"], bounds(&route["shaft"]))
        })
        .expect("A continuous bounded ribbon must actually be submitted");
    let head = meshes
        .iter()
        .find(|mesh| {
            mesh["vertices"] == 3
                && mesh["indices"] == 3
                && same_bounds(&mesh["bounds"], bounds(&route["head"]))
        })
        .expect("The destination must have a visible arrowhead");
    for mesh in [shaft, head] {
        assert_eq!(mesh["valid"], true);
    }
    let vertices = shaft["vertex_data"].as_array().unwrap();
    assert_eq!(vertices[0]["color"], source_color);
    assert_eq!(vertices[1]["color"], source_color);
    assert!((point(&vertices[0]["position"])[0] - point(&vertices[1]["position"])[0]).abs() >= 3.0);
    let head_vertices = head["vertex_data"].as_array().unwrap();
    assert_eq!(
        point(&head_vertices[0]["position"]),
        to,
        "Arrow tip must touch the destination strip"
    );
    assert_eq!(head_vertices[0]["color"], route["target_color"]);
    assert_eq!(
        vertices[6]["color"], head_vertices[1]["color"],
        "Gradient must continue smoothly into the arrowhead"
    );
    assert_eq!(vertices[7]["color"], head_vertices[2]["color"]);
    assert_eq!(
        vertices[6]["position"][1], head_vertices[1]["position"][1],
        "Shaft and arrowhead must share their join"
    );
    assert_eq!(vertices[7]["position"][1], head_vertices[2]["position"][1]);
    assert_ne!(vertices[2]["color"], source_color);
    assert_ne!(vertices[4]["color"], route["target_color"]);
    for vertex in &vertices[2..] {
        for component in 0..3 {
            let start = source_color[component].as_u64().unwrap();
            let end = route["target_color"][component].as_u64().unwrap();
            let value = vertex["color"][component].as_u64().unwrap();
            assert!((start.min(end)..=start.max(end)).contains(&value));
        }
    }
}

#[test]
fn routing_gradients_connect_both_directions_and_master_without_gaps() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let mut project = Project::new();
        let left = project.add_bus("Amber group");
        let source = project.add_midi_track("Blue source");
        let right = project.add_bus("Green space");
        for (id, color) in [
            (left, [225, 160, 65]),
            (source, [70, 115, 210]),
            (right, [60, 180, 125]),
        ] {
            project.track_mut(id).unwrap().color = color;
        }
        project.track_mut(source).unwrap().clips.push(Clip::midi(
            "Route phrase",
            0.0,
            4.0,
            vec![Note::new(60, 0.0, 1.0, 100)],
        ));
        project.set_output(source, Some(left)).unwrap();
        project.add_send(source, right, 0.25, true).unwrap();
        let path = app.directory.path().join("routing-polish.rill");
        project.save(&path).unwrap();
        let baseline = app.command(json!({"command":"open","path":path}));
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(
            json!({"command":"viewport","width":1100,"height":900,"pixels_per_point":density}),
        );
        app.click("Route phrase");
        app.click("Mixer");
        let state = painted(&mut app);
        let routes = state["mixer"]["routes"].as_array().unwrap();
        assert_eq!(routes.len(), 2);
        assert!(point(&routes[0]["from"])[0] > point(&routes[0]["to"])[0]);
        assert!(point(&routes[1]["from"])[0] < point(&routes[1]["to"])[0]);
        assert!(bounds(&routes[0]["lane"])[3] <= bounds(&routes[1]["lane"])[1]);
        for route in routes {
            assert_route(&state, route, &project);
        }
        unchanged(&state, &baseline);
        app.click("Amber group");
        app.key("Escape");
        let state = painted(&mut app);
        let routes = state["mixer"]["routes"].as_array().unwrap();
        assert_eq!(routes.len(), 1);
        assert!(routes[0]["target"].is_null());
        assert_route(&state, &routes[0], &project);
        unchanged(&state, &baseline);
        app.quit();
    }
}

#[test]
fn inactive_context_requests_one_bounded_blur_and_preserves_foreground_input() {
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = Harness::new();
        let mut project = Project::new();
        let track = project.add_midi_track("Focus source");
        project.track_mut(track).unwrap().clips.push(Clip::midi(
            "Focus phrase",
            0.0,
            4.0,
            vec![Note::new(60, 0.0, 1.0, 100)],
        ));
        let path = app.directory.path().join("blur-context.rill");
        project.save(&path).unwrap();
        let baseline = app.command(json!({"command":"open","path":path}));
        app.command(json!({"command":"system_theme","dark":dark}));
        app.command(
            json!({"command":"viewport","width":1100,"height":900,"pixels_per_point":density}),
        );
        app.click("Focus phrase");
        assert_eq!(app.inspect()["backdrop_blur"], json!([]));
        app.click("Window");
        app.click("Effects");
        let assert_blur = |state: &Value, cached: bool| {
            let blur = state["backdrop_blur"].as_array().unwrap();
            assert_eq!(blur.len(), 1, "Nested focus must not compound blur passes");
            assert_eq!(blur[0]["cached_source"], cached);
            assert_eq!(
                blur[0]["gpu_available"], false,
                "Headless testing must stay display-free"
            );
            let rect = bounds(&blur[0]["rect"]);
            assert!(rect[0] >= 0.0 && rect[1] >= 0.0 && rect[2] <= 1100.0 && rect[3] <= 900.0);
            for (axis, length) in [rect[2] - rect[0], rect[3] - rect[1]]
                .into_iter()
                .enumerate()
            {
                let size = blur[0]["target_size"][axis].as_f64().unwrap();
                assert!(size > 0.0 && size <= 512.0);
                assert!(size <= (length * density / 4.0).ceil());
            }
            unchanged(state, &baseline);
        };
        let utility = painted(&mut app);
        assert_eq!(utility["popup_context"]["interactive"], false);
        assert_blur(&utility, true);
        let stable = painted(&mut app);
        assert_eq!(stable["backdrop_blur"], utility["backdrop_blur"]);
        // A real foreground action still works above the softened caller.
        app.click("+ Insert");
        let nested = painted(&mut app);
        assert!(
            nested["visible_labels"]
                .as_array()
                .unwrap()
                .contains(&json!("Device library"))
        );
        assert_blur(&nested, true);
        app.key("Escape");
        app.click("File");
        let menu = painted(&mut app);
        assert_blur(&menu, false);
        assert!(
            menu["visible_labels"]
                .as_array()
                .unwrap()
                .contains(&json!("New"))
        );
        app.key("Escape");
        assert_blur(&painted(&mut app), true);
        app.key("Escape");
        let restored = painted(&mut app);
        assert_eq!(restored["backdrop_blur"], json!([]));
        unchanged(&restored, &baseline);
        app.quit();
    }
}

#[test]
#[ignore = "requires a native desktop or xvfb-run with OpenGL"]
fn native_utility_blurs_background_detail_in_both_themes() {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let directory = tempfile::tempdir().unwrap();
    let mut project = Project::new();
    for index in 0..8 {
        let track = project.add_midi_track(format!("Detailed background {index}"));
        project.track_mut(track).unwrap().color = [100, 125, 215];
        project.track_mut(track).unwrap().clips.push(Clip::midi(
            format!("Phrase {index}"),
            0.0,
            8.0,
            (0..8)
                .map(|note| Note::new(60 + note, f64::from(note), 0.5, 100))
                .collect(),
        ));
    }
    let session = directory.path().join("native-blur.rill");
    project.save(&session).unwrap();
    let binary =
        std::env::var_os("RILL_E2E_BINARY").unwrap_or_else(|| env!("CARGO_BIN_EXE_rill").into());
    let config = directory.path().join("config");
    std::fs::create_dir_all(&config).unwrap();
    for appearance in ["Dark", "Light"] {
        std::fs::write(
            config.join("settings.json"),
            json!({"appearance":appearance}).to_string(),
        )
        .unwrap();
        let mut captures = Vec::new();
        for view in ["arrangement", "properties"] {
            let screenshot = directory.path().join(format!("{appearance}-{view}.png"));
            let mut child = Command::new(&binary)
                .arg("--screenshot")
                .arg(&screenshot)
                .arg("--project")
                .arg(&session)
                .args(["--view", view])
                .env("RILL_CONFIG_DIR", &config)
                .env("RILL_CACHE_DIR", directory.path().join("cache"))
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(30);
            let status = loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break status;
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("Native blur screenshot did not finish for {appearance}/{view}");
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            assert!(
                status.success(),
                "Native {appearance}/{view} failed: {status}"
            );
            captures.push(image::open(screenshot).unwrap().to_rgb8());
        }
        assert_eq!(captures[0].dimensions(), captures[1].dimensions());
        let detail = |image: &image::RgbImage| {
            // This caller-only region contains track names, routing captions,
            // sliders and scale marks, safely outside the centered utility.
            // Normalize by image variance so dimming alone cannot pass.
            let mut sum = 0.0;
            let mut squared = 0.0;
            let mut differences = 0.0;
            let mut count = 0.0;
            let luma = |x, y| {
                let p = image.get_pixel(x, y);
                f64::from(p[0]) * 0.2126 + f64::from(p[1]) * 0.7152 + f64::from(p[2]) * 0.0722
            };
            for y in 220..700.min(image.height() - 1) {
                for x in 24..220 {
                    let value = luma(x, y);
                    sum += value;
                    squared += value * value;
                    differences +=
                        (value - luma(x + 1, y)).powi(2) + (value - luma(x, y + 1)).powi(2);
                    count += 1.0;
                }
            }
            let variance = squared / count - (sum / count).powi(2);
            assert!(variance > 1.0, "Inactive caller must remain visible");
            (differences / count / variance).sqrt()
        };
        let sharp = detail(&captures[0]);
        let blurred = detail(&captures[1]);
        assert!(
            blurred < sharp * 0.8,
            "{appearance} inactive content must lose fine detail, not just dim: sharp {sharp:.3}, blurred {blurred:.3}"
        );
    }
}
