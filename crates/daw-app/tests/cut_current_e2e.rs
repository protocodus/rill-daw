//! A four-loop audio collage, authored entirely with the running app's editing UI.
//! Run with RILL_FOUR_LOOP_SOURCES pointing at the four original WAV loops.
mod common;

use common::Harness;
use daw_core::{AudioData, ClipContent, Project};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

const PALETTE: f64 = 40.0;
const RATE: u32 = 48_000;

fn menu(app: &mut Harness, parent: &str, item: &str) -> Value {
    let opened = app.click(parent);
    let occurrence = opened["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| *v == item)
        .count()
        - 1;
    app.command(json!({"command":"click","label":item,"occurrence":occurrence}))
}

fn chord(app: &mut Harness, key: &str) -> Value {
    app.command(json!({"command":"key","key":key,"cmd":true}))
}

fn center(rect: &Value) -> (f64, f64) {
    (
        (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) * 0.5,
        (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) * 0.5,
    )
}

fn pointer(app: &mut Harness, phase: &str, p: (f64, f64)) -> Value {
    app.command(json!({"command":"pointer","phase":phase,"x":p.0,"y":p.1}))
}

fn select(app: &mut Harness, id: u64) -> Value {
    app.click("Arranger");
    app.key("Q");
    let state = app.click("Fit");
    let rect = &state["timeline"]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .unwrap()["rect"];
    let p = center(rect);
    let clicked = pointer(app, "click", p);
    assert_eq!(
        clicked["selected_clip"],
        id,
        "wanted clip {id} at {p:?}, tool {}, target {:?}, overlapping {:?}",
        state["timeline"]["tool"],
        state["project"]["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|t| t["clips"].as_array().unwrap())
            .find(|c| c["id"] == id),
        state["timeline"]["clips"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| {
                let r = &c["rect"];
                p.0 >= r["min"][0].as_f64().unwrap()
                    && p.0 <= r["max"][0].as_f64().unwrap()
                    && p.1 >= r["min"][1].as_f64().unwrap()
                    && p.1 <= r["max"][1].as_f64().unwrap()
            })
            .collect::<Vec<_>>()
    );
    clicked
}

fn audio_point(state: &Value, beat: f64) -> (f64, f64) {
    let v = &state["audio_editor"];
    (
        v["rect"][0].as_f64().unwrap()
            + (beat - v["start_beat"].as_f64().unwrap()) * v["pixels_per_beat"].as_f64().unwrap(),
        (v["rect"][1].as_f64().unwrap() + v["rect"][3].as_f64().unwrap()) * 0.5,
    )
}

fn editor(app: &mut Harness, id: u64) -> Value {
    select(app, id);
    menu(app, "Window", "Clip Editor");
    app.key("P");
    app.click("Fit")
}

fn rename_track(app: &mut Harness, name: &str) {
    let before = app.inspect();
    let id = before["selected_track"].as_u64().unwrap();
    let old = before["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .unwrap()["name"]
        .as_str()
        .unwrap()
        .to_owned();
    menu(app, "Window", "Properties");
    let state = app.inspect();
    let occurrence = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| *v == "Track")
        .count()
        - 1;
    app.command(json!({"command":"click","label":"Track","occurrence":occurrence}));
    let state = app.inspect();
    let occurrence = state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| **v == old)
        .count()
        - 1;
    app.command(json!({"command":"click","label":old,"occurrence":occurrence}));
    chord(app, "A");
    app.command(json!({"command":"text","text":name}));
    app.key("Enter");
    app.key("Escape");
}

fn rename_clip(app: &mut Harness, name: &str) {
    let state = menu(app, "Window", "Properties");
    let r = &state["clip_name_rect"];
    pointer(
        app,
        "click",
        (
            (r[0].as_f64().unwrap() + r[2].as_f64().unwrap()) * 0.5,
            (r[1].as_f64().unwrap() + r[3].as_f64().unwrap()) * 0.5,
        ),
    );
    chord(app, "A");
    app.command(json!({"command":"text","text":name}));
    app.key("Enter");
    app.key("Escape");
}

fn copy_cue(app: &mut Harness, track: usize, offset: f64, length: f64) {
    eprintln!("Isolate source track {track}, offset {offset}, length {length}");
    let state = app.inspect();
    let source = state["project"]["tracks"][track]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| {
            let start = clip["start"].as_f64().unwrap();
            start >= PALETTE
                && start <= PALETTE + offset + 1e-5
                && start + clip["length"].as_f64().unwrap() >= PALETTE + offset + length - 1e-5
        })
        .unwrap_or_else(|| panic!("No source cue track {track} offset {offset} length {length}"));
    let id = source["id"].as_u64().unwrap();
    let state = editor(app, id);
    let a = audio_point(&state, PALETTE + offset);
    let b = audio_point(&state, PALETTE + offset + length);
    let state = app
        .command(json!({"command":"pointer","phase":"drag","x":a.0,"y":a.1,"to_x":b.0,"to_y":b.1}));
    assert_eq!(
        state["audio_editor"]["range"],
        json!([PALETTE + offset, PALETTE + offset + length])
    );
    let before = state["project"].clone();
    let isolated = menu(app, "Clip", "Split selection");
    let id = isolated["selected_clip"].as_u64().unwrap();
    let clip = isolated["project"]["tracks"][track]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .unwrap();
    assert!((clip["offset"].as_f64().unwrap() - offset).abs() < 1e-5);
    assert!((clip["length"].as_f64().unwrap() - length).abs() < 1e-5);
    // A substantial two-boundary isolation remains one undoable edit.
    assert_eq!(chord(app, "Z")["project"], before);
    app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}));
    // History restores the edit; select its isolated cue again before copying.
    editor(app, id);
    let role = [
        "Kick",
        "Backbeat",
        "Hat",
        "Bass cue",
        "Keys cue",
        "Chord cue",
    ][track];
    rename_clip(app, &format!("{role} {}", offset + 1.0));
    chord(app, "C");
}

