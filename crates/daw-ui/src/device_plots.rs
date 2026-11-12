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

fn label(painter: &Painter, at: Pos2, align: Align2, text: &str) -> Rect {
    painter.text(
        at,
        align,
        text,
        TextRole::Note.font(),
        theme::palette(painter.ctx()).muted,
    )
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
            paint_bubble(
                painter,
                inner,
                center_of(node),
                NODE_RADIUS + tokens::SPACE_SM,
                readout,
            );
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

/// How a [`TransferPlot`] rules and labels its axes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransferAxis {
    /// Quarters of the range, labelled at its ends and middle: "-1 0 +1".
    #[default]
    Linear,
    /// A line every 20 dB, labelled in dB.
    Decibels,
}

/// The one draggable point of a [`TransferPlot`]: a threshold, a ceiling.
#[derive(Clone, Copy, Debug)]
pub struct TransferHandle<'a> {
    pub x: f32,
    pub y: f32,
    /// Printed at the handle: "T".
    pub label: &'a str,
    /// The handle's value in words, shown beside it while it is hovered,
    /// dragged or focused.
    pub readout: &'a str,
}

/// Input→output curve sampled evenly over `x_range`, with the live level.
#[derive(Clone, Copy, Debug)]
pub struct TransferPlot<'a> {
    pub curve: &'a [f32],
    pub x_range: (f32, f32),
    pub y_range: (f32, f32),
    pub axis: TransferAxis,
    /// The live input level, as a dot on the curve.
    pub input: Option<f32>,
    /// The span of x the signal sweeps: that part of the curve is drawn at
    /// full strength and the rest dimmed. The whole curve when `None`.
    pub lit: Option<(f32, f32)>,
    /// A span of x marked by a faint band: a knee.
    pub shade: Option<(f32, f32)>,
    /// Without one the plot only shows.
    pub handle: Option<TransferHandle<'a>>,
    pub size: Vec2,
}

impl Default for TransferPlot<'_> {
    fn default() -> Self {
        Self {
            curve: &[],
            x_range: (-1.0, 1.0),
            y_range: (-1.0, 1.0),
            axis: TransferAxis::Linear,
            input: None,
            lit: None,
            shade: None,
            handle: None,
            size: Vec2::splat(144.0),
        }
    }
}

pub struct TransferPlotOutput {
    pub response: Response,
    /// Where a drag puts the handle, in plot units.
    pub dragged: Option<(f32, f32)>,
    /// Arrow keys while the handle has focus, as steps along x and y; a
    /// tenth of a step with Shift.
    pub nudged: Option<(f32, f32)>,
    /// Wheel notches turned over a plot that has a handle. A surrounding
    /// scroll area stays still meanwhile.
    pub scrolled: Option<f32>,
    /// On the handle.
    pub double_clicked: bool,
    /// The handle's own response, for a context menu.
    pub handle: Option<Response>,
}

const HANDLE_RADIUS: f32 = 6.0;

/// Where a [`TransferPlot`] draws its handle for the point `at`, and the
/// top left of its label of `size`. The handle stays whole inside `inner`.
/// The label goes up and to the left, where a rising curve leaves room, or
/// down and to the right at the top of the frame: never down-left or
/// up-right, where the curve runs.
fn place_handle(inner: Rect, at: Pos2, size: Vec2) -> (Pos2, Pos2) {
    let room = inner.shrink(HANDLE_RADIUS);
    let center = if room.is_positive() {
        at.clamp(room.min, room.max)
    } else {
        at
    };
    let gap = HANDLE_RADIUS + 2.0;
    let label = if center.y - gap - size.y >= inner.top() {
        pos2(
            (center.x - gap - size.x).max(inner.left() + 1.0),
            center.y - gap - size.y,
        )
    } else {
        pos2(
            (center.x + gap).min(inner.right() - size.x - 1.0),
            center.y + gap,
        )
    };
    (center, label)
}

