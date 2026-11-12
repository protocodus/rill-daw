//! Visualizations for device panels. They draw plain data supplied by the
//! caller (response curves, peaks, sample slices), own no audio state, and
//! use the shared palette. Interactive ones return the requested edits and
//! leave the value with the caller.
use crate::{SampleDrop, text::TextRole, theme, tokens};
use egui::{
    Align2, Color32, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, Vec2,
    WidgetInfo, WidgetType, pos2, vec2,
};

pub const FREQUENCY_MIN: f32 = 20.0;
pub const FREQUENCY_MAX: f32 = 20_000.0;

/// Frequency of point `index` on a log grid of `count` points from 20 Hz to
/// 20 kHz. Response curves handed to [`frequency_plot`] share this grid.
pub fn log_grid_frequency(index: usize, count: usize) -> f32 {
    let at = index as f32 / count.saturating_sub(1).max(1) as f32;
    FREQUENCY_MIN * (FREQUENCY_MAX / FREQUENCY_MIN).powf(at)
}

fn frequency_fraction(frequency: f32) -> f32 {
    ((frequency.max(FREQUENCY_MIN) / FREQUENCY_MIN).ln() / (FREQUENCY_MAX / FREQUENCY_MIN).ln())
        .clamp(0.0, 1.0)
}

fn fraction_frequency(fraction: f32) -> f32 {
    FREQUENCY_MIN * (FREQUENCY_MAX / FREQUENCY_MIN).powf(fraction.clamp(0.0, 1.0))
}

fn frame(ui: &mut Ui, size: Vec2, sense: Sense) -> (Rect, Response) {
    ui.allocate_exact_size(size.max(Vec2::splat(21.0)), sense)
}

fn paint_frame(painter: &Painter, rect: Rect, focused: bool) {
    let colors = theme::palette(painter.ctx());
    painter.rect_filled(rect, tokens::RADIUS, colors.bg);
    painter.rect_stroke(
        rect,
        tokens::RADIUS,
        Stroke::new(
            if focused {
                tokens::FOCUS_WIDTH
            } else {
                tokens::BORDER_WIDTH
            },
            if focused { colors.text } else { colors.line },
        ),
        StrokeKind::Inside,
    );
}

fn label(painter: &Painter, at: Pos2, align: Align2, text: &str) {
    painter.text(
        at,
        align,
        text,
        TextRole::Note.font(),
        theme::palette(painter.ctx()).muted,
    );
}

fn describe(response: &Response, ui: &Ui, text: String) {
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Other, ui.is_enabled(), text.clone()));
}

fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

/// Magnitude bins from an FFT of `sample_rate`: bin `i` is at
/// `i * sample_rate / (2 * (len - 1))`.
#[derive(Clone, Copy, Debug)]
pub struct Spectrum<'a> {
    pub bins_db: &'a [f32],
    pub sample_rate: f32,
    /// Tilt about 1 kHz. 4.5 dB per octave draws pink-ish material level,
    /// as analyzers behind an equalizer curve do; 0 shows the bins as they are.
    pub tilt_db_per_octave: f32,
    /// Each drawn point is the mean power of the bins within this many
    /// octaves around it; 0 reads the bins directly.
    pub smoothing_octaves: f32,
}

const SPECTRUM_FLOOR: f32 = -90.0;

impl<'a> Spectrum<'a> {
    /// Bins as they are: no tilt, no smoothing.
    pub fn new(bins_db: &'a [f32], sample_rate: f32) -> Self {
        Self {
            bins_db,
            sample_rate,
            tilt_db_per_octave: 0.0,
            smoothing_octaves: 0.0,
        }
    }

    /// The level drawn at `frequency`, smoothed and tilted. Where the
    /// smoothing band holds no bin (low frequencies), the level is
    /// interpolated between the two neighbouring bins, so the trace neither
    /// stair-steps there nor skips bins at the top.
    pub fn level_db(&self, frequency: f32) -> f32 {
        let last = self.bins_db.len().saturating_sub(1);
        if last == 0 || self.sample_rate <= 0.0 {
            return SPECTRUM_FLOOR;
        }
        let bin = |index: usize| finite(self.bins_db[index.min(last)], SPECTRUM_FLOOR);
        let position = |frequency: f32| {
            (frequency / (self.sample_rate * 0.5) * last as f32).clamp(0.0, last as f32)
        };
        let half = 2f32.powf(self.smoothing_octaves.max(0.0) * 0.5);
        let first = position(frequency / half).ceil() as usize;
        let end = position(frequency * half).floor() as usize;
        let db = if first <= end {
            let power: f32 = (first..=end)
                .map(|index| 10f32.powf(bin(index) / 10.0))
                .sum();
            10.0 * (power / (end - first + 1) as f32).max(1e-12).log10()
        } else {
            let at = position(frequency);
            let low = at.floor() as usize;
            bin(low) + (bin(low + 1) - bin(low)) * at.fract()
        };
        db + self.tilt_db_per_octave * (frequency.max(1.0) / 1000.0).log2()
    }
}

