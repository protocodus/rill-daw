//! One inexpensive focus layer for on-demand windows and native popup controls.
use egui::{Context, Id, LayerId, Order};

fn backdrop_layer(order: Order) -> LayerId {
    LayerId::new(order, Id::new("rill_popup_backdrop"))
}

pub(crate) fn is_popup_backdrop_layer(layer: LayerId) -> bool {
    layer.id == backdrop_layer(layer.order).id
}

/// Paint once after the workspace and its utilities. A window leaves the caller
/// visible beneath a dim layer; an anchored menu dims the window as well. Native
/// popup layers keep their order above the dimming. This adds no hit targets,
/// focus, animation, or background processing. Tooltips do not dim the screen.
pub fn paint_popup_backdrop(ctx: &Context, utility_open: bool) {
    crate::backdrop_blur::begin_blur_frame(ctx);
    let native_popup = egui::Popup::is_any_open(ctx);
    if !utility_open && !native_popup {
        return;
    }
    let layer = backdrop_layer(if native_popup {
        Order::Foreground
    } else {
        Order::Background
    });
    ctx.move_to_top(layer);
    if native_popup {
        // Place all current menus (including submenus) above a single scrim.
        // Sublayers preserve their native relative order, so a newly opened
        // menu or a previously visited window cannot end up under the scrim.
        let popups = ctx.memory(|memory| {
            memory
                .layer_ids()
                .filter(|id| id.order == Order::Foreground && *id != layer)
                .collect::<Vec<_>>()
        });
        for popup in popups {
            ctx.set_sublayer(layer, popup);
        }
    }
    let painter = ctx.layer_painter(layer);
    crate::backdrop_blur::paint_utility_blur(ctx, &painter, native_popup);
    painter.rect_filled(
        ctx.content_rect(),
        0.0,
        crate::theme::palette(ctx).popup_scrim,
    );
}