/// A value's text in a small raised box at `anchor`, kept inside `bounds`:
/// right of the anchor, or left of it where the frame is too close.
fn paint_bubble(painter: &Painter, bounds: Rect, anchor: Pos2, gap: f32, text: &str) {
    let colors = theme::palette(painter.ctx());
    let galley = painter.layout_no_wrap(text.to_owned(), TextRole::Note.font(), colors.text);
    let size = galley.size() + vec2(2.0 * tokens::SPACE_XS, tokens::SPACE_XS);
    let left = if anchor.x + gap + size.x <= bounds.right() {
        anchor.x + gap
    } else {
        anchor.x - gap - size.x
    };
    let top =
        (anchor.y - size.y * 0.5).clamp(bounds.top(), (bounds.bottom() - size.y).max(bounds.top()));
    let bubble = Rect::from_min_size(pos2(left.max(bounds.left()), top), size);
    painter.rect(
        bubble,
        tokens::RADIUS,
        colors.raised,
        Stroke::new(tokens::BORDER_WIDTH, colors.line),
        StrokeKind::Inside,
    );
    painter.galley(bubble.center() - galley.size() * 0.5, galley, colors.text);
}

/// A whole number of decibels or units as an axis prints it: "-20", "0", "+1".
fn axis_number(value: f32, signed: bool) -> String {
    if value == 0.0 {
        "0".into()
    } else if signed {
        format!("{value:+.0}")
    } else {
        format!("{value:.0}")
    }
}

/// The y of `curve` (sampled evenly from `x0` to `x1`) at `x`.
fn curve_at(curve: &[f32], x0: f32, x1: f32, x: f32, fallback: f32) -> f32 {
    let last = (curve.len() - 1) as f32;
    let position = ((x - x0) / (x1 - x0)).clamp(0.0, 1.0) * last;
    let low = position.floor() as usize;
    let high = (low + 1).min(curve.len() - 1);
    let (a, b) = (finite(curve[low], fallback), finite(curve[high], fallback));
    a + (b - a) * position.fract()
}

