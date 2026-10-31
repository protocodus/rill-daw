//! Landmark glyphs identify actual values without changing slider interaction.
use daw_ui::{SliderScaleGeometry, SliderValueMarker, slider_value_marker, theme};
use egui::{Color32, Context, FullOutput, RawInput, Rect, Shape, pos2, vec2};
use std::ops::RangeInclusive;

#[test]
fn landmarks_resolve_overlap_and_preserve_numeric_meaning() {
    use SliderValueMarker::{Center, Maximum, Minimum, Zero};
    for (value, range, expected) in [
        (0.0, 0.0..=1.0, Minimum),
        (1.0, 0.0..=1.0, Maximum),
        (0.5, 0.0..=1.0, Center),
        (0.0, -1.0..=0.0, Maximum),
        (0.0, -1.0..=1.0, Zero),
        (-0.0, -1.0..=1.0, Zero),
        (0.0, -2.0..=6.0, Zero),
        (2.0, -2.0..=6.0, Center),
        (-1.0, 1.0..=-1.0, Minimum),
        (1.0, 1.0..=-1.0, Maximum),
        (2.0, 6.0..=-2.0, Center),
        (f64::from(0.2_f32), 0.1..=0.3, Center),
        (5e-20, 1e-20..=9e-20, Center),
        (1e-20, 1e-20..=9e-20, Minimum),
        (9e-20, 1e-20..=9e-20, Maximum),
    ] {
        assert_eq!(
            slider_value_marker(value, range.clone()),
            Some(expected),
            "value {value} in {range:?}"
        );
    }
    for (value, range) in [
        (0.001, -1.0..=1.0),
        (0.5001, 0.0..=1.0),
        (0.9999, 0.0..=1.0),
        (0.0001, 0.0..=1.0),
        (0.25, 0.0..=1.0),
        (6e-20, 1e-20..=9e-20),
        (0.0, 1e-20..=9e-20),
        (-1.000001, -1.0..=1.0),
        (1.000001, -1.0..=1.0),
        (0.0, 0.0..=0.0),
        (f64::NAN, -1.0..=1.0),
        (f64::INFINITY, -1.0..=1.0),
        (0.0, f64::NEG_INFINITY..=1.0),
        (0.0, -1.0..=f64::NAN),
    ] {
        assert_eq!(
            slider_value_marker(value, range.clone()),
            None,
            "ordinary, invalid or out-of-range value {value} in {range:?}"
        );
    }
}

fn shapes(output: &FullOutput) -> Vec<&Shape> {
    fn collect<'a>(shape: &'a Shape, flat: &mut Vec<&'a Shape>) {
        match shape {
            Shape::Vec(children) => {
                for child in children {
                    collect(child, flat);
                }
            }
            _ => flat.push(shape),
        }
    }
    let mut flat = Vec::new();
    for shape in &output.shapes {
        collect(&shape.shape, &mut flat);
    }
    flat
}

fn assert_marker(
    output: &FullOutput,
    geometry: &SliderScaleGeometry,
    expected: SliderValueMarker,
    color: Color32,
) {
    let matching: Vec<_> = shapes(output)
        .into_iter()
        .filter(|shape| match shape {
            Shape::Path(path) => {
                path.closed
                    && path.points.len() == 3
                    && path.fill == color
                    && Rect::from_points(&path.points)
                        .center()
                        .distance(geometry.handle)
                        < 0.001
            }
            Shape::Circle(circle) => {
                circle.center.distance(geometry.handle) < 0.001
                    && (circle.fill == color || circle.stroke.color == color)
            }
            _ => false,
        })
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "one landmark at the handle: {matching:?}"
    );
    let bounds = match (expected, matching[0]) {
        (SliderValueMarker::Minimum | SliderValueMarker::Maximum, Shape::Path(path)) => {
            let bounds = Rect::from_points(&path.points);
            let tip_y = if expected == SliderValueMarker::Minimum {
                bounds.bottom()
            } else {
                bounds.top()
            };
            let tips: Vec<_> = path
                .points
                .iter()
                .filter(|point| (point.y - tip_y).abs() < 0.001)
                .collect();
            assert_eq!(tips.len(), 1, "triangle points down at min and up at max");
            assert!((tips[0].x - geometry.handle.x).abs() < 0.001);
            assert!(bounds.width() >= 1.0 && bounds.height() >= 1.0);
            bounds
        }
        (SliderValueMarker::Center, Shape::Circle(circle)) => {
            assert_eq!(circle.fill, color, "center is a filled dot");
            assert!(circle.radius >= 0.5);
            Rect::from_center_size(
                circle.center,
                vec2(circle.radius * 2.0, circle.radius * 2.0),
            )
        }
        (SliderValueMarker::Zero, Shape::Circle(circle)) => {
            assert_eq!(circle.fill, Color32::TRANSPARENT, "zero is a hollow circle");
            assert_eq!(circle.stroke.color, color);
            assert!(circle.stroke.width >= 0.5);
            assert!(circle.radius >= 0.5);
            let extent = circle.radius + circle.stroke.width * 0.5;
            Rect::from_center_size(circle.center, vec2(extent * 2.0, extent * 2.0))
        }
        (expected, shape) => panic!("Wrong shape for {expected:?}: {shape:?}"),
    };
    let thumb = Rect::from_center_size(geometry.handle, geometry.handle_half_extent * 2.0);
    assert!(
        thumb.expand(0.01).contains_rect(bounds),
        "{bounds:?} outside {thumb:?}"
    );
    assert!(geometry.rail.expand(0.01).contains_rect(bounds));
    assert!(
        !shapes(output).iter().any(|shape| matches!(shape,
            Shape::LineSegment { points, stroke }
                if stroke.width == 1.5 && points[0].lerp(points[1], 0.5).distance(geometry.handle) < 0.001)),
        "special glyph replaces the handle index instead of overlapping it"
    );
}

