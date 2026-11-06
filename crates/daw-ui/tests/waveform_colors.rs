use daw_ui::{
    TimelineViewport, WaveformColumn, paint_audio_waveform_raw_with_coloring,
    prepare_audio_waveform_with_coloring, theme,
};
use egui::{Color32, Context, Rect, Shape, pos2};

fn columns() -> Vec<WaveformColumn> {
    [0.0, 0.1, 0.5, 1.0]
        .into_iter()
        .enumerate()
        .map(|(index, amplitude)| WaveformColumn {
            beat: index as f64,
            channel: 0,
            minimum: -amplitude,
            maximum: amplitude,
        })
        .collect()
}

fn distance(a: Color32, b: Color32) -> u32 {
    a.to_array()
        .into_iter()
        .zip(b.to_array())
        .map(|(a, b)| u32::from(a.abs_diff(b)))
        .sum()
}

#[test]
fn rendered_peak_columns_blend_from_neutral_to_clip_color_without_changing_geometry() {
    for dark in [false, true] {
        let ctx = Context::default();
        ctx.set_theme(if dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        theme::apply(&ctx);
        let palette = theme::palette(&ctx);
        let center = palette.line;
        let color = Color32::from_rgb(45, 180, 210);
        let neutral = daw_ui::waveform_quiet_color(color, palette.muted);
        let rect = Rect::from_min_max(pos2(10.0, 20.0), pos2(210.0, 220.0));
        let view = TimelineViewport {
            pixels_per_beat: 50.0,
            row_height: 100.0,
            ..Default::default()
        };
        for density in [1.0, 2.0] {
            let colored = prepare_audio_waveform_with_coloring(
                rect,
                view,
                &columns(),
                1,
                1.0,
                color,
                density,
                center,
                1.0,
                true,
            );
            let flat = prepare_audio_waveform_with_coloring(
                rect,
                view,
                &columns(),
                1,
                1.0,
                color,
                density,
                center,
                1.0,
                false,
            );
            assert_eq!(colored.stats.vertices, 16);
            assert_eq!(colored.stats.indices, 54);
            assert_eq!(colored.stats.vertices, flat.stats.vertices);
            assert_eq!(colored.stats.indices, flat.stats.indices);
            assert_eq!(colored.bytes(), flat.bytes());
            let output = ctx.run(Default::default(), |ctx| {
                let painter = ctx.layer_painter(egui::LayerId::background());
                colored.paint(&painter);
                flat.paint(&painter);
            });
            let meshes: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let Shape::Mesh(mesh) = &shape.shape {
                        Some(mesh)
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(meshes.len(), 2);
            let (colored, flat) = (meshes[0], meshes[1]);
            assert_eq!(colored.indices, flat.indices);
            for (a, b) in colored.vertices.iter().zip(&flat.vertices) {
                assert_eq!(a.pos, b.pos);
                assert_eq!(a.uv, b.uv);
                assert_eq!(a.color.a(), b.color.a());
            }
            let fills: Vec<_> = colored
                .vertices
                .as_chunks::<4>()
                .0
                .iter()
                .map(|vertices| {
                    assert_eq!(vertices[0].color, Color32::TRANSPARENT);
                    assert_eq!(vertices[3].color, Color32::TRANSPARENT);
                    assert_eq!(vertices[1].color, vertices[2].color);
                    vertices[1].color
                })
                .collect();
            assert_eq!(fills[0], neutral);
            assert_eq!(fills[3], color);
            assert!(distance(fills[0], color) > distance(fills[1], color));
            assert!(distance(fills[1], color) > distance(fills[2], color));
            assert!(distance(fills[2], color) > 0);
            assert!(
                flat.vertices
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|vertices| vertices[1].color == color && vertices[2].color == color)
            );
        }
    }
}

#[test]
fn immediate_fallback_matches_peak_colors_and_keeps_ghost_preview_opacity() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let palette = theme::palette(&ctx);
    let color = Color32::from_rgb(45, 180, 210).gamma_multiply(0.45);
    let neutral = daw_ui::waveform_quiet_color(color, palette.muted);
    let prepared = prepare_audio_waveform_with_coloring(
        Rect::from_min_max(pos2(10.0, 20.0), pos2(210.0, 220.0)),
        TimelineViewport {
            pixels_per_beat: 50.0,
            row_height: 100.0,
            ..Default::default()
        },
        &columns(),
        1,
        1.0,
        color,
        1.0,
        palette.line,
        1.0,
        true,
    );
    let output = ctx.run(Default::default(), |ctx| {
        let painter = ctx.layer_painter(egui::LayerId::background());
        paint_audio_waveform_raw_with_coloring(
            &painter,
            Rect::from_min_max(pos2(10.0, 20.0), pos2(210.0, 220.0)),
            TimelineViewport {
                pixels_per_beat: 50.0,
                row_height: 100.0,
                ..Default::default()
            },
            &columns(),
            1,
            1.0,
            color,
            1.0,
            true,
        );
        prepared.paint(&painter);
    });
    let fills: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| {
            if let Shape::LineSegment { stroke, .. } = &shape.shape {
                Some(stroke.color)
            } else {
                None
            }
        })
        .skip(1)
        .take(4)
        .collect();
    assert_eq!(fills.len(), 4);
    assert_eq!(
        fills[0],
        neutral.gamma_multiply(f32::from(color.a()) / 255.0)
    );
    assert_eq!(fills[3], color);
    assert!(fills.iter().all(|fill| fill.a().abs_diff(color.a()) <= 1));
    assert!(distance(fills[0], color) > distance(fills[1], color));
    assert!(distance(fills[1], color) > distance(fills[2], color));
    let mesh = output
        .shapes
        .iter()
        .find_map(|shape| {
            if let Shape::Mesh(mesh) = &shape.shape {
                Some(mesh)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(
        fills,
        mesh.vertices
            .as_chunks::<4>()
            .0
            .iter()
            .map(|vertices| vertices[1].color)
            .collect::<Vec<_>>()
    );
    let centers: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| {
            if let Shape::LineSegment { stroke, .. } = &shape.shape {
                Some(stroke.color)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(centers[0], palette.line);
    assert_eq!(centers.last().unwrap(), &palette.line);
}

#[test]
fn amplitude_colors_clamp_over_range_and_invalid_values() {
    let color = Color32::from_rgb(40, 180, 210);
    let neutral = Color32::GRAY;
    assert_eq!(
        daw_ui::waveform_amplitude_color(color, neutral, 10.0),
        color
    );
    for value in [-1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(
            daw_ui::waveform_amplitude_color(color, neutral, value),
            neutral
        );
    }
    let options = daw_ui::SpectrumViewOptions::default();
    assert!(options.enabled && options.waveform_coloring);
}

#[test]
fn fit_and_display_headroom_do_not_recolor_the_same_audio_level() {
    let ctx = Context::default();
    let neutral = Color32::GRAY;
    let color = Color32::from_rgb(45, 180, 210);
    let rect = Rect::from_min_max(pos2(10.0, 20.0), pos2(210.0, 220.0));
    let view = TimelineViewport {
        pixels_per_beat: 50.0,
        row_height: 100.0,
        ..Default::default()
    };
    let columns = |peak: f32| {
        vec![
            WaveformColumn {
                beat: 0.0,
                channel: 0,
                minimum: -peak,
                maximum: peak,
            },
            WaveformColumn {
                beat: 1.0,
                channel: 0,
                minimum: -peak,
                maximum: peak,
            },
        ]
    };
    let normal = prepare_audio_waveform_with_coloring(
        rect,
        view,
        &columns(0.05),
        1,
        1.2,
        color,
        1.0,
        neutral,
        1.0,
        true,
    );
    let fitted = prepare_audio_waveform_with_coloring(
        rect,
        view,
        &columns(1.0),
        1,
        1.0,
        color,
        1.0,
        neutral,
        0.05,
        true,
    );
    let loud_with_headroom = prepare_audio_waveform_with_coloring(
        rect,
        view,
        &columns(1.0),
        1,
        1.2,
        color,
        1.0,
        neutral,
        1.0,
        true,
    );
    let output = ctx.run(Default::default(), |ctx| {
        let painter = ctx.layer_painter(egui::LayerId::background());
        normal.paint(&painter);
        fitted.paint(&painter);
        loud_with_headroom.paint(&painter);
    });
    let meshes: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| {
            if let Shape::Mesh(mesh) = &shape.shape {
                Some(mesh)
            } else {
                None
            }
        })
        .collect();
    let solid = |index: usize| {
        meshes[index]
            .vertices
            .iter()
            .filter(|vertex| vertex.color.a() > 0)
            .map(|vertex| vertex.color)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        solid(0),
        solid(1),
        "Fit changes height while retaining actual audio-level colors"
    );
    assert!(
        solid(0)
            .iter()
            .all(|fill| *fill != color && distance(*fill, neutral) < distance(*fill, color))
    );
    assert!(
        solid(2).iter().all(|fill| *fill == color),
        "0 dBFS reaches full clip color even with display headroom"
    );
    assert_ne!(meshes[0].vertices[1].pos, meshes[1].vertices[1].pos);
}

fn contrast(a: Color32, b: Color32) -> f32 {
    let luminance = |color: Color32| {
        let rgb = color.to_srgba_unmultiplied();
        let linear = |value: u8| {
            let value = f32::from(value) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(rgb[0]) + 0.7152 * linear(rgb[1]) + 0.0722 * linear(rgb[2])
    };
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn quiet_endpoint_retains_readable_contrast_and_desaturated_hue_in_both_themes() {
    for palette in [theme::LIGHT, theme::DARK] {
        for peak in [
            Color32::from_rgb(45, 180, 210),
            Color32::from_rgb(180, 70, 95),
            Color32::from_rgb(60, 150, 95),
        ] {
            let quiet = daw_ui::waveform_quiet_color(peak, palette.muted);
            for background in [palette.bg, palette.raised] {
                assert!(
                    contrast(quiet, background) >= 3.0,
                    "quiet endpoint must be visible over waveform surfaces"
                );
                assert!(
                    contrast(peak, background) >= 2.0,
                    "representative saturated endpoint must also be visible"
                );
            }
            let chroma = |color: Color32| {
                let rgb = [color.r(), color.g(), color.b()];
                rgb.iter().max().unwrap() - rgb.iter().min().unwrap()
            };
            assert!(chroma(quiet) < chroma(peak) / 2);
            assert_ne!(
                quiet, palette.line,
                "center-line tone must remain independent"
            );
            if palette.bg.intensity() > 0.5 {
                assert!(quiet.intensity() < 0.5);
            } else {
                assert!(quiet.intensity() > 0.5);
            }
        }
    }
}

#[test]
fn both_theme_endpoints_cover_extreme_clip_colors_and_styled_fallback_matches_mesh() {
    for palette in [theme::LIGHT, theme::DARK] {
        for clip in [
            Color32::BLACK,
            Color32::WHITE,
            Color32::YELLOW,
            Color32::from_rgb(20, 225, 245),
            Color32::from_rgb(4, 10, 40),
        ] {
            let peak = daw_ui::waveform_peak_color(clip, palette);
            let quiet = daw_ui::waveform_quiet_color(clip, palette.muted);
            for background in [palette.bg, palette.raised] {
                assert!(
                    contrast(peak, background) >= 3.15,
                    "peak must remain readable: {clip:?} → {peak:?}"
                );
                assert!(
                    contrast(quiet, background) >= 3.0,
                    "quiet must remain readable: {clip:?}"
                );
            }
            if contrast(clip, palette.bg) >= 3.2 && contrast(clip, palette.raised) >= 3.2 {
                assert_eq!(peak, clip);
            }
            let rect = Rect::from_min_max(pos2(10.0, 20.0), pos2(210.0, 220.0));
            let view = TimelineViewport {
                pixels_per_beat: 50.0,
                row_height: 100.0,
                ..Default::default()
            };
            let style = daw_ui::WaveformColorStyle {
                peak,
                quiet,
                center: palette.line,
                amplitude_reference: 1.0,
                amplitude_coloring: true,
            };
            let prepared =
                daw_ui::prepare_audio_waveform_styled(rect, view, &columns(), 1, 1.0, 1.0, style);
            let ctx = Context::default();
            let output = ctx.run(Default::default(), |ctx| {
                let painter = ctx.layer_painter(egui::LayerId::background());
                daw_ui::paint_audio_waveform_styled(
                    &painter,
                    rect,
                    view,
                    &columns(),
                    1,
                    1.0,
                    style,
                );
                prepared.paint(&painter);
            });
            let lines: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let Shape::LineSegment { stroke, .. } = &shape.shape {
                        Some(stroke.color)
                    } else {
                        None
                    }
                })
                .collect();
            let mesh = output
                .shapes
                .iter()
                .find_map(|shape| {
                    if let Shape::Mesh(mesh) = &shape.shape {
                        Some(mesh)
                    } else {
                        None
                    }
                })
                .unwrap();
            assert_eq!(lines[0], palette.line);
            assert_eq!(lines[5], palette.line);
            let fills = mesh
                .vertices
                .as_chunks::<4>()
                .0
                .iter()
                .map(|vertices| vertices[1].color)
                .collect::<Vec<_>>();
            assert_eq!(&lines[1..5], fills);
            assert_eq!(fills[0], quiet);
            assert_eq!(fills[3], peak);
            assert_eq!(mesh.vertices.len(), 16);
            assert_eq!(mesh.indices.len(), 54);
        }
        let transparent = Color32::from_rgba_unmultiplied(255, 240, 10, 100);
        assert_eq!(daw_ui::waveform_peak_color(transparent, palette).a(), 100);
    }
}