/// Input→output curve of a shaper (linear) or a dynamics processor (dB),
/// with the live operating point. With a [`TransferHandle`] the curve is
/// edited where it is drawn: the handle follows the pointer from where it
/// was grabbed (one tenth as far with Shift) and takes the arrow keys when
/// focused, and the wheel is reported for a second value. The caller also
/// shows the values as dials, so dragging is never the only way.
pub fn transfer_plot(ui: &mut Ui, plot: &TransferPlot<'_>) -> TransferPlotOutput {
    let (rect, response) = frame(ui, plot.size, Sense::hover());
    let colors = theme::palette(ui.ctx());
    let inner = rect.shrink(1.0);
    let (x0, x1) = plot.x_range;
    let (y0, y1) = plot.y_range;
    let fraction = |value: f32, low: f32, high: f32| ((value - low) / (high - low)).clamp(0.0, 1.0);
    let to_point = |x: f32, y: f32| {
        pos2(
            inner.left() + inner.width() * fraction(x, x0, x1),
            inner.bottom() - inner.height() * fraction(y, y0, y1),
        )
    };
    let fine = ui.input(|input| if input.modifiers.shift { 0.1 } else { 1.0 });
    let (mut dragged, mut nudged, mut scrolled) = (None, None, None);
    let mut double_clicked = false;
    let mut union = response.clone();
    let hit = plot.handle.map(|handle| {
        let hit = ui.interact(
            Rect::from_center_size(
                to_point(handle.x, handle.y),
                Vec2::splat(tokens::HIT_TARGET),
            ),
            response.id.with("handle"),
            Sense::click_and_drag(),
        );
        if hit.clicked() || hit.drag_started() {
            hit.request_focus();
        }
        double_clicked = hit.double_clicked();
        // A focused handle is a parameter control: Undo/Redo still reach the song.
        crate::parameter_focus::register_parameter_history_focus(&hit);
        if hit.dragged_by(egui::PointerButton::Primary) {
            let delta = hit.drag_delta();
            let next = (
                (handle.x + delta.x / inner.width() * (x1 - x0) * fine).clamp(x0, x1),
                (handle.y - delta.y / inner.height() * (y1 - y0) * fine).clamp(y0, y1),
            );
            if next != (handle.x, handle.y) {
                dragged = Some(next);
            }
        }
        if hit.has_focus() {
            let mut steps = (0.0, 0.0);
            ui.input_mut(|input| {
                let mut pressed = |key| {
                    input.consume_key(egui::Modifiers::NONE, key)
                        || input.consume_key(egui::Modifiers::SHIFT, key)
                };
                for (key, x, y) in [
                    (egui::Key::ArrowRight, 1.0, 0.0),
                    (egui::Key::ArrowLeft, -1.0, 0.0),
                    (egui::Key::ArrowUp, 0.0, 1.0),
                    (egui::Key::ArrowDown, 0.0, -1.0),
                ] {
                    if pressed(key) {
                        steps.0 += x * fine;
                        steps.1 += y * fine;
                    }
                }
            });
            if steps != (0.0, 0.0) {
                nudged = Some(steps);
            }
        }
        if response.hovered() || hit.hovered() {
            let notches = wheel_notches(ui);
            if notches != 0.0 {
                scrolled = Some(notches);
            }
            // On every frame over the plot: egui spreads a notch over the
            // frames that follow it.
            ui.input_mut(|input| input.smooth_scroll_delta = Vec2::ZERO);
        }
        union = union.union(hit.clone());
        hit
    });
    if dragged.is_some() || nudged.is_some() || scrolled.is_some() {
        union.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        paint_frame(painter, rect, false);
        if let Some((from, to)) = plot.shade {
            painter.rect_filled(
                Rect::from_x_y_ranges(to_point(from, y0).x..=to_point(to, y0).x, inner.y_range()),
                0.0,
                colors.muted.gamma_multiply(0.1),
            );
        }
        // Grid values, the same on both axes where their ranges are.
        let lines = |low: f32, high: f32| -> Vec<(f32, bool)> {
            match plot.axis {
                TransferAxis::Linear => (1..4)
                    .map(|step| (low + (high - low) * step as f32 / 4.0, step == 2))
                    .collect(),
                TransferAxis::Decibels => {
                    let first = (low / 20.0).floor() as i32 + 1;
                    (first..)
                        .map(|step| step as f32 * 20.0)
                        .take_while(|value| *value < high)
                        .map(|value| (value, true))
                        .collect()
                }
            }
        };
        let signed = plot.axis == TransferAxis::Linear;
        for (value, labelled) in lines(x0, x1) {
            let x = to_point(value, y0).x;
            painter.line_segment(
                [pos2(x, inner.top()), pos2(x, inner.bottom())],
                Stroke::new(1.0_f32, colors.grid_minor),
            );
            if labelled {
                label(
                    painter,
                    pos2(x + 3.0, inner.bottom() - 2.0),
                    Align2::LEFT_BOTTOM,
                    &axis_number(value, signed),
                );
            }
        }
        for (value, labelled) in lines(y0, y1) {
            let y = to_point(x0, value).y;
            painter.line_segment(
                [pos2(inner.left(), y), pos2(inner.right(), y)],
                Stroke::new(1.0_f32, colors.grid_minor),
            );
            // The middle of a linear plot is named once, on the x axis.
            if labelled && plot.axis == TransferAxis::Decibels {
                label(
                    painter,
                    pos2(inner.left() + 3.0, y - 1.0),
                    Align2::LEFT_BOTTOM,
                    &axis_number(value, signed),
                );
            }
        }
        // The ends: the low one in the corner both axes share, clear of the
        // diagonal that starts there.
        label(
            painter,
            inner.left_bottom() + vec2(tokens::FONT_SMALL + 5.0, -2.0),
            Align2::LEFT_BOTTOM,
            &axis_number(x0, signed),
        );
        label(
            painter,
            inner.right_bottom() + vec2(-3.0, -2.0),
            Align2::RIGHT_BOTTOM,
            &axis_number(x1, signed),
        );
        label(
            painter,
            inner.left_top() + vec2(3.0, 1.0),
            Align2::LEFT_TOP,
            &match plot.axis {
                TransferAxis::Linear => axis_number(y1, signed),
                TransferAxis::Decibels => format!("{} dB", axis_number(y1, signed)),
            },
        );
        painter.line_segment(
            [to_point(x0, y0.max(x0)), to_point(x1, y1.min(x1))],
            Stroke::new(1.0_f32, colors.grid_bar),
        );
        if plot.curve.len() >= 2 {
            let last = (plot.curve.len() - 1) as f32;
            let point = |index: usize| {
                to_point(
                    x0 + (x1 - x0) * index as f32 / last,
                    finite(plot.curve[index], y0),
                )
            };
            let on_curve = |x: f32| to_point(x, curve_at(plot.curve, x0, x1, x, y0));
            let whole: Vec<Pos2> = (0..plot.curve.len()).map(point).collect();
            match plot.lit {
                None => {
                    painter.add(Shape::line(whole, Stroke::new(2.0_f32, colors.accent)));
                }
                Some((from, to)) => {
                    painter.add(Shape::line(
                        whole,
                        Stroke::new(2.0_f32, colors.accent.gamma_multiply(0.45)),
                    ));
                    let (from, to) = (from.clamp(x0, x1), to.clamp(x0, x1));
                    if to > from {
                        let mut lit = vec![on_curve(from)];
                        lit.extend((0..plot.curve.len()).map(point).filter(|point| {
                            point.x > on_curve(from).x && point.x < on_curve(to).x
                        }));
                        lit.push(on_curve(to));
                        painter.add(Shape::line(lit, Stroke::new(2.0_f32, colors.accent)));
                    }
                }
            }
            if let Some(input) = plot.input.filter(|input| input.is_finite()) {
                let dot = on_curve(input.clamp(x0, x1));
                for end in [pos2(dot.x, inner.bottom()), pos2(inner.left(), dot.y)] {
                    painter.line_segment([dot, end], Stroke::new(1.0_f32, colors.muted));
                }
                painter.circle_filled(dot, 4.5, colors.text);
            }
        }
        if let (Some(handle), Some(hit)) = (plot.handle, &hit) {
            let active = hit.hovered() || hit.dragged() || hit.has_focus();
            let color = if active { colors.text } else { colors.muted };
            let galley =
                painter.layout_no_wrap(handle.label.to_owned(), TextRole::Note.font(), color);
            let (center, label_at) =
                place_handle(inner, to_point(handle.x, handle.y), galley.size());
            painter.circle_filled(center, HANDLE_RADIUS, colors.accent);
            painter.circle_stroke(
                center,
                HANDLE_RADIUS,
                Stroke::new(1.5_f32, if active { colors.text } else { colors.bg }),
            );
            painter.galley(label_at, galley, color);
            if active && !handle.readout.is_empty() {
                paint_bubble(
                    painter,
                    inner,
                    center,
                    HANDLE_RADIUS + tokens::SPACE_SM,
                    handle.readout,
                );
            }
        }
    }
    describe(&union, ui, "Transfer curve".into());
    TransferPlotOutput {
        response: union,
        dragged,
        nudged,
        scrolled,
        double_clicked,
        handle: hit,
    }
}