fn arrangement(app: &mut Harness) -> Value {
    app.click("Arranger");
    let state = if app.inspect()["project"]["tracks"]
        .as_array()
        .unwrap()
        .is_empty()
    {
        app.inspect()
    } else {
        app.click("Fit")
    };
    let t = &state["timeline"];
    let p = center(&t["canvas"]);
    let width = t["canvas"]["max"][0].as_f64().unwrap() - t["canvas"]["min"][0].as_f64().unwrap();
    let pixels = width / 58.0;
    let delta = (pixels / t["pixels_per_beat"].as_f64().unwrap()).ln() / 0.005;
    app.command(json!({"command":"scroll","x":p.0,"y":p.1,"delta":delta,"ctrl":true}));
    app.command(json!({"command":"scroll","x":p.0,"y":p.1,"delta":100000.0,"shift":true}))
}

fn paste(app: &mut Harness, beat: f64) -> u64 {
    let state = app.inspect();
    let t = &state["timeline"];
    let x = t["canvas"]["min"][0].as_f64().unwrap()
        + (beat - t["start_beat"].as_f64().unwrap()) * t["pixels_per_beat"].as_f64().unwrap();
    let y = t["canvas"]["min"][1].as_f64().unwrap() - 18.0;
    pointer(app, "click", (x, y));
    let state = chord(app, "V");
    let id = state["selected_clip"].as_u64().unwrap();
    let track = state["project"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == state["selected_track"])
        .unwrap();
    let clip = track["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .unwrap();
    assert!(
        (clip["start"].as_f64().unwrap() - beat).abs() < 1e-5,
        "paste at {beat}: {clip}"
    );
    id
}

fn cues(app: &mut Harness, track: usize, offset: f64, length: f64, positions: &[f64]) -> Vec<u64> {
    copy_cue(app, track, offset, length);
    arrangement(app);
    positions
        .iter()
        .map(|&beat| {
            let id = paste(app, beat);
            let state = app.inspect();
            let clip = state["project"]["tracks"][track]["clips"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == id)
                .unwrap();
            assert!(
                (clip["length"].as_f64().unwrap() - length).abs() < 1e-5,
                "pasted cue length"
            );
            assert!(
                (clip["offset"].as_f64().unwrap() - offset).abs() < 1e-5,
                "pasted cue source offset"
            );
            id
        })
        .collect()
}

fn fade(app: &mut Harness, id: u64, key: &str, length: &str) {
    editor(app, id);
    let state = app.key(key);
    let rect = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|r| r["text"].as_str().is_some_and(|s| s.ends_with(" beats")))
        .unwrap();
    pointer(app, "double_click", center(rect));
    chord(app, "A");
    app.command(json!({"command":"text","text":length}));
    app.key("Enter");
}

