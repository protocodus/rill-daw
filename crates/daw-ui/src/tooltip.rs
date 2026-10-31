//! Read-only tooltips that never intercept a click on the workspace below.
use egui::{
    Context, Frame, Id, LayerId, Order, PopupAnchor, Pos2, Rect, RectAlign, Response, WidgetText,
};

/// Add a native-styled, delayed text hint without creating a pointer-blocking
/// overlay. Repeated hints for one response stack in call order. Use native
/// `on_hover_ui` when a tooltip intentionally contains interactive content.
///
/// In egui 0.33 even an `Area::interactable(false)` registers hover rectangles
/// that occlude lower layers during hit testing. Paint the native frame and text
/// directly: no widgets, hit rectangles, accessibility duplicates, or event
/// forwarding are needed for an explanatory hint.
pub fn hover_text(response: Response, text: impl Into<WidgetText>) -> Response {
    if !response.enabled() || !egui::Tooltip::should_show_tooltip(&response, false) {
        return response;
    }
    let ctx = &response.ctx;
    let pass = ctx.cumulative_pass_nr();
    let stack_id = response.id.with("read_only_tooltip_stack");
    let previous = ctx.data(|data| data.get_temp::<(u64, Rect, usize)>(stack_id));
    let (bounds, count) = previous
        .filter(|(last_pass, _, _)| *last_pass == pass)
        .map(|(_, bounds, count)| (bounds, count))
        .unwrap_or_else(|| {
            (
                PopupAnchor::from(&response)
                    .rect(response.id, ctx)
                    .unwrap_or(response.interact_rect),
                0,
            )
        });
    let rect = paint_read_only_text(
        ctx,
        response.id.with(("read_only_tooltip", count)),
        bounds,
        text.into(),
    );
    ctx.data_mut(|data| {
        data.insert_temp(stack_id, (pass, bounds.union(rect), count + 1));
    });
    response
}

/// Immediate, noninteractive feedback for an active pointer gesture. Call only
/// while the gesture is held; no hover delay, persistent state, or input target
/// is created. The returned bounds and painting stay within the viewport.
pub fn pointer_text(ctx: &Context, id: Id, pointer: Pos2, text: impl Into<WidgetText>) -> Rect {
    let anchor = Rect::from_center_size(
        pointer,
        egui::vec2(crate::tokens::SPACE_XL, crate::tokens::SPACE_XL),
    );
    paint_read_only_text(ctx, id, anchor, text.into())
}

fn paint_read_only_text(ctx: &Context, id: Id, bounds: Rect, text: WidgetText) -> Rect {
    let style = ctx.style();
    let frame = Frame::popup(&style);
    let screen = ctx.content_rect();
    let galley = text.into_galley_impl(
        ctx,
        &style,
        egui::text::TextWrapping {
            max_width: style
                .spacing
                .tooltip_width
                .min((screen.width() - frame.total_margin().sum().x).max(1.0)),
            ..Default::default()
        },
        egui::TextStyle::Body.into(),
        egui::Align::Min,
    );
    let size = galley.size() + frame.total_margin().sum();
    let gap = crate::tokens::SPACE_XS;
    let align = RectAlign::find_best_align(
        std::iter::once(RectAlign::BOTTOM_START)
            .chain(RectAlign::BOTTOM_START.symmetries())
            .chain(RectAlign::MENU_ALIGNS),
        screen,
        bounds,
        gap,
        size,
    )
    .unwrap_or(RectAlign::BOTTOM_START);
    let mut rect = align.align_rect(&bounds, size, gap);
    // Match native constrained popup placement when a long hint cannot fit in
    // its preferred direction; retain a bounded viewport clip for tiny windows.
    let shift = egui::vec2(
        (screen.right() - rect.right())
            .min(0.0)
            .max(screen.left() - rect.left()),
        (screen.bottom() - rect.bottom())
            .min(0.0)
            .max(screen.top() - rect.top()),
    );
    rect = rect.translate(shift);
    let content = rect - frame.total_margin();
    let layer = LayerId::new(Order::Tooltip, id);
    ctx.move_to_top(layer);
    let painter = ctx.layer_painter(layer).with_clip_rect(screen);
    painter.add(frame.paint(content));
    painter.galley(content.min, galley, style.visuals.text_color());
    rect.intersect(screen)
}