/// What [`reduction_meter`] keeps below its bar for the held value. The
/// reduction axis of [`gain_reduction_history`] ends as far above its own
/// bottom, so at equal heights the trace and the bar read against the same
/// ticks.
const REDUCTION_READOUT: f32 = tokens::FONT_SMALL + tokens::SPACE_SM;

/// The top of a reduction axis and its length, in a frame of this height.
fn reduction_axis(rect: Rect) -> (f32, f32) {
    let inner = rect.shrink(1.0);
    (inner.top(), (inner.height() - REDUCTION_READOUT).max(1.0))
}

/// Levels over time with the gain reduction they caused.
#[derive(Clone, Copy, Debug)]
pub struct LevelHistory<'a> {
    /// Oldest first: input peak and output peak (linear), reduction in dB.
    pub columns: &'a [[f32; 3]],
    /// The level axis runs from this to 0 dB over the whole height, as a
    /// [`TransferPlot`] in decibels of the same range and height does.
    pub floor_db: f32,
    /// The reduction axis hangs from the top, as [`reduction_meter`]'s.
    pub max_reduction_db: f32,
    /// A level drawn as a horizontal line: the threshold.
    pub threshold_db: Option<f32>,
    pub threshold_label: &'a str,
    /// Whether the line can be dragged.
    pub draggable: bool,
    /// What the columns span, printed at the right: "8 s".
    pub span_seconds: f32,
    pub size: Vec2,
}