fn reverse(app: &mut Harness, id: u64) {
    editor(app, id);
    menu(app, "Process", "Reverse");
}

fn polish_cue_edges(app: &mut Harness) {
    // A later audio cue takes priority during an overlap. Trim the outgoing
    // cue to that audible boundary so its release reaches zero before the cut.
    let state = app.inspect();
    let mut trims = Vec::new();
    for track in [0, 4] {
        let mut clips = state["project"]["tracks"][track]["clips"]
            .as_array()
            .unwrap()
            .iter()
            .collect::<Vec<_>>();
        clips.sort_by(|a, b| {
            a["start"]
                .as_f64()
                .unwrap()
                .total_cmp(&b["start"].as_f64().unwrap())
        });
        for pair in clips.windows(2) {
            let start = pair[0]["start"].as_f64().unwrap();
            let end = start + pair[0]["length"].as_f64().unwrap();
            let next = pair[1]["start"].as_f64().unwrap();
            if end > next {
                trims.push((track, pair[0]["id"].as_u64().unwrap(), next - end));
            }
        }
    }
    assert_eq!(trims.len(), 10);
    for (_, id, delta) in &trims {
        editor(app, *id);
        let state = app.key("Q");
        let handle = &state["audio_editor"]["end_handle"];
        let x = (handle[0].as_f64().unwrap() + handle[2].as_f64().unwrap()) * 0.5;
        let y = (handle[1].as_f64().unwrap() + handle[3].as_f64().unwrap()) * 0.5;
        let movement = delta * state["audio_editor"]["pixels_per_beat"].as_f64().unwrap();
        app.command(
            json!({"command":"pointer","phase":"drag","x":x,"y":y,"to_x":x+movement,"to_y":y}),
        );
    }
    let state = app.inspect();
    let keys = state["project"]["tracks"][4]["clips"]
        .as_array()
        .unwrap()
        .clone();
    for clip in &keys {
        let id = clip["id"].as_u64().unwrap();
        let audio = &clip["content"]["Audio"];
        if audio["fade_in"].as_f64().unwrap() < 0.005 {
            fade(app, id, "I", "0.005");
        }
        if audio["fade_out"].as_f64().unwrap() < 0.020 {
            fade(app, id, "O", "0.020");
        }
    }
    for (track, id, _) in trims {
        if track == 0 {
            fade(app, id, "O", "0.020");
        }
    }
}

fn balance(app: &mut Harness) {
    app.click("Mixer");
    for (index, db) in [-6.5_f64, -8.5, -11.5, -6.5, -7.5, -4.5]
        .into_iter()
        .enumerate()
    {
        let state = app.inspect();
        let track = &state["project"]["tracks"][index];
        let name = track["name"].as_str().unwrap();
        let label = format!("{:.1} dB", 20.0 * track["gain"].as_f64().unwrap().log10());
        let labels = state["label_rects"].as_array().unwrap();
        let header = labels.iter().find(|r| r["text"] == name).unwrap();
        let x = center(header).0;
        let rect = labels
            .iter()
            .filter(|r| r["text"] == label)
            .min_by(|a, b| (center(a).0 - x).abs().total_cmp(&(center(b).0 - x).abs()))
            .unwrap();
        pointer(app, "double_click", center(rect));
        chord(app, "A");
        app.command(json!({"command":"text","text":format!("{db:.1}")}));
        let state = app.key("Enter");
        assert!(
            (state["project"]["tracks"][index]["gain"].as_f64().unwrap()
                - 10.0_f64.powf(db / 20.0))
            .abs()
                < 1e-5,
            "precise gain for {name}"
        );
    }
    app.click("Arranger");
}

