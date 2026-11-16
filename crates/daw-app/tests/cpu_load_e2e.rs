//! The same menu-bar meter handles real OS samples and a saturated logical core.
mod common;
use common::{Harness, wait_until};
use serde_json::{Value, json};

fn visible(state: &Value, text: &str) -> bool {
    state["visible_labels"]
        .as_array()
        .unwrap()
        .iter()
        .any(|label| label == text)
}

#[test]
fn system_cpu_snapshot_and_core_detail_are_available_while_idle() {
    let mut app = Harness::new();
    let state = wait_until(&mut app, |state| {
        state["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .any(|label| label.starts_with("CPU ") && label.contains('%'))
    });
    let rect = state["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            item["text"]
                .as_str()
                .is_some_and(|s| s.starts_with("CPU ") && s.contains('%'))
        })
        .unwrap();
    let x = (rect["min"][0].as_f64().unwrap() + rect["max"][0].as_f64().unwrap()) / 2.;
    let y = (rect["min"][1].as_f64().unwrap() + rect["max"][1].as_f64().unwrap()) / 2.;
    let details = app.command(json!({"command":"pointer","phase":"click","x":x,"y":y}));
    for label in ["CPU load", "Total", "Core 1"] {
        assert!(visible(&details, label), "missing {label}");
    }
    let after = app.key("Escape");
    assert!(!visible(&after, "CPU load"));
    assert_eq!(state["history"], after["history"]);
    app.quit();
}

#[test]
fn a_saturated_core_or_total_lights_the_warning_and_recovery_clears_it() {
    let mut app = Harness::new();
    let initial = app.command(json!({"command":"cpu_sample", "total":36, "cores":[10,20,100,14]}));
    assert!(visible(&initial, "CPU 36% · Overload"));
    let details = app.click("CPU 36% · Overload");
    for label in [
        "Total",
        "Core 1",
        "Core 2",
        "Core 3",
        "Core 4",
        "100% · Overload",
        "36%",
    ] {
        assert!(visible(&details, label), "missing {label}");
    }
    let painted = app.command(json!({"command":"inspect","paint":true}));
    let colors = |text: &str| {
        painted["painted_text"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["text"] == text)
            .unwrap()["colors"]
            .clone()
    };
    assert_eq!(colors("CPU 36% · Overload"), colors("100% · Overload"));
    assert_ne!(colors("100% · Overload"), colors("36%"));

    let total = app.command(json!({"command":"cpu_sample","total":97,"cores":[95,99]}));
    assert!(visible(&total, "CPU 97% · Overload") && visible(&total, "97% · Overload"));
    let normal = app.command(json!({"command":"cpu_sample","total":25,"cores":[20,30]}));
    assert!(visible(&normal, "CPU 25%") && visible(&normal, "30%"));
    assert!(
        !normal["visible_labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().is_some_and(|s| s.contains("Overload")))
    );
    let closed = app.click("CPU 25%");
    assert!(!visible(&closed, "CPU load"));
    assert_eq!(initial["project"], closed["project"]);
    assert_eq!(initial["history"], closed["history"]);
    app.quit();
}

#[test]
fn core_detail_scrolls_to_the_last_logical_processor() {
    let mut app = Harness::new();
    app.command(json!({"command":"viewport","width":800,"height":600}));
    let mut cores = vec![10; 64];
    cores[63] = 99;
    app.command(json!({"command":"cpu_sample","total":11,"cores":cores}));
    let opened = app.click("CPU 11% · Overload");
    assert!(visible(&opened, "Core 1"));
    assert!(!visible(&opened, "Core 64"));
    let label = opened["label_rects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["text"] == "Core 1")
        .unwrap();
    let x = label["min"][0].as_f64().unwrap() + 10.0;
    let y = label["min"][1].as_f64().unwrap() + 5.0;
    let end = app.command(json!({"command":"scroll","x":x,"y":y,"delta":-10000.0}));
    assert!(visible(&end, "Core 64") && visible(&end, "99% · Overload"));
    app.quit();
}