pub struct LevelHistoryOutput {
    pub response: Response,
    /// Where a drag or the arrow keys put the threshold, in dB.
    pub threshold: Option<f32>,
    /// The line's own response, for a context menu.
    pub line: Option<Response>,
}

/// Scrolling level history: the input as a dim fill, the output as a line
/// over it, and the gain reduction as a trace hanging from the top. The
/// threshold line drags vertically (one tenth as far with Shift) and moves
/// by arrow keys when focused; the caller also shows it as a dial.
pub fn gain_reduction_history(ui: &mut Ui, history: &LevelHistory<'_>) -> LevelHistoryOutput {
    let (rect, response) = frame(ui, history.size, Sense::hover());
    let colors = theme::palette(ui.ctx());
    let inner = rect.shrink(1.0);
    let floor = history.floor_db.min(-1.0);
    let level_y =
        |db: f32| inner.bottom() - inner.height() * ((db - floor) / -floor).clamp(0.0, 1.0);
    let (top, length) = reduction_axis(rect);
    let mut threshold = None;
    let mut union = response.clone();
    let line = history
        .threshold_db
        .filter(|_| history.draggable)
        .map(|at| {
            let y = level_y(at);
            let hit = ui
                .interact(
                    Rect::from_x_y_ranges(
                        inner.x_range(),
                        y - tokens::HIT_TARGET * 0.5..=y + tokens::HIT_TARGET * 0.5,
                    ),
                    response.id.with("threshold"),
                    Sense::click_and_drag(),
                )
                .on_hover_cursor(egui::CursorIcon::ResizeVertical);
            if hit.clicked() || hit.drag_started() {
                hit.request_focus();
            }
            crate::parameter_focus::register_parameter_history_focus(&hit);
            let fine = ui.input(|input| if input.modifiers.shift { 0.1 } else { 1.0 });
            let mut next = at;
            if hit.dragged_by(egui::PointerButton::Primary) {
                next += hit.drag_delta().y / inner.height() * floor * fine;
            }
            if hit.has_focus() {
                ui.input_mut(|input| {
                    let mut pressed = |key| {
                        input.consume_key(egui::Modifiers::NONE, key)
                            || input.consume_key(egui::Modifiers::SHIFT, key)
                    };
                    if pressed(egui::Key::ArrowUp) {
                        next += fine;
                    }
                    if pressed(egui::Key::ArrowDown) {
                        next -= fine;
                    }
                });
            }
            let next = next.clamp(floor, 0.0);
            if next != at {
                threshold = Some(next);
            }
            union = union.union(hit.clone());
            hit
        });
    if threshold.is_some() {
        union.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let painter = ui.painter().with_clip_rect(inner);
        paint_frame(ui.painter(), rect, false);
        let mut db = (floor / 12.0).ceil() * 12.0;
        while db < 0.0 {
            if db > floor {
                painter.line_segment(
                    [
                        pos2(inner.left(), level_y(db)),
                        pos2(inner.right(), level_y(db)),
                    ],
                    Stroke::new(1.0_f32, colors.grid_minor),
                );
            }
            db += 12.0;
        }
        let count = history.columns.len();
        if count >= 2 {
            let x = |index: usize| inner.left() + inner.width() * index as f32 / (count - 1) as f32;
            let level = |value: f32| level_y(20.0 * finite(value, 0.0).abs().max(1e-6).log10());
            let trace = |column: usize| -> Vec<Pos2> {
                history
                    .columns
                    .iter()
                    .enumerate()
                    .map(|(index, values)| pos2(x(index), level(values[column])))
                    .collect()
            };
            fill_to(
                &painter,
                &trace(0),
                inner.bottom(),
                colors.muted.gamma_multiply(0.3),
            );
            painter.add(Shape::line(
                trace(1),
                Stroke::new(1.0_f32, colors.text.gamma_multiply(0.7)),
            ));
            let reduction: Vec<Pos2> = history
                .columns
                .iter()
                .enumerate()
                .map(|(index, values)| {
                    let share = finite(values[2], 0.0) / history.max_reduction_db.max(1.0);
                    pos2(x(index), top + length * share.clamp(0.0, 1.0))
                })
                .collect();
            painter.add(Shape::line(reduction, Stroke::new(2.0_f32, colors.accent)));
        }
        let span = label(
            &painter,
            inner.right_bottom() + vec2(-4.0, -2.0),
            Align2::RIGHT_BOTTOM,
            &format!("{:.0} s", history.span_seconds),
        );
        if let Some(at) = history.threshold_db {
            let y = level_y(at);
            let active = line
                .as_ref()
                .is_some_and(|hit| hit.hovered() || hit.dragged() || hit.has_focus());
            painter.line_segment(
                [pos2(inner.left(), y), pos2(inner.right(), y)],
                Stroke::new(
                    if active { 1.5_f32 } else { 1.0_f32 },
                    if active { colors.text } else { colors.muted },
                ),
            );
            // Above the line, or below it where the frame is too close.
            let above = y - tokens::FONT_SMALL - 4.0 > inner.top();
            let color = if active { colors.text } else { colors.muted };
            let galley = painter.layout_no_wrap(
                history.threshold_label.to_owned(),
                TextRole::Note.font(),
                color,
            );
            let top = if above {
                y - 2.0 - galley.size().y
            } else {
                y + 2.0
            };
            let mut at = pos2(inner.right() - 4.0 - galley.size().x, top);
            // Near the floor the span stands in that corner: beside it.
            if Rect::from_min_size(at, galley.size()).intersects(span) {
                at.x = span.left() - tokens::SPACE_SM - galley.size().x;
            }
            painter.galley(at, galley, color);
        }
        // The scale over the traces, so it reads through them.
        let mut db = (floor / 12.0).ceil() * 12.0;
        while db < 0.0 {
            if db > floor {
                label(
                    &painter,
                    pos2(inner.left() + 3.0, level_y(db) - 1.0),
                    Align2::LEFT_BOTTOM,
                    &format!("{db:.0}"),
                );
            }
            db += 12.0;
        }
        label(
            &painter,
            inner.left_top() + vec2(3.0, 1.0),
            Align2::LEFT_TOP,
            "0 dB",
        );
    }
    describe(&union, ui, "Level and gain reduction history".into());
    LevelHistoryOutput {
        response: union,
        threshold,
        line,
    }
}

