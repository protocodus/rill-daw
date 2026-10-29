//! Device cards are exercised through the production headless application UI.
//! Fixtures contain metadata and cached PNGs only: no native editors or devices.
mod common;

use common::Harness;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const FIRST_PRODUCER: &str = "000 Card Audio 東京";
const SECOND_PRODUCER: &str = "001 Second Makers";
const LONG_NAME: &str = "Card Synth 00 · Étoile 東京 🎹 Extended Polyphonic Instrument";

struct Fixture(tempfile::TempDir);

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        for (group, producer) in [(0, FIRST_PRODUCER), (1, SECOND_PRODUCER)] {
            let resources = root
                .path()
                .join(format!("Cards{group}.vst3/Contents/Resources"));
            std::fs::create_dir_all(&resources).unwrap();
            let classes = (0..12)
                .map(|offset| {
                    let index = group * 12 + offset;
                    let name = if index == 0 {
                        LONG_NAME.to_owned()
                    } else {
                        format!("Card Synth {index:02}")
                    };
                    json!({"CID":format!("{:032X}", 1000 + index), "Category":"Audio Module Class",
                    "Name":name,"Sub Categories":["Instrument","Synth"],"Version":"1",
                    "SDKVersion":"VST3.7.9","Cardinality":2147483647})
                })
                .collect::<Vec<_>>();
            std::fs::write(resources.join("moduleinfo.json"), json!({
                "Name":format!("Cards {group}"), "Version":"1", "Factory Info":{"Vendor":producer}, "Classes":classes
            }).to_string()).unwrap();
        }
        Self(root)
    }

    fn app(&self, cache_previews: bool) -> Harness {
        let app = Harness::with_vst_path(self.0.path());
        if cache_previews {
            let cache =
                daw_plugins::catalog::PluginCatalog::new(app.directory.path().join("cache"));
            for entry in cache.scan(&[self.0.path().to_owned()]).unwrap() {
                // One intentionally absent preview verifies the stable placeholder.
                if entry.plugin.name == "Card Synth 03" {
                    continue;
                }
                cache
                    .capture(&entry.plugin, |_, path| {
                        image::RgbaImage::from_pixel(80, 50, image::Rgba([40, 80, 160, 255]))
                            .save(path)?;
                        Ok(())
                    })
                    .unwrap();
            }
        }
        app
    }
}

