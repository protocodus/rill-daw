//! Direct arranger editing and reusable automation through real app input.
mod common;
use common::Harness;
use daw_core::{AudioData, AutomationParameter, AutomationPoint, AutomationTarget, Clip, Project};
use serde_json::{Value, json};

fn lane(state: &Value) -> &Value {
    &state["project"]["tracks"][2]["automation"]
}
fn diagram(state: &Value) -> &Value {
    &state["timeline"]["automation"][0]
}
fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) / 2.0,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) / 2.0,
    )
}
fn point(state: &Value, beat: f64, value: f64) -> (f64, f64) {
    let rect = &diagram(state)["rect"];
    (
        rect["min"][0].as_f64().unwrap()
            + (beat - state["timeline"]["start_beat"].as_f64().unwrap())
                * state["timeline"]["pixels_per_beat"].as_f64().unwrap(),
        rect["max"][1].as_f64().unwrap()
            - value * (rect["max"][1].as_f64().unwrap() - rect["min"][1].as_f64().unwrap()),
    )
}
fn click(app: &mut Harness, at: (f64, f64), button: &str) -> Value {
    app.command(json!({"command":"pointer","phase":"click","x":at.0,"y":at.1,"button":button}))
}
fn pattern_header(state: &Value, index: usize) -> (f64, f64) {
    let rect = &diagram(state)["clips"][index]["rect"];
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) / 2.0,
        rect["min"][1].as_f64().unwrap() + 10.0,
    )
}
fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export","path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}
fn sample(audio: &AudioData, beat: f64, expected: f32) {
    let frame = (beat * 24000.0).round() as usize;
    assert!(
        (audio.samples[frame * 2] - expected).abs() < 1e-4,
        "beat {beat}: {} != {expected}",
        audio.samples[frame * 2]
    );
}