/// Vertical gain-reduction bar that reads downward from 0 dB.
#[derive(Clone, Copy, Debug)]
pub struct ReductionMeter {
    pub reduction_db: f32,
    /// The largest reduction since the caller last reset it, marked on the
    /// bar and printed under it.
    pub hold_db: f32,
    pub max_reduction_db: f32,
    pub size: Vec2,
}

const REDUCTION_BAR_WIDTH: f32 = tokens::SPACE_MD;

/// Gain-reduction bar with a scale beside it and the held maximum under it.
/// A click is reported through the response; the caller then resets its hold.
pub fn reduction_meter(ui: &mut Ui, meter: &ReductionMeter) -> Response {
    let (rect, response) = frame(ui, meter.size, Sense::click());
    let colors = theme::palette(ui.ctx());
    let max = meter.max_reduction_db.max(1.0);
    let (top, length) = reduction_axis(rect);
    let y = |db: f32| top + length * (finite(db, 0.0) / max).clamp(0.0, 1.0);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        paint_frame(painter, rect, false);
        let bar = Rect::from_min_max(
            pos2(rect.left() + tokens::SPACE_XS, top),
            pos2(
                rect.left() + tokens::SPACE_XS + REDUCTION_BAR_WIDTH.min(rect.width() * 0.4),
                top + length,
            ),
        );
        painter.rect_filled(bar, 1.0, colors.control_fill);
        painter.rect_filled(
            Rect::from_min_max(bar.min, pos2(bar.right(), y(meter.reduction_db))),
            1.0,
            colors.accent,
        );
        // A short meter names only the ticks that have room.
        let mut named = f32::NEG_INFINITY;
        for tick in [0.0, 3.0, 6.0, 12.0, 24.0, 48.0] {
            if tick > max {
                break;
            }
            painter.line_segment(
                [
                    pos2(bar.right() + 2.0, y(tick)),
                    pos2(bar.right() + 5.0, y(tick)),
                ],
                Stroke::new(1.0_f32, colors.muted),
            );
            // The ends keep their numbers inside the axis.
            let align = if tick == 0.0 {
                Align2::LEFT_TOP
            } else if tick == max {
                Align2::LEFT_BOTTOM
            } else {
                Align2::LEFT_CENTER
            };
            let above = match align {
                Align2::LEFT_TOP => 0.0,
                Align2::LEFT_BOTTOM => tokens::FONT_SMALL,
                _ => tokens::FONT_SMALL * 0.5,
            };
            if y(tick) - above >= named {
                label(
                    painter,
                    pos2(bar.right() + 8.0, y(tick)),
                    align,
                    &format!("{tick:.0}"),
                );
                named = y(tick) - above + tokens::FONT_SMALL;
            }
        }
        if meter.hold_db > 0.0 {
            painter.line_segment(
                [
                    pos2(bar.left(), y(meter.hold_db)),
                    pos2(bar.right(), y(meter.hold_db)),
                ],
                Stroke::new(1.5_f32, colors.text),
            );
        }
        painter.text(
            pos2(rect.center().x, rect.bottom() - 2.0),
            Align2::CENTER_BOTTOM,
            if meter.hold_db >= 0.05 {
                format!("-{:.1}", finite(meter.hold_db, 0.0))
            } else {
                "0.0".into()
            },
            TextRole::Note.font(),
            colors.text,
        );
    }
    describe(
        &response,
        ui,
        format!(
            "Gain reduction {:.1} dB, held {:.1} dB",
            finite(meter.reduction_db, 0.0),
            finite(meter.hold_db, 0.0)
        ),
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

/// Harmonic levels as bars.
#[derive(Clone, Copy, Debug)]
pub struct HarmonicsView<'a> {
    /// One level per harmonic, in dB; drawn relative to the first, so the
    /// fundamental stands at 0 dB.
    pub levels_db: &'a [f32],
    /// The bottom of the axis, below zero.
    pub floor_db: f32,
    pub size: Vec2,
}

