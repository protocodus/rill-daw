//! One inexpensive focus layer for on-demand windows and native popup controls.
use egui::{Color32, Context, Id, LayerId, Order};

fn backdrop_layer(order: Order) -> LayerId {
    LayerId::new(order, Id::new("rill_popup_backdrop"))
}

pub(crate) fn is_popup_backdrop_layer(layer: LayerId) -> bool {
    layer.id == backdrop_layer(layer.order).id
}

/// A quiet focus cue for routine menus, parameter editors and utility windows.
/// Explicit decision dialogs use a stronger scrim with the same bounded blur.
pub fn popup_backdrop_color(ctx: &Context) -> Color32 {
    crate::theme::palette(ctx)
        .popup_scrim
        .gamma_multiply(1.0 - crate::tokens::GOLDEN_RATIO_INVERSE)
}

/// Paint once after the workspace and its utilities. A window leaves the caller
/// readable beneath a light dim layer; an anchored menu dims the window as well. Native
/// popup layers keep their order above the dimming. This adds no hit targets,
/// focus, animation, or background processing. Tooltips do not dim the screen.
pub fn paint_popup_backdrop(ctx: &Context, utility_open: bool) {
    crate::backdrop_blur::begin_blur_frame(ctx);
    let native_popup = egui::Popup::is_any_open(ctx);
    if !utility_open && !native_popup {
        return;
    }
    // A modal already owns the focus layer. Never blur or dim it a second time.
    if !crate::backdrop_blur::backdrop_blur_info(ctx).is_empty() {
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
    crate::backdrop_blur::paint_modal_blur(ctx, &painter);
    painter.rect_filled(ctx.content_rect(), 0.0, popup_backdrop_color(ctx));
}
