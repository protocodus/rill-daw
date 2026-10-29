use daw_ui::{CurveEdit, CurvePoint, CurveView};
use egui::{Id, Sense, Ui, vec2};

pub struct State {
    points: Vec<CurvePoint>,
    selected: Option<u64>,
    fade: daw_ui::CrossfadeCurve,
}

impl Default for State {
    fn default() -> Self {
        Self {
            points: vec![
                CurvePoint {
                    id: 1,
                    x: 0.0,
                    value: 0.18,
                },
                CurvePoint {
                    id: 2,
                    x: 2.0,
                    value: 0.72,
                },
                CurvePoint {
                    id: 3,
                    x: 4.0,
                    value: 0.40,
                },
                CurvePoint {
                    id: 4,
                    x: 6.0,
                    value: 0.84,
                },
                CurvePoint {
                    id: 5,
                    x: 8.0,
                    value: 0.58,
                },
            ],
            selected: Some(3),
            fade: daw_ui::CrossfadeCurve {
                control1: [0.20, 0.64],
                control2: [0.76, 0.90],
            },
        }
    }
}

fn sample(points: &[CurvePoint], at: f64) -> f32 {
    let Some(first) = points.first() else {
        return 0.0;
    };
    if at <= first.x {
        return first.value;
    }
    for pair in points.windows(2) {
        if at <= pair[1].x {
            let amount = ((at - pair[0].x) / (pair[1].x - pair[0].x).max(f64::EPSILON)) as f32;
            return egui::lerp(pair[0].value..=pair[1].value, amount);
        }
    }
    points.last().unwrap().value
}

pub fn show(ui: &mut Ui, state: &mut State) {
    ui.columns(2, |columns| {
        let left = &mut columns[0];
        daw_ui::text::subtitle(left, "Breakpoint editor");
        daw_ui::text::note(left, "curve_editor · selected point · quarter-beat snap");
        left.add_space(daw_ui::tokens::SPACE_SM);
        let points = state.points.clone();
        let response = daw_ui::curve_editor(
            left,
            Id::new("gallery_breakpoints"),
            &points,
            &mut state.selected,
            CurveView {
                start: 0.0,
                end: 8.0,
                snap: 0.25,
                height: 270.0,
            },
            |at| sample(&points, at),
        );
        match response.edit {
            Some(CurveEdit::Add { x, value }) => {
                let id = state.points.iter().map(|point| point.id).max().unwrap_or(0) + 1;
                state.points.push(CurvePoint { id, x, value });
            }
            Some(CurveEdit::Move { id, x, value }) => {
                if let Some(point) = state.points.iter_mut().find(|point| point.id == id) {
                    point.x = x;
                    point.value = value;
                }
            }
            Some(CurveEdit::Delete { id }) => state.points.retain(|point| point.id != id),
            // This basic point editor has no segment handles; the spline
            // timeline variant additionally emits per-segment slope changes.
            Some(CurveEdit::SetCurve { .. }) | None => {}
        }
        state.points.sort_by(|a, b| a.x.total_cmp(&b.x));
        daw_ui::text::note(left, "Click to add · drag to move · Delete to remove");

        let right = &mut columns[1];
        daw_ui::text::subtitle(right, "Complementary crossfade");
        daw_ui::text::note(right, "crossfade_editor · two editable Bézier handles");
        right.add_space(daw_ui::tokens::SPACE_SM);
        daw_ui::crossfade_editor(right, Id::new("gallery_crossfade"), &mut state.fade);
        right.add_space(daw_ui::tokens::SPACE_SM);
        daw_ui::text::body(right, "Outgoing and incoming gains share one curve.");
        daw_ui::text::note(right, "Drag a handle · arrows adjust · Home resets");
    });
    ui.add_space(daw_ui::tokens::SPACE_XL);
    daw_ui::text::subtitle(ui, "Compact timeline overlays");
    ui.add_space(daw_ui::tokens::SPACE_SM);
    let color = daw_ui::theme::palette(ui.ctx()).accent;
    ui.columns(2, |columns| {
        daw_ui::text::note(&mut columns[0], "paint_curve");
        let width = columns[0].available_width();
        let (rect, _) = columns[0].allocate_exact_size(vec2(width, 90.0), Sense::hover());
        daw_ui::paint_curve(
            columns[0].painter(),
            rect.shrink(8.0),
            0.0,
            8.0,
            color,
            |at| sample(&state.points, at),
        );
        daw_ui::text::note(&mut columns[1], "paint_crossfade");
        let width = columns[1].available_width();
        let (rect, _) = columns[1].allocate_exact_size(vec2(width, 90.0), Sense::hover());
        daw_ui::paint_crossfade(columns[1].painter(), rect.shrink(8.0), state.fade);
    });
}
