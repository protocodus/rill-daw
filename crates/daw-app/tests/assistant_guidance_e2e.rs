//! Discoverable AI guidance uses real controls without requiring a provider key.
//! Scripted providers exercise the production agent and shared song dispatcher.
mod common;

use common::Harness;
use daw_ai::{PROMPT_EXAMPLES, PromptCategory, PromptExample};
use daw_core::{Clip, Note, Project};
use daw_ui::ASSISTANT_PROMPT_LIMIT;
use serde_json::{Value, json};

fn example(id: &str) -> &'static PromptExample {
    PROMPT_EXAMPLES.iter().find(|item| item.id == id).unwrap()
}

fn visible(state: &Value, text: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| label == text)
}

fn footer_visible(state: &Value, text: &str) -> bool {
    let top = state["assistant"]["prompt_rect"]["max"][1]
        .as_f64()
        .unwrap();
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .any(|rect| rect["text"] == text && rect["min"][1].as_f64().is_some_and(|y| y >= top))
}

fn label<'a>(state: &'a Value, text: &str) -> &'a Value {
    state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|label| label["text"] == text)
        .unwrap_or_else(|| panic!("Missing {text:?}: {}", state["visible_labels"]))
}

fn unchanged(actual: &Value, expected: &Value) {
    for field in [
        "project",
        "history",
        "song_session",
        "selected_track",
        "selected_clip",
        "selected_clips",
        "selection_span",
        "modified",
    ] {
        assert_eq!(actual[field], expected[field], "Changed {field}");
    }
}

fn focus_prompt(app: &mut Harness) -> Value {
    let state = app.inspect();
    let rect = &state["assistant"]["prompt_rect"];
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5;
    app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}))
}

fn replace_prompt(app: &mut Harness, text: &str) -> Value {
    focus_prompt(app);
    app.command(json!({"command":"key","key":"A","cmd":true}));
    app.command(json!({"command":"text","text":text}))
}

fn song_tool(app: &mut Harness, name: &str, arguments: Value) -> Value {
    let state = app.command(json!({"command":"song_tool","name":name,"arguments":arguments}));
    assert_eq!(state["tool_result"]["ok"], true, "{state}");
    state
}

fn insert_example(app: &mut Harness, item: &PromptExample) -> Value {
    let scroll = |app: &mut Harness, state: &Value, delta: f64| {
        // Aim between the fixed tabs and composer, inside the real browsing
        // body. Later examples may require a wheel gesture at generous gaps.
        let prompt = &state["assistant"]["prompt_rect"];
        let x = (prompt["min"][0].as_f64().unwrap() + prompt["max"][0].as_f64().unwrap()) * 0.5;
        let y = (label(state, "Tips")["max"][1].as_f64().unwrap()
            + prompt["min"][1].as_f64().unwrap())
            * 0.5;
        let scrolled = app.command(json!({"command":"scroll","x":x,"y":y,"delta":delta}));
        assert_eq!(
            scrolled["assistant"]["prompt_rect"], *prompt,
            "Browsing examples must not displace the composer"
        );
        scrolled
    };
    let mut state = app.click("Examples");
    if !visible(&state, item.category.label()) {
        state = scroll(app, &state, 10_000.0);
        assert!(visible(&state, item.category.label()));
    }
    state = app.click(item.category.label());
    for _ in 0..24 {
        if visible(&state, item.title) {
            break;
        }
        state = scroll(app, &state, -96.0);
    }
    assert!(
        visible(&state, item.title),
        "Example was not reachable by mouse: {}",
        item.title
    );
    app.click(item.title)
}

fn more(app: &mut Harness, action: &str) -> Value {
    app.click("More");
    app.click(action)
}