fn checkpoint(app: &mut Harness) {
    if let Some(destination) = std::env::var_os("RILL_SONG_ARTIFACT_DIR") {
        let destination = Path::new(&destination);
        std::fs::create_dir_all(destination).unwrap();
        let path = destination.join("Cut Current working.rill");
        app.command(json!({"command":"file_dialog","kind":"save","path":path}));
        menu(app, "File", "Save as…");
        app.command(json!({"command":"wait_project"}));
        eprintln!("Durable working checkpoint: {}", path.display());
    }
}

fn slip_one_source_beat(app: &mut Harness, id: u64) {
    let state = editor(app, id);
    let p = audio_point(&state, 24.375);
    let delta = (100.0 / state["audio_editor"]["pixels_per_beat"].as_f64().unwrap()).ln() / 0.005;
    app.command(json!({"command":"scroll","x":p.0,"y":p.1,"delta":delta,"ctrl":true}));
    let state = app.key("L");
    assert_eq!(state["audio_editor"]["tool"], "Slip");
    let before = state["project"].clone();
    let p = audio_point(&state, 24.375);
    let ppb = state["audio_editor"]["pixels_per_beat"].as_f64().unwrap();
    let slipped = app.command(
        json!({"command":"pointer","phase":"drag","x":p.0,"y":p.1,"to_x":p.0-ppb,"to_y":p.1}),
    );
    let clip = slipped["project"]["tracks"][4]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .unwrap();
    assert_eq!(
        (
            clip["start"].as_f64().unwrap(),
            clip["length"].as_f64().unwrap()
        ),
        (24.0, 0.75)
    );
    assert!((clip["offset"].as_f64().unwrap() - 13.0).abs() < 1e-5);
    assert_eq!(chord(app, "Z")["project"], before);
    assert_eq!(
        app.command(json!({"command":"key","key":"Z","cmd":true,"shift":true}))["project"],
        slipped["project"]
    );
}

fn export(app: &mut Harness, path: &Path) -> AudioData {
    menu(app, "File", "Export WAV…");
    if app.inspect()["export_include_effect_tails"] == true {
        app.click("Include effect tails");
    }
    app.command(json!({"command":"file_dialog","kind":"export","path":path}));
    app.click("Export…");
    app.command(json!({"command":"wait_export"}));
    AudioData::load_wav(path).unwrap()
}

