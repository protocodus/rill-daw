//! Original Rill outline icons. Resolution-independent geometry, shared weight,
//! native button layout/accessibility, and no dependency on a symbol font.
use egui::{
    Atom, Button, Color32, Id, Rect, Response, Shape, Stroke, Ui, Vec2, Widget, WidgetText,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Select,
    Pencil,
    Cut,
    Copy,
    Paste,
    Duplicate,
    Link,
    Unlink,
    Glue,
    Stretch,
    Slip,
    Erase,
    Stack,
    FadeIn,
    FadeOut,
    Cursor,
    Fit,
    Start,
    End,
    Play,
    Pause,
    Stop,
    Record,
    Metronome,
    Loop,
    Undo,
    Redo,
    Add,
    Delete,
    Close,
    Check,
    Warning,
    Settings,
    Folder,
    Library,
    Instrument,
    Effect,
    Mixer,
    Routing,
    Automation,
    Save,
    Open,
    Editor,
    Export,
    Search,
    Refresh,
    Reverse,
    Normalize,
    Waveform,
    History,
    Up,
    Down,
    Back,
    More,
    Grip,
    View,
    ChevronDown,
    ChevronRight,
    Star,
    StarFilled,
}

impl Icon {
    pub fn for_label(label: &str) -> Option<Self> {
        Some(match label {
            "Select" => Self::Select,
            "Pencil" => Self::Pencil,
            "Cut" | "Cut notes" | "Cut selection" | "Split at cursor" | "Split transients" => {
                Self::Cut
            }
            "Copy" | "Copy notes" | "Copy path" => Self::Copy,
            "Paste" | "Paste notes at cursor" => Self::Paste,
            "Duplicate" | "Duplicate clip" | "Duplicate selection" => Self::Duplicate,
            "Ghost copy" => Self::Link,
            "Stretch" => Self::Stretch,
            "Slip" => Self::Slip,
            "Make independent" => Self::Unlink,
            "Glue" | "Glue selected" | "Legato" | "Join" | "Join clips…" => Self::Glue,
            "Erase" => Self::Erase,
            "Multi-draw" | "Generate…" => Self::Stack,
            "Fade in" => Self::FadeIn,
            "Fade out" => Self::FadeOut,
            "Cursor" => Self::Cursor,
            "Fit" | "Fit preview" => Self::Fit,
            "Start" | "Clip start" | "Set start" => Self::Start,
            "Clip end" | "Set end" | "Snap end" => Self::End,
            "Play" => Self::Play,
            "Pause" => Self::Pause,
            "Stop" => Self::Stop,
            "Record" | "Arm recording" | "Recording" | "Inputs…" => Self::Record,
            "Stop recording" => Self::Stop,
            "Click" | "Metronome" => Self::Metronome,
            "Loop" | "Range" => Self::Loop,
            "Add notes" => Self::Add,
            "New variation" => Self::Refresh,
            "Crossfade…" => Self::Automation,
            "Undo" => Self::Undo,
            "Redo" => Self::Redo,
            "Delete"
            | "Delete selection"
            | "Ripple delete selection"
            | "Delete point"
            | "Delete track"
            | "Remove" => Self::Delete,
            "Cancel" | "Cancel quantize" | "Close" | "Dismiss" => Self::Close,
            "Apply" | "Apply quantize" | "Assign to selected track" => Self::Check,
            "Setup" | "Properties" | "Manage" => Self::Settings,
            "Add folder" | "Browse…" => Self::Folder,
            "Library" | "Browse effects" => Self::Library,
            "Instrument" | "Notes" | "Add instrument…" | "Create instrument track" => {
                Self::Instrument
            }
            "Effects" | "Add to effect chain" => Self::Effect,
            "Mixer" | "Channel" => Self::Mixer,
            "Routing" | "Create send effect" => Self::Routing,
            "Edit automation" | "Add automation track" | "Add automation track…" | "Automate" => {
                Self::Automation
            }
            "Save" | "Save as…" => Self::Save,
            "Open…" => Self::Open,
            "Open editor" | "Show editor" => Self::Editor,
            "Export WAV…" | "Export MIDI…" => Self::Export,
            "Search" => Self::Search,
            "Scan" | "Rescan" | "Refresh" | "Refresh preview" | "Refresh parameters" => {
                Self::Refresh
            }
            "Reverse" => Self::Reverse,
            "Normalize" => Self::Normalize,
            "Analyze" | "Transients" => Self::Waveform,
            "History…" => Self::History,
            "Semitone up" | "Octave up" => Self::Up,
            "Semitone down" | "Octave down" => Self::Down,
            "Back" => Self::Back,
            "More" => Self::More,
            "View" => Self::View,
            "Favorite" | "Favorites" | "Add to favorites" => Self::Star,
            "Remove from favorites" => Self::StarFilled,
            "Add point" | "Add parameter" | "New" => Self::Add,
            _ => return None,
        })
    }