fn assert_controls_fit(state: &Value, width: f64, height: f64) {
    let primary = if state["assistant"]["submit_state"] == "SetupRequired" {
        "Connect AI"
    } else {
        "Send"
    };
    let mut controls = vec![
        "Conversation",
        "Examples",
        "Tools",
        "Tips",
        "AI settings",
        "More",
        "AI prompt",
        primary,
    ];
    controls.extend(
        ["Stop", "Undo", "Edit last prompt"]
            .into_iter()
            .filter(|name| footer_visible(state, name)),
    );
    for name in controls {
        let rect = label(state, name);
        let (left, top, right, bottom) = (
            rect["min"][0].as_f64().unwrap(),
            rect["min"][1].as_f64().unwrap(),
            rect["max"][0].as_f64().unwrap(),
            rect["max"][1].as_f64().unwrap(),
        );
        assert!(
            left >= 0.0 && top >= 0.0 && right <= width && bottom <= height,
            "{name} is clipped at {width}×{height}: {rect}"
        );
    }
    let prompt = &state["assistant"]["prompt_rect"];
    assert!(prompt["min"][0].as_f64().unwrap() >= 0.0);
    assert!(prompt["min"][1].as_f64().unwrap() >= 0.0);
    assert!(prompt["max"][0].as_f64().unwrap() <= width);
    assert!(prompt["max"][1].as_f64().unwrap() <= height);
}

#[test]
fn examples_and_tips_preserve_drafts_and_keep_actions_visible_in_both_themes() {
    for dark in [false, true] {
        let mut app = Harness::new();
        let original = app.inspect();
        app.command(json!({"command":"viewport","width":800,"height":600}));
        app.command(json!({"command":"system_theme","dark":dark}));
        app.click("AI");
        let start = app.click("Conversation");
        assert!(visible(&start, "Examples"));
        let browse = app.click("Examples");
        assert_eq!(browse["assistant"]["page"], "Examples");
        unchanged(&browse, &original);

        for category in PromptCategory::ALL {
            let chosen = PROMPT_EXAMPLES
                .iter()
                .find(|item| item.category == category)
                .unwrap();
            app.click("Conversation");
            let before = replace_prompt(&mut app, "Keep my existing idea.");
            let inserted = insert_example(&mut app, chosen);
            assert_eq!(
                inserted["assistant"]["prompt"],
                format!("Keep my existing idea.\n\n{}", chosen.prompt)
            );
            assert_eq!(inserted["assistant"]["page"], "Conversation");
            assert_eq!(inserted["assistant"]["busy"], false);
            assert_eq!(inserted["assistant"]["messages"], json!([]));
            unchanged(&inserted, &before);
            assert_controls_fit(&app.inspect(), 800.0, 600.0);
        }

        let before_tips = app.inspect();
        let tips = app.click("Tips");
        assert_eq!(tips["assistant"]["page"], "Tips");
        assert!(
            visible(&tips, "Better prompts, better music"),
            "{}",
            tips["visible_labels"]
        );
        assert!(visible(&tips, "Name the scope"));
        assert_controls_fit(&tips, 800.0, 600.0);
        unchanged(&tips, &before_tips);
        assert_eq!(
            tips["assistant"]["prompt"],
            before_tips["assistant"]["prompt"]
        );

        // Long help content remains mouse-accessible without moving the composer.
        let tip = label(&tips, "Name the scope");
        let x = (tip["min"][0].as_f64().unwrap() + tip["max"][0].as_f64().unwrap()) * 0.5;
        let y = tip["max"][1].as_f64().unwrap() + 12.0;
        let mut scrolled = tips;
        for _ in 0..24 {
            if visible(&scrolled, "Connect your provider") {
                break;
            }
            scrolled = app.command(json!({"command":"scroll","x":x,"y":y,"delta":-96.0}));
        }
        assert!(visible(&scrolled, "Connect your provider"));
        assert_controls_fit(&scrolled, 800.0, 600.0);
        unchanged(&scrolled, &before_tips);
        app.quit();
    }
}