/// Fills between a trace and a horizontal line. A mesh, because the area is
/// not convex and feathered strips would show their seams.
fn fill_to(painter: &Painter, trace: &[Pos2], y: f32, color: Color32) {
    let mut mesh = egui::Mesh::default();
    for (index, point) in trace.iter().enumerate() {
        mesh.colored_vertex(*point, color);
        mesh.colored_vertex(pos2(point.x, y), color);
        if index > 0 {
            let at = 2 * index as u32;
            mesh.add_triangle(at - 2, at - 1, at);
            mesh.add_triangle(at - 1, at, at + 1);
        }
    }
    painter.add(Shape::mesh(mesh));
}

fn paint_spectrum(painter: &Painter, rect: Rect, spectrum: Spectrum<'_>, alpha: f32) {
    if spectrum.bins_db.len() < 2 {
        return;
    }
    let colors = theme::palette(painter.ctx());
    let columns = (rect.width() / 2.0).max(2.0) as usize;
    let mut points = Vec::with_capacity(columns + 2);
    points.push(pos2(rect.left(), rect.bottom()));
    for column in 0..=columns {
        let fraction = column as f32 / columns as f32;
        let db = spectrum
            .level_db(fraction_frequency(fraction))
            .clamp(SPECTRUM_FLOOR, 0.0);
        points.push(pos2(
            rect.left() + rect.width() * fraction,
            rect.bottom() - rect.height() * (db - SPECTRUM_FLOOR) / -SPECTRUM_FLOOR,
        ));
    }
    points.push(pos2(rect.right(), rect.bottom()));
    fill_to(
        painter,
        &points[1..=columns + 1],
        rect.bottom(),
        colors.muted.gamma_multiply(alpha),
    );
    painter.add(Shape::line(
        points[1..=columns + 1].to_vec(),
        Stroke::new(1.0_f32, colors.muted.gamma_multiply(alpha * 2.5)),
    ));
}

fn paint_frequency_grid(painter: &Painter, rect: Rect) {
    let colors = theme::palette(painter.ctx());
    for frequency in [
        30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0, 200.0, 300.0, 400.0, 500.0, 600.0, 700.0, 800.0,
        900.0, 2000.0, 3000.0, 4000.0, 5000.0, 6000.0, 7000.0, 8000.0, 9000.0,
    ] {
        let x = rect.left() + rect.width() * frequency_fraction(frequency);
        painter.line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(1.0_f32, colors.grid_minor),
        );
    }
    for (frequency, name) in [(100.0, "100"), (1000.0, "1k"), (10_000.0, "10k")] {
        let x = rect.left() + rect.width() * frequency_fraction(frequency);
        painter.line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(1.0_f32, colors.grid_bar),
        );
        label(
            painter,
            pos2(x + 3.0, rect.bottom() - 2.0),
            Align2::LEFT_BOTTOM,
            name,
        );
    }
}

/// A response curve on the shared log grid, in dB.
#[derive(Clone, Copy, Debug)]
pub struct FrequencyCurve<'a> {
    pub db: &'a [f32],
    /// The device's own response, drawn in the accent; others are muted.
    pub emphasis: bool,
}

/// A draggable node: a band, a cutoff or a marker on the response.
#[derive(Clone, Copy, Debug)]
pub struct FrequencyNode<'a> {
    pub frequency: f32,
    pub gain_db: f32,
    pub selected: bool,
    pub color: Option<Color32>,
    /// False for nodes that only move along the frequency axis (cutoffs).
    pub gain_editable: bool,
    /// Printed at the node; its position among the nodes, from 1, when `None`.
    pub label: Option<&'a str>,
    /// A node that is switched off is drawn hollow and can still be moved.
    pub enabled: bool,
    /// The node's values in words ("1.20 kHz  +3.5 dB  Q 1.40"), shown beside
    /// it while it is hovered, dragged or focused.
    pub readout: Option<&'a str>,
}

impl FrequencyNode<'_> {
    /// An enabled, unselected node that moves along both axes.
    pub fn new(frequency: f32, gain_db: f32) -> Self {
        Self {
            frequency,
            gain_db,
            selected: false,
            color: None,
            gain_editable: true,
            label: None,
            enabled: true,
            readout: None,
        }
    }
}

pub struct FrequencyPlot<'a> {
    pub curves: &'a [FrequencyCurve<'a>],
    pub spectrum: Option<Spectrum<'a>>,
    pub nodes: &'a [FrequencyNode<'a>],
    /// The gain axis runs from −`range_db` to +`range_db`.
    pub range_db: f32,
    /// How far a node's gain can be dragged; a node beyond the axis is drawn
    /// at its edge. Equal to `range_db` when the axis shows the whole range.
    pub limit_db: f32,
    pub size: Vec2,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeDrag {
    pub index: usize,
    pub frequency: f32,
    pub gain_db: f32,
}

pub struct FrequencyPlotOutput {
    pub response: Response,
    pub drags: Vec<NodeDrag>,
    pub clicked: Option<usize>,
    pub double_clicked: Option<usize>,
    /// The node under the pointer and the wheel notches turned over it. A
    /// surrounding scroll area stays still while the pointer is on a node.
    pub scrolled: Option<(usize, f32)>,
    /// The node the pointer is dragging.
    pub dragging: Option<usize>,
    /// Each node's own response, in node order, for context menus.
    pub nodes: Vec<Response>,
}

const NODE_RADIUS: f32 = 5.0;