const HARMONIC_BAR_MAX: f32 = 34.0;

/// Where the bars of a [`HarmonicsView`] of this frame stand: the plot area
/// and each bar's rectangle. Bars are centered in equal slots and no wider
/// than 34 points, so a wide view does not turn into blocks.
pub fn harmonic_bars(view: &HarmonicsView<'_>, rect: Rect) -> (Rect, Vec<Rect>) {
    let text = tokens::FONT_SMALL + tokens::SPACE_XS;
    // The scale's numbers at the left, a bar's level above and its number below.
    let area = Rect::from_min_max(
        rect.min + vec2(tokens::SPACE_XL, text + tokens::SPACE_XS),
        rect.max - vec2(tokens::SPACE_SM, text),
    );
    let floor = view.floor_db.min(-1.0);
    let reference = view
        .levels_db
        .first()
        .copied()
        .map_or(0.0, |db| finite(db, 0.0));
    let slot = area.width() / view.levels_db.len().max(1) as f32;
    let width = (slot * 0.6).min(HARMONIC_BAR_MAX);
    let bars = view
        .levels_db
        .iter()
        .enumerate()
        .map(|(index, db)| {
            let level = (finite(*db, f32::NEG_INFINITY) - reference).clamp(floor, 0.0);
            let x = area.left() + slot * (index as f32 + 0.5);
            Rect::from_min_max(
                pos2(
                    x - width * 0.5,
                    area.bottom() - area.height() * (level - floor) / -floor,
                ),
                pos2(x + width * 0.5, area.bottom()),
            )
        })
        .collect();
    (area, bars)
}