    pub fn paint(self, painter: &egui::Painter, rect: Rect, color: Color32) {
        let size = rect.width().min(rect.height());
        let origin = rect.center() - Vec2::splat(size * 0.5);
        let point = |x: f32, y: f32| origin + Vec2::new(x, y) * (size / 24.0);
        let stroke = Stroke::new((size / 24.0 * 1.8).max(1.0), color);
        let line = |points: &[(f32, f32)]| {
            painter.add(Shape::line(
                points.iter().map(|&(x, y)| point(x, y)).collect(),
                stroke,
            ));
        };
        let box_at = |x: f32, y: f32, w: f32, h: f32| {
            painter.rect_stroke(
                Rect::from_min_max(point(x, y), point(x + w, y + h)),
                1.0,
                stroke,
                egui::StrokeKind::Inside,
            );
        };
        let circle = |x: f32, y: f32, r: f32| {
            painter.circle_stroke(point(x, y), r * size / 24.0, stroke);
        };
        match self {
            Self::View => {
                line(&[
                    (2., 12.),
                    (5., 8.),
                    (8., 6.),
                    (12., 5.),
                    (16., 6.),
                    (19., 8.),
                    (22., 12.),
                    (19., 16.),
                    (16., 18.),
                    (12., 19.),
                    (8., 18.),
                    (5., 16.),
                    (2., 12.),
                ]);
                circle(12., 12., 3.5);
            }
            Self::ChevronDown => line(&[(5., 9.), (12., 16.), (19., 9.)]),
            Self::Star | Self::StarFilled => {
                let points: Vec<_> = (0..10)
                    .map(|index| {
                        let radius = if index % 2 == 0 { 10.0 } else { 4.2 };
                        let angle = std::f32::consts::PI * (index as f32 / 5.0 - 0.5);
                        point(12. + radius * angle.cos(), 12.8 + radius * angle.sin())
                    })
                    .collect();
                if self == Self::StarFilled {
                    // A star is not convex, but every edge is visible from its
                    // center, so a triangle fan fills it exactly.
                    let mut mesh = egui::Mesh::default();
                    mesh.colored_vertex(point(12., 12.8), color);
                    for vertex in &points {
                        mesh.colored_vertex(*vertex, color);
                    }
                    for index in 0..10u32 {
                        mesh.add_triangle(0, index + 1, (index + 1) % 10 + 1);
                    }
                    painter.add(mesh);
                }
                let mut outline = points;
                outline.push(outline[0]);
                painter.add(Shape::line(outline, stroke));
            }
            Self::ChevronRight => line(&[(9., 5.), (16., 12.), (9., 19.)]),
            Self::Warning => {
                line(&[(12., 3.), (22., 21.), (2., 21.), (12., 3.)]);
                line(&[(12., 9.), (12., 14.)]);
                painter.circle_filled(point(12., 17.5), size / 24.0, color);
            }
            Self::Select => line(&[(5., 3.), (19., 13.), (12., 14.), (9., 21.), (5., 3.)]),
            Self::Pencil => {
                line(&[
                    (4., 20.),
                    (5., 14.),
                    (16., 3.),
                    (21., 8.),
                    (10., 19.),
                    (4., 20.),
                ]);
                line(&[(14., 5.), (19., 10.)]);
            }
            Self::Cut => {
                circle(5., 6., 3.);
                circle(5., 18., 3.);
                line(&[(7., 8.), (20., 20.)]);
                line(&[(7., 16.), (20., 4.)]);
            }
            Self::Copy | Self::Duplicate => {
                box_at(8., 8., 12., 13.);
                line(&[(16., 5.), (16., 3.), (3., 3.), (3., 16.), (5., 16.)]);
                if self == Self::Duplicate {
                    line(&[(11., 14.), (17., 14.)]);
                    line(&[(14., 11.), (14., 17.)]);
                }
            }
            Self::Link => {
                line(&[
                    (9., 15.),
                    (7., 17.),
                    (4., 17.),
                    (2., 15.),
                    (2., 12.),
                    (8., 6.),
                    (11., 6.),
                    (13., 8.),
                    (13., 11.),
                ]);
                line(&[
                    (15., 9.),
                    (17., 7.),
                    (20., 7.),
                    (22., 9.),
                    (22., 12.),
                    (16., 18.),
                    (13., 18.),
                    (11., 16.),
                    (11., 13.),
                ]);
                line(&[(8., 16.), (16., 8.)]);
            }
            Self::Unlink => {
                line(&[
                    (8., 12.),
                    (4., 16.),
                    (4., 19.),
                    (6., 21.),
                    (9., 21.),
                    (13., 17.),
                ]);
                line(&[
                    (11., 7.),
                    (15., 3.),
                    (18., 3.),
                    (20., 5.),
                    (20., 8.),
                    (16., 12.),
                ]);
                line(&[(3., 3.), (21., 21.)]);
            }
            Self::Paste => {
                box_at(5., 5., 14., 16.);
                box_at(9., 2., 6., 5.);
                line(&[(9., 12.), (15., 12.)]);
                line(&[(9., 16.), (15., 16.)]);
            }
            Self::Glue => {
                box_at(3., 8., 7., 8.);
                box_at(14., 8., 7., 8.);
                line(&[(10., 12.), (14., 12.)]);
            }
            Self::Stretch => {
                box_at(7., 7., 10., 10.);
                line(&[(1., 12.), (7., 12.)]);
                line(&[(17., 12.), (23., 12.)]);
                line(&[(4., 9.), (1., 12.), (4., 15.)]);
                line(&[(20., 9.), (23., 12.), (20., 15.)]);
            }
            Self::Slip => {
                line(&[(3., 4.), (3., 20.)]);
                line(&[(21., 4.), (21., 20.)]);
                line(&[(6., 12.), (18., 12.)]);
                line(&[(9., 9.), (6., 12.), (9., 15.)]);
                line(&[(15., 9.), (18., 12.), (15., 15.)]);
            }
            Self::Erase => {
                line(&[
                    (3., 14.),
                    (12., 4.),
                    (21., 12.),
                    (13., 21.),
                    (9., 21.),
                    (3., 14.),
                ]);
                line(&[(7., 10.), (16., 18.)]);
                line(&[(12., 21.), (22., 21.)]);
            }
            Self::Stack => {
                box_at(3., 3., 13., 4.);
                box_at(6., 10., 13., 4.);
                box_at(9., 17., 13., 4.);
            }
            Self::FadeIn => {
                line(&[(3., 4.), (3., 21.), (22., 21.)]);
                line(&[(5., 19.), (8., 18.), (13., 14.), (17., 8.), (21., 3.)]);
            }
            Self::FadeOut => {
                line(&[(3., 4.), (3., 21.), (22., 21.)]);
                line(&[(5., 3.), (8., 8.), (13., 14.), (17., 18.), (21., 19.)]);
            }
            Self::Cursor => {
                line(&[(12., 2.), (12., 22.)]);
                line(&[(8., 3.), (16., 3.)]);
                line(&[(8., 21.), (16., 21.)]);
            }
            Self::Fit => {
                for (x, y, dx, dy) in [
                    (3., 3., 1., 1.),
                    (21., 3., -1., 1.),
                    (3., 21., 1., -1.),
                    (21., 21., -1., -1.),
                ] {
                    line(&[(x + 6. * dx, y), (x, y), (x, y + 6. * dy)]);
                }
            }
            Self::Start => {
                line(&[(4., 4.), (4., 20.)]);
                line(&[(20., 4.), (8., 12.), (20., 20.), (20., 4.)]);
            }
            Self::End => {
                line(&[(20., 4.), (20., 20.)]);
                line(&[(4., 4.), (16., 12.), (4., 20.), (4., 4.)]);
            }
            Self::Play => {
                painter.add(Shape::convex_polygon(
                    vec![point(6., 3.), point(21., 12.), point(6., 21.)],
                    color,
                    Stroke::NONE,
                ));
            }
            Self::Pause => {
                box_at(5., 3., 4., 18.);
                box_at(15., 3., 4., 18.);
            }
            Self::Stop => box_at(4., 4., 16., 16.),
            Self::Record => {
                painter.circle_filled(point(12., 12.), size * 0.30, color);
            }
            Self::Metronome => {
                line(&[(8., 3.), (16., 3.), (21., 21.), (3., 21.), (8., 3.)]);
                line(&[(12., 18.), (18., 6.)]);
                line(&[(7., 17.), (17., 17.)]);
            }
            Self::Loop | Self::Refresh => {
                line(&[(4., 9.), (4., 5.), (20., 5.), (20., 11.)]);
                line(&[(17., 8.), (20., 11.), (23., 8.)]);
                line(&[(20., 15.), (20., 19.), (4., 19.), (4., 13.)]);
                line(&[(1., 16.), (4., 13.), (7., 16.)]);
            }
            Self::Undo => {
                line(&[(8., 4.), (3., 9.), (8., 14.)]);
                line(&[(3., 9.), (15., 9.), (20., 12.), (20., 18.)]);
            }
            Self::Redo => {
                line(&[(16., 4.), (21., 9.), (16., 14.)]);
                line(&[(21., 9.), (9., 9.), (4., 12.), (4., 18.)]);
            }
            Self::Add => {
                line(&[(12., 4.), (12., 20.)]);
                line(&[(4., 12.), (20., 12.)]);
            }
            Self::Delete => {
                line(&[(3., 6.), (21., 6.)]);
                line(&[(8., 6.), (8., 3.), (16., 3.), (16., 6.)]);
                line(&[(5., 6.), (6., 21.), (18., 21.), (19., 6.)]);
                line(&[(10., 10.), (10., 17.)]);
                line(&[(14., 10.), (14., 17.)]);
            }
            Self::Close => {
                line(&[(5., 5.), (19., 19.)]);
                line(&[(19., 5.), (5., 19.)]);
            }
            Self::Check => line(&[(3., 12.), (9., 18.), (21., 5.)]),
            Self::Settings | Self::Mixer => {
                for (x, y) in [(5., 8.), (12., 16.), (19., 10.)] {
                    line(&[(x, 3.), (x, y - 2.)]);
                    line(&[(x, y + 2.), (x, 21.)]);
                    box_at(x - 2., y - 2., 4., 4.);
                }
            }
            Self::Folder | Self::Open => line(&[
                (2., 6.),
                (9., 6.),
                (11., 9.),
                (22., 9.),
                (20., 21.),
                (2., 21.),
                (2., 6.),
            ]),
            Self::Editor => {
                box_at(3., 4., 18., 16.);
                line(&[(3., 9.), (21., 9.)]);
                circle(8., 14.5, 2.5);
                line(&[(14., 13.), (18., 13.)]);
                line(&[(14., 17.), (18., 17.)]);
            }
            Self::Library => {
                box_at(3., 3., 7., 7.);
                box_at(14., 3., 7., 7.);
                box_at(3., 14., 7., 7.);
                box_at(14., 14., 7., 7.);
            }
            Self::Instrument => {
                box_at(2., 4., 20., 16.);
                line(&[(7., 4.), (7., 20.)]);
                line(&[(12., 4.), (12., 20.)]);
                line(&[(17., 4.), (17., 20.)]);
                for x in [6., 11., 16.] {
                    painter.rect_filled(
                        Rect::from_min_max(point(x, 4.), point(x + 2., 12.)),
                        0.0,
                        color,
                    );
                }
            }
            Self::Effect => {
                box_at(4., 3., 16., 18.);
                circle(12., 9., 3.);
                circle(12., 17., 1.);
            }
            Self::Routing => {
                box_at(2., 9., 6., 6.);
                box_at(16., 2., 6., 6.);
                box_at(16., 16., 6., 6.);
                line(&[(8., 12.), (12., 12.), (12., 5.), (16., 5.)]);
                line(&[(12., 12.), (12., 19.), (16., 19.)]);
            }
            Self::Automation => {
                line(&[(3., 18.), (10., 7.), (21., 13.)]);
                for (x, y) in [(3., 18.), (10., 7.), (21., 13.)] {
                    circle(x, y, 2.);
                }
            }
            Self::Save => {
                line(&[
                    (3., 3.),
                    (18., 3.),
                    (21., 6.),
                    (21., 21.),
                    (3., 21.),
                    (3., 3.),
                ]);
                box_at(7., 3., 9., 6.);
                box_at(7., 14., 10., 7.);
            }
            Self::Export => {
                line(&[(4., 13.), (4., 21.), (20., 21.), (20., 13.)]);
                line(&[(12., 16.), (12., 2.)]);
                line(&[(7., 7.), (12., 2.), (17., 7.)]);
            }
            Self::Search => {
                circle(10., 10., 7.);
                line(&[(15., 15.), (22., 22.)]);
            }
            Self::Reverse => {
                line(&[(3., 7.), (21., 7.), (17., 3.)]);
                line(&[(21., 17.), (3., 17.), (7., 21.)]);
            }
            Self::Normalize => {
                line(&[(3., 3.), (21., 3.)]);
                line(&[(3., 21.), (21., 21.)]);
                line(&[(12., 18.), (12., 6.)]);
                line(&[(8., 10.), (12., 6.), (16., 10.)]);
            }
            Self::Waveform => {
                for (x, h) in [
                    (3., 3.),
                    (7., 6.),
                    (11., 9.),
                    (15., 5.),
                    (19., 8.),
                    (23., 2.),
                ] {
                    line(&[(x, 12. - h), (x, 12. + h)]);
                }
            }
            Self::History => {
                circle(12., 12., 9.);
                line(&[(12., 6.), (12., 12.), (17., 15.)]);
            }
            Self::Up => {
                line(&[(12., 21.), (12., 3.)]);
                line(&[(5., 10.), (12., 3.), (19., 10.)]);
            }
            Self::Down => {
                line(&[(12., 3.), (12., 21.)]);
                line(&[(5., 14.), (12., 21.), (19., 14.)]);
            }
            Self::Back => {
                line(&[(15., 5.), (8., 12.), (15., 19.)]);
            }
            Self::More => {
                for x in [5., 12., 19.] {
                    circle(x, 12., 1.3);
                }
            }
            Self::Grip => {
                for x in [8., 16.] {
                    for y in [5., 12., 19.] {
                        painter.circle_filled(point(x, y), 1.5 * size / 24.0, color);
                    }
                }
            }
        }
    }
}