#[derive(Clone, Copy, Debug)]
struct Bounds {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

impl Bounds {
    fn from(value: &Value) -> Self {
        Self {
            left: value["min"][0].as_f64().unwrap(),
            top: value["min"][1].as_f64().unwrap(),
            right: value["max"][0].as_f64().unwrap(),
            bottom: value["max"][1].as_f64().unwrap(),
        }
    }
    fn width(self) -> f64 {
        self.right - self.left
    }
    fn height(self) -> f64 {
        self.bottom - self.top
    }
    fn contains(self, inner: Self) -> bool {
        inner.left >= self.left - 1.0
            && inner.top >= self.top - 1.0
            && inner.right <= self.right + 1.0
            && inner.bottom <= self.bottom + 1.0
    }
    fn overlaps(self, other: Self) -> bool {
        self.left < other.right - 1.0
            && self.right > other.left + 1.0
            && self.top < other.bottom - 1.0
            && self.bottom > other.top + 1.0
    }
}

fn wait(app: &mut Harness, condition: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(35);
    loop {
        let state = app.inspect();
        if condition(&state) {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "Device browser timed out: {}",
            state["library"]
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn cards(state: &Value) -> &[Value] {
    state["library"]["cards"].as_array().unwrap()
}

fn open(app: &mut Harness) -> Value {
    app.click("Window");
    app.click("Library");
    app.click("Instruments");
    app.click("Search instruments");
    app.command(json!({"command":"text", "text":"Card Synth"}));
    wait(app, |state| state["library"]["total"] == 24)
}

fn query(app: &mut Harness, current: &str, next: &str) -> Value {
    app.click(if current.is_empty() {
        "Search instruments"
    } else {
        current
    });
    app.command(json!({"command":"key", "key":"A", "cmd":true}));
    app.command(json!({"command":"text", "text":next}));
    app.inspect()
}

fn pointer(app: &mut Harness, card: &Value, phase: &str) -> Value {
    let rect = Bounds::from(&card["rect"]);
    app.command(json!({"command":"pointer", "phase":phase,
        "x":(rect.left + rect.right) * 0.5, "y":rect.top + 36.0}))
}

fn selected_name(state: &Value) -> &str {
    cards(state)
        .iter()
        .find(|card| card["selected"] == true)
        .and_then(|card| card["name"].as_str())
        .unwrap_or_else(|| panic!("Selected card must remain visible: {}", state["library"]))
}

fn assert_layout(state: &Value, width: f64, height: f64) {
    let catalog = Bounds::from(&state["library"]["catalog"]);
    assert!(
        catalog.left >= 0.0
            && catalog.top >= 0.0
            && catalog.right <= width + 1.0
            && catalog.bottom <= height + 1.0
    );
    assert!(state["library"]["columns"].as_u64().unwrap() >= 2);
    let visible = cards(state);
    assert!(visible.len() < 24, "Rows should be virtualized");
    let complete: Vec<_> = visible
        .iter()
        .filter(|card| catalog.contains(Bounds::from(&card["rect"])))
        .collect();
    assert!(
        complete.len() >= 2,
        "A complete row of cards must fit: {}",
        state["library"]
    );
    for (index, card) in complete.iter().enumerate() {
        let rect = Bounds::from(&card["rect"]);
        assert!(rect.width() >= 150.0 && rect.height() >= 130.0);
        for other in complete.iter().skip(index + 1) {
            assert!(
                !rect.overlaps(Bounds::from(&other["rect"])),
                "Cards overlap"
            );
        }
        for text in [
            card["name"].as_str().unwrap(),
            card["producer"].as_str().unwrap(),
        ] {
            let labels = state["label_rects"].as_array().unwrap();
            assert!(
                labels
                    .iter()
                    .any(|label| label["text"] == text && rect.contains(Bounds::from(label))),
                "Card must paint readable text inside its bounds: {text}, {rect:?}"
            );
        }
    }
    let footer = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|label| label["text"] == "Add instrument track")
        .unwrap();
    assert!(
        Bounds::from(footer).bottom <= height + 1.0,
        "Insertion action must remain visible"
    );
}

#[test]
fn cards_paint_cached_previews_name_and_producer_and_reflow_at_both_densities() {
    let fixture = Fixture::new();
    for (dark, density) in [(false, 1.0), (true, 2.0)] {
        let mut app = fixture.app(true);
        app.command(json!({"command":"system_theme", "dark":dark}));
        app.command(
            json!({"command":"viewport", "width":1100, "height":760, "pixels_per_point":density}),
        );
        open(&mut app);
        wait(&mut app, |state| {
            cards(state).iter().any(|card| card["preview"] == true)
        });
        let expanded = app.command(json!({"command":"inspect", "paint":true}));
        assert_layout(&expanded, 1100.0, 760.0);
        let wide_columns = expanded["library"]["columns"].as_u64().unwrap();
        let preview = cards(&expanded)
            .iter()
            .find(|card| card["preview"] == true)
            .unwrap();
        assert!(Bounds::from(&preview["rect"]).contains(Bounds::from(&preview["preview_rect"])));
        assert!(
            expanded["painted_images"]
                .as_array()
                .unwrap()
                .iter()
                .any(|image| image["texture_id"] == preview["texture_id"]
                    && Bounds::from(&preview["preview_rect"])
                        .contains(Bounds::from(&image["rect"]))),
            "Cached native preview must appear in actual image paint commands"
        );
        let missing = cards(&expanded)
            .iter()
            .find(|card| card["name"] == "Card Synth 03")
            .unwrap();
        assert_eq!(missing["preview"], false);
        assert!(
            (Bounds::from(&preview["rect"]).height() - Bounds::from(&missing["rect"]).height())
                .abs()
                < 1.0,
            "A missing preview must not change the card geometry"
        );
        app.command(
            json!({"command":"viewport", "width":800, "height":600, "pixels_per_point":density}),
        );
        let compact = app.command(json!({"command":"inspect", "paint":true}));
        assert_layout(&compact, 800.0, 600.0);
        assert!(compact["library"]["columns"].as_u64().unwrap() <= wide_columns);
        let compact_card = cards(&compact)
            .iter()
            .find(|card| card["texture_id"] == preview["texture_id"])
            .unwrap();
        assert!(
            Bounds::from(&compact_card["rect"]).height() < Bounds::from(&preview["rect"]).height()
        );
        let image = compact["painted_images"]
            .as_array()
            .unwrap()
            .iter()
            .find(|image| image["texture_id"] == compact_card["texture_id"])
            .expect("Compact cards retain their cached preview");
        let image_bounds = Bounds::from(&image["rect"]);
        assert!(Bounds::from(&compact_card["preview_rect"]).contains(image_bounds));
        assert!(
            (image_bounds.width() / image_bounds.height() - 1.6).abs() < 0.01,
            "Compact preview preserves the source aspect ratio"
        );
        assert_eq!(
            compact["project"], expanded["project"],
            "Browsing/resizing must not edit the project"
        );
        app.command(
            json!({"command":"viewport", "width":1600, "height":1200, "pixels_per_point":density}),
        );
        let settled = app.inspect();
        for _ in 0..20 {
            let next = app.inspect();
            assert_eq!(
                next["library"]["catalog"], settled["library"]["catalog"],
                "The browser must not grow or recenter between frames"
            );
            assert_eq!(
                next["library"]["footer"], settled["library"]["footer"],
                "Preview loading must not move the insertion controls"
            );
        }
        app.quit();
    }
}

#[test]
fn card_grid_keyboard_and_filters_keep_selection_visible_and_empty_results_safe() {
    let fixture = Fixture::new();
    let mut app = fixture.app(false);
    app.command(json!({"command":"viewport", "width":1100, "height":760}));
    let state = open(&mut app);
    let names: Vec<_> = state["plugin_catalog"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["name"].as_str())
        .filter(|name| name.starts_with("Card Synth"))
        .map(str::to_owned)
        .collect();
    let columns = state["library"]["columns"].as_u64().unwrap() as usize;
    pointer(&mut app, &cards(&state)[0], "click");
    let right = app.key("ArrowRight");
    assert_eq!(selected_name(&right), names[1]);
    assert_eq!(
        selected_name(&app.key("ArrowUp")),
        names[1],
        "Up on the first row must preserve its column"
    );
    let down = app.key("ArrowDown");
    assert_eq!(selected_name(&down), names[1 + columns]);
    let up = app.key("ArrowUp");
    assert_eq!(selected_name(&up), names[1]);
    assert_eq!(selected_name(&app.key("ArrowLeft")), names[0]);
    let mut bottom = app.inspect();
    for _ in 0..(24 / columns + 2) {
        bottom = app.key("ArrowDown");
    }
    assert!(bottom["library"]["scroll_offset"].as_f64().unwrap() > 0.0);
    let selected = cards(&bottom)
        .iter()
        .find(|card| card["selected"] == true)
        .unwrap();
    assert!(
        Bounds::from(&bottom["library"]["catalog"]).contains(Bounds::from(&selected["rect"])),
        "Keyboard scrolling must reveal the entire selected card"
    );
    assert!(bottom["project"]["tracks"].as_array().unwrap().is_empty());
    let late_name = selected_name(&bottom).to_owned();
    let late_fingerprint = bottom["library"]["selected"].clone();
    let down_at_end = app.key("ArrowDown");
    assert_eq!(
        down_at_end["library"]["selected"], late_fingerprint,
        "Down on the last row must preserve the selected column"
    );
    app.click("All developers");
    let narrowed = app.click(SECOND_PRODUCER);
    assert_eq!(narrowed["library"]["total"], 12);
    assert_eq!(
        narrowed["library"]["selected"], late_fingerprint,
        "Producer filtering must retain a matching selection"
    );
    assert_eq!(selected_name(&narrowed), late_name);
    let selected = cards(&narrowed)
        .iter()
        .find(|card| card["selected"] == true)
        .unwrap();
    assert!(
        Bounds::from(&narrowed["library"]["catalog"]).contains(Bounds::from(&selected["rect"])),
        "A retained selection must stay visible after the producer filter changes row indices"
    );
    app.click(SECOND_PRODUCER);
    app.click("All developers");

    let empty = query(&mut app, "Card Synth", "No such device Ω");
    assert_eq!(empty["library"]["total"], 0);
    assert!(cards(&empty).is_empty());
    assert!(empty["library"]["selected"].is_null());
    let after_enter = app.key("Enter");
    assert!(
        after_enter["project"]["tracks"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(after_enter["popups"]["library"], true);

    query(&mut app, "No such device Ω", "Card Synth");
    app.click("All developers");
    let filtered = app.click(SECOND_PRODUCER);
    assert_eq!(filtered["library"]["total"], 12);
    assert!(
        cards(&filtered)
            .iter()
            .all(|card| card["producer"] == SECOND_PRODUCER)
    );
    assert!(filtered["library"]["scroll_offset"].as_f64().unwrap() < 1.0);
    app.quit();
}

#[test]
fn card_clicks_select_while_double_click_creates_or_assigns_the_chosen_instrument() {
    let fixture = Fixture::new();
    let mut app = fixture.app(false);
    let state = open(&mut app);
    let first = pointer(&mut app, &cards(&state)[0], "click");
    let second = pointer(&mut app, &cards(&first)[1], "click");
    assert!(
        second["project"]["tracks"].as_array().unwrap().is_empty(),
        "Fast clicks on distinct cards must not activate"
    );
    assert_eq!(
        selected_name(&second),
        cards(&first)[1]["name"].as_str().unwrap()
    );
    let chosen = cards(&second)
        .iter()
        .find(|card| card["selected"] == true)
        .unwrap()
        .clone();
    let created = pointer(&mut app, &chosen, "double_click");
    assert_eq!(created["project"]["tracks"].as_array().unwrap().len(), 1);
    assert_eq!(created["project"]["tracks"][0]["name"], chosen["name"]);
    assert_eq!(created["popups"]["instrument"], true);

    app.click("Track");
    let added = app.click("Add MIDI track");
    assert_eq!(added["project"]["tracks"].as_array().unwrap().len(), 2);
    let destination = added["selected_track"].clone();
    let tracks_before = added["project"]["tracks"].clone();
    app.command(json!({"command":"text", "text":"Card Synth 00"}));
    let chooser = wait(&mut app, |state| state["library"]["total"] == 1);
    let selected = cards(&chooser)[0].clone();
    assert_eq!(selected["name"], LONG_NAME);
    let assigned = pointer(&mut app, &selected, "double_click");
    let tracks = assigned["project"]["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), 2, "Assignment must reuse the MIDI track");
    let target = tracks
        .iter()
        .find(|track| track["id"] == destination)
        .unwrap();
    assert_eq!(
        target["instrument"]["Vst3"]["class_id"],
        format!("{:032X}", 1000)
    );
    assert_eq!(
        target["clips"],
        tracks_before
            .as_array()
            .unwrap()
            .iter()
            .find(|track| track["id"] == destination)
            .unwrap()["clips"]
    );
    assert_eq!(assigned["popups"]["instrument"], false);
    assert_eq!(assigned["popups"]["library"], false);
    assert_eq!(assigned["view"], "Arrangement");
    app.quit();
}