/// Wheel notches turned this frame.
fn wheel_notches(ui: &Ui) -> f32 {
    ui.input(|input| {
        input
            .events
            .iter()
            .map(|event| match event {
                egui::Event::MouseWheel { unit, delta, .. } => match unit {
                    egui::MouseWheelUnit::Line => delta.y,
                    // A wheel notch scrolls about this many points.
                    egui::MouseWheelUnit::Point => delta.y / 50.0,
                    egui::MouseWheelUnit::Page => delta.y * 10.0,
                },
                _ => 0.0,
            })
            .sum()
    })
}

/// Log-frequency magnitude graph. Nodes follow the pointer from where they
/// were grabbed (one tenth as far with Shift) or move by arrow keys when
/// focused; devices also show each node's numbers as dials, so dragging is
/// never the only way. Where nodes overlap, the selected one is on top.
pub fn frequency_plot(ui: &mut Ui, plot: &FrequencyPlot<'_>) -> FrequencyPlotOutput {
    let (rect, response) = frame(ui, plot.size, Sense::hover());
    let colors = theme::palette(ui.ctx());
    let range = plot.range_db.max(1.0);
    let limit = plot.limit_db.max(range);
    let inner = rect.shrink(1.0);
    let gain_y = |db: f32| inner.center().y - inner.height() * 0.5 * (db / range).clamp(-1.0, 1.0);
    let center_of = |node: &FrequencyNode<'_>| {
        pos2(
            inner.left() + inner.width() * frequency_fraction(node.frequency),
            gain_y(node.gain_db),
        )
    };
    let mut drags = Vec::new();
    let (mut clicked, mut double_clicked, mut scrolled, mut dragging) = (None, None, None, None);
    let mut union = response.clone();
    let fine = ui.input(|input| if input.modifiers.shift { 0.1 } else { 1.0 });
    // The last widget registered at a point takes the pointer there.
    let mut order: Vec<usize> = (0..plot.nodes.len()).collect();
    order.sort_by_key(|index| plot.nodes[*index].selected);
    let mut hits: Vec<Option<Response>> = vec![None; plot.nodes.len()];
    for index in order {
        let node = &plot.nodes[index];
        let hit = ui.interact(
            Rect::from_center_size(center_of(node), Vec2::splat(tokens::HIT_TARGET)),
            response.id.with(("node", index)),
            Sense::click_and_drag(),
        );
        if hit.clicked() || hit.drag_started() {
            clicked = Some(index);
            hit.request_focus();
        }
        if hit.double_clicked() {
            double_clicked = Some(index);
        }
        // A focused node is a parameter control: Undo/Redo still reach the song.
        crate::parameter_focus::register_parameter_history_focus(&hit);
        let mut next = (node.frequency, node.gain_db);
        if hit.dragged_by(egui::PointerButton::Primary) {
            dragging = Some(index);
            let delta = hit.drag_delta();
            next.0 *= (FREQUENCY_MAX / FREQUENCY_MIN).powf(delta.x / inner.width() * fine);
            next.1 -= delta.y / inner.height() * 2.0 * range * fine;
        }
        if hit.hovered() {
            let notches = wheel_notches(ui);
            if notches != 0.0 {
                scrolled = Some((index, notches));
            }
            // On every frame over the node: egui spreads a notch over the
            // frames that follow it.
            ui.input_mut(|input| input.smooth_scroll_delta = Vec2::ZERO);
        }
        if hit.has_focus() {
            ui.input_mut(|input| {
                let mut nudge = |key| {
                    input.consume_key(egui::Modifiers::NONE, key)
                        || input.consume_key(egui::Modifiers::SHIFT, key)
                };
                if nudge(egui::Key::ArrowRight) {
                    next.0 *= 2f32.powf(fine / 12.0);
                }
                if nudge(egui::Key::ArrowLeft) {
                    next.0 /= 2f32.powf(fine / 12.0);
                }
                if nudge(egui::Key::ArrowUp) {
                    next.1 += 0.5 * fine;
                }
                if nudge(egui::Key::ArrowDown) {
                    next.1 -= 0.5 * fine;
                }
            });
        }
        if !node.gain_editable {
            next.1 = node.gain_db;
        }
        let next = (
            next.0.clamp(FREQUENCY_MIN, FREQUENCY_MAX),
            next.1.clamp(-limit, limit),
        );
        if next != (node.frequency, node.gain_db) {
            drags.push(NodeDrag {
                index,
                frequency: next.0,
                gain_db: next.1,
            });
        }
        union = union.union(hit.clone());
        hits[index] = Some(hit);
    }
    let hits: Vec<Response> = hits.into_iter().flatten().collect();
    if !drags.is_empty() {
        union.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        paint_frame(painter, rect, false);
        if let Some(spectrum) = plot.spectrum {
            paint_spectrum(painter, inner, spectrum, 0.16);
        }
        paint_frequency_grid(painter, inner);
        for db in [-range * 0.5, 0.0, range * 0.5] {
            painter.line_segment(
                [
                    pos2(inner.left(), gain_y(db)),
                    pos2(inner.right(), gain_y(db)),
                ],
                Stroke::new(
                    1.0_f32,
                    if db == 0.0 {
                        colors.grid_bar
                    } else {
                        colors.grid_minor
                    },
                ),
            );
            label(
                painter,
                pos2(inner.left() + 3.0, gain_y(db) - 1.0),
                Align2::LEFT_BOTTOM,
                &format!("{db:+.0}"),
            );
        }
        // Curves run past the axis at high gains; the frame cuts them.
        let clipped = painter.with_clip_rect(inner);
        for curve in plot.curves {
            if curve.db.len() < 2 {
                continue;
            }
            let y =
                |db: f32| inner.center().y - inner.height() * 0.5 * (db / range).clamp(-2.0, 2.0);
            let points: Vec<Pos2> = curve
                .db
                .iter()
                .enumerate()
                .map(|(index, db)| {
                    pos2(
                        inner.left() + inner.width() * index as f32 / (curve.db.len() - 1) as f32,
                        y(finite(*db, 0.0)),
                    )
                })
                .collect();
            if curve.emphasis {
                fill_to(
                    &clipped,
                    &points,
                    gain_y(0.0),
                    colors.accent.gamma_multiply(0.12),
                );
            }
            clipped.add(Shape::line(
                points,
                Stroke::new(
                    if curve.emphasis { 2.0_f32 } else { 1.0_f32 },
                    if curve.emphasis {
                        colors.accent
                    } else {
                        colors.muted
                    },
                ),
            ));
        }
        for (index, node) in plot.nodes.iter().enumerate() {
            let center = center_of(node);
            let color = node.color.unwrap_or(colors.accent);
            let radius = if node.selected {
                NODE_RADIUS + 1.5
            } else {
                NODE_RADIUS
            };
            if node.enabled {
                painter.circle_filled(center, radius, color);
            } else {
                painter.circle_filled(center, radius, colors.bg);
            }
            painter.circle_stroke(
                center,
                radius,
                Stroke::new(
                    1.5_f32,
                    if !node.enabled {
                        colors.muted
                    } else if node.selected {
                        colors.text
                    } else {
                        colors.bg
                    },
                ),
            );
            // Above the node, or below it where the frame is too close.
            let above = center.y - 10.0 - tokens::FONT_SMALL > inner.top();
            let name = node
                .label
                .map_or_else(|| format!("{}", index + 1), str::to_owned);
            let galley = painter.layout_no_wrap(
                name,
                TextRole::Note.font(),
                if node.selected {
                    colors.text
                } else {
                    colors.muted
                },
            );
            // A node at the frame's edge keeps its whole name inside.
            let half = galley.size().x * 0.5;
            let x = center
                .x
                .clamp(inner.left() + half + 2.0, inner.right() - half - 2.0);
            let y = if above {
                center.y - 10.0 - galley.size().y
            } else {
                center.y + 10.0
            };
            painter.galley(pos2(x - half, y), galley, colors.text);
        }
        for (node, hit) in plot.nodes.iter().zip(&hits) {
            let Some(readout) = node.readout else {
                continue;
            };
            if !(hit.hovered() || hit.dragged() || hit.has_focus()) {
                continue;
            }
            let center = center_of(node);
            let galley =
                painter.layout_no_wrap(readout.to_owned(), TextRole::Note.font(), colors.text);
            let size = galley.size() + vec2(2.0 * tokens::SPACE_XS, tokens::SPACE_XS);
            let gap = NODE_RADIUS + tokens::SPACE_SM;
            // Right of the node, or left of it where the frame is too close.
            let left = if center.x + gap + size.x <= inner.right() {
                center.x + gap
            } else {
                center.x - gap - size.x
            };
            let top = (center.y - size.y * 0.5).clamp(inner.top(), inner.bottom() - size.y);
            let bubble = Rect::from_min_size(pos2(left.max(inner.left()), top), size);
            painter.rect(
                bubble,
                tokens::RADIUS,
                colors.raised,
                Stroke::new(tokens::BORDER_WIDTH, colors.line),
                StrokeKind::Inside,
            );
            painter.galley(bubble.center() - galley.size() * 0.5, galley, colors.text);
        }
    }
    describe(
        &union,
        ui,
        format!(
            "Frequency response, {} curve(s), {} node(s)",
            plot.curves.len(),
            plot.nodes.len()
        ),
    );
    FrequencyPlotOutput {
        response: union,
        drags,
        clicked,
        double_clicked,
        scrolled,
        dragging,
        nodes: hits,
    }
}

