use egui::{Context, Id, RawInput};

fn controls(ctx: &Context, draw: bool) -> Vec<Id> {
    let mut ids = Vec::new();
    let _ = ctx.run(RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            if !draw {
                return;
            }
            let mut value = 0.5_f32;
            ids.push(
                ui.add(daw_ui::slider(&mut value, 0.0..=1.0).show_value(false))
                    .id,
            );
            ids.push(ui.add(daw_ui::knob(&mut value, 0.0..=1.0).compact()).id);
            let rect =
                egui::Rect::from_min_size(ui.next_widget_position(), egui::vec2(235.0, 21.0));
            let mut pan = 0.0;
            ids.push(
                daw_ui::track_mix_controls_at(
                    ui, rect, "compact", &mut value, &mut pan, false, false,
                )
                .gain
                .id,
            );
            ui.allocate_space(rect.size());
            ids.push(ui.text_edit_singleline(&mut String::new()).id);
            ids.push(ui.add(daw_ui::number(&mut value)).id);
        });
    });
    ids
}

#[test]
fn only_focused_non_text_parameters_claim_document_history() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let ids = controls(&ctx, true);
    assert!(!daw_ui::parameter_owns_document_history(&ctx));
    for (index, id) in ids.iter().enumerate() {
        ctx.memory_mut(|memory| memory.request_focus(*id));
        controls(&ctx, true);
        assert!(ctx.memory(|memory| memory.has_focus(*id)));
        assert!(ctx.wants_keyboard_input());
        assert_eq!(
            daw_ui::parameter_owns_document_history(&ctx),
            index < 3,
            "Only rail, dial and relative value can claim document Undo; control {index}"
        );
    }
}

#[test]
fn popup_and_disappearing_parameters_cannot_reuse_a_stale_focus_claim() {
    let ctx = Context::default();
    daw_ui::theme::apply(&ctx);
    let ids = controls(&ctx, true);
    ctx.memory_mut(|memory| memory.request_focus(ids[0]));
    controls(&ctx, true);
    assert!(daw_ui::parameter_owns_document_history(&ctx));
    egui::Popup::open_id(&ctx, Id::new("exact_value"));
    assert!(!daw_ui::parameter_owns_document_history(&ctx));
    egui::Popup::close_all(&ctx);
    controls(&ctx, false);
    controls(&ctx, false);
    assert!(!daw_ui::parameter_owns_document_history(&ctx));
}
