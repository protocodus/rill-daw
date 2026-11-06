//! Explicit primary fills must survive a menu's otherwise quiet row treatment.
use daw_ui::{self as controls, IconButton, theme};
use egui::{Color32, Context, Event, FullOutput, Modifiers, RawInput, Rect, Response, Shape};

const CUSTOM: Color32 = Color32::from_rgb(187, 93, 32);

struct Frame {
    opener: Response,
    rows: Vec<Response>,
    output: FullOutput,
}

fn frame(ctx: &Context, events: Vec<Event>, enabled: bool) -> Frame {
    let mut opener = None;
    let mut rows = Vec::new();
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 800.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                opener = Some(
                    controls::choice_menu(ui, "Properties", |ui| {
                        let style = ui.style().clone();
                        rows.push(
                            ui.add_enabled_ui(enabled, |ui| {
                                controls::dialog_primary(ui, "Apply value")
                            })
                            .inner,
                        );
                        rows.push(controls::button(ui, "Ordinary row"));
                        rows.push(ui.add(IconButton::new("Selected row").selected(true)));
                        rows.push(ui.add(IconButton::new("Explicit fill").fill(CUSTOM)));
                        rows.push(
                            ui.add(
                                IconButton::new("Quiet fill")
                                    .fill(CUSTOM)
                                    .frame_when_inactive(false),
                            ),
                        );
                        rows.push(
                            ui.add(IconButton::new("Frameless fill").fill(CUSTOM).frame(false)),
                        );
                        rows.push(
                            ui.add(IconButton::new("Transparent fill").fill(Color32::TRANSPARENT)),
                        );
                        rows.push(
                            ui.add(
                                IconButton::new("Selected fill")
                                    .fill(CUSTOM)
                                    .selected(true)
                                    .frame_when_inactive(false),
                            ),
                        );
                        assert_eq!(*ui.style(), style, "button styles remain local");
                    })
                    .response,
                );
            });
        },
    );
    Frame {
        opener: opener.unwrap(),
        rows,
        output,
    }
}

fn pointer(pos: egui::Pos2, pressed: bool) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        },
    ]
}

fn open(ctx: &Context) -> Frame {
    let initial = frame(ctx, vec![], true);
    let target = initial.opener.rect.center();
    frame(ctx, pointer(target, true), true);
    frame(ctx, pointer(target, false), true);
    frame(ctx, vec![], true);
    frame(ctx, vec![Event::PointerGone], true)
}

fn fill(output: &FullOutput, rect: Rect) -> Color32 {
    fn visit(shape: &Shape, rect: Rect) -> Option<Color32> {
        match shape {
            Shape::Vec(shapes) => shapes.iter().find_map(|shape| visit(shape, rect)),
            Shape::Rect(shape)
                if shape.rect == rect
                    && shape.blur_width == 0.0
                    && shape.brush.is_none()
                    && shape.fill != Color32::TRANSPARENT =>
            {
                Some(shape.fill)
            }
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|shape| visit(&shape.shape, rect))
        .unwrap_or(Color32::TRANSPARENT)
}

fn text_color(ctx: &Context, output: &FullOutput, label: &str) -> Color32 {
    let shape = output
        .shapes
        .iter()
        .find(|shape| matches!(&shape.shape, Shape::Text(text) if text.galley.text() == label))
        .unwrap_or_else(|| panic!("Missing painted {label}"));
    // Painter opacity can already be baked into galley mesh vertices while
    // layout-job colors remain unchanged. Inspect the glyphs sent to the
    // renderer instead of inferring their color from layout metadata.
    let mut colors = Vec::new();
    for primitive in ctx.tessellate(vec![shape.clone()], output.pixels_per_point) {
        if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
            for vertex in mesh.vertices {
                if vertex.color != Color32::TRANSPARENT && !colors.contains(&vertex.color) {
                    colors.push(vertex.color);
                }
            }
        }
    }
    assert_eq!(colors.len(), 1, "plain {label} glyph colors: {colors:?}");
    colors[0]
}