/// Standalone spectrum on the same log axis as [`frequency_plot`].
pub fn spectrum(ui: &mut Ui, data: Spectrum<'_>, size: Vec2) -> Response {
    let (rect, response) = frame(ui, size, Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        paint_frame(painter, rect, false);
        let inner = rect.shrink(1.0);
        paint_frequency_grid(painter, inner);
        paint_spectrum(painter, inner, data, 0.3);
    }
    describe(&response, ui, "Spectrum".into());
    response
}

/// Input→output curve sampled evenly over `x_range`, with the live level.
pub struct TransferPlot<'a> {
    pub curve: &'a [f32],
    pub x_range: (f32, f32),
    pub y_range: (f32, f32),
    pub input: Option<f32>,
    pub size: Vec2,
}

pub fn transfer_plot(ui: &mut Ui, plot: &TransferPlot<'_>) -> Response {
    let (rect, response) = frame(ui, plot.size, Sense::hover());
    let colors = theme::palette(ui.ctx());
    let inner = rect.shrink(1.0);
    let (x0, x1) = plot.x_range;
    let (y0, y1) = plot.y_range;
    let to_point = |x: f32, y: f32| {
        pos2(
            inner.left() + inner.width() * ((x - x0) / (x1 - x0)).clamp(0.0, 1.0),
            inner.bottom() - inner.height() * ((y - y0) / (y1 - y0)).clamp(0.0, 1.0),
        )
    };
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        paint_frame(painter, rect, false);
        for step in 1..4 {
            let at = step as f32 / 4.0;
            painter.line_segment(
                [
                    pos2(inner.left() + inner.width() * at, inner.top()),
                    pos2(inner.left() + inner.width() * at, inner.bottom()),
                ],
                Stroke::new(1.0_f32, colors.grid_minor),
            );
            painter.line_segment(
                [
                    pos2(inner.left(), inner.top() + inner.height() * at),
                    pos2(inner.right(), inner.top() + inner.height() * at),
                ],
                Stroke::new(1.0_f32, colors.grid_minor),
            );
        }
        painter.line_segment(
            [to_point(x0, y0.max(x0)), to_point(x1, y1.min(x1))],
            Stroke::new(1.0_f32, colors.grid_bar),
        );
        if plot.curve.len() >= 2 {
            let last = (plot.curve.len() - 1) as f32;
            painter.add(Shape::line(
                plot.curve
                    .iter()
                    .enumerate()
                    .map(|(index, y)| {
                        to_point(x0 + (x1 - x0) * index as f32 / last, finite(*y, y0))
                    })
                    .collect(),
                Stroke::new(2.0_f32, colors.accent),
            ));
        }
        if let Some(input) = plot.input.filter(|input| input.is_finite())
            && plot.curve.len() >= 2
        {
            let last = (plot.curve.len() - 1) as f32;
            let position = ((input - x0) / (x1 - x0)).clamp(0.0, 1.0) * last;
            let low = position.floor() as usize;
            let high = (low + 1).min(plot.curve.len() - 1);
            let output = plot.curve[low] + (plot.curve[high] - plot.curve[low]) * position.fract();
            let dot = to_point(input.clamp(x0, x1), finite(output, y0));
            for end in [pos2(dot.x, inner.bottom()), pos2(inner.left(), dot.y)] {
                painter.line_segment([dot, end], Stroke::new(1.0_f32, colors.muted));
            }
            painter.circle_filled(dot, 4.5, colors.text);
        }
    }
    describe(&response, ui, "Transfer curve".into());
    response
}

