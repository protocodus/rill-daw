//! Context-preserving navigation shared by utility windows.
use egui::{Context, Rect, Ui};
use std::sync::{Arc, Mutex};

/// A related destination. Application identifiers and navigation stay with the caller.
pub struct ViewLink<T> {
    pub value: T,
    pub label: &'static str,
    pub enabled: bool,
}

/// A compact, wrapping row of links with the current destination marked.
pub fn view_links<T: Copy + PartialEq>(
    ui: &mut Ui,
    current: T,
    links: &[ViewLink<T>],
) -> Option<T> {
    let mut selected = None;
    ui.horizontal_wrapped(|ui| {
        for link in links {
            if ui
                .add_enabled(
                    link.enabled,
                    crate::button_widget(link.label)
                        .selected(current == link.value)
                        .frame_when_inactive(current == link.value)
                        .frame(true)
                        .wrap_mode(egui::TextWrapMode::Extend),
                )
                .clicked()
            {
                selected = Some(link.value);
            }
        }
    });
    selected
}

/// Frozen paint of a caller view. It creates no widgets, focus, or hit targets.
/// Text is tessellated on replay so accessibility and automation only see the
/// active view. Arc sharing keeps nested navigation snapshots inexpensive.
#[derive(Clone)]
pub struct ViewBackdrop {
    shapes: Arc<Vec<egui::epaint::ClippedShape>>,
    // Keep preview textures alive even if their originating library view closes.
    _textures: Arc<Vec<egui::TextureHandle>>,
    paint_cache: Arc<Mutex<Option<BackdropPaint>>>,
    pub bounds: Rect,
}

struct BackdropPaint {
    density: f32,
    font_size: [usize; 2],
    shapes: Vec<egui::epaint::ClippedShape>,
}

impl Default for ViewBackdrop {
    fn default() -> Self {
        Self {
            shapes: Arc::default(),
            _textures: Arc::default(),
            paint_cache: Arc::default(),
            bounds: Rect::NOTHING,
        }
    }
}

impl ViewBackdrop {
    pub fn capture(ctx: &Context, bounds: Rect) -> Self {
        Self::capture_over(ctx, bounds, None, false)
    }

    /// Retain the caller of a modal, including foreground menus or an earlier
    /// modal. Capture before drawing the new dialog. Existing shared scrims are
    /// omitted so its own backdrop is the only dimming layer on replay.
    pub fn capture_dialog_context(ctx: &Context, bounds: Rect) -> Self {
        Self::capture_over(ctx, bounds, None, true)
    }

    /// Keep the original vector/text source underneath a new utility, rather
    /// than capturing already tessellated background text into another cache.
    pub fn capture_overlay(ctx: &Context, bounds: Rect, background: &Self) -> Self {
        Self::capture_over(ctx, bounds, Some(background), false)
    }

    fn capture_over(
        ctx: &Context,
        bounds: Rect,
        background: Option<&Self>,
        include_foreground: bool,
    ) -> Self {
        let layers = ctx.memory(|memory| memory.layer_ids().collect::<Vec<_>>());
        let maximum_order = if include_foreground {
            egui::Order::Foreground
        } else {
            egui::Order::Middle
        };
        let scrim = crate::theme::palette(ctx).popup_scrim;
        let content_rect = ctx.content_rect();
        let mut shapes =
            background.map_or_else(Vec::new, |background| background.shapes.as_ref().clone());
        for layer in layers {
            if layer.order > maximum_order
                || crate::popup_backdrop::is_popup_backdrop_layer(layer)
                || (background.is_some() && layer.order == egui::Order::Background)
            {
                continue;
            }
            let transform = ctx.layer_transform_to_global(layer);
            ctx.graphics(|graphics| {
                if let Some(list) = graphics.get(layer) {
                    for original in list.all_entries() {
                        let mut shape = original.clone();
                        if let Some(transform) = transform {
                            shape.transform(transform);
                        }
                        // egui Modal places its full-viewport scrim in the same
                        // layer as its contents. Keep the parent dialog but do
                        // not bake that scrim into the next dialog's backdrop.
                        if include_foreground
                            && layer.order == egui::Order::Foreground
                            && matches!(&shape.shape, egui::Shape::Rect(rect)
                                if rect.rect == content_rect
                                    && rect.fill == scrim
                                    && rect.stroke.width == 0.0
                                    && rect.brush.is_none())
                        {
                            continue;
                        }
                        shape.clip_rect = shape.clip_rect.intersect(bounds);
                        if shape.clip_rect.is_positive() {
                            shapes.push(shape);
                        }
                    }
                }
            });
        }
        let mut ids = std::collections::HashSet::new();
        fn textures(shape: &egui::Shape, ids: &mut std::collections::HashSet<egui::TextureId>) {
            match shape {
                egui::Shape::Mesh(mesh) => {
                    ids.insert(mesh.texture_id);
                }
                egui::Shape::Rect(rect) if rect.brush.is_some() => {
                    ids.insert(rect.fill_texture_id());
                }
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| textures(shape, ids)),
                _ => {}
            }
        }
        for shape in &shapes {
            textures(&shape.shape, &mut ids);
        }
        let manager = ctx.tex_manager();
        let retained = ids
            .into_iter()
            .filter_map(|id| {
                // User textures are owned by the renderer rather than egui's
                // texture manager; only retain managed textures that still exist.
                let mut textures = manager.write();
                textures.meta(id)?;
                textures.retain(id);
                drop(textures);
                Some(egui::TextureHandle::new(manager.clone(), id))
            })
            .collect();
        Self {
            shapes: Arc::new(shapes),
            _textures: Arc::new(retained),
            paint_cache: Arc::default(),
            bounds,
        }
    }

    pub fn shape_count(&self) -> usize {
        self.shapes.len()
    }

    pub fn paint(&self, ui: &Ui) {
        let clip = ui.clip_rect().intersect(ui.max_rect());
        let density = ui.ctx().pixels_per_point();
        let font_size = ui
            .ctx()
            .tex_manager()
            .read()
            .meta(egui::TextureId::default())
            .map_or([0, 0], |meta| meta.size);
        let mut cache = self
            .paint_cache
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if cache
            .as_ref()
            .is_none_or(|cache| cache.density != density || cache.font_size != font_size)
        {
            let mut source = self.shapes.as_ref().clone();
            fn refresh_text(ctx: &Context, shape: &mut egui::Shape) {
                match shape {
                    egui::Shape::Text(text) => {
                        text.galley = ctx
                            .fonts_mut(|fonts| fonts.layout_job(text.galley.job.as_ref().clone()));
                    }
                    egui::Shape::Vec(shapes) => {
                        shapes.iter_mut().for_each(|shape| refresh_text(ctx, shape))
                    }
                    _ => {}
                }
            }
            for shape in &mut source {
                refresh_text(ui.ctx(), &mut shape.shape);
            }
            let shapes = ui
                .ctx()
                .tessellate(source, density)
                .into_iter()
                .filter_map(|primitive| match primitive.primitive {
                    egui::epaint::Primitive::Mesh(mesh) => Some(egui::epaint::ClippedShape {
                        clip_rect: primitive.clip_rect,
                        shape: egui::Shape::mesh(mesh),
                    }),
                    _ => None,
                })
                .collect();
            *cache = Some(BackdropPaint {
                density,
                font_size,
                shapes,
            });
        }
        if let Some(cache) = cache.as_ref() {
            for shape in &cache.shapes {
                ui.painter()
                    .with_clip_rect(shape.clip_rect.intersect(clip))
                    .add(shape.shape.clone());
            }
        }
    }
}