#[test]
fn conversation_starters_preserve_drafts_and_never_send_in_both_themes() {
    for dark in [false, true] {
        let mut app = Harness::new();
        app.command(json!({"command":"viewport","width":800,"height":600}));
        app.command(json!({"command":"system_theme","dark":dark}));
        app.click("AI");
        let original = app.inspect();
        assert_eq!(original["assistant"]["page"], "Conversation");
        assert_controls_fit(&original, 800.0, 600.0);
        assert!(!footer_visible(&original, "Stop"));
        assert!(!footer_visible(&original, "Undo"));

        // The first view offers a useful range of existing musical examples.
        let starters: Vec<_> = PROMPT_EXAMPLES
            .iter()
            .filter(|item| visible(&original, item.title))
            .collect();
        assert_eq!(starters.len(), 3, "{}", original["visible_labels"]);
        for (index, item) in starters.iter().enumerate() {
            assert!(
                starters[..index]
                    .iter()
                    .all(|other| other.category != item.category)
            );
            let rect = label(&original, item.title);
            assert!(rect["min"][0].as_f64().unwrap() >= 0.0, "{rect}");
            assert!(rect["min"][1].as_f64().unwrap() >= 0.0, "{rect}");
            assert!(rect["max"][0].as_f64().unwrap() <= 800.0, "{rect}");
            assert!(rect["max"][1].as_f64().unwrap() <= 600.0, "{rect}");
            assert!(
                rect["max"][1].as_f64().unwrap()
                    <= label(&original, "AI prompt")["min"][1].as_f64().unwrap(),
                "Starter overlaps the composer: {rect}"
            );
        }

        // An armed scripted provider makes accidental submission observable.
        app.command(json!({"command":"assistant_script","turns":[
            {"text":"This response must remain unused until Send is clicked."}
        ]}));
        let mut expected = String::new();
        for item in &starters {
            if !expected.is_empty() {
                expected.push_str("\n\n");
            }
            expected.push_str(item.prompt);
            let inserted = app.click(item.title);
            assert_eq!(inserted["assistant"]["prompt"], expected);
            assert_eq!(inserted["assistant"]["page"], "Conversation");
            assert_eq!(inserted["assistant"]["busy"], false);
            assert_eq!(inserted["assistant"]["messages"], json!([]));
            assert_eq!(inserted["assistant"]["request_prompt"], "");
            assert_eq!(inserted["assistant"]["submit_state"], "Ready");
            unchanged(&inserted, &original);
            assert_controls_fit(&inserted, 800.0, 600.0);
        }

        let item = starters[0];
        let draft = "Keep my opening idea — and its dynamics.";
        replace_prompt(&mut app, draft);
        let appended = app.click(item.title);
        assert_eq!(
            appended["assistant"]["prompt"],
            format!("{draft}\n\n{}", item.prompt)
        );
        unchanged(&appended, &original);

        // Count characters, not UTF-8 bytes. A complete example fits exactly at
        // the limit, but never truncates an existing draft to make itself fit.
        let available = ASSISTANT_PROMPT_LIMIT - item.prompt.chars().count() - 2;
        let full_draft = "é".repeat(available + 1);
        replace_prompt(&mut app, &full_draft);
        let protected = app.click(item.title);
        assert_eq!(protected["assistant"]["prompt"], full_draft);
        assert_eq!(protected["assistant"]["busy"], false);
        assert_eq!(protected["assistant"]["messages"], json!([]));
        unchanged(&protected, &original);

        let fitting_draft = "é".repeat(available);
        replace_prompt(&mut app, &fitting_draft);
        let filled = app.click(item.title);
        assert_eq!(
            filled["assistant"]["prompt"],
            format!("{fitting_draft}\n\n{}", item.prompt)
        );
        assert_eq!(
            filled["assistant"]["prompt"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            ASSISTANT_PROMPT_LIMIT
        );
        assert_eq!(filled["assistant"]["busy"], false);
        assert_eq!(filled["assistant"]["messages"], json!([]));
        assert_eq!(filled["assistant"]["request_prompt"], "");
        assert_eq!(filled["assistant"]["submit_state"], "Ready");
        unchanged(&filled, &original);
        assert_controls_fit(&filled, 800.0, 600.0);

        let examples = app.click("Examples");
        assert_eq!(examples["assistant"]["page"], "Examples");
        assert_eq!(
            examples["assistant"]["prompt"],
            filled["assistant"]["prompt"]
        );
        unchanged(&examples, &original);
        app.quit();
    }
}

#[test]
fn editable_composition_example_and_follow_up_share_normal_song_undo() {
    let mut app = Harness::new();
    let original = app.inspect();
    app.click("AI");
    let inserted = insert_example(&mut app, example("compose_song"));
    assert_eq!(
        inserted["assistant"]["prompt"],
        example("compose_song").prompt
    );
    unchanged(&inserted, &original);
    let prompt = format!(
        "{}\n\nUse velocities 80 for chords, 60 for arpeggio, and 90 for bass.",
        example("compose_song").prompt
    );
    replace_prompt(&mut app, &prompt);
    // More than eight activity messages must not displace the user's musical
    // brief from the semantic conversation used by the follow-up request.
    let mut calls: Vec<_> = (0..10)
        .map(|index| {
            json!({
                "id":format!("context-{index}"),"name":"get_song_context","arguments":{}
            })
        })
        .collect();
    calls.push(json!({"id":"start-song","name":"compose_progression","arguments":{
            "session_id":original["song_session"]["id"],
            "expected_revision":original["song_session"]["revision"],
            "name":"D minor idea","start":0.0,"root":"D4","scale":"natural minor",
            "degrees":[1,6,3,7],"smooth":true,"beats_per_chord":8.0,
            "parts":[
                {"part":{"kind":"chords","name":"Warm chords","gate":1.0,"velocity":80}},
                {"part":{"kind":"arpeggio","name":"Quiet arp","style":"up","step":0.5,"gate":0.5,"velocity":60}},
                {"part":{"kind":"bass","name":"Root fifth bass","style":"root_fifth","octave":2,"step":1.0,"gate":0.75,"velocity":90}}
            ]
        }}));
    app.command(json!({"command":"assistant_script","turns":[
        {"calls":calls},
        {"text":"Created eight bars of chords, arpeggio and bass in D minor. These MIDI tracks still need instruments."}
    ]}));
    app.click("Send");
    let created = app.command(json!({"command":"wait_assistant","timeout_ms":10000}));
    assert_eq!(created["assistant"]["busy"], false);
    assert_eq!(created["assistant"]["prompt"], "");
    assert!(
        created["assistant"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["role"] == "User" && message["text"] == prompt)
    );
    assert!(
        created["assistant"]["messages"]
            .to_string()
            .contains("still need instruments")
    );
    assert_eq!(
        created["assistant"]["conversation"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(created["assistant"]["conversation"][0]["role"], "User");
    assert_eq!(created["assistant"]["conversation"][0]["text"], prompt);
    assert_eq!(created["assistant"]["conversation"][1]["role"], "Assistant");
    let tracks = created["project"]["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), 3);
    assert_eq!(created["project"]["tempo"], original["project"]["tempo"]);
    assert_eq!(created["history"]["undo"].as_array().unwrap().len(), 1);
    for (track, velocity) in tracks.iter().zip([80, 60, 90]) {
        let clip = &track["clips"][0];
        assert_eq!(clip["start"], 0.0);
        assert_eq!(clip["length"], 32.0);
        let notes = clip["content"]["Midi"]["notes"].as_array().unwrap();
        assert!(!notes.is_empty());
        assert!(notes.iter().all(|note| note["velocity"] == velocity));
    }
    let chords = tracks[0]["clips"][0]["content"]["Midi"]["notes"]
        .as_array()
        .unwrap();
    assert_eq!(
        chords
            .iter()
            .take(3)
            .map(|note| note["pitch"].clone())
            .collect::<Vec<_>>(),
        vec![json!(62), json!(65), json!(69)]
    );
    let bass_id = tracks[2]["id"].clone();

    // A conversational refinement uses the same checked session and ordinary
    // history as the original example, and remains editable until Send.
    let follow_up = "Set the new bass channel to -6 dB and leave the other parts unchanged.";
    replace_prompt(&mut app, follow_up);
    app.command(json!({"command":"assistant_script","turns":[
        {"calls":[{"id":"quieter-bass","name":"set_mix","arguments":{
            "session_id":created["song_session"]["id"],
            "expected_revision":created["song_session"]["revision"],
            "channels":[{"track":bass_id,"gain_db":-6.0}]
        }}]},
        {"text":"Set the bass to -6 dB; chords and arpeggio are unchanged."}
    ]}));
    app.click("Send");
    let mixed = app.command(json!({"command":"wait_assistant","timeout_ms":10000}));
    let conversation = mixed["assistant"]["conversation"].as_array().unwrap();
    assert_eq!(conversation.len(), 4);
    assert_eq!(conversation[0]["text"], prompt);
    assert_eq!(conversation[2]["text"], follow_up);
    assert!(conversation.iter().all(|message| message["role"] != "Tool"));
    let gain = mixed["project"]["tracks"][2]["gain"].as_f64().unwrap();
    assert!((gain - 10.0_f64.powf(-6.0 / 20.0)).abs() < 1e-6);
    for index in [0, 1] {
        assert_eq!(
            mixed["project"]["tracks"][index],
            created["project"]["tracks"][index]
        );
    }
    assert_eq!(mixed["history"]["undo"].as_array().unwrap().len(), 2);
    assert_eq!(app.click("Undo")["project"], created["project"]);
    assert_eq!(app.click("Undo")["project"], original["project"]);
    app.quit();
}

#[test]
fn failed_prompt_can_be_edited_and_new_chat_preserves_the_unsent_draft_and_song() {
    let mut app = Harness::new();
    let initial = app.inspect();
    let created = song_tool(
        &mut app,
        "create_midi_part",
        json!({
            "session_id":initial["song_session"]["id"],
            "expected_revision":initial["song_session"]["revision"],
            "name":"Existing idea","start":0.0,"length":4.0,
            "notes":[{"pitch":60,"start":0.0,"length":1.0,"velocity":90}]
        }),
    );
    let track = &created["project"]["tracks"][0];
    let original = song_tool(
        &mut app,
        "set_selection",
        json!({
            "track_id":track["id"],"clip_id":track["clips"][0]["id"]
        }),
    );
    app.click("AI");
    let inserted = insert_example(&mut app, example("learn_arrangement"));
    let prompt = inserted["assistant"]["prompt"].as_str().unwrap().to_owned();
    // Exhaustion is a deterministic provider failure, after the app accepts the
    // user message; it exercises recovery without real credentials or networking.
    app.command(json!({"command":"assistant_script","turns":[]}));
    app.click("Send");
    let failed = app.command(json!({"command":"wait_assistant","timeout_ms":10000}));
    assert_eq!(failed["assistant"]["busy"], false);
    assert!(
        failed["assistant"]["status"]
            .as_str()
            .unwrap()
            .contains("ran out of responses")
    );
    assert_eq!(failed["assistant"]["prompt"], "");
    unchanged(&failed, &original);

    let restored = more(&mut app, "Edit last prompt");
    assert_eq!(restored["assistant"]["prompt"], prompt);
    assert_eq!(restored["assistant"]["busy"], false);
    assert_eq!(
        restored["assistant"]["messages"],
        failed["assistant"]["messages"]
    );
    unchanged(&restored, &failed);
    let revised = "Suggest a four-section arrangement plan without editing the song.";
    replace_prompt(&mut app, revised);
    let protected = more(&mut app, "Edit last prompt");
    assert_eq!(protected["assistant"]["prompt"], revised);
    if protected["native_menu"]["open"] == true {
        app.key("Escape");
    }
    app.command(json!({"command":"assistant_script","turns":[
        {"calls":[{"id":"context","name":"get_song_context","arguments":{}}]},
        {"text":"Use the existing phrase in an intro, develop the main section, contrast it, then return for an ending. No song edits were made."}
    ]}));
    app.click("Send");
    let completed = app.command(json!({"command":"wait_assistant","timeout_ms":10000}));
    assert_eq!(completed["assistant"]["status"], "Done");
    unchanged(&completed, &original);
    let draft = "Keep this unsent idea for the next conversation.";
    replace_prompt(&mut app, draft);
    let cleared = more(&mut app, "New chat");
    assert_eq!(cleared["assistant"]["messages"], json!([]));
    assert_eq!(cleared["assistant"]["conversation"], json!([]));
    assert_eq!(cleared["assistant"]["status"], "");
    assert_eq!(cleared["assistant"]["prompt"], draft);
    assert_eq!(cleared["assistant"]["page"], "Conversation");
    assert_eq!(cleared["assistant"]["busy"], false);
    unchanged(&cleared, &completed);
    app.quit();
}

#[test]
fn ai_settings_returns_to_the_draft_and_context_reflects_actual_selected_music() {
    let mut app = Harness::new();
    let mut project = Project::new();
    project.name = "Context study".into();
    project.tempo = 90.0;
    project.beats_per_bar = 3;
    let track = project.add_midi_track("Context piano");
    let notes = vec![Note::new(60, 0.0, 1.0, 80), Note::new(64, 1.0, 1.0, 90)];
    let note_ids: Vec<_> = notes.iter().map(|note| note.id).collect();
    let clip = Clip::midi("Context phrase", 6.0, 6.0, notes);
    let clip_id = clip.id;
    project.track_mut(track).unwrap().clips.push(clip);
    let path = app.directory.path().join("assistant-context.rill");
    project.save(&path).unwrap();
    app.command(json!({"command":"open","path":path}));
    song_tool(
        &mut app,
        "set_selection",
        json!({"track_id":track,"clip_id":clip_id,"note_ids":note_ids}),
    );
    let selected = song_tool(&mut app, "transport", json!({"action":"seek","beat":7.5}));
    app.click("AI");
    let context = app.inspect();
    let text = context["assistant"]["context"].as_str().unwrap();
    for part in [
        "Context study",
        "90 BPM",
        "3/4",
        "Context piano",
        "Context phrase",
        "2 selected notes",
        "Cursor: beat 7.50",
    ] {
        assert!(text.contains(part), "Missing {part:?} in context: {text}");
    }
    assert_eq!(context["assistant"]["configured"], false);
    assert_eq!(context["assistant"]["provider"], "OpenAI");
    unchanged(&context, &selected);
    let draft = "Explain the selected notes without changing this phrase.";
    replace_prompt(&mut app, draft);
    let setup = app.click("AI settings");
    assert_eq!(setup["popups"]["setup"], true);
    assert!(visible(&setup, "Provider"));
    assert!(visible(&setup, "Model"));
    assert!(visible(&setup, "API key"));
    assert_eq!(setup["assistant"]["prompt"], draft);
    unchanged(&setup, &selected);
    let canceled = app.click("Cancel");
    assert!(visible(&canceled, "AI prompt"));
    assert_eq!(canceled["assistant"]["prompt"], draft);
    unchanged(&canceled, &selected);

    app.click("AI settings");
    let applied = app.click("Apply");
    assert!(visible(&applied, "AI prompt"));
    assert_eq!(applied["assistant"]["prompt"], draft);
    assert_eq!(
        applied["assistant"]["context"],
        context["assistant"]["context"]
    );
    unchanged(&applied, &selected);

    // A clip selection and an explicit time selection are distinct musical
    // scopes. Drive the actual Cursor tool to verify the displayed range too.
    app.key("Escape");
    app.click("Cursor");
    let view = app.inspect();
    let timeline = &view["timeline"];
    let left = timeline["canvas"]["min"][0].as_f64().unwrap();
    let beat = timeline["start_beat"].as_f64().unwrap();
    let scale = timeline["pixels_per_beat"].as_f64().unwrap();
    let row = &timeline["tracks"][0]["header"];
    let top = row["min"][1].as_f64().unwrap();
    let bottom = row["max"][1].as_f64().unwrap();
    let range = app.command(json!({"command":"pointer","phase":"drag",
        "x":left+(6.0-beat)*scale,"y":top+4.0,
        "to_x":left+(12.0-beat)*scale,"to_y":bottom-4.0
    }));
    assert_eq!(range["selection_span"], json!([6.0, 12.0]));
    app.click("AI");
    let ranged = app.inspect();
    assert!(
        ranged["assistant"]["context"]
            .as_str()
            .unwrap()
            .contains("Range: beats 6.00–12.00")
    );
    unchanged(&ranged, &range);

    // A new document must not reuse a draft referring to the prior song's IDs
    // or selection. The ordinary File action owns this session transition.
    app.key("Escape");
    app.click("File");
    let replacement = app.click("New");
    assert_ne!(
        replacement["song_session"]["id"],
        selected["song_session"]["id"]
    );
    assert_eq!(replacement["assistant"]["prompt"], "");
    assert_eq!(replacement["assistant"]["conversation"], json!([]));
    assert_eq!(replacement["assistant"]["status"], "");
    app.quit();
}