/// Bars for the harmonics of a shaper, with a dB scale, each bar's number
/// under it and its level over it where it rises above the floor.
pub fn harmonics_view(ui: &mut Ui, view: &HarmonicsView<'_>) -> Response {
    let (rect, response) = frame(ui, view.size, Sense::hover());
    let colors = theme::palette(ui.ctx());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        paint_frame(painter, rect, false);
        let (area, bars) = harmonic_bars(view, rect);
        let floor = view.floor_db.min(-1.0);
        let reference = view
            .levels_db
            .first()
            .copied()
            .map_or(0.0, |db| finite(db, 0.0));
        let mut db = 0.0;
        while db > floor {
            let y = area.bottom() - area.height() * (db - floor) / -floor;
            painter.line_segment(
                [pos2(area.left(), y), pos2(area.right(), y)],
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
                pos2(area.left() - tokens::SPACE_XS, y),
                Align2::RIGHT_CENTER,
                &format!("{db:.0}"),
            );
            db -= 20.0;
        }
        painter.line_segment(
            [area.left_bottom(), area.right_bottom()],
            Stroke::new(1.0_f32, colors.grid_bar),
        );
        for (index, (bar, db)) in bars.iter().zip(view.levels_db).enumerate() {
            let level = finite(*db, f32::NEG_INFINITY) - reference;
            if level > floor {
                painter.rect_filled(
                    *bar,
                    1.0,
                    if index == 0 {
                        colors.accent
                    } else {
                        colors.accent.gamma_multiply(0.7)
                    },
                );
                label(
                    painter,
                    pos2(bar.center().x, bar.top() - 2.0),
                    Align2::CENTER_BOTTOM,
                    &format!("{level:.0}"),
                );
            }
            label(
                painter,
                pos2(bar.center().x, rect.bottom() - 2.0),
                Align2::CENTER_BOTTOM,
                &format!("{}", index + 1),
            );
        }
    }
    describe(
        &response,
        ui,
        format!("Harmonics: {}", view.levels_db.len()),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_handle_and_its_label_stay_inside_the_frame_and_apart() {
        let inner = Rect::from_min_size(pos2(10.0, 20.0), vec2(208.0, 208.0));
        let size = vec2(8.0, 14.0);
        let points = [
            inner.left_top(),
            inner.right_top(),
            inner.left_bottom(),
            inner.right_bottom(),
            inner.center(),
            inner.right_top() + vec2(-30.0, 12.0),
        ];
        for at in points {
            let (center, label) = place_handle(inner, at, size);
            let handle = Rect::from_center_size(center, Vec2::splat(2.0 * HANDLE_RADIUS));
            let label = Rect::from_min_size(label, size);
            assert!(inner.contains_rect(handle), "{at:?}: handle {handle:?}");
            assert!(inner.contains_rect(label), "{at:?}: label {label:?}");
            assert!(!label.intersects(handle), "{at:?}: {label:?} on {handle:?}");
            // A rising curve through the handle runs down-left and
            // up-right of it: the label is in neither quarter.
            let up_left = label.right() <= center.x && label.bottom() <= center.y;
            let down_right = label.left() >= center.x && label.top() >= center.y;
            let beside = label.top() >= handle.bottom() || label.bottom() <= handle.top();
            assert!(up_left || down_right || beside, "{at:?}: {label:?}");
        }
        // At the top right, a ceiling: below the flat curve.
        let (center, label) = place_handle(inner, inner.right_top(), size);
        assert!(label.y > center.y + HANDLE_RADIUS);
    }
}