/// Uses native Button/AtomLayout so padding, wrapping, disabled/selected states,
/// keyboard activation and accessible labels exactly match ordinary controls.
pub struct IconButton {
    button: Button<'static>,
    icon: Option<Icon>,
    id: Id,
    selected: bool,
    accessible_label: Option<String>,
    role: ButtonRole,
    disclosure: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ButtonRole {
    Action,
    Tool,
    ToolChoice,
    View,
}

impl IconButton {
    pub fn new(label: impl Into<WidgetText>) -> Self {
        let label = label.into();
        let icon = Icon::for_label(label.text());
        Self::from_parts(label, icon)
    }
    pub fn with_icon(icon: Icon, label: impl Into<WidgetText>) -> Self {
        Self::from_parts(label.into(), Some(icon))
    }
    /// An exclusive interaction mode, with a persistent active indicator.
    pub fn tool(selected: bool, label: impl Into<WidgetText>) -> Self {
        let label = label.into();
        let accessible_label = label.text().to_owned();
        let mut button = Self::new(label)
            .selected(selected)
            .frame_when_inactive(selected);
        button.role = ButtonRole::Tool;
        button.accessible_label = Some(accessible_label);
        button
    }
    /// A tool chooser keeps the active-mode treatment and adds a visible menu arrow.
    pub fn tool_choice(selected: bool, label: impl Into<WidgetText>) -> Self {
        let label = label.into();
        let accessible_label = label.text().to_owned();
        let icon = Icon::for_label(label.text());
        let mut button = Self::from_parts_with_disclosure(label, icon, true)
            .selected(selected)
            .frame_when_inactive(true);
        button.role = ButtonRole::ToolChoice;
        button.accessible_label = Some(accessible_label);
        button
    }
    /// Presentation commands have an outline rather than an action fill.
    pub fn presentation(mut self) -> Self {
        self.role = ButtonRole::View;
        self.button = self.button.frame(true).frame_when_inactive(true);
        self
    }
    /// An icon-only control still exposes its action to assistive technology.
    pub fn icon_only(icon: Icon, accessible_label: impl Into<String>) -> Self {
        let mut button = Self::from_parts("".into(), Some(icon));
        button.accessible_label = Some(accessible_label.into());
        button
    }
    fn from_parts(label: WidgetText, icon: Option<Icon>) -> Self {
        Self::from_parts_with_disclosure(label, icon, false)
    }
    fn from_parts_with_disclosure(label: WidgetText, icon: Option<Icon>, disclosure: bool) -> Self {
        let id = Id::new("rill-button-icon");
        let mut atoms = egui::Atoms::new(label.clone());
        if icon.is_some() {
            let atom = Atom::custom(id, Vec2::splat(18.0));
            if label.text().is_empty() {
                atoms = egui::Atoms::new(atom);
            } else {
                atoms.push_left(atom);
            }
        }
        if disclosure {
            atoms.push_right(Atom::custom(
                Id::new("rill-button-disclosure"),
                Vec2::splat(12.0),
            ));
        }
        let button = Button::new(atoms);
        Self {
            button: button
                .min_size(egui::vec2(0.0, crate::tokens::CONTROL_HEIGHT))
                .corner_radius(crate::tokens::RADIUS),
            icon,
            id,
            selected: false,
            accessible_label: None,
            role: ButtonRole::Action,
            disclosure,
        }
    }
    pub fn fill(mut self, fill: Color32) -> Self {
        self.button = self.button.fill(fill);
        self
    }
    pub fn stroke(mut self, stroke: Stroke) -> Self {
        self.button = self.button.stroke(stroke);
        self
    }
    pub fn sense(mut self, sense: egui::Sense) -> Self {
        self.button = self.button.sense(sense);
        self
    }
    pub fn small(mut self) -> Self {
        self.button = self.button.small();
        self
    }
    pub fn min_size(mut self, size: Vec2) -> Self {
        self.button = self.button.min_size(size);
        self
    }
    pub fn frame(mut self, frame: bool) -> Self {
        self.button = self.button.frame(frame);
        self
    }
    pub fn frame_when_inactive(mut self, frame: bool) -> Self {
        self.button = self.button.frame_when_inactive(frame);
        self
    }
    pub fn truncate(mut self) -> Self {
        self.button = self.button.truncate();
        self
    }
    pub fn wrap_mode(mut self, wrap_mode: egui::TextWrapMode) -> Self {
        self.button = self.button.wrap_mode(wrap_mode);
        self
    }
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self.button = self.button.selected(selected);
        self
    }
}
impl Widget for IconButton {
    fn ui(self, ui: &mut Ui) -> Response {
        // Keep this native widget in its parent's layout: a nested `scope`
        // prevents horizontal_wrapped from wrapping individual buttons.
        let previous_style = ui.style().clone();
        ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::NONE;
        ui.visuals_mut().widgets.hovered.bg_stroke = Stroke::NONE;
        if self.role == ButtonRole::View {
            let colors = crate::theme::palette(ui.ctx());
            ui.visuals_mut().widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
            ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::new(1.0_f32, colors.line);
            ui.visuals_mut().widgets.hovered.bg_stroke = Stroke::new(1.0_f32, colors.accent);
        }
        let native_event_index = self
            .accessible_label
            .as_ref()
            .map(|_| ui.output(|output| output.events.len()));
        let output = self.button.atom_ui(ui);
        ui.set_style(previous_style);
        if let (Some(label), Some(index)) = (self.accessible_label, native_event_index) {
            // A custom atom has no text, so native Button emits an unlabelled
            // activation/focus event. Replace that event while retaining every
            // earlier event, then update the native accessibility node as well.
            ui.output_mut(|output| {
                if output.events.get(index).is_some_and(|event| {
                    let info = event.widget_info();
                    info.typ == egui::WidgetType::Button
                        && (matches!(self.role, ButtonRole::Tool | ButtonRole::ToolChoice)
                            || info.label.is_none())
                }) {
                    output.events.remove(index);
                }
            });
            output.response.widget_info(|| {
                if self.role == ButtonRole::Tool {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::RadioButton,
                        ui.is_enabled(),
                        self.selected,
                        &label,
                    )
                } else {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &label)
                }
            });
        }
        if matches!(self.role, ButtonRole::Tool | ButtonRole::ToolChoice) && self.selected {
            // A shape cue remains legible without relying on accent color alone.
            let rect = output.response.rect;
            let indicator = Rect::from_min_max(
                egui::pos2(rect.left() + crate::tokens::SPACE_SM, rect.bottom() - 3.0),
                egui::pos2(rect.right() - crate::tokens::SPACE_SM, rect.bottom()),
            );
            ui.painter().rect_filled(
                indicator,
                1.0,
                ui.style()
                    .interact_selectable(&output.response, true)
                    .fg_stroke
                    .color,
            );
        }
        if let (Some(icon), Some(rect)) = (self.icon, output.rect(self.id)) {
            let color = if self.selected {
                ui.visuals().selection.stroke.color
            } else {
                ui.style().interact(&output.response).text_color()
            };
            icon.paint(ui.painter(), rect, color);
        }
        if self.disclosure
            && let Some(rect) = output.rect(Id::new("rill-button-disclosure"))
        {
            Icon::ChevronDown.paint(
                ui.painter(),
                rect,
                ui.style().interact(&output.response).text_color(),
            );
        }
        output.response
    }
}
