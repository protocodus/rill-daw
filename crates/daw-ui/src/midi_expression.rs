//! Exact note-expression controls. The caller owns musical time conversion and
//! retained packets; this component never rounds a value merely by displaying it.
use egui::{Id, Rect, Ui};

fn integer_control(
    ui: &mut Ui,
    value: &mut u32,
    maximum: u32,
    name: &str,
    speed: f64,
) -> egui::Response {
    let mut exact = f64::from(*value);
    let response = crate::number_stepper_integer(
        ui,
        &mut exact,
        1.0,
        0.0..=f64::from(maximum),
        name,
        |ui, value| {
            ui.add(
                crate::number(value)
                    .range(0.0..=f64::from(maximum))
                    .speed(speed)
                    .max_decimals(0)
                    .update_while_editing(false),
            )
        },
    )
    .response;
    if response.changed() {
        *value = exact.round().clamp(0.0, f64::from(maximum)) as u32;
    }
    response
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidiExpressionKind {
    Pressure,
    Pitch,
    RegisteredController,
    AssignableController,
}

impl MidiExpressionKind {
    pub const ALL: [Self; 4] = [
        Self::Pressure,
        Self::Pitch,
        Self::RegisteredController,
        Self::AssignableController,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Pressure => "Pressure",
            Self::Pitch => "Pitch bend",
            Self::RegisteredController => "Registered controller",
            Self::AssignableController => "Assignable controller",
        }
    }
    pub fn status(self) -> u8 {
        match self {
            Self::Pressure => 10,
            Self::Pitch => 6,
            Self::RegisteredController => 0,
            Self::AssignableController => 1,
        }
    }
    pub fn from_status(status: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.status() == status)
    }
    pub fn has_controller(self) -> bool {
        matches!(
            self,
            Self::RegisteredController | Self::AssignableController
        )
    }
    pub fn initial_value(self) -> u32 {
        if self == Self::Pitch { 0x8000_0000 } else { 0 }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MidiExpressionPoint {
    pub id: u64,
    /// Audible quarter-note beats after the note start, converted by the host.
    pub beat: f64,
    pub kind: Option<MidiExpressionKind>,
    pub controller: u8,
    pub value: u32,
}

#[derive(Clone, Debug)]
pub struct MidiExpressionControls {
    pub velocity: u16,
    pub release_velocity: u16,
    pub pitch_semitones: f64,
    pub selected: Option<u64>,
    pub kind: MidiExpressionKind,
    pub controller: u8,
    pub beat: f64,
    pub value: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidiExpressionAction {
    Select(u64),
    New,
    Add,
    Update,
    Remove,
}

pub struct MidiExpressionResponse {
    pub velocity_changed: bool,
    pub release_changed: bool,
    pub pitch_changed: bool,
    pub action: Option<MidiExpressionAction>,
    pub velocity_rect: Rect,
    pub release_rect: Rect,
    pub pitch_rect: Rect,
    pub time_rect: Rect,
    pub value_rect: Rect,
    pub normalized_rect: Rect,
    pub controller_rect: Rect,
    pub rows: Vec<(u64, Rect)>,
}

/// A compact exact-value editor plus a virtualized list of timestamped events.
/// It deliberately makes no interpolation promise: retained UMP events remain
/// exact events, and all edits are owned by the caller's cancellable draft.
pub fn midi_expression_editor(
    ui: &mut Ui,
    id: Id,
    controls: &mut MidiExpressionControls,
    points: &[MidiExpressionPoint],
    maximum_beat: f64,
) -> MidiExpressionResponse {
    let mut result = MidiExpressionResponse {
        velocity_changed: false,
        release_changed: false,
        pitch_changed: false,
        action: None,
        velocity_rect: Rect::NOTHING,
        release_rect: Rect::NOTHING,
        pitch_rect: Rect::NOTHING,
        time_rect: Rect::NOTHING,
        value_rect: Rect::NOTHING,
        normalized_rect: Rect::NOTHING,
        controller_rect: Rect::NOTHING,
        rows: Vec::new(),
    };
    ui.push_id(id, |ui| {
        crate::form_grid(ui, "note_values", |ui| {
            let label = crate::form_label(ui, "Attack velocity");
            let mut exact = u32::from(controls.velocity);
            let response = integer_control(ui,&mut exact,u32::from(u16::MAX),"Attack velocity",128.0).labelled_by(label.id).on_hover_text("Exact 16-bit attack velocity, 0–65535. Legacy notes convert only when a value is changed.");
            if response.changed() { controls.velocity = exact as u16; }
            result.velocity_changed = response.changed(); result.velocity_rect = response.rect; ui.end_row();
            let label = crate::form_label(ui, "Release velocity");
            let mut exact = u32::from(controls.release_velocity);
            let response = integer_control(ui,&mut exact,u32::from(u16::MAX),"Release velocity",128.0).labelled_by(label.id);
            if response.changed() { controls.release_velocity = exact as u16; }
            result.release_changed = response.changed(); result.release_rect = response.rect; ui.end_row();
            let label = crate::form_label(ui, "Note pitch");
            let response = crate::number_stepper(ui,&mut controls.pitch_semitones,1.0/512.0,0.0..=(65535.0/512.0),"Note pitch",|ui,value|ui.add(crate::number(value).range(0.0..=(65535.0/512.0)).speed(1.0/512.0).max_decimals(9).custom_formatter(|value,_|value.to_string()).suffix(" st").update_while_editing(false))).response.labelled_by(label.id).on_hover_text("MIDI 2.0 pitch attribute, in semitones, with 1/512-semitone resolution. The note's channel and wire identity are retained.");
            result.pitch_changed = response.changed(); result.pitch_rect = response.rect; ui.end_row();
        });
        crate::section_gap(ui);
        crate::text::label(ui, "Per-note events");
        if points.is_empty() { crate::text::note(ui, "No expression points. Add pressure, pitch bend or a per-note controller."); }
        else {
            let height = crate::tokens::CONTROL_HEIGHT;
            let viewport_height = 4.0 * height + 3.0 * ui.spacing().item_spacing.y;
            egui::ScrollArea::vertical().id_salt("expression_points").max_height(viewport_height).auto_shrink([false,true]).show_rows(ui,height,points.len(),|ui,range| {
                for index in range {
                    let point = points[index];
                    let caption = if let Some(kind) = point.kind {
                        format!("{:.6} · {}{} · {}",point.beat,kind.label(),if kind.has_controller(){format!(" {}",point.controller)}else{String::new()},point.value)
                    } else { format!("{:.6} · Retained raw packet · 0x{:08X}",point.beat,point.value) };
                    let response = crate::selectable(ui,controls.selected == Some(point.id),caption).on_hover_text("Time is in audible beats after the note start. Raw packets without a supported editor are retained unchanged.");
                    result.rows.push((point.id,response.rect));
                    if response.clicked() { result.action = Some(MidiExpressionAction::Select(point.id)); }
                }
            });
        }
        crate::section_gap(ui);
        let selected = controls.selected.and_then(|id| points.iter().find(|point| point.id == id));
        let editable = selected.is_none_or(|point| point.kind.is_some());
        let mut point_changed = false;
        ui.add_enabled_ui(editable, |ui| {
            crate::form_grid(ui,"expression_value",|ui| {
                crate::form_label(ui,"Expression");
                let before = controls.kind;
                // Four options, but their names are too long to sit side by side as toggle buttons.
                crate::select("expression_kind",controls.kind.label()).width(crate::tokens::FORM_FIELD_WIDTH).show_ui(ui,|ui| {
                    for kind in MidiExpressionKind::ALL { crate::selectable_value(ui,&mut controls.kind,kind,kind.label()); }
                });
                if before != controls.kind { controls.value = controls.kind.initial_value(); point_changed = true; }
                ui.end_row();
                let label = crate::form_label(ui,"Controller");
                let mut exact = u32::from(controls.controller);
                let response = ui.add_enabled_ui(controls.kind.has_controller(),|ui|integer_control(ui,&mut exact,127,"Controller",1.0)).inner.labelled_by(label.id);
                if response.changed() { controls.controller = exact as u8; }
                point_changed |= response.changed(); result.controller_rect = response.rect; ui.end_row();
                let label = crate::form_label(ui,"After note");
                let maximum_beat = maximum_beat.max(controls.beat);
                let response = crate::number_stepper(ui,&mut controls.beat,0.01,0.0..=maximum_beat,"Expression time",|ui,value|ui.add(crate::number(value).range(0.0..=maximum_beat).speed(0.01).max_decimals(9).suffix(" beats").update_while_editing(false))).response.labelled_by(label.id);
                point_changed |= response.changed(); result.time_rect = response.rect; ui.end_row();
                let label = crate::form_label(ui,"Exact value");
                let response = integer_control(ui,&mut controls.value,u32::MAX,"Expression value",16_777_216.0).labelled_by(label.id).on_hover_text("Exact unsigned 32-bit value. Changing another field never quantizes this value.");
                point_changed |= response.changed(); result.value_rect = response.rect; ui.end_row();
                let label = crate::form_label(ui,"Normalized");
                let mut normalized = f64::from(controls.value) / f64::from(u32::MAX);
                let response = ui.add(crate::Slider::new(&mut normalized,0.0..=1.0).show_value(false)).labelled_by(label.id);
                if response.changed() { controls.value = (normalized*f64::from(u32::MAX)).round().clamp(0.0,f64::from(u32::MAX)) as u32; point_changed = true; }
                result.normalized_rect = response.rect;
                ui.end_row();
            });
        });
        if !editable { crate::text::note(ui,"This raw expression is read-only. New point adds a supported expression; Remove point deletes only this event."); }
        if point_changed && controls.selected.is_some() { result.action = Some(MidiExpressionAction::Update); }
        crate::parameter_row(ui,|ui| {
            if crate::button(ui,"New point").clicked() { result.action = Some(MidiExpressionAction::New); }
            if ui.add_enabled(editable && controls.selected.is_none(),crate::button_widget("Add point")).clicked() { result.action = Some(MidiExpressionAction::Add); }
            if ui.add_enabled(controls.selected.is_some(),crate::button_widget("Remove point")).clicked() { result.action = Some(MidiExpressionAction::Remove); }
            if controls.kind == MidiExpressionKind::Pitch && ui.add_enabled(editable,crate::button_widget("Center pitch")).clicked() {
                controls.value = 0x8000_0000;
                if controls.selected.is_some() { result.action = Some(MidiExpressionAction::Update); }
            }
        });
    });
    result
}