/// Scrolling input envelope (peaks, linear, oldest first) with the gain
/// reduction trace (dB, positive) hanging from the top.
pub fn gain_reduction_history(
    ui: &mut Ui,
    columns: &[(f32, f32)],
    size: Vec2,
    max_reduction_db: f32,
) -> Response {
    let (rect, response) = frame(ui, size, Sense::hover());
    let colors = theme::palette(ui.ctx());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        paint_frame(painter, rect, false);
        let inner = rect.shrink(1.0);
        let count = columns.len().max(1);
        let width = inner.width() / count as f32;
        let mut trace = Vec::with_capacity(columns.len());
        for (index, (peak, reduction)) in columns.iter().enumerate() {
            let x = inner.left() + width * (index as f32 + 0.5);
            let level =
                ((20.0 * finite(*peak, 0.0).max(1e-6).log10() + 60.0) / 60.0).clamp(0.0, 1.0);
            painter.line_segment(
                [
                    pos2(x, inner.bottom()),
                    pos2(x, inner.bottom() - inner.height() * level),
                ],
                Stroke::new(width.max(1.0), colors.muted.gamma_multiply(0.35)),
            );
            trace.push(pos2(
                x,
                inner.top()
                    + inner.height() * (finite(*reduction, 0.0) / max_reduction_db).clamp(0.0, 1.0),
            ));
        }
        painter.line_segment(
            [inner.left_top(), inner.right_top()],
            Stroke::new(1.0_f32, colors.grid_bar),
        );
        if trace.len() >= 2 {
            painter.add(Shape::line(trace, Stroke::new(2.0_f32, colors.accent)));
        }
        label(
            painter,
            inner.left_top() + vec2(3.0, 1.0),
            Align2::LEFT_TOP,
            "0 dB",
        );
        label(
            painter,
            inner.left_bottom() + vec2(3.0, -1.0),
            Align2::LEFT_BOTTOM,
            &format!("−{max_reduction_db:.0} dB"),
        );
    }
    describe(&response, ui, "Gain reduction history".into());
    response
}

/// Vertical gain-reduction bar that reads downward from 0 dB. `hold_db` marks
/// the largest recent reduction.
pub fn reduction_meter(
    ui: &mut Ui,
    reduction_db: f32,
    hold_db: f32,
    max_reduction_db: f32,
    size: Vec2,
) -> Response {
    let (rect, response) = frame(ui, size, Sense::hover());
    let colors = theme::palette(ui.ctx());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, 1.0, colors.bg);
        let fraction = |db: f32| (finite(db, 0.0) / max_reduction_db).clamp(0.0, 1.0);
        let fill = Rect::from_min_max(
            rect.min,
            pos2(
                rect.right(),
                rect.top() + rect.height() * fraction(reduction_db),
            ),
        );
        painter.rect_filled(fill, 1.0, colors.accent);
        let hold = rect.top() + rect.height() * fraction(hold_db);
        painter.line_segment(
            [pos2(rect.left(), hold), pos2(rect.right(), hold)],
            Stroke::new(1.5_f32, colors.text),
        );
    }
    describe(
        &response,
        ui,
        format!("Gain reduction {:.1} dB", finite(reduction_db, 0.0)),
    );
    response
}

