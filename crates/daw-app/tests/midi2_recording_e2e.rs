//! Recording UI → timestamped UMP input → editable song → isolated native VST3.
//! The virtual device shares the production capture queue and audio callback;
//! the instrument is a compiled VST3, rather than a protocol-worker substitute.
mod common;

#[allow(dead_code)]
#[path = "../../daw-plugins/tests/common/editor_probe_fixture.rs"]
mod editor_probe_fixture;

use common::Harness;
use daw_core::{
    AudioData, Clip, ClipContent, Instrument, Midi2NoteData, MidiInputChannel, MidiNoteExpression,
    Note, Project, UmpPacket,
};
use editor_probe_fixture::{CLASS_ID, EditorProbeFixture};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const BEAT_FRAMES: f64 = 24_000.0;

fn project(state: &Value) -> Project {
    serde_json::from_value(state["project"].clone()).unwrap()
}

fn setup(fixture: Option<&EditorProbeFixture>) -> (Harness, u64) {
    let mut app = Harness::with_virtual_audio(fixture.map(|fixture| fixture.path.as_path()), None);
    let mut song = Project::new();
    song.tempo = 120.0;
    let track = song.add_midi_track("Expressive input");
    song.track_mut(track).unwrap().gain = 1.0;
    if let Some(fixture) = fixture {
        song.set_instrument(
            track,
            Instrument::Vst3 {
                path: fixture.path.to_string_lossy().into_owned(),
                class_id: Some(CLASS_ID.into()),
            },
        )
        .unwrap();
    }
    let path = app.directory.path().join("ump-recording.json");
    song.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    app.command(json!({"command":"viewport","width":1280,"height":800}));
    app.click("Expressive input");
    (app, track)
}

fn menu(app: &mut Harness, parent: &str, action: &str) -> Value {
    app.click(parent);
    app.click(action)
}

fn recording(app: &mut Harness) -> Value {
    menu(app, "Window", "Recording")
}

// The recording body's scrolling remains an actual mouse interaction. This
// also checks the new capability/group rows are reachable in a real popup.
fn expose(app: &mut Harness, text: &str) {
    for attempt in 0..18 {
        let state = app.inspect();
        let window = state["window_rects"].as_array().unwrap().last().unwrap();
        if state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| {
                label["text"] == text
                    && label["min"][1].as_f64().unwrap() >= window["min"][1].as_f64().unwrap()
                    && label["max"][1].as_f64().unwrap() <= window["max"][1].as_f64().unwrap()
                    && label["max"][1].as_f64().unwrap() - label["min"][1].as_f64().unwrap()
                        >= f64::from(daw_ui::tokens::FONT_BODY)
            })
        {
            return;
        }
        app.command(
            json!({"command":"scroll","delta":if attempt == 0 {10000.0} else {-55.0},
            "x":window["max"][0].as_f64().unwrap()-32.0,
            "y":window["min"][1].as_f64().unwrap()+140.0}),
        );
        for _ in 0..12 {
            app.inspect();
        }
    }
    panic!("Recording control {text:?} cannot be reached");
}

fn select(app: &mut Harness, current: &str, choice: &str, prefix: &str) {
    expose(app, current);
    app.click(current);
    for _ in 0..8 {
        let state = app.inspect();
        if state["visible_labels"]
            .as_array()
            .unwrap()
            .contains(&json!(choice))
        {
            break;
        }
        let row = state["label_rects"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|row| {
                row["text"]
                    .as_str()
                    .is_some_and(|text| text.starts_with(prefix))
            })
            .expect("Open input picker");
        app.command(json!({"command":"scroll","delta":-180.0,
            "x":(row["min"][0].as_f64().unwrap()+row["max"][0].as_f64().unwrap())*0.5,
            "y":(row["min"][1].as_f64().unwrap()+row["max"][1].as_f64().unwrap())*0.5}));
    }
    app.click(choice);
}

