use super::*;

#[test]
fn every_workspace_fits_and_keeps_output_and_keyboard_fixed() {
    let mut failures = Vec::new();
    for appearance in [
        daw_ui::theme::Appearance::Light,
        daw_ui::theme::Appearance::Dark,
    ] {
        for descriptor in registry() {
            let mut panel = Panel::new(descriptor);
            panel.ctx.set_theme(appearance.preference());
            let pages = panels::workspaces::tabs(descriptor).len().max(1);
            let bands = if descriptor.id == "r1-equalizer" {
                6
            } else {
                1
            };
            let pads = if descriptor.id == "r1-drums" { 16 } else { 1 };
            let mut output = None;
            let mut keyboard = None;
            for pad in 0..pads {
                for band in 0..bands {
                    for page in 0..pages {
                        panel.state.page = page;
                        panel.state.pad = pad;
                        panel.state.selected = Some(band);
                        panel.frame(vec![]);
                        let used = panel.frame(vec![]);
                        if used.width() > panel_width(descriptor) + 1.0 || used.height() > 620.0 {
                            failures.push(format!(
                                "{} page {page}, band {band}, pad {pad}: {used:?}",
                                descriptor.id
                            ));
                        }
                        let current = panel.state.output_rect.expect("persistent output");
                        if let Some(previous) = output {
                            if previous != current {
                                failures.push(format!(
                                    "{} output moved on page {page}: {previous:?} -> {current:?}",
                                    descriptor.id
                                ));
                            }
                        }
                        output = Some(current);
                        if let Some(current) = panel.state.performance_rects {
                            if let Some(previous) = keyboard {
                                if previous != current {
                                    failures.push(format!(
                                        "{} keyboard moved on page {page}",
                                        descriptor.id
                                    ));
                                }
                            }
                            keyboard = Some(current);
                        }
                        assert_eq!(panel.host.values, panel.host.defaults);
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "Workspace overflow: {}",
        failures.join("\n")
    );
}

#[test]
fn task_navigation_is_mouse_reachable_and_does_not_edit_sound() {
    for descriptor in registry() {
        let pages = panels::workspaces::tabs(descriptor).len();
        if pages < 2 {
            continue;
        }
        let mut panel = Panel::new(descriptor);
        for page in (0..pages).rev() {
            let at = panel.state.tab_rects[page].center();
            panel.click(at, 1);
            assert_eq!(panel.state.page, page, "{} navigation", descriptor.id);
            assert_eq!(panel.host.values, panel.host.defaults);
            assert!(panel.host.singles.is_empty() && panel.host.batch_ids.is_empty());
        }
    }
}

#[test]
fn equalizer_band_selection_is_independent_of_enable_and_edits_selected_band() {
    use equalizer::*;
    let mut panel = Panel::new(&DESCRIPTOR);
    assert_eq!(panel.state.selected, Some(1));
    let before = panel.host.values.clone();
    for index in 0..6 {
        let at = panel.state.band_rects[index].center();
        panel.click(at, 1);
        assert_eq!(panel.state.selected, Some(index));
        assert_eq!(panel.host.values, before);
        let frequency = match index {
            0 => HP_FREQUENCY,
            5 => LP_FREQUENCY,
            _ => BANDS[index - 1] + BAND_FREQUENCY,
        };
        assert!(
            panel
                .state
                .control_rects
                .iter()
                .any(|(id, _)| *id == frequency)
        );
    }
    panel.click(panel.state.band_rects[2].center(), 1);
    panel.set(BANDS[1] + BAND_ON, 0.0);
    let id = BANDS[1] + BAND_GAIN;
    let cell = panel.cell(id);
    panel.drag(cell.center(), cell.center() - vec2(0.0, 20.0));
    assert_ne!(panel.host.value(id), before[&id]);
    assert_eq!(
        panel.host.value(BANDS[0] + BAND_GAIN),
        before[&(BANDS[0] + BAND_GAIN)]
    );
    panel.click(panel.state.band_rects[1].center(), 1);
    panel.click(panel.state.band_rects[2].center(), 1);
    assert_ne!(panel.host.value(id), before[&id]);
    assert_eq!(panel.host.value(BANDS[1] + BAND_ON), 0.0);
    panel.set(BANDS[1] + BAND_TYPE, 3.0);
    assert!(panel.state.disabled.contains(&id));
    assert!(!panel.state.disabled.contains(&(BANDS[1] + BAND_Q)));
}

#[test]
fn tone_workspaces_keep_every_parameter_reachable() {
    for descriptor in [&filter::DESCRIPTOR, &equalizer::DESCRIPTOR] {
        let pages = panels::workspaces::tabs(descriptor).len().max(1);
        let bands = if descriptor.id == "r1-equalizer" {
            6
        } else {
            1
        };
        let mut ids = std::collections::BTreeSet::new();
        for page in 0..pages {
            for band in 0..bands {
                let workspace = panels::workspaces::build(descriptor, 0, page, band);
                ids.extend(
                    workspace
                        .layout
                        .sections
                        .iter()
                        .flat_map(|section| section.ids()),
                );
                ids.extend(workspace.output);
                ids.extend(workspace.toolbar);
            }
        }
        assert_eq!(
            ids,
            descriptor.parameters.iter().map(|spec| spec.id).collect()
        );
    }
}

#[test]
fn loaded_samples_keep_task_output_and_keyboard_geometry() {
    use daw_dsp::sample_player::{PlayerKind, PlayerNode};
    for descriptor in [&sampler::DESCRIPTOR, &drums::DESCRIPTOR] {
        let mut panel = Panel::new(descriptor);
        let pages = panels::workspaces::tabs(descriptor).len();
        let empty: Vec<_> = (0..pages)
            .map(|page| {
                panel.state.page = page;
                panel.frame(vec![]);
                (
                    panel.frame(vec![]),
                    panel.state.output_rect,
                    panel.state.performance_rects,
                )
            })
            .collect();
        panel.host.sample = Some(SampleSummary {
            name: "A very long recording name that still leaves the Load and Clear actions reachable.wav".into(),
            seconds: 2.5,
            peaks: vec![(-0.3, 0.5); 128],
            audible_region: None,
        });
        for (page, (bounds, output, keyboard)) in empty.iter().enumerate() {
            panel.state.page = page;
            panel.frame(vec![]);
            let loaded = panel.frame(vec![]);
            assert_eq!(
                &loaded, bounds,
                "{} sample identity changed frame",
                descriptor.id
            );
            assert_eq!(&panel.state.output_rect, output);
            assert_eq!(&panel.state.performance_rects, keyboard);
        }
        for kind in [
            PlayerKind::All {
                children: vec![PlayerNode::sample(2, 0)],
            },
            PlayerKind::RoundRobin {
                children: vec![PlayerNode::sample(2, 0)],
            },
        ] {
            panel.host.players.insert(
                0,
                PlayerNode {
                    id: 1,
                    name: "A long named player group with layered or alternating recordings".into(),
                    kind,
                },
            );
            for (page, (bounds, output, keyboard)) in empty.iter().enumerate() {
                panel.state.page = page;
                panel.frame(vec![]);
                let grouped = panel.frame(vec![]);
                assert_eq!(&grouped, bounds, "{} group changed frame", descriptor.id);
                assert_eq!(&panel.state.output_rect, output);
                assert_eq!(&panel.state.performance_rects, keyboard);
            }
        }
    }
}
