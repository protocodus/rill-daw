//! Musical labels remain useful when a close view contains no bar line.
use daw_ui::{TimelineViewport, paint_timeline_ruler, theme, tokens};
use egui::{Context, Pos2, RawInput, Rect, Shape, Vec2, pos2};

#[derive(Debug)]
struct Label {
    text: String,
    position: Pos2,
    bounds: Rect,
}

struct Ruler {
    labels: Vec<Label>,
    ticks: Vec<[Pos2; 2]>,
}

fn render(ctx: &Context, rect: Rect, start: f64, scale: f32, meter: u8) -> Ruler {
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 300.0))),
            ..Default::default()
        },
        |ctx| {
            paint_timeline_ruler(
                &ctx.layer_painter(egui::LayerId::background()),
                rect,
                TimelineViewport {
                    start_beat: start,
                    pixels_per_beat: scale,
                    ..Default::default()
                },
                100.0,
                meter,
            );
        },
    );
    let mut ruler = Ruler {
        labels: Vec::new(),
        ticks: Vec::new(),
    };
    for painted in output.shapes {
        match painted.shape {
            Shape::Text(text) => {
                let bounds = text
                    .galley
                    .rect
                    .union(text.galley.mesh_bounds)
                    .translate(text.pos.to_vec2());
                assert!(
                    rect.intersect(painted.clip_rect).contains_rect(bounds),
                    "A partial ruler label can imply the wrong musical position: {bounds:?}"
                );
                ruler.labels.push(Label {
                    text: text.galley.text().to_owned(),
                    position: text.pos,
                    bounds,
                });
            }
            Shape::LineSegment { points, .. } => ruler.ticks.push(points),
            _ => {}
        }
    }
    ruler
}

fn label<'a>(ruler: &'a Ruler, value: &str) -> &'a Label {
    ruler
        .labels
        .iter()
        .find(|label| label.text == value)
        .unwrap_or_else(|| panic!("Missing {value}: {:?}", ruler.labels))
}

#[test]
fn a_view_between_whole_beats_keeps_musical_and_clock_anchors_aligned() {
    let rect = Rect::from_min_size(
        pos2(30.0, 30.0),
        Vec2::new(900.0, tokens::TIMELINE_RULER_HEIGHT),
    );
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for density in [1.0, 2.0] {
            let ctx = Context::default();
            theme::apply(&ctx);
            theme::set_appearance(&ctx, appearance);
            ctx.set_pixels_per_point(density);
            for (meter, bar_start, bar_number, clock) in [
                (3, 0.0, "1", "0:00.15"),
                (4, 0.0, "1", "0:00.15"),
                (3, 24.0, "9", "0:14.55"),
                (4, 32.0, "9", "0:19.35"),
            ] {
                let ruler = render(&ctx, rect, bar_start + 0.125, 1600.0, meter);
                assert!(
                    !ruler.labels.iter().any(|label| label.text == bar_number),
                    "The bar line lies offscreen in this fixture"
                );
                let quarter = label(&ruler, &format!("{bar_number}.1.2"));
                let elapsed = label(&ruler, clock);
                let anchor = rect.left() + 0.125 * 1600.0;
                for text in [quarter, elapsed] {
                    assert!((text.position.x - anchor - tokens::SPACE_XS).abs() < 0.01);
                }
                assert!(quarter.bounds.bottom() + 2.0 <= elapsed.bounds.top());
                assert!(
                    ruler.ticks.iter().any(|points| {
                        (points[0].x - anchor).abs() < 0.01
                            && points[0].x == points[1].x
                            && (points[1].y - points[0].y - 3.0).abs() < 0.01
                    }),
                    "The finer tick must share the label and canvas time anchor"
                );
            }
        }
    }
}

#[test]
fn zoom_levels_add_useful_detail_without_overlapping_or_partial_labels() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        for density in [1.0, 2.0] {
            let ctx = Context::default();
            theme::apply(&ctx);
            theme::set_appearance(&ctx, appearance);
            ctx.set_pixels_per_point(density);
            for meter in [3, 4] {
                for width in [96.0, 400.0, 1200.0] {
                    for scale in [48.0, 80.0, 160.0, 320.0] {
                        let rect = Rect::from_min_size(
                            pos2(30.0, 30.0),
                            Vec2::new(width, tokens::TIMELINE_RULER_HEIGHT),
                        );
                        for start in [0.0, 0.125, 40_000.125] {
                            let ruler = render(&ctx, rect, start, scale, meter);
                            for (index, current) in ruler.labels.iter().enumerate() {
                                for previous in &ruler.labels[..index] {
                                    assert!(
                                        !current.bounds.intersects(previous.bounds),
                                        "Labels collide at scale {scale}, start {start}: {previous:?}, {current:?}"
                                    );
                                }
                            }
                            if scale < 80.0 {
                                assert!(ruler.labels.iter().all(|label| {
                                    label.text.contains(':') || !label.text.contains('.')
                                }));
                            }
                        }
                    }
                }
            }
            let roomy = Rect::from_min_size(
                pos2(30.0, 30.0),
                Vec2::new(900.0, tokens::TIMELINE_RULER_HEIGHT),
            );
            let beats = render(&ctx, roomy, 0.0, 80.0, 4);
            assert_eq!(label(&beats, "1").text, "1");
            assert_eq!(label(&beats, "1.2").text, "1.2");
            assert!(!beats.labels.iter().any(|label| label.text == "1.1.3"));
            let halves = render(&ctx, roomy, 0.0, 160.0, 4);
            label(&halves, "1.1.3");
            assert!(!halves.labels.iter().any(|label| label.text == "1.1.2"));
            let quarters = render(&ctx, roomy, 0.0, 320.0, 4);
            label(&quarters, "1.1.2");
            label(&quarters, "1.2");
            label(&quarters, "1");
        }
    }
}

#[test]
fn invalid_viewports_cannot_submit_nonfinite_labels_or_ticks() {
    let ctx = Context::default();
    theme::apply(&ctx);
    let rect = Rect::from_min_size(
        pos2(30.0, 30.0),
        Vec2::new(900.0, tokens::TIMELINE_RULER_HEIGHT),
    );
    for (rect, start, scale) in [
        (rect, f64::NAN, 1600.0),
        (rect, 0.0, f32::NAN),
        (rect, 0.0, f32::INFINITY),
        (rect, 0.0, 0.0),
        (Rect::NOTHING, 0.0, 1600.0),
    ] {
        let ruler = render(&ctx, rect, start, scale, 4);
        assert!(ruler.labels.is_empty() && ruler.ticks.is_empty());
    }
}

#[test]
fn elapsed_clock_carries_rounded_seconds_and_keeps_only_useful_precision() {
    for (seconds, precision, expected) in [
        (0.0, 2, "0:00"),
        (15.0, 2, "0:15"),
        (165.0, 2, "2:45"),
        (18_955.0, 2, "5:15:55"),
        (0.15, 2, "0:00.15"),
        (1.5, 2, "0:01.5"),
        (1.234, 3, "0:01.234"),
        (1.234, usize::MAX, "0:01.234"),
        (59.999, 2, "1:00"),
        (3599.999, 2, "1:00:00"),
        (3599.9996, 3, "1:00:00"),
        (59.6, 0, "1:00"),
        (-1.0, 2, "0:00"),
        (f64::NAN, 2, "—"),
        (f64::INFINITY, 2, "—"),
    ] {
        assert_eq!(
            daw_ui::format_clock_time(seconds, precision),
            expected,
            "{seconds} with {precision} fractional digits"
        );
    }
}