fn slider_frame(
    ctx: &Context,
    value: &mut f64,
    range: RangeInclusive<f64>,
    vertical: bool,
    compact: bool,
    enabled: bool,
) -> (SliderScaleGeometry, FullOutput) {
    let mut scale = None;
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.spacing_mut().slider_width = 180.0;
                ui.add_enabled_ui(enabled, |ui| {
                    let mut slider = daw_ui::slider(value, range.clone())
                        .clamping(egui::SliderClamping::Edits)
                        .show_value(false)
                        .handle_shape(egui::style::HandleShape::Rect {
                            aspect_ratio: daw_ui::tokens::GOLDEN_RATIO_INVERSE,
                        });
                    if vertical {
                        slider = slider.vertical();
                    }
                    if compact {
                        slider = slider.compact();
                    }
                    let response = ui.add(slider);
                    assert!(!response.changed(), "painting cannot edit a parameter");
                    scale = daw_ui::slider_scale_geometry(ctx, response.id);
                });
            });
        },
    );
    (scale.unwrap(), output)
}

#[test]
fn native_sliders_paint_distinct_bounded_landmarks_at_every_size_and_density() {
    use SliderValueMarker::{Center, Maximum, Minimum, Zero};
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for density in [1.0, 1.25, 2.0] {
            let ctx = Context::default();
            theme::apply(&ctx);
            theme::set_appearance(&ctx, appearance);
            ctx.set_pixels_per_point(density);
            let _ = ctx.run(RawInput::default(), |_| {});
            for vertical in [false, true] {
                for compact in [false, true] {
                    for enabled in [false, true] {
                        for (stored, range, expected) in [
                            (0.0, 0.0..=1.0, Minimum),
                            (1.0, 0.0..=1.0, Maximum),
                            (0.5, 0.0..=1.0, Center),
                            (0.0, -1.0..=1.0, Zero),
                            (0.0, -2.0..=6.0, Zero),
                        ] {
                            let mut value = stored;
                            let (scale, output) =
                                slider_frame(&ctx, &mut value, range, vertical, compact, enabled);
                            let colors = theme::palette(&ctx);
                            assert_marker(
                                &output,
                                &scale,
                                expected,
                                if enabled {
                                    colors.text
                                } else {
                                    colors
                                        .muted
                                        .gamma_multiply(ctx.style().visuals.disabled_alpha())
                                },
                            );
                            assert_eq!(value, stored);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn arranger_mix_rails_reuse_landmarks_without_changing_channel_values() {
    for (gain, pan, gain_marker, pan_marker) in [
        (
            0.0,
            -1.0,
            SliderValueMarker::Minimum,
            SliderValueMarker::Minimum,
        ),
        (1.0, 0.0, SliderValueMarker::Zero, SliderValueMarker::Zero),
        (
            1.5,
            1.0,
            SliderValueMarker::Maximum,
            SliderValueMarker::Maximum,
        ),
    ] {
        let ctx = Context::default();
        theme::apply(&ctx);
        let mut actual_gain = gain;
        let mut actual_pan = pan;
        let mut scales = None;
        let output = ctx.run(RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let response = daw_ui::track_mix_controls_at(
                    ui,
                    Rect::from_min_size(pos2(40.0, 40.0), vec2(240.0, 42.0)),
                    "markers",
                    &mut actual_gain,
                    &mut actual_pan,
                    false,
                    false,
                );
                assert!(!response.gain.changed() && !response.pan.changed());
                scales = Some((response.gain_scale.unwrap(), response.pan_scale.unwrap()));
            });
        });
        let (gain_scale, pan_scale) = scales.unwrap();
        let color = theme::palette(&ctx).text;
        assert_marker(&output, &gain_scale, gain_marker, color);
        assert_marker(&output, &pan_scale, pan_marker, color);
        assert_eq!((actual_gain, actual_pan), (gain, pan));
    }
}

#[test]
fn gain_sliders_mark_unity_as_zero_while_preserving_the_native_range() {
    use SliderValueMarker::{Center, Maximum, Minimum, Zero};
    for vertical in [false, true] {
        for (stored, expected) in [
            (0.0_f32, Minimum),
            (0.75, Center),
            (1.0, Zero),
            (1.5, Maximum),
        ] {
            let ctx = Context::default();
            theme::apply(&ctx);
            let mut value = stored;
            let mut scale = None;
            let output = ctx.run(RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.spacing_mut().slider_width = 180.0;
                    let mut slider = daw_ui::slider(&mut value, 0.0..=1.5)
                        .zero_reference(1.0)
                        .show_value(false)
                        .handle_shape(egui::style::HandleShape::Rect {
                            aspect_ratio: daw_ui::tokens::GOLDEN_RATIO_INVERSE,
                        });
                    if vertical {
                        slider = slider.vertical();
                    }
                    let response = ui.add(slider);
                    assert!(!response.changed());
                    scale = daw_ui::slider_scale_geometry(ctx, response.id);
                });
            });
            let scale = scale.unwrap();
            assert_eq!(scale.value_marker(), Some(expected));
            assert_marker(&output, &scale, expected, theme::palette(&ctx).text);
            assert!(
                scale
                    .handle
                    .distance(scale.minimum.lerp(scale.maximum, stored / 1.5))
                    < 0.001,
                "displayed 0 dB must not shift native gain mapping"
            );
            assert_eq!(
                value, stored,
                "painting must preserve the stored linear gain"
            );
        }
    }
}