fn contrast(a: Color32, b: Color32) -> f32 {
    let luminance = |color: Color32| {
        let channel = |byte: u8| {
            let value = f32::from(byte) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
    };
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn primary_menu_fills_keep_readable_text_without_changing_quiet_or_selected_rows() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        let ctx = Context::default();
        theme::apply(&ctx);
        theme::set_appearance(&ctx, appearance);
        ctx.style_mut(|style| style.animation_time = 0.0);
        let colors = theme::palette(&ctx);
        let idle = open(&ctx);
        assert_eq!(idle.rows.len(), 8);
        let primary = &idle.rows[0];
        assert!(!primary.hovered() && !primary.has_focus());
        let background = fill(&idle.output, primary.rect);
        let foreground = text_color(&ctx, &idle.output, "Apply value");
        assert_eq!(background, colors.action);
        assert_eq!(foreground, theme::contrast_text(background));
        assert!(contrast(foreground, background) >= 4.5);
        assert_eq!(fill(&idle.output, idle.rows[1].rect), Color32::TRANSPARENT);
        assert_eq!(text_color(&ctx, &idle.output, "Ordinary row"), colors.text);
        assert_eq!(fill(&idle.output, idle.rows[2].rect), colors.track_selected);
        assert_eq!(fill(&idle.output, idle.rows[3].rect), CUSTOM);
        for index in [4, 5, 6] {
            assert_eq!(
                fill(&idle.output, idle.rows[index].rect),
                Color32::TRANSPARENT
            );
        }
        assert_eq!(fill(&idle.output, idle.rows[7].rect), CUSTOM);

        let hovered = frame(&ctx, vec![Event::PointerMoved(primary.rect.center())], true);
        assert!(hovered.rows[0].hovered());
        assert_eq!(fill(&hovered.output, hovered.rows[0].rect), colors.action);
        for (before, after) in idle.rows.iter().zip(&hovered.rows) {
            assert_eq!(
                before.rect, after.rect,
                "painting must not change hit geometry"
            );
            assert_eq!(before.id, after.id);
        }
    }
}

#[test]
fn primary_menu_buttons_retain_disabled_dimming_and_native_activation() {
    for appearance in [theme::Appearance::Light, theme::Appearance::Dark] {
        let ctx = Context::default();
        theme::apply(&ctx);
        theme::set_appearance(&ctx, appearance);
        ctx.style_mut(|style| style.animation_time = 0.0);
        let enabled = open(&ctx);
        let primary = &enabled.rows[0];
        let disabled = frame(&ctx, vec![], false);
        assert_eq!(disabled.rows[0].rect, primary.rect);
        assert_eq!(disabled.rows[0].id, primary.id);
        assert!(!disabled.rows[0].enabled());
        let disabled_alpha = ctx.style().visuals.disabled_alpha();
        assert_eq!(
            fill(&disabled.output, primary.rect),
            fill(&enabled.output, primary.rect).gamma_multiply(disabled_alpha),
            "native disabled opacity still applies to the explicit fill"
        );
        let disabled_text = text_color(&ctx, &disabled.output, "Apply value");
        let enabled_text = text_color(&ctx, &enabled.output, "Apply value");
        assert_eq!(
            disabled_text,
            enabled_text.gamma_multiply(disabled_alpha),
            "effective glyph color retains the native disabled opacity"
        );
        assert!(disabled_text.a() < enabled_text.a());
        frame(&ctx, pointer(primary.rect.center(), true), false);
        assert!(!frame(&ctx, pointer(primary.rect.center(), false), false).rows[0].clicked());

        // Native hit testing uses the prior frame's enabled state. Present
        // the re-enabled control before beginning the next pointer gesture.
        frame(&ctx, vec![], true);
        frame(&ctx, pointer(primary.rect.center(), true), true);
        assert!(frame(&ctx, pointer(primary.rect.center(), false), true).rows[0].clicked());
        primary.request_focus();
        let activated = frame(
            &ctx,
            vec![Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            true,
        );
        assert!(activated.rows[0].clicked());
    }
}