fn configure(app: &mut Harness, track: u64) -> Value {
    recording(app);
    app.click("No MIDI input");
    let selected = app.click("Rill virtual MIDI input");
    assert_eq!(selected["recording"]["midi_ports"][0]["ump_input"], true);
    assert_eq!(selected["recording"]["midi_ports"][0]["source_protocol"], 2);
    expose(app, "MIDI 2.0 · UMP");
    select(app, "All channels", "Channel 5", "Channel ");
    select(app, "Group 1", "Group 3", "Group ");
    expose(app, "Arm recording");
    let state = app.click("Arm recording");
    let song = project(&state);
    assert_eq!(song.track(track).unwrap().midi_input_group, 2);
    assert_eq!(
        song.track(track).unwrap().midi_input,
        Some(MidiInputChannel::Channel { channel: 4 })
    );
    app.key("Escape")
}

fn wait(app: &mut Harness, phase: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let state = app.inspect();
        if state["recording"]["phase"] == phase {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "Recording did not settle: {state}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn start(app: &mut Harness) {
    app.click("Record");
    let state = wait(app, "recording");
    assert_eq!(state["error"], false, "{}", state["status"]);
}

fn stop(app: &mut Harness) -> Value {
    app.click("Stop");
    let state = wait(app, "idle");
    assert_eq!(state["error"], false, "{}", state["status"]);
    state
}

fn words(group: u8, channel: u8, status: u8, index: u8, detail: u8, data: u32) -> [u32; 2] {
    [
        0x4000_0000
            | u32::from(group) << 24
            | u32::from(status) << 20
            | u32::from(channel) << 16
            | u32::from(index) << 8
            | u32::from(detail),
        data,
    ]
}

fn ump(app: &mut Harness, frame: u32, packet: [u32; 2]) {
    app.command(json!({"command":"midi_ump_input","words":packet,"offset_frames":frame}));
}

fn pump(app: &mut Harness, frames: usize) {
    app.command(json!({"command":"pump_audio","frames":frames}));
}

fn notes(clip: &Clip) -> &[Note] {
    let ClipContent::Midi { notes } = &clip.content else {
        panic!("Expected MIDI clip")
    };
    notes
}

fn frame(beat: f64) -> usize {
    let value = beat * BEAT_FRAMES;
    assert!((value - value.round()).abs() < 1e-6);
    value.round() as usize
}

fn export(app: &mut Harness, name: &str) -> AudioData {
    let path = app.directory.path().join(name);
    app.command(json!({"command":"export","path":path}));
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}

#[test]
fn group_and_channel_capture_keeps_precise_notes_expressions_undo_archive_and_native_polyphony() {
    let fixture = EditorProbeFixture::build_expressive();
    let (mut app, track) = setup(Some(&fixture));
    let before = configure(&mut app, track)["project"].clone();
    start(&mut app);
    // Both notes sound at 60.5 but retain independent wire addresses and voices.
    let pitch = 60 * 512 + 256;
    ump(&mut app, 100, words(2, 4, 9, 72, 3, 32768 << 16 | pitch));
    ump(&mut app, 300, words(2, 4, 9, 73, 3, 32769 << 16 | pitch));
    ump(&mut app, 500, words(2, 4, 10, 72, 0, 0x4000_0001));
    ump(&mut app, 700, words(2, 4, 10, 72, 0, 0x4000_0002));
    // Pitch bend needs a receiver range; preserve its exact packet, without
    // pretending a generic VST receiver has agreed that range.
    ump(&mut app, 900, words(2, 4, 6, 72, 0, 0x8000_0001));
    ump(&mut app, 1100, words(2, 4, 10, 73, 0, 0x8000_0001));
    ump(&mut app, 1300, words(2, 4, 6, 73, 0, 0x8000_0002));
    ump(&mut app, 1700, words(2, 4, 8, 72, 1, 11113 << 16 | 1234));
    ump(&mut app, 1900, words(2, 4, 8, 73, 2, 11114 << 16 | 2345));
    for (group, channel) in [(1, 4), (2, 5)] {
        ump(&mut app, 100, words(group, channel, 9, 90, 0, 65535 << 16));
        ump(&mut app, 2000, words(group, channel, 8, 90, 0, 0));
    }
    pump(&mut app, 2400);
    let captured = stop(&mut app);
    let song = project(&captured);
    let clips = &song.track(track).unwrap().clips;
    assert_eq!(clips.len(), 1);
    let captured_notes = notes(&clips[0]);
    assert_eq!(
        captured_notes.len(),
        2,
        "Other groups/channels cannot enter the take"
    );
    assert_eq!(captured_notes[0].velocity, captured_notes[1].velocity);
    for (index, expected_start, expected_end, velocity, release, expression_data) in [
        (
            0,
            100,
            1700,
            32768,
            11113,
            vec![0x4000_0001, 0x4000_0002, 0x8000_0001],
        ),
        (1, 300, 1900, 32769, 11114, vec![0x8000_0001, 0x8000_0002]),
    ] {
        let note = &captured_notes[index];
        assert_eq!(frame(note.start), expected_start);
        assert_eq!(frame(note.start + note.length), expected_end);
        assert_eq!(note.pitch_semitones(), 60.5);
        let data = note.midi2.as_ref().unwrap();
        assert_eq!(
            (data.protocol, data.group, data.channel, data.note_index),
            (2, 2, 4, 72 + index as u8)
        );
        assert_eq!((data.velocity, data.release_velocity), (velocity, release));
        assert_eq!(
            (data.attribute_type, data.attribute_data),
            (3, pitch as u16)
        );
        assert_eq!(
            (data.release_attribute_type, data.release_attribute_data),
            if index == 0 { (1, 1234) } else { (2, 2345) }
        );
        assert_eq!(
            data.expressions
                .iter()
                .map(|point| point.packet.words[1])
                .collect::<Vec<_>>(),
            expression_data
        );
        for point in &data.expressions {
            let message = point.packet.midi2_channel_voice().unwrap();
            assert_eq!(
                (message.group, message.channel, message.index),
                (2, 4, 72 + index as u8)
            );
        }
    }
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before);
    assert_eq!(
        menu(&mut app, "Edit", "Redo")["project"],
        captured["project"]
    );
    let rendered = export(&mut app, "precise-recording.wav");
    let v1 = 32768.0 / 65535.0;
    let v2 = 32769.0 / 65535.0;
    for (at, expected) in [
        (200, v1 * 0.5),
        (400, (v1 + v2) * 0.5),
        (800, (v1 * 0.25 + v2) * 0.5),
        (1500, (v1 * 0.25 + v2 * 0.5) * 0.5),
        (1800, v2 * 0.5 * 0.5),
        (2100, 0.0),
    ] {
        assert!(
            (f64::from(rendered.samples[at * 2]) - expected).abs() < 1e-6,
            "Native VST3 frame {at} must preserve precise velocity and independently addressed expression"
        );
        assert_eq!(
            rendered.samples[at * 2 + 1],
            0.0,
            "Native channel5 maps to the left leg"
        );
    }
    let archive = app.directory.path().join("precise-capture.rill");
    app.command(json!({"command":"save","path":archive}));
    app = app.restart();
    let recalled = app.command(json!({"command":"open","path":archive}));
    assert_eq!(recalled["project"], captured["project"]);
    assert_eq!(recalled["recording"]["armed"], json!([]));
    assert_eq!(
        export(&mut app, "precise-recalled.wav").samples,
        rendered.samples
    );
    // Input group affects future input, never the group encoded in the song.
    app.click("Expressive input");
    recording(&mut app);
    select(&mut app, "Group 3", "Group 4", "Group ");
    app.key("Escape");
    assert_eq!(
        export(&mut app, "new-input-group.wav").samples,
        rendered.samples
    );
    app.quit();
}