#[test]
#[ignore = "musical dogfooding; set RILL_FOUR_LOOP_SOURCES to the four original loops"]
fn four_loops_become_individual_hits_and_rearranged_cues_without_changing_sources() {
    let source_dir =
        PathBuf::from(std::env::var_os("RILL_FOUR_LOOP_SOURCES").expect("four-loop source folder"));
    let names = [
        "01 Drum palette.wav",
        "02 Plucked bass.wav",
        "03 Tine keys.wav",
        "04 Soft chords.wav",
    ];
    let originals: Vec<_> = names
        .iter()
        .map(|name| std::fs::read(source_dir.join(name)).unwrap())
        .collect();
    let mut app = Harness::new();
    let mut project = Project::new();
    project.name = "Cut Current".into();
    project.tempo = 105.0;
    let seed = app.directory.path().join("Cut Current template.json");
    project.save(&seed).unwrap();
    app.command(json!({"command":"open","path":seed}));
    app.command(json!({"command":"viewport","width":1800,"height":1200}));
    let sources: Vec<_> = names
        .iter()
        .map(|name| app.directory.path().join(name))
        .collect();
    for (i, path) in sources.iter().enumerate() {
        std::fs::write(path, &originals[i]).unwrap();
    }
    // The only model fixture is an empty tempo template. Import every full loop
    // through the same drop gesture as a user; three drum lanes share one WAV.
    arrangement(&mut app);
    for (index, voice) in [0, 0, 0, 1, 2, 3].into_iter().enumerate() {
        app.command(json!({"command":"drop","paths":[sources[voice]],"beat":PALETTE}));
        rename_track(
            &mut app,
            [
                "Kick",
                "Snare and rim",
                "Hats",
                "Plucked bass",
                "Tine keys",
                "Soft chords",
            ][index],
        );
    }
    assert_eq!(
        app.inspect()["project"]["tracks"].as_array().unwrap().len(),
        6
    );

    // Two-bar intro; four bars grow from backbeat to syncopation; a sparse
    // rim/hat break makes room for the keys; the last bar resolves to A minor.
    cues(
        &mut app,
        0,
        0.0,
        0.75,
        &[4., 6., 8., 10.5, 12., 14., 16., 18., 20., 22., 28., 30.],
    );
    cues(
        &mut app,
        0,
        5.0,
        0.75,
        &[9.5, 15.5, 17.5, 19.5, 23.25, 29.5, 31.25],
    );
    cues(
        &mut app,
        1,
        1.0,
        0.75,
        &[9., 11., 13., 15., 17., 19., 21., 23., 29., 31.],
    );
    cues(&mut app, 1, 6.0, 0.5, &[18.75, 22.75, 27.5, 27.75]);
    cues(&mut app, 1, 3.0, 0.5, &[5., 7., 25., 27.]);
    let hats: Vec<_> = (8..48)
        .filter(|n| !matches!(n % 8, 7))
        .map(|n| n as f64 * 0.5)
        .chain([26., 27., 28., 28.5, 29., 29.5, 30., 30.5, 31.])
        .collect();
    cues(&mut app, 2, 2.0, 0.25, &hats);
    cues(
        &mut app,
        2,
        7.0,
        0.25,
        &[7.5, 11.5, 15.5, 19.5, 23.5, 27.25, 31.5, 31.75],
    );
    cues(&mut app, 2, 4.0, 0.75, &[3., 15., 23., 30.75]);
    checkpoint(&mut app);

    cues(&mut app, 3, 0., 2., &[4., 6., 28., 30.]);
    cues(&mut app, 3, 2., 2., &[8., 10., 12., 14.]);
    cues(&mut app, 3, 4., 2., &[16., 18.]);
    cues(&mut app, 3, 6., 2., &[20., 22.]);
    let bass_pickup = cues(&mut app, 3, 10., 1., &[27.])[0];
    fade(&mut app, bass_pickup, "O", "0.25");
    checkpoint(&mut app);

    let notes: [&[f64]; 16] = [
        &[0., 4., 31.],
        &[1., 6., 30.],
        &[2., 5., 29.],
        &[3., 7., 28.],
        &[8., 12.],
        &[9., 13.5],
        &[10., 14.],
        &[11., 15.],
        &[16., 18.],
        &[17., 19.],
        &[16.5],
        &[18.5],
        &[20., 24.],
        &[21., 25.5],
        &[22., 26.],
        &[23., 27.],
    ];
    let mut key_ids = Vec::new();
    for (note, positions) in notes.iter().enumerate() {
        key_ids.extend(cues(&mut app, 4, note as f64, 0.75, positions));
    }
    // Reverse one source cue as an audible pickup; a final longer fade yields
    // an intentional resolution instead of a loop abruptly stopping.
    let pickup = app.inspect()["project"]["tracks"][4]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["start"] == 27.0)
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    reverse(&mut app, pickup);
    let slipped_cue = app.inspect()["project"]["tracks"][4]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["start"] == 24.0)
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    slip_one_source_beat(&mut app, slipped_cue);
    let state = app.inspect();
    for id in key_ids.iter().filter(|id| {
        state["project"]["tracks"][4]["clips"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| {
                c["id"] == **id
                    && c["start"].as_f64().unwrap() >= 28.0
                    && c["start"].as_f64().unwrap() < 32.0
            })
    }) {
        fade(&mut app, *id, "O", "0.25");
    }
    checkpoint(&mut app);

    let pad_a = cues(&mut app, 5, 0., 2., &[0., 4., 28., 30.]);
    let pad_a_tail = cues(&mut app, 5, 2., 1.5, &[2.25, 6.25]);
    let pad_f = cues(&mut app, 5, 4., 2., &[8., 12.]);
    let pad_f_tail = cues(&mut app, 5, 6., 1.5, &[10.25, 14.25]);
    let pad_c = cues(&mut app, 5, 8., 2., &[16., 18.]);
    let pad_g = cues(&mut app, 5, 12., 2., &[20., 22.]);
    let pad_break = cues(&mut app, 5, 14., 1.5, &[24.5, 26.25]);
    reverse(&mut app, pad_break[0]);
    for id in pad_a
        .iter()
        .chain(&pad_a_tail)
        .chain(&pad_f)
        .chain(&pad_f_tail)
        .chain(&pad_c)
        .chain(&pad_g)
        .chain(&pad_break)
    {
        fade(&mut app, *id, "I", "0.125");
        fade(&mut app, *id, "O", "0.25");
    }
    fade(&mut app, *pad_a.last().unwrap(), "O", "1.5");
    checkpoint(&mut app);

    // Erase only the audition palette clips. Full original source files stay
    // referenced by their independent arrangement slices and remain recoverable.
    loop {
        let state = app.inspect();
        let id = state["project"]["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|t| t["clips"].as_array().unwrap())
            .find(|c| c["start"].as_f64().unwrap() >= PALETTE)
            .map(|c| c["id"].as_u64().unwrap());
        let Some(id) = id else { break };
        select(&mut app, id);
        app.key("Delete");
    }
    polish_cue_edges(&mut app);
    balance(&mut app);
    let state = app.inspect();
    let project: Project = serde_json::from_value(state["project"].clone()).unwrap();
    project.validate().unwrap();
    assert!((project.end_beat() - 32.).abs() < 1e-5);
    assert!(
        project.tracks[..3]
            .iter()
            .flat_map(|t| &t.clips)
            .all(|c| c.length <= 0.75)
    );
    assert!(
        project.tracks[..3]
            .iter()
            .map(|t| t.clips.len())
            .sum::<usize>()
            >= 80
    );
    let mut paths = BTreeSet::new();
    for track in &project.tracks {
        assert!(track.clips.len() >= 10);
        for clip in &track.clips {
            let ClipContent::Audio { path, .. } = &clip.content else {
                panic!("pure audio song")
            };
            paths.insert(path.clone());
            assert!(clip.start < 32.);
        }
    }
    assert_eq!(paths.len(), 4);
    for (i, path) in sources.iter().enumerate() {
        assert_eq!(std::fs::read(path).unwrap(), originals[i]);
        assert_eq!(
            std::fs::read(source_dir.join(names[i])).unwrap(),
            originals[i]
        );
    }
    let bundle = app.directory.path().join("Cut Current.rill");
    app.command(json!({"command":"file_dialog","kind":"save","path":bundle}));
    menu(&mut app, "File", "Save as…");
    app.command(json!({"command":"wait_project"}));
    if let Some(destination) = std::env::var_os("RILL_SONG_ARTIFACT_DIR") {
        let destination = Path::new(&destination);
        std::fs::create_dir_all(destination).unwrap();
        std::fs::copy(&bundle, destination.join("Cut Current.rill")).unwrap();
        eprintln!(
            "Final native project ready: {}",
            destination.join("Cut Current.rill").display()
        );
    }
    let preview_path = app.directory.path().join("preview.wav");
    let preview = export(&mut app, &preview_path);
    if let Some(destination) = std::env::var_os("RILL_SONG_ARTIFACT_DIR") {
        std::fs::copy(
            &preview_path,
            Path::new(&destination).join("Cut Current.wav"),
        )
        .unwrap();
    }
    assert_eq!((preview.sample_rate, preview.channels), (RATE, 2));
    assert!((preview.frames() as f64 - RATE as f64 * 32. * 60. / 105.).abs() < 2.);
    let peak = preview.samples.iter().map(|s| s.abs()).fold(0f32, f32::max);
    assert!(
        preview.samples.iter().all(|s| s.is_finite()) && peak < 1.,
        "peak {peak}"
    );
    // These transitions previously chopped finite keys/kick sustain, producing
    // a 0.05 one-sample step. Check the actual export at the edited boundaries.
    for beat in [14.0, 16.0, 17.0, 18.0, 20.0, 30.0] {
        let frame = (beat * f64::from(RATE) * 60.0 / 105.0).round() as usize;
        for channel in 0..2 {
            let step = (preview.samples[frame * 2 + channel]
                - preview.samples[(frame - 1) * 2 + channel])
                .abs();
            assert!(step < 0.01, "hard cue cutoff at beat {beat}: {step}");
        }
    }
    for bar in 0..8 {
        let start = (bar as f64 * 4. * 60. / 105. * RATE as f64).round() as usize * 2;
        let end = (((bar + 1) as f64 * 4. * 60. / 105. * RATE as f64).round() as usize * 2)
            .min(preview.samples.len());
        assert!(
            preview.samples[start..end]
                .iter()
                .map(|s| f64::from(*s).powi(2))
                .sum::<f64>()
                > 1.
        );
    }
    // Hide only the private test copies, preserving the originals. A fresh
    // process must recover the four full sources from the portable bundle.
    for path in &sources {
        std::fs::rename(path, path.with_extension("original-hidden.wav")).unwrap();
    }
    let mut app = app.restart();
    app.command(json!({"command":"file_dialog","kind":"open","path":bundle}));
    menu(&mut app, "File", "Open…");
    let restored = app.command(json!({"command":"wait_project"}));
    let recalled: Project = serde_json::from_value(restored["project"].clone()).unwrap();
    let embedded: BTreeSet<_> = recalled
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .map(|c| match &c.content {
            ClipContent::Audio { path, .. } => path,
            _ => panic!(),
        })
        .collect();
    assert_eq!(embedded.len(), 4);
    for path in &embedded {
        let bytes = std::fs::read(path).unwrap();
        assert!(
            originals.contains(&bytes),
            "embedded audio must preserve full original bytes"
        );
    }
    let final_path = app.directory.path().join("Cut Current.wav");
    assert_eq!(export(&mut app, &final_path).samples, preview.samples);
    for (i, name) in names.iter().enumerate() {
        assert_eq!(std::fs::read(source_dir.join(name)).unwrap(), originals[i]);
    }
    if let Some(destination) = std::env::var_os("RILL_SONG_ARTIFACT_DIR") {
        let destination = Path::new(&destination);
        std::fs::create_dir_all(destination).unwrap();
        std::fs::copy(&bundle, destination.join("Cut Current.rill")).unwrap();
        std::fs::copy(&final_path, destination.join("Cut Current.wav")).unwrap();
        let counts: Vec<_> = recalled.tracks.iter().map(|t| t.clips.len()).collect();
        std::fs::write(destination.join("arrangement.json"),serde_json::to_vec_pretty(&json!({"name":"Cut Current","tempo":105,"bars":8,"original_sources":names,"track_roles":["Kick","Snare and rim","Hats","Bass","Keys","Chords"],"clip_counts":counts,"peak_dbfs":20.*f64::from(peak).log10(),"project":recalled})).unwrap()).unwrap();
    }
    app.quit();
}