/// Oscilloscope. With `sync`, the trace starts at the first rising zero
/// crossing so a periodic signal holds still.
pub fn scope(ui: &mut Ui, samples: &[f32], sync: bool, size: Vec2) -> Response {
    let (rect, response) = frame(ui, size, Sense::hover());
    let colors = theme::palette(ui.ctx());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        paint_frame(painter, rect, false);
        let inner = rect.shrink(1.0);
        painter.line_segment(
            [inner.left_center(), inner.right_center()],
            Stroke::new(1.0_f32, colors.grid_bar),
        );
        let start = if sync {
            (1..samples.len() / 2)
                .find(|&index| samples[index - 1] <= 0.0 && samples[index] > 0.0)
                .unwrap_or(0)
        } else {
            0
        };
        let shown = &samples[start..];
        if shown.len() >= 2 {
            painter.add(Shape::line(
                shown
                    .iter()
                    .enumerate()
                    .map(|(index, sample)| {
                        pos2(
                            inner.left() + inner.width() * index as f32 / (shown.len() - 1) as f32,
                            inner.center().y
                                - inner.height() * 0.5 * finite(*sample, 0.0).clamp(-1.0, 1.0),
                        )
                    })
                    .collect(),
                Stroke::new(1.5_f32, colors.accent),
            ));
        }
    }
    describe(&response, ui, "Oscilloscope".into());
    response
}

/// The current wavetable frame in front, neighbors stacked faintly behind.
/// `position` runs 0 to 1 across `frames`.
pub fn wavetable_view(ui: &mut Ui, frames: &[&[f32]], position: f32, size: Vec2) -> Response {
    let (rect, response) = frame(ui, size, Sense::hover());
    let colors = theme::palette(ui.ctx());
    if ui.is_rect_visible(rect) && !frames.is_empty() {
        let painter = ui.painter();
        paint_frame(painter, rect, false);
        let current =
            (finite(position, 0.0).clamp(0.0, 1.0) * (frames.len() - 1) as f32).round() as usize;
        let depth = 5usize;
        let offset = vec2(4.0, -3.0);
        let inner = rect.shrink2(vec2(tokens::SPACE_SM, tokens::SPACE_SM));
        let area = Rect::from_min_max(
            pos2(inner.left(), inner.top() - offset.y * depth as f32),
            pos2(inner.right() - offset.x * depth as f32, inner.bottom()),
        );
        let draw = |index: usize, behind: usize, color: Color32, width: f32| {
            let frame = frames[index];
            if frame.len() < 2 {
                return;
            }
            let shift = offset * behind as f32;
            painter.add(Shape::line(
                frame
                    .iter()
                    .enumerate()
                    .map(|(point, sample)| {
                        pos2(
                            area.left() + area.width() * point as f32 / (frame.len() - 1) as f32,
                            area.center().y
                                - area.height() * 0.5 * finite(*sample, 0.0).clamp(-1.0, 1.0),
                        ) + shift
                    })
                    .collect(),
                Stroke::new(width, color),
            ));
        };
        for behind in (1..=depth).rev() {
            if current + behind < frames.len() {
                draw(
                    current + behind,
                    behind,
                    colors.muted.gamma_multiply(0.7 - behind as f32 * 0.1),
                    1.0,
                );
            }
        }
        draw(current, 0, colors.accent, 2.0);
    }
    describe(&response, ui, "Wavetable".into());
    response
}

/// One LFO cycle (−1 to 1) with the moving phase marker.
pub fn lfo_view(ui: &mut Ui, shape: &[f32], phase: Option<f32>, size: Vec2) -> Response {
    let (rect, response) = frame(ui, size, Sense::hover());
    let colors = theme::palette(ui.ctx());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        paint_frame(painter, rect, false);
        let inner = rect.shrink2(vec2(tokens::SPACE_SM, tokens::SPACE_SM));
        painter.line_segment(
            [inner.left_center(), inner.right_center()],
            Stroke::new(1.0_f32, colors.grid_bar),
        );
        let at = |index: usize| {
            pos2(
                inner.left() + inner.width() * index as f32 / (shape.len() - 1) as f32,
                inner.center().y
                    - inner.height() * 0.5 * finite(shape[index], 0.0).clamp(-1.0, 1.0),
            )
        };
        if shape.len() >= 2 {
            painter.add(Shape::line(
                (0..shape.len()).map(at).collect(),
                Stroke::new(2.0_f32, colors.accent),
            ));
            if let Some(phase) = phase.filter(|phase| phase.is_finite()) {
                let phase = phase.rem_euclid(1.0);
                let position = phase * (shape.len() - 1) as f32;
                let low = position.floor() as usize;
                let high = (low + 1).min(shape.len() - 1);
                let point = at(low).lerp(at(high), position.fract());
                painter.line_segment(
                    [pos2(point.x, inner.top()), pos2(point.x, inner.bottom())],
                    Stroke::new(1.0_f32, colors.muted),
                );
                painter.circle_filled(point, 4.0, colors.modulation);
            }
        }
    }
    describe(&response, ui, "LFO shape".into());
    response
}