#[test]
fn controller_only_ump_capture_is_editable_undoable_and_portable() {
    let (mut app, track) = setup(None);
    let before = configure(&mut app, track)["project"].clone();
    start(&mut app);
    let controllers = [
        words(2, 4, 11, 74, 0, 0x1234_5678),
        words(2, 4, 13, 0, 0, 0x89ab_cdef),
        words(2, 4, 14, 0, 0, 0x8000_0001),
    ];
    for (index, packet) in controllers.into_iter().enumerate() {
        ump(&mut app, 100 + index as u32 * 100, packet);
    }
    ump(&mut app, 150, words(3, 4, 11, 74, 0, u32::MAX));
    ump(&mut app, 250, words(2, 3, 11, 74, 0, u32::MAX));
    pump(&mut app, 1000);
    let captured = stop(&mut app);
    let song = project(&captured);
    let clips = &song.track(track).unwrap().clips;
    assert_eq!(
        clips.len(),
        1,
        "A controller-only performance still creates a clip"
    );
    assert!(notes(&clips[0]).is_empty());
    assert_eq!(clips[0].midi_events.len(), 3);
    for (index, event) in clips[0].midi_events.iter().enumerate() {
        assert_eq!(frame(event.beat), 100 + index * 100);
        assert_eq!(&event.packet.words[..2], &controllers[index]);
    }
    assert_eq!(menu(&mut app, "Edit", "Undo")["project"], before);
    assert_eq!(
        menu(&mut app, "Edit", "Redo")["project"],
        captured["project"]
    );
    let archive = app.directory.path().join("controller-only.rill");
    app.command(json!({"command":"save","path":archive}));
    app = app.restart();
    assert_eq!(
        app.command(json!({"command":"open","path":archive}))["project"],
        captured["project"]
    );
    app.quit();
}