#[test]
fn pattern_copy_paste_derivation_and_both_trim_edges_preserve_source_points() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let target = project.add_midi_track("Source");
    project.add_midi_track("Other");
    let track = project
        .add_automation_track(
            "Curve",
            AutomationTarget {
                track: target,
                parameter: AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    project.track_mut(track).unwrap().collapsed = false;
    let original = project.create_automation_clip(track, 1.0, 2.0).unwrap();
    let path = app.directory.path().join("pattern-tools.daw.json");
    project.save(&path).unwrap();
    let initial = app.command(json!({"command":"open","path":path}));
    click(&mut app, pattern_header(&initial, 0), "secondary");
    let ghost = app.click("Ghost copy");
    click(&mut app, pattern_header(&ghost, 1), "secondary");
    app.click("Make independent");
    let independent = app.click("Apply and disconnect");
    assert!(lane(&independent)["clips"][1]["link_group"].is_null());
    // Points are drawn with Pencil, as notes are in the piano roll.
    app.click("Pencil");
    let plot = &diagram(&independent)["clips"][1]["plot"];
    let x = (plot["min"][0].as_f64().unwrap() + plot["max"][0].as_f64().unwrap()) / 2.0;
    let y = plot["max"][1].as_f64().unwrap()
        - 0.8 * (plot["max"][1].as_f64().unwrap() - plot["min"][1].as_f64().unwrap());
    let derived = click(&mut app, (x, y), "primary");
    assert_eq!(
        lane(&derived)["clips"][0]["points"],
        lane(&initial)["clips"][0]["points"]
    );
    assert_eq!(
        lane(&derived)["clips"][1]["points"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let at = center(&diagram(&derived)["clips"][1]["points"][1]["rect"]);
    click(&mut app, at, "secondary");
    let smooth = app.click("Smooth segment");
    assert_eq!(
        lane(&smooth)["clips"][1]["points"][1]["interpolation"],
        "Smooth"
    );
    click(&mut app, pattern_header(&smooth, 0), "secondary");
    app.click("Copy");
    let state = app.inspect();
    let at = point(&state, 8.0, 0.7);
    click(&mut app, at, "secondary");
    let pasted = app.click("Paste automation clip");
    assert_eq!(lane(&pasted)["clips"][2]["start"], 8.0);
    assert!(lane(&pasted)["clips"][2]["link_group"].is_null());
    click(&mut app, pattern_header(&pasted, 0), "secondary");
    let copied = app.click("Duplicate after");
    let duplicate = lane(&copied)["clips"][3]["id"].as_u64().unwrap();
    assert_ne!(duplicate, original);
    app.click("Select");
    let header = pattern_header(&copied, 2);
    let ppb = copied["timeline"]["pixels_per_beat"].as_f64().unwrap();
    let moved=app.command(json!({"command":"pointer","phase":"drag","x":header.0,"y":header.1,"to_x":header.0+3.0*ppb,"to_y":header.1}));
    let rect = &diagram(&moved)["clips"][2]["rect"];
    let at = (
        rect["min"][0].as_f64().unwrap() + 2.0,
        rect["min"][1].as_f64().unwrap() + 10.0,
    );
    let trimmed=app.command(json!({"command":"pointer","phase":"drag","x":at.0,"y":at.1,"to_x":at.0+0.25*ppb,"to_y":at.1}));
    assert_eq!(lane(&trimmed)["clips"][3]["start"], 6.25);
    assert_eq!(lane(&trimmed)["clips"][3]["offset"], 0.25);
    assert_eq!(lane(&trimmed)["clips"][3]["length"], 1.75);
    let rect = &diagram(&trimmed)["clips"][2]["rect"];
    let at = (
        rect["max"][0].as_f64().unwrap() - 2.0,
        rect["min"][1].as_f64().unwrap() + 10.0,
    );
    let extended=app.command(json!({"command":"pointer","phase":"drag","x":at.0,"y":at.1,"to_x":at.0+0.5*ppb,"to_y":at.1}));
    assert_eq!(lane(&extended)["clips"][3]["length"], 2.25);
    assert_eq!(
        lane(&extended)["clips"][3]["points"],
        lane(&copied)["clips"][3]["points"]
    );
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undone["project"], trimmed["project"]);
    let state = app.inspect();
    click(&mut app, pattern_header(&state, 2), "secondary");
    let removed = app.click("Delete");
    assert_eq!(lane(&removed)["clips"].as_array().unwrap().len(), 3);
    app.quit();
}

#[test]
fn points_patterns_ghosts_placement_precedence_undo_and_archive_work_in_arranger() {
    let mut app = Harness::new();
    let source = app.directory.path().join("automation-source.wav");
    AudioData {
        sample_rate: 48000,
        channels: 1,
        samples: vec![0.4; 288000],
    }
    .write_wav(&source)
    .unwrap();
    let original_bytes = std::fs::read(&source).unwrap();
    let mut project = Project::new();
    project.tempo = 120.0;
    let channel = project.add_audio_track("Automated source");
    project.track_mut(channel).unwrap().gain = 0.9;
    project.track_mut(channel).unwrap().clips.push(Clip::audio(
        "Source",
        0.0,
        12.0,
        source.to_string_lossy(),
    ));
    let other = project.add_midi_track("Other channel");
    let automation = project
        .add_automation_track(
            "Source gain",
            AutomationTarget {
                track: channel,
                parameter: AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    project.track_mut(automation).unwrap().collapsed = false;
    project.automation_lane_mut(automation).unwrap().points = vec![
        AutomationPoint::new(0.0, 0.2),
        AutomationPoint::new(8.0, 0.2),
    ];
    let path = app.directory.path().join("arranger-automation.daw.json");
    project.save(&path).unwrap();
    let before = app.command(json!({"command":"open","path":path}));
    // The lane follows its target even though another channel precedes it in storage.
    let order = before["timeline"]["track_colors"].as_array().unwrap();
    assert_eq!(
        order
            .iter()
            .map(|row| row["track"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![channel, automation, other]
    );
    let at = point(&before, 1.0, 0.6);
    let selecting = click(&mut app, at, "primary");
    assert_eq!(
        selecting["project"], before["project"],
        "only Pencil draws arranger points"
    );
    app.click("Pencil");
    let added = click(&mut app, at, "primary");
    assert_eq!(lane(&added)["points"].as_array().unwrap().len(), 3);
    let undo = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(undo["project"], before["project"]);
    let at = point(&undo, 2.0, 0.7);
    click(&mut app, at, "secondary");
    let created = app.click("Insert clip");
    assert_eq!(lane(&created)["clips"].as_array().unwrap().len(), 1);
    assert_eq!(lane(&created)["clips"][0]["start"], 2.0);
    let plot = &diagram(&created)["clips"][0]["plot"];
    let x =
        plot["min"][0].as_f64().unwrap() + created["timeline"]["pixels_per_beat"].as_f64().unwrap();
    let y = plot["max"][1].as_f64().unwrap()
        - 0.8 * (plot["max"][1].as_f64().unwrap() - plot["min"][1].as_f64().unwrap());
    let edited = click(&mut app, (x, y), "primary");
    assert_eq!(
        lane(&edited)["clips"][0]["points"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let at = center(&diagram(&edited)["clips"][0]["points"][1]["rect"]);
    click(&mut app, at, "secondary");
    let stepped = app.click("Step segment");
    assert_eq!(
        lane(&stepped)["clips"][0]["points"][1]["interpolation"],
        "Step"
    );
    let rendered = export(&mut app, "precedence.wav");
    sample(&rendered, 1.0, 0.12);
    sample(&rendered, 2.5, 0.3);
    sample(&rendered, 3.5, 0.48);
    sample(&rendered, 6.5, 0.12);
    click(&mut app, pattern_header(&stepped, 0), "secondary");
    let copied = app.click("Ghost copy");
    assert_eq!(lane(&copied)["clips"].as_array().unwrap().len(), 2);
    assert_eq!(
        lane(&copied)["clips"][0]["link_group"],
        lane(&copied)["clips"][1]["link_group"]
    );
    let at = pattern_header(&copied, 1);
    let ppb = copied["timeline"]["pixels_per_beat"].as_f64().unwrap();
    app.click("Select");
    let moved=app.command(json!({"command":"pointer","phase":"drag","x":at.0,"y":at.1,"to_x":at.0+2.0*ppb,"to_y":at.1}));
    assert_eq!(lane(&moved)["clips"][1]["start"], 8.0);
    app.click("Pencil");
    let at = center(&diagram(&moved)["clips"][0]["points"][1]["rect"]);
    let unchanged = click(&mut app, at, "primary");
    assert_eq!(
        unchanged["project"], moved["project"],
        "a point click without movement is not an edit"
    );
    assert_eq!(unchanged["history"], moved["history"]);
    let plot = &diagram(&moved)["clips"][0]["plot"];
    let delta = (plot["max"][1].as_f64().unwrap() - plot["min"][1].as_f64().unwrap()) * 0.2;
    let shared = app.command(
        json!({"command":"pointer","phase":"drag","x":at.0,"y":at.1,"to_x":at.0,"to_y":at.1+delta}),
    );
    for index in 0..2 {
        assert!(
            (lane(&shared)["clips"][index]["points"][1]["value"]
                .as_f64()
                .unwrap()
                - 0.6)
                .abs()
                < 0.01
        );
    }
    let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
    assert_eq!(
        undone["project"], moved["project"],
        "one undo restores the complete precise drag"
    );
    let redone = app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));
    assert_eq!(redone["project"], shared["project"]);
    // Clearing arranger points falls through to the manual gain outside clips.
    let state = app.inspect();
    let at = point(&state, 1.0, 0.5);
    click(&mut app, at, "secondary");
    app.click("Clear arranger points");
    let default = export(&mut app, "default.wav");
    sample(&default, 1.0, 0.36);
    sample(&default, 3.5, 0.36);
    sample(&default, 9.5, 0.36);
    let archive = app.directory.path().join("automation-patterns.rill");
    app.command(json!({"command":"save","path":archive}));
    let recalled = app.command(json!({"command":"open","path":archive}));
    assert_eq!(lane(&recalled)["clips"], lane(&shared)["clips"]);
    assert_eq!(default.samples, export(&mut app, "recalled.wav").samples);
    assert_eq!(std::fs::read(source).unwrap(), original_bytes);
    app.quit();
}

#[test]
fn automation_starts_compact_and_saved_expansion_preserves_the_curve_and_sound() {
    let mut app = Harness::new();
    let source = app.directory.path().join("source.wav");
    AudioData {
        sample_rate: 8_000,
        channels: 1,
        samples: vec![0.4; 16_000],
    }
    .write_wav(&source)
    .unwrap();
    let mut project = Project::new();
    let channel = project.add_audio_track("Voice");
    project.track_mut(channel).unwrap().clips.push(Clip::audio(
        "Voice clip",
        0.0,
        4.0,
        source.to_string_lossy(),
    ));
    let automation = project
        .add_automation_track(
            "Voice volume",
            AutomationTarget {
                track: channel,
                parameter: AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    project
        .create_automation_clip(automation, 0.0, 4.0)
        .unwrap();
    let path = app.directory.path().join("automation-heights.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    let compact = app.set_track_density("Detailed");
    let height = |state: &Value| {
        let rect = &state["timeline"]["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["track"] == automation)
            .unwrap()["header"];
        rect["max"][1].as_f64().unwrap() - rect["min"][1].as_f64().unwrap()
    };
    assert_eq!(compact["project"]["tracks"][1]["collapsed"], true);
    let compact_audio = export(&mut app, "compact.wav");
    let expanded = app.click_track_disclosure(automation);
    assert_eq!(expanded["project"]["tracks"][1]["collapsed"], false);
    assert!(height(&expanded) > height(&compact));
    assert_eq!(
        expanded["project"]["tracks"][1]["automation"],
        compact["project"]["tracks"][1]["automation"]
    );
    assert!(height(&compact) <= 22.0);
    assert_eq!(
        diagram(&expanded)["clips"][0]["id"],
        compact["project"]["tracks"][1]["automation"]["clips"][0]["id"]
    );
    assert_eq!(
        export(&mut app, "expanded.wav").samples,
        compact_audio.samples
    );

    app.command(json!({"command":"save","path":path}));
    let reopened = app.command(json!({"command":"open","path":path}));
    assert_eq!(
        reopened["project"]["tracks"][1],
        expanded["project"]["tracks"][1]
    );
    assert_eq!(height(&reopened), height(&expanded));
    let collapsed = app.click_track_disclosure(automation);
    assert_eq!(
        collapsed["project"]["tracks"][1],
        compact["project"]["tracks"][1]
    );
    assert_eq!(height(&collapsed), height(&compact));
    app.quit();
}

#[test]
fn linked_automation_stretch_asks_once_and_applies_the_chosen_scope() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let channel = project.add_midi_track("Source");
    project.add_midi_track("Other");
    let track = project
        .add_automation_track(
            "Curve",
            AutomationTarget {
                track: channel,
                parameter: AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    project.track_mut(track).unwrap().collapsed = false;
    let clip = project.create_automation_clip(track, 0.0, 2.0).unwrap();
    project
        .copy_automation_clip(track, clip, 6.0, true)
        .unwrap();
    project
        .copy_automation_clip(track, clip, 10.0, true)
        .unwrap();
    let path = app.directory.path().join("ghost-scope.daw.json");
    project.save(&path).unwrap();
    let initial = app.command(json!({"command":"open","path":path}));
    for choice in [
        "Cancel",
        "Propagate change",
        "Apply and disconnect",
        "Apply and delete all copies",
    ] {
        let current = app.inspect();
        click(&mut app, pattern_header(&current, 0), "secondary");
        let stretch = app.click("Stretch");
        let at = pattern_header(&stretch, 0);
        let ppb = stretch["timeline"]["pixels_per_beat"].as_f64().unwrap();
        let prompt = app.command(json!({"command":"pointer","phase":"drag","x":at.0,"y":at.1,"to_x":at.0+ppb,"to_y":at.1}));
        assert_eq!(prompt["project"], initial["project"]);
        assert_eq!(prompt["history"]["undo"], initial["history"]["undo"]);
        let applied = app.click(choice);
        let clips = lane(&applied)["clips"].as_array().unwrap();
        if choice == "Cancel" {
            assert_eq!(applied["project"], initial["project"]);
            assert_eq!(applied["history"]["undo"], initial["history"]["undo"]);
            continue;
        }
        assert_eq!(clips[0]["length"], 3.0);
        if choice == "Propagate change" {
            assert_eq!(clips.len(), 3);
            assert_eq!(clips[1]["start"], 6.0);
            assert_eq!(clips[2]["start"], 10.0);
            for peer in clips {
                assert_eq!(peer["length"], 3.0);
                assert_eq!(peer["link_group"], lane(&initial)["clips"][0]["link_group"]);
                assert_eq!(peer["points"][1]["beat"], 3.0);
            }
        } else {
            assert!(clips[0]["link_group"].is_null());
            if choice == "Apply and disconnect" {
                assert_eq!(clips[1..], lane(&initial)["clips"].as_array().unwrap()[1..]);
            } else {
                assert_eq!(clips.len(), 1);
            }
        }
        assert_eq!(applied["history"]["undo"].as_array().unwrap().len(), 1);
        let undone = app.command(json!({"command":"key","key":"Z","cmd":true}));
        assert_eq!(undone["project"], initial["project"]);
    }
    app.quit();
}

#[test]
fn panning_an_automation_clip_keeps_its_paint_and_hit_targets_inside_the_grid() {
    let mut app = Harness::new();
    let mut project = Project::new();
    let channel = project.add_midi_track("Source");
    project.add_midi_track("Other");
    let track = project
        .add_automation_track(
            "Curve",
            AutomationTarget {
                track: channel,
                parameter: AutomationParameter::TrackGain,
            },
        )
        .unwrap();
    project.track_mut(track).unwrap().collapsed = false;
    project.create_automation_clip(track, 0.0, 6.0).unwrap();
    let path = app.directory.path().join("panned-automation.daw.json");
    project.save(&path).unwrap();
    let initial = app.command(json!({"command":"open","path":path}));
    let (x, y) = center(&diagram(&initial)["rect"]);
    let panned = app.command(json!({"command":"pointer","phase":"drag","button":"middle","x":x,"y":y,"to_x":x-80.0,"to_y":y}));
    assert!(panned["timeline"]["start_beat"].as_f64().unwrap() > 0.0);
    let plot_left = diagram(&panned)["rect"]["min"][0].as_f64().unwrap();
    let clip = &diagram(&panned)["clips"][0];
    for region in ["rect", "title_rect", "plot"] {
        assert!(clip[region]["min"][0].as_f64().unwrap() >= plot_left);
    }
    let paint = app.command(json!({"command":"inspect","paint":true}));
    let edge = clip["plot"]["max"][0].as_f64().unwrap();
    let path = paint["painted_paths"]
        .as_array()
        .unwrap()
        .iter()
        .find(|path| {
            let points = path["points"].as_array().unwrap();
            points.len() > 2
                && points[0][0].as_f64().unwrap() < plot_left - 1.0
                && (points.last().unwrap()[0].as_f64().unwrap() - edge).abs() < 1.0
        })
        .expect("partially offscreen automation curve was painted");
    assert!(path["clip"]["min"][0].as_f64().unwrap() >= plot_left);
    let header = click(
        &mut app,
        (plot_left - 35.0, center(&clip["title_rect"]).1),
        "secondary",
    );
    let labels = header["visible_labels"].as_array().unwrap();
    assert!(!labels.iter().any(|label| label == "Duplicate after"));
    assert_eq!(header["project"], initial["project"]);
    app.quit();
}