/// Draggable sample markers, in 0 to 1 across the sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleMarker {
    Start,
    End,
    LoopStart,
    LoopEnd,
}

pub struct SampleViewData<'a> {
    /// Min and max of each peak column, −1 to 1.
    pub peaks: &'a [(f32, f32)],
    pub start: f32,
    pub end: f32,
    pub loop_range: Option<(f32, f32)>,
    /// Voice positions, 0 to 1.
    pub playheads: &'a [f32],
}

pub struct SampleViewOutput {
    pub response: Response,
    pub drags: Vec<(SampleMarker, f32)>,
    pub dropped: Option<String>,
}

/// Sample waveform with start, end and loop markers and voice playheads.
/// A focused marker moves with the arrow keys (Shift for fine steps); the
/// panel also offers each position as a knob. A sample dragged from the
/// library can be dropped on the view.
pub fn sample_view(ui: &mut Ui, data: &SampleViewData<'_>, size: Vec2) -> SampleViewOutput {
    let (rect, response) = frame(ui, size, Sense::hover());
    let colors = theme::palette(ui.ctx());
    let inner = rect.shrink(1.0);
    let x_of = |at: f32| inner.left() + inner.width() * at.clamp(0.0, 1.0);
    let mut markers = vec![
        (SampleMarker::Start, data.start),
        (SampleMarker::End, data.end),
    ];
    if let Some((start, end)) = data.loop_range {
        markers.push((SampleMarker::LoopStart, start));
        markers.push((SampleMarker::LoopEnd, end));
    }
    let mut drags = Vec::new();
    let mut union = response.clone();
    for (marker, at) in &markers {
        let hit = ui.interact(
            Rect::from_x_y_ranges(
                x_of(*at) - tokens::HIT_TARGET * 0.5..=x_of(*at) + tokens::HIT_TARGET * 0.5,
                inner.y_range(),
            ),
            response.id.with(*marker as u8),
            Sense::click_and_drag(),
        );
        let fine = ui.input(|input| if input.modifiers.shift { 0.1 } else { 1.0 });
        let mut next = *at;
        if hit.dragged() {
            if let Some(pointer) = hit.interact_pointer_pos().filter(|_| fine == 1.0) {
                next = (pointer.x - inner.left()) / inner.width();
            } else {
                next += hit.drag_delta().x / inner.width() * fine;
            }
        }
        if hit.clicked() {
            hit.request_focus();
        }
        if hit.has_focus() {
            ui.input_mut(|input| {
                let step = 0.005 * fine;
                if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight)
                    || input.consume_key(egui::Modifiers::SHIFT, egui::Key::ArrowRight)
                {
                    next += step;
                }
                if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft)
                    || input.consume_key(egui::Modifiers::SHIFT, egui::Key::ArrowLeft)
                {
                    next -= step;
                }
            });
        }
        let next = next.clamp(0.0, 1.0);
        if next != *at {
            drags.push((*marker, next));
        }
        union = union.union(hit);
    }
    if !drags.is_empty() {
        union.mark_changed();
    }
    let dropped = union
        .dnd_release_payload::<SampleDrop>()
        .map(|payload| payload.0.clone());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        paint_frame(painter, rect, false);
        painter.line_segment(
            [inner.left_center(), inner.right_center()],
            Stroke::new(1.0_f32, colors.grid_bar),
        );
        let columns = data.peaks.len();
        for (index, (low, high)) in data.peaks.iter().enumerate() {
            let at = (index as f32 + 0.5) / columns as f32;
            let inside = at >= data.start && at <= data.end;
            let to_y = |value: f32| {
                inner.center().y - inner.height() * 0.5 * finite(value, 0.0).clamp(-1.0, 1.0)
            };
            painter.line_segment(
                [
                    pos2(x_of(at), to_y(*high)),
                    pos2(x_of(at), to_y(*low).max(to_y(*high) + 1.0)),
                ],
                Stroke::new(
                    (inner.width() / columns as f32).max(1.0),
                    if inside {
                        colors.note
                    } else {
                        colors.muted.gamma_multiply(0.4)
                    },
                ),
            );
        }
        if let Some((start, end)) = data.loop_range {
            painter.rect_filled(
                Rect::from_x_y_ranges(x_of(start)..=x_of(end), inner.y_range()),
                0.0,
                colors.accent.gamma_multiply(0.12),
            );
        }
        for (marker, at) in &markers {
            let color = if matches!(marker, SampleMarker::Start | SampleMarker::End) {
                colors.text
            } else {
                colors.accent
            };
            painter.line_segment(
                [
                    pos2(x_of(*at), inner.top()),
                    pos2(x_of(*at), inner.bottom()),
                ],
                Stroke::new(1.5_f32, color),
            );
            let flag = match marker {
                SampleMarker::Start | SampleMarker::LoopStart => 1.0,
                _ => -1.0,
            };
            painter.add(Shape::convex_polygon(
                vec![
                    pos2(x_of(*at), inner.top()),
                    pos2(x_of(*at) + 8.0 * flag, inner.top()),
                    pos2(x_of(*at), inner.top() + 8.0),
                ],
                color,
                Stroke::NONE,
            ));
        }
        for playhead in data.playheads {
            painter.line_segment(
                [
                    pos2(x_of(*playhead), inner.top()),
                    pos2(x_of(*playhead), inner.bottom()),
                ],
                Stroke::new(1.0_f32, colors.cursor),
            );
        }
        if union.dnd_hover_payload::<SampleDrop>().is_some() {
            painter.rect_stroke(
                rect,
                tokens::RADIUS,
                Stroke::new(tokens::FOCUS_WIDTH, colors.action),
                StrokeKind::Inside,
            );
        }
    }
    describe(&union, ui, "Sample waveform".into());
    SampleViewOutput {
        response: union,
        drags,
        dropped,
    }
}