#[test]
fn malformed_packets_are_rejected_without_losing_the_active_take_or_valid_following_input() {
    let (mut app, track) = setup(None);
    configure(&mut app, track);
    start(&mut app);
    ump(&mut app, 100, words(2, 4, 9, 60, 0, 43001 << 16));
    for malformed in [
        vec![],
        vec![0x4294_3c00],
        vec![0x2294_3c00, 0],
        vec![0x4294_3c00, 0, 0, 0, 0],
    ] {
        let result = app.raw_command(json!({"command":"midi_ump_input","words":malformed}));
        assert_eq!(
            result["ok"], false,
            "Malformed UMP must not enter the input queue"
        );
        let state = app.inspect();
        assert_eq!(state["recording"]["phase"], "recording");
        assert_eq!(
            state["error"], false,
            "Validation failure cannot stop an active take"
        );
    }
    ump(&mut app, 700, words(2, 4, 8, 60, 0, 21001 << 16));
    ump(&mut app, 800, words(2, 4, 9, 62, 0, 51019 << 16));
    ump(&mut app, 1200, words(2, 4, 8, 62, 0, 31001 << 16));
    pump(&mut app, 1600);
    let state = stop(&mut app);
    let song = project(&state);
    let captured = notes(&song.track(track).unwrap().clips[0]);
    assert_eq!(captured.len(), 2);
    for (note, start, end, velocity, release) in [
        (&captured[0], 100, 700, 43001, 21001),
        (&captured[1], 800, 1200, 51019, 31001),
    ] {
        assert_eq!(
            (frame(note.start), frame(note.start + note.length)),
            (start, end)
        );
        let data = note.midi2.as_ref().unwrap();
        assert_eq!((data.velocity, data.release_velocity), (velocity, release));
    }
    app.quit();
}