/// Echo taps as decaying bars (left above the axis, right below it) over the
/// reverb's decay envelope.
pub struct DelayViewData<'a> {
    /// (time in seconds, left gain, right gain), gains 0 to 1.
    pub taps: &'a [(f32, f32, f32)],
    pub decay_seconds: f32,
    pub pre_delay_seconds: f32,
    /// The time axis runs from 0 to this.
    pub span_seconds: f32,
}

pub fn delay_view(ui: &mut Ui, data: &DelayViewData<'_>, size: Vec2) -> Response {
    let (rect, response) = frame(ui, size, Sense::hover());
    let colors = theme::palette(ui.ctx());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        paint_frame(painter, rect, false);
        let inner = rect.shrink2(vec2(tokens::SPACE_SM, tokens::SPACE_SM));
        let span = data.span_seconds.max(0.01);
        let x_of = |seconds: f32| inner.left() + inner.width() * (seconds / span).clamp(0.0, 1.0);
        let axis = inner.center().y;
        painter.line_segment(
            [pos2(inner.left(), axis), pos2(inner.right(), axis)],
            Stroke::new(1.0_f32, colors.grid_bar),
        );
        if data.decay_seconds > 0.0 {
            let start = data.pre_delay_seconds.max(0.0);
            let mut upper = Vec::new();
            for step in 0..=80 {
                let seconds = start + (span - start).max(0.0) * step as f32 / 80.0;
                let level = (-6.9 * (seconds - start) / data.decay_seconds).exp();
                upper.push(pos2(x_of(seconds), axis - inner.height() * 0.5 * level));
            }
            let mut shape = upper.clone();
            shape.extend(
                upper
                    .iter()
                    .rev()
                    .map(|point| pos2(point.x, 2.0 * axis - point.y)),
            );
            painter.add(Shape::convex_polygon(
                shape,
                colors.muted.gamma_multiply(0.18),
                Stroke::NONE,
            ));
        }
        let bar = (inner.width() / 90.0).clamp(2.0, 6.0);
        for (seconds, left, right) in data.taps {
            let x = x_of(*seconds);
            painter.line_segment(
                [
                    pos2(x, axis),
                    pos2(x, axis - inner.height() * 0.5 * left.clamp(0.0, 1.0)),
                ],
                Stroke::new(bar, colors.accent),
            );
            painter.line_segment(
                [
                    pos2(x, axis),
                    pos2(x, axis + inner.height() * 0.5 * right.clamp(0.0, 1.0)),
                ],
                Stroke::new(bar, colors.accent.gamma_multiply(0.6)),
            );
        }
        label(painter, inner.left_bottom(), Align2::LEFT_BOTTOM, "0");
        label(
            painter,
            inner.right_bottom(),
            Align2::RIGHT_BOTTOM,
            &format!("{span:.1} s"),
        );
    }
    describe(&response, ui, format!("Echo taps: {}", data.taps.len()));
    response
}

/// Bars for harmonics 1 to 10, in dB (−80 to 0).
pub fn harmonics_view(ui: &mut Ui, harmonics_db: &[f32; 10], size: Vec2) -> Response {
    let (rect, response) = frame(ui, size, Sense::hover());
    let colors = theme::palette(ui.ctx());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        paint_frame(painter, rect, false);
        let inner = rect.shrink2(vec2(
            tokens::SPACE_SM,
            tokens::SPACE_SM + tokens::FONT_SMALL,
        ));
        let slot = inner.width() / 10.0;
        for (index, db) in harmonics_db.iter().enumerate() {
            let level = (finite(*db, -80.0).clamp(-80.0, 0.0) + 80.0) / 80.0;
            let x = inner.left() + slot * (index as f32 + 0.5);
            painter.rect_filled(
                Rect::from_min_max(
                    pos2(x - slot * 0.3, inner.bottom() - inner.height() * level),
                    pos2(x + slot * 0.3, inner.bottom()),
                ),
                1.0,
                if index == 0 {
                    colors.accent
                } else {
                    colors.accent.gamma_multiply(0.7)
                },
            );
            label(
                painter,
                pos2(x, rect.bottom() - 2.0),
                Align2::CENTER_BOTTOM,
                &format!("{}", index + 1),
            );
        }
    }
    describe(&response, ui, "Harmonics".into());
    response
}