#[test]
fn unsupported_controls_and_extra_groups_warn_without_mutating_or_discarding_performance() {
    let fixture = EditorProbeFixture::build_expressive();
    let (mut app, track) = setup(Some(&fixture));
    let mut song = project(&app.inspect());
    let bend = UmpPacket::from_words(&words(2, 4, 6, 72, 0, 0x8765_4321)).unwrap();
    let mut captured_notes = Vec::new();
    for (group, index, velocity) in [(2, 72, 43001), (3, 74, 51019)] {
        let on = UmpPacket::from_words(&words(
            group,
            4,
            9,
            index,
            3,
            velocity << 16 | (60 * 512 + 256),
        ))
        .unwrap();
        let mut data = Midi2NoteData::from_note_on(on.midi2_channel_voice().unwrap()).unwrap();
        data.release_velocity = 11113;
        if group == 2 {
            data.expressions.push(MidiNoteExpression::new(0.25, bend));
        }
        let mut note = Note::new(60, 0.0, 1.0, 80);
        note.set_midi2(data).unwrap();
        captured_notes.push(note);
    }
    song.track_mut(track).unwrap().clips.push(Clip::midi(
        "Retained performance",
        0.0,
        1.0,
        captured_notes,
    ));
    let source = app.directory.path().join("unsupported-performance.rill");
    song.save(&source).unwrap();
    let baseline = app.command(json!({"command":"open","path":source}));
    let has_label = |state: &Value, text: &str| {
        state["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label.as_str().is_some_and(|label| label.contains(text)))
    };
    let warning = "Extra MIDI groups require separate tracks";
    app.click("Play");
    common::wait_until(&mut app, |state| {
        state["engine"]["playing"] == true && state["playback_loading"] == false
    });
    pump(&mut app, 2048);
    let warned = common::wait_until(&mut app, |state| {
        state["recording"]["midi"]["unsupported_events"]
            .as_u64()
            .unwrap()
            > 0
            && state["recording"]["midi"]["unsupported_groups"]
                .as_u64()
                .unwrap()
                > 0
            && has_label(state, warning)
    });
    assert_eq!(
        warned["error"], false,
        "Unsupported controls are a capability advisory"
    );
    assert!(has_label(&warned, "Performance data retained"));
    assert_eq!(warned["project"], baseline["project"]);
    assert_eq!(warned["history"], baseline["history"]);
    app.click("Dismiss");
    for _ in 0..4 {
        pump(&mut app, 512);
        let acknowledged = app.inspect();
        assert_eq!(
            acknowledged["recording"]["midi"]["unsupported_events"],
            warned["recording"]["midi"]["unsupported_events"]
        );
        assert_eq!(
            acknowledged["recording"]["midi"]["unsupported_groups"],
            warned["recording"]["midi"]["unsupported_groups"]
        );
        assert!(
            !has_label(&acknowledged, warning),
            "An unchanged advisory must stay dismissed while playback continues"
        );
        assert_eq!(acknowledged["project"], baseline["project"]);
        assert_eq!(acknowledged["history"], baseline["history"]);
    }
    app.click("Stop");
    let rendered = export(&mut app, "unsupported-controls.wav");
    assert!(
        rendered.samples.iter().any(|sample| sample.abs() > 0.1),
        "Supported notes still render through the actual native instrument"
    );
    let completed = app.inspect();
    let report = &completed["finishing"]["last_export"]["report"];
    assert!(report["unsupported_midi_events"].as_u64().unwrap() > 0);
    assert!(report["unsupported_midi_groups"].as_u64().unwrap() > 0);
    assert!(
        has_label(&completed, "Some MIDI controls are unsupported"),
        "The completed WAV must disclose its MIDI playback limitations"
    );
    assert_eq!(completed["project"], baseline["project"]);
    assert_eq!(completed["history"], baseline["history"]);
    let archive = app.directory.path().join("retained-unsupported.rill");
    app.command(json!({"command":"save","path":archive}));
    app = app.restart();
    let recalled = app.command(json!({"command":"open","path":archive}));
    assert_eq!(
        recalled["project"], baseline["project"],
        "An unsupported packet and the additional group must survive portable recall exactly"
    );
    app.quit();
}
