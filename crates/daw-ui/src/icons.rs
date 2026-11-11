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
    Disk,
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
    Help,
    FitSelection,
    Crossfade,
    Input,
    Arranger,
    Grid,
    Nudge,
    Sparkle,
    Import,
    Info,
    Reset,
    Marker,
    Freeze,
    Note,
    Wand,
    Clip,
    Filter,
    Power,
}

impl Icon {
    pub fn for_label(label: &str) -> Option<Self> {
        Self::for_exact_label(label).or_else(|| Self::for_label_pattern(label))
    }

    /// Labels built from a verb and a subject ("Add bus", "Cancel export",
    /// "Snap 1/16") share the verb's icon, so new commands get one without a
    /// table entry. Exact labels above win.
    fn for_label_pattern(label: &str) -> Option<Self> {
        let label = label.trim_end_matches('…').trim_end_matches("...");
        // Menu-bar titles and on-screen keys stay text; "+ Insert" already
        // carries its own plus.
        if matches!(
            label,
            "File" | "Edit" | "Track" | "Window" | "Help" | "Space" | "Backspace" | "123"
        ) || label.starts_with("+ ")
        {
            return None;
        }
        let first = label.split(' ').next().unwrap_or(label);
        Some(match first {
            "AI" | "Assistant" => Self::Sparkle,
            "Add" | "New" | "Insert" | "Create" => {
                if label.contains("marker") {
                    Self::Marker
                } else {
                    Self::Add
                }
            }
            "Delete" | "Remove" => Self::Delete,
            "Cancel" | "Close" | "Dismiss" | "Don't" => Self::Close,
            "Apply" | "Done" | "Continue" | "Use" | "Recommended" | "Keep" => Self::Check,
            "Split" | "Slice" | "Cut" | "Trim" | "Strip" => Self::Cut,
            "Copy" => Self::Copy,
            "Paste" => Self::Paste,
            "Clear" if label.contains("filter") => Self::Filter,
            "Clear" | "Silence" => Self::Erase,
            "Reset" | "Center" | "Return" => Self::Reset,
            "Retry" | "Refresh" | "Rescan" | "Read" => Self::Refresh,
            "Select" => Self::Select,
            "Export" => Self::Export,
            "Import" | "Recover" => Self::Import,
            "Fit" => Self::Fit,
            "Save" => Self::Save,
            "Rename" | "Type" => Self::Pencil,
            "Loop" | "Repeat" => Self::Loop,
            "Freeze" => Self::Freeze,
            "Choose" | "Change" | "Keyboard" | "Try" => Self::Instrument,
            "Snap" | "Quantize" | "Grid" => Self::Grid,
            "Nudge" => Self::Nudge,
            "Arranger" => Self::Arranger,
            "Show" | "Open" => {
                if label.contains("Arranger") {
                    Self::Arranger
                } else if label.contains("Mixer") || label.contains("channel") {
                    Self::Mixer
                } else {
                    Self::Editor
                }
            }
            "Details" | "Device" | "About" | "Inspect" => Self::Info,
            "Search" => Self::Search,
            "Markers" => Self::Marker,
            "Takes" | "Layer" | "Group" => Self::Stack,
            "Audio" if label.contains("takes") => Self::Stack,
            "Audio" if label.contains("setup") => Self::Settings,
            "Transpose" | "Pitch" | "Note" | "Length" | "Key" => Self::Note,
            "Process" | "Humanize" | "Smooth" | "Boundary" | "Expression" => Self::Wand,
            "Clip" => Self::Clip,
            "Plug-in" => Self::Folder,
            "Previous" => Self::Back,
            "Next" => Self::ChevronRight,
            "Up" => Self::Up,
            "Down" => Self::Down,
            "Quit" => Self::Power,
            "Route" | "Master" | "Expand" | "Enable" => Self::Routing,
            _ => return None,
        })
    }

    fn for_exact_label(label: &str) -> Option<Self> {
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
            "Fit selection" => Self::FitSelection,
            "Tool help" => Self::Help,
            "Start" | "Clip start" | "Set start" => Self::Start,
            "Clip end" | "Set end" | "Snap end" => Self::End,
            "Play" => Self::Play,
            "Pause" => Self::Pause,
            "Stop" => Self::Stop,
            "Record" | "Arm recording" | "Recording" => Self::Record,
            "Input" | "Inputs…" => Self::Input,
            "Stop recording" => Self::Stop,
            "Click" | "Metronome" => Self::Metronome,
            "Loop" | "Range" => Self::Loop,
            "Add notes" => Self::Add,
            "New variation" => Self::Refresh,
            "Crossfade…" | "Crossfade" => Self::Crossfade,
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
            "Device browser" | "Browse effects" => Self::Library,
            "Instrument" | "Notes" | "Add instrument…" | "Create instrument track" => {
                Self::Instrument
            }
            "Effects" | "Add to effect chain" => Self::Effect,
            "Mixer" => Self::Mixer,
            "Channel" => Self::Settings,
            "Routing" | "Create send effect" => Self::Routing,
            "Edit automation" | "Add automation track" | "Add automation track…" | "Automate" => {
                Self::Automation
            }
            "Save" | "Save as…" => Self::Save,
            "Disk-streamed audio" => Self::Disk,
            "Open…" => Self::Open,
            "Open editor" | "Show editor" => Self::Editor,
            "Export WAV…" | "Export MIDI…" => Self::Export,
            "Search" => Self::Search,
            "Scan" | "Rescan" | "Refresh" | "Refresh preview" | "Refresh parameters" => {
                Self::Refresh
            }
            "Reverse" => Self::Reverse,
            "Normalize" => Self::Normalize,
            "Analyze" | "Transients" | "Warp" => Self::Waveform,
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
            Self::Settings => {
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
                // Alternating slopes keep the envelope recognizable even at
                // compact control size, rather than resembling a flat rail.
                line(&[(2., 16.), (7., 7.), (12., 17.), (17., 6.), (22., 13.)]);
            }
            Self::Disk => {
                // A drive enclosure and its status light, distinct from the
                // floppy-disk Save action.
                line(&[
                    (5., 3.),
                    (19., 3.),
                    (22., 15.),
                    (22., 21.),
                    (2., 21.),
                    (2., 15.),
                    (5., 3.),
                ]);
                line(&[(2., 15.), (22., 15.)]);
                circle(18., 18., 1.);
                line(&[(6., 18.), (11., 18.)]);
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
            Self::Help => {
                circle(12., 12., 9.);
                line(&[
                    (9., 9.5),
                    (10., 7.5),
                    (12., 7.),
                    (14., 7.5),
                    (15., 9.5),
                    (14., 11.5),
                    (12., 12.5),
                    (12., 14.5),
                ]);
                painter.circle_filled(point(12., 17.5), size / 24.0, color);
            }
            Self::FitSelection => {
                line(&[(3., 8.), (3., 3.), (8., 3.)]);
                line(&[(16., 3.), (21., 3.), (21., 8.)]);
                line(&[(21., 16.), (21., 21.), (16., 21.)]);
                line(&[(8., 21.), (3., 21.), (3., 16.)]);
                box_at(8., 8., 8., 8.);
            }
            Self::Crossfade => {
                line(&[(3., 5.), (8., 6.), (13., 10.), (17., 16.), (21., 19.)]);
                line(&[(3., 19.), (7., 16.), (11., 10.), (16., 6.), (21., 5.)]);
            }
            Self::Input => {
                line(&[(3., 12.), (14., 12.)]);
                line(&[(10., 8.), (14., 12.), (10., 16.)]);
                line(&[(17., 4.), (21., 4.), (21., 20.), (17., 20.)]);
            }
            // The two home views: five track rows, five channel strips.
            Self::Arranger => {
                for y in [4., 8., 12., 16., 20.] {
                    line(&[(3., y), (21., y)]);
                }
            }
            Self::Mixer => {
                for x in [4., 8., 12., 16., 20.] {
                    line(&[(x, 3.), (x, 21.)]);
                }
            }
            Self::Grid => {
                box_at(3., 3., 18., 18.);
                line(&[(3., 12.), (21., 12.)]);
                line(&[(12., 3.), (12., 21.)]);
            }
            Self::Nudge => {
                line(&[(3., 12.), (21., 12.)]);
                line(&[(7., 8.), (3., 12.), (7., 16.)]);
                line(&[(17., 8.), (21., 12.), (17., 16.)]);
            }
            Self::Sparkle => line(&[
                (12., 2.),
                (14., 10.),
                (22., 12.),
                (14., 14.),
                (12., 22.),
                (10., 14.),
                (2., 12.),
                (10., 10.),
                (12., 2.),
            ]),
            Self::Import => {
                line(&[(12., 3.), (12., 15.)]);
                line(&[(7., 10.), (12., 15.), (17., 10.)]);
                line(&[(4., 15.), (4., 21.), (20., 21.), (20., 15.)]);
            }
            Self::Info => {
                circle(12., 12., 9.);
                line(&[(12., 11.), (12., 17.)]);
                painter.circle_filled(point(12., 7.5), size / 24.0, color);
            }
            Self::Reset => {
                // Three quarters of a circle, opening at the top left, with
                // the arrowhead pointing back along it.
                let arc: Vec<_> = (0..=12)
                    .map(|step| {
                        let angle = std::f32::consts::PI * (-0.75 + 1.5 * step as f32 / 12.0);
                        point(12. + 8. * angle.sin(), 13. - 8. * angle.cos())
                    })
                    .collect();
                painter.add(Shape::line(arc, stroke));
                line(&[(3., 3.), (6.3, 7.3), (10.5, 5.)]);
            }
            Self::Marker => {
                line(&[(6., 3.), (6., 21.)]);
                line(&[(6., 4.), (18., 7.5), (6., 11.)]);
            }
            Self::Freeze => {
                line(&[(12., 3.), (12., 21.)]);
                line(&[(4.2, 7.5), (19.8, 16.5)]);
                line(&[(4.2, 16.5), (19.8, 7.5)]);
            }
            Self::Note => {
                circle(8., 17., 3.);
                line(&[(11., 17.), (11., 4.), (18., 6.)]);
            }
            Self::Wand => {
                line(&[(4., 20.), (13., 11.)]);
                line(&[(17., 3.), (17., 11.)]);
                line(&[(13., 7.), (21., 7.)]);
            }
            Self::Clip => {
                box_at(3., 6., 18., 12.);
                line(&[(3., 10.), (21., 10.)]);
            }
            Self::Filter => line(&[
                (3., 5.),
                (21., 5.),
                (14., 13.),
                (14., 20.),
                (10., 18.),
                (10., 13.),
                (3., 5.),
            ]),
            Self::Power => {
                let arc: Vec<_> = (0..=12)
                    .map(|step| {
                        let angle = std::f32::consts::PI * (0.2 + 1.6 * step as f32 / 12.0);
                        point(12. + 8. * angle.sin(), 13. - 8. * angle.cos())
                    })
                    .collect();
                painter.add(Shape::line(arc, stroke));
                line(&[(12., 3.), (12., 12.)]);
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
/// The native button is assembled when shown, so a menu row can drop its icon
/// while a toolbar keeps it, without changing the caller's builder calls.
pub struct IconButton {
    label: WidgetText,
    icon: Option<Icon>,
    selected: bool,
    accessible_label: Option<String>,
    role: ButtonRole,
    disclosure: bool,
    frame: Option<bool>,
    frame_when_inactive: bool,
    fill: Option<Color32>,
    stroke: Option<Stroke>,
    label_color: Option<Color32>,
    sense: Option<egui::Sense>,
    small: bool,
    min_size: Vec2,
    truncate: bool,
    wrap_mode: Option<egui::TextWrapMode>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ButtonRole {
    Action,
    Ai,
    Automation,
    Tool,
    ToolChoice,
    View,
    Tab,
}

#[derive(Clone, Default)]
struct IconOnlyLabels {
    pass: u64,
    labels: Vec<(String, Rect)>,
}

const ICON_ONLY_LABELS: &str = "rill_icon_only_labels";

fn register_icon_only_label(ctx: &egui::Context, label: &str, rect: Rect) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| {
        let entry = data.get_temp_mut_or_default::<IconOnlyLabels>(Id::new(ICON_ONLY_LABELS));
        if entry.pass != pass {
            entry.pass = pass;
            entry.labels.clear();
        }
        entry.labels.push((label.to_owned(), rect));
    });
}

/// Accessible names and bounds of the icon-only controls shown by the latest
/// pass that painted any. Pointer automation resolves these exactly as it
/// resolves painted text. Call `reset_icon_only_labels` before running a
/// frame so a control that disappeared is not reported from an earlier frame.
pub fn icon_only_labels(ctx: &egui::Context) -> Vec<(String, Rect)> {
    ctx.data(|data| {
        data.get_temp::<IconOnlyLabels>(Id::new(ICON_ONLY_LABELS))
            .map(|entry| entry.labels)
            .unwrap_or_default()
    })
}

pub fn reset_icon_only_labels(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<IconOnlyLabels>(Id::new(ICON_ONLY_LABELS)));
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
    /// A named family keeps its label and width while its icon identifies the
    /// active tool. Assistive technology also receives the full active name.
    pub fn tool_group_choice(label: &str, icon: Icon, active_tool: Option<&str>) -> Self {
        let mut button = Self::from_parts_with_disclosure(label.into(), Some(icon), true)
            .selected(active_tool.is_some())
            .quiet();
        button.role = ButtonRole::ToolChoice;
        button.accessible_label = Some(match active_tool {
            Some(tool) => format!("{label} · {tool}"),
            None => label.to_owned(),
        });
        button
    }
    /// Quiet exclusive page navigation: full-size targets and a persistent
    /// glowing border, with a surface only on hover or keyboard focus.
    pub fn tab(selected: bool, label: impl Into<WidgetText>) -> Self {
        let label = label.into();
        let accessible_label = label.text().to_owned();
        let mut button = Self::from_parts(label, None)
            .selected(selected)
            .frame_when_inactive(false);
        button.role = ButtonRole::Tab;
        button.accessible_label = Some(accessible_label);
        button
    }
    /// Presentation commands have an outline rather than an action fill.
    pub fn presentation(mut self) -> Self {
        self.role = ButtonRole::View;
        self.frame(true).frame_when_inactive(true)
    }
    /// A content command can share its icon with a presentation control.
    pub fn action(mut self) -> Self {
        self.role = ButtonRole::Action;
        self
    }
    pub fn ai(mut self) -> Self {
        self.role = ButtonRole::Ai;
        self
    }
    pub fn automation(mut self) -> Self {
        self.role = ButtonRole::Automation;
        self
    }
    /// Secondary commands share the toolbar surface until hovered or focused.
    /// Apply after `selected` so active choices retain their state fill.
    pub fn quiet(mut self) -> Self {
        self.frame_when_inactive = self.selected;
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
        Self {
            label,
            icon,
            selected: false,
            accessible_label: None,
            role: if icon == Some(Icon::Automation) {
                ButtonRole::Automation
            } else if matches!(
                icon,
                Some(
                    Icon::Back
                        | Icon::Arranger
                        | Icon::Info
                        | Icon::View
                        | Icon::Mixer
                        | Icon::Routing
                        | Icon::Editor
                        | Icon::Library
                        | Icon::History
                        | Icon::Settings
                )
            ) {
                ButtonRole::View
            } else {
                ButtonRole::Action
            },
            disclosure,
            frame: None,
            frame_when_inactive: true,
            fill: None,
            stroke: None,
            label_color: None,
            sense: None,
            small: false,
            min_size: egui::vec2(0.0, crate::tokens::CONTROL_HEIGHT),
            truncate: false,
            wrap_mode: None,
        }
    }
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
    pub fn fill(mut self, fill: Color32) -> Self {
        self.fill = Some(fill);
        self
    }
    /// Text and icon color over an explicit fill, such as a solid primary action.
    pub fn label_color(mut self, color: Color32) -> Self {
        self.label_color = Some(color);
        self
    }
    pub fn stroke(mut self, stroke: Stroke) -> Self {
        self.stroke = Some(stroke);
        self.frame = Some(true);
        self
    }
    pub fn sense(mut self, sense: egui::Sense) -> Self {
        self.sense = Some(sense);
        self
    }
    pub fn small(mut self) -> Self {
        self.small = true;
        self
    }
    pub fn min_size(mut self, size: Vec2) -> Self {
        self.min_size = size;
        self
    }
    pub fn frame(mut self, frame: bool) -> Self {
        self.frame = Some(frame);
        self
    }
    pub fn frame_when_inactive(mut self, frame: bool) -> Self {
        self.frame_when_inactive = frame;
        self
    }
    pub fn truncate(mut self) -> Self {
        self.truncate = true;
        self
    }
    pub fn wrap_mode(mut self, wrap_mode: egui::TextWrapMode) -> Self {
        self.wrap_mode = Some(wrap_mode);
        self
    }
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
}
impl Widget for IconButton {
    fn ui(self, ui: &mut Ui) -> Response {
        // Keep this native widget in its parent's layout: a nested `scope`
        // prevents horizontal_wrapped from wrapping individual buttons.
        let previous_style = ui.style().clone();
        let in_menu = egui::containers::menu::is_in_menu(ui);
        if in_menu {
            // Native menus install compact defaults after the shared theme.
            // Keep ordinary label padding on shared controls in those menus.
            ui.spacing_mut().button_padding = egui::vec2(
                crate::tokens::BUTTON_PADDING_X,
                crate::tokens::BUTTON_PADDING_Y,
            );
        }
        // Menu rows are text only, so every label in a menu starts at the same
        // x. Icon-only controls keep their glyph; it is their whole identity.
        let icon_only = self.label.text().is_empty();
        let icon = self.icon.filter(|_| !in_menu || icon_only);
        let icon_id = Id::new("rill-button-icon");
        let disclosure_id = Id::new("rill-button-disclosure");
        let mut atoms = egui::Atoms::new(self.label.clone());
        if icon.is_some() {
            let atom = Atom::custom(icon_id, Vec2::splat(crate::tokens::ICON_SIZE));
            if icon_only {
                atoms = egui::Atoms::new(atom);
            } else {
                atoms.push_left(atom);
            }
        }
        if self.disclosure {
            atoms.push_right(Atom::custom(disclosure_id, Vec2::splat(12.0)));
        }
        let mut button = Button::new(atoms)
            .min_size(self.min_size)
            .corner_radius(crate::tokens::RADIUS)
            .selected(self.selected);
        if self.small {
            button = button.small();
        }
        if let Some(sense) = self.sense {
            button = button.sense(sense);
        }
        if self.truncate {
            button = button.truncate();
        }
        if let Some(wrap_mode) = self.wrap_mode {
            button = button.wrap_mode(wrap_mode);
        }
        let colors = crate::theme::palette(ui.ctx());
        let inherited = ui.visuals().override_text_color.filter(|color| {
            *color == colors.automation
                || *color == colors.automation_override
                || *color == colors.ai
        });
        // Modes and views stay neutral; the action tint marks commands that
        // change the document, so a toolbar reads as mostly quiet text.
        let purpose = match self.role {
            ButtonRole::View | ButtonRole::Tab | ButtonRole::Tool | ButtonRole::ToolChoice => {
                colors.text
            }
            ButtonRole::Ai => colors.ai,
            ButtonRole::Automation => colors.automation,
            _ if in_menu => inherited.unwrap_or(colors.text),
            _ => inherited.unwrap_or(colors.action),
        };
        let colored = purpose == colors.action
            || purpose == colors.ai
            || purpose == colors.automation
            || purpose == colors.automation_override;
        let foreground = self.label_color.unwrap_or(purpose);
        ui.visuals_mut().override_text_color = Some(foreground);
        let visuals = &mut ui.visuals_mut().widgets;
        for visuals in [
            &mut visuals.inactive,
            &mut visuals.hovered,
            &mut visuals.active,
            &mut visuals.open,
        ] {
            visuals.fg_stroke.color = foreground;
        }
        if colored {
            let visuals = &mut ui.visuals_mut().widgets;
            visuals.inactive.weak_bg_fill = colors.panel.lerp_to_gamma(purpose, 0.12);
            visuals.hovered.weak_bg_fill = colors.panel.lerp_to_gamma(purpose, 0.20);
            visuals.active.weak_bg_fill = colors.panel.lerp_to_gamma(purpose, 0.28);
            visuals.open.weak_bg_fill = colors.panel.lerp_to_gamma(purpose, 0.20);
            visuals.active.bg_stroke.color = purpose;
            visuals.hovered.bg_stroke.color = purpose;
        }
        let selected_mode = self.selected
            && matches!(
                self.role,
                ButtonRole::Tool | ButtonRole::ToolChoice | ButtonRole::Tab
            );
        if in_menu {
            let visuals = &mut ui.visuals_mut().widgets;
            visuals.inactive.weak_bg_fill = Color32::TRANSPARENT;
            visuals.inactive.bg_stroke = Stroke::NONE;
            visuals.hovered.bg_stroke = Stroke::NONE;
            visuals.active.bg_stroke = Stroke::NONE;
            visuals.open.bg_stroke = Stroke::NONE;
            ui.visuals_mut().selection.stroke = Stroke::NONE;
        } else {
            ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::new(
                crate::tokens::BORDER_WIDTH,
                colors
                    .line
                    .gamma_multiply(crate::tokens::GOLDEN_RATIO_INVERSE),
            );
            if self.role == ButtonRole::View {
                ui.visuals_mut().widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::new(1.0_f32, colors.line);
                ui.visuals_mut().widgets.hovered.bg_stroke = Stroke::new(1.0_f32, colors.accent);
            }
        }
        let native_event_index = self
            .accessible_label
            .as_ref()
            .map(|_| ui.output(|output| output.events.len()));
        let background = ui.painter().add(egui::Shape::Noop);
        let has_frame = self.frame.unwrap_or(ui.visuals().button_frame);
        let frame_when_inactive = if in_menu {
            // Ordinary menu rows stay quiet, but a primary action's explicit
            // idle fill must remain behind its contrasting label color.
            self.selected || (self.fill.is_some() && self.frame_when_inactive)
        } else {
            self.frame_when_inactive
        };
        // Keep native frame padding and hit geometry, but paint its material in
        // the reserved slot beneath the native text, icons and accessibility.
        let output = button
            .frame_when_inactive(true)
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE)
            .frame(has_frame)
            .atom_ui(ui);
        let response = &output.response;
        if has_frame
            && (frame_when_inactive
                || response.hovered()
                || response.is_pointer_button_down_on()
                || response.has_focus())
        {
            let visuals = ui.style().interact_selectable(response, self.selected);
            let fill = self.fill.unwrap_or(visuals.weak_bg_fill);
            let stroke = if in_menu {
                Stroke::NONE
            } else {
                self.stroke.unwrap_or(visuals.bg_stroke)
            };
            let material = crate::surface::ComponentMaterial {
                elevated: !in_menu
                    && self.fill.is_some_and(|fill| fill != Color32::TRANSPARENT)
                    && ui.is_enabled(),
                active: !in_menu
                    && ui.is_enabled()
                    && !selected_mode
                    && (self.selected || response.has_focus()),
            };
            ui.painter().set(
                background,
                crate::surface::component_shape(
                    ui.painter(),
                    response.rect,
                    crate::tokens::RADIUS,
                    fill,
                    stroke,
                    material,
                ),
            );
        }
        ui.set_style(previous_style);
        if let (Some(label), Some(index)) = (self.accessible_label.as_deref(), native_event_index) {
            // A custom atom has no text, so native Button emits an unlabelled
            // activation/focus event. Replace that event while retaining every
            // earlier event, then update the native accessibility node as well.
            ui.output_mut(|output| {
                if output.events.get(index).is_some_and(|event| {
                    let info = event.widget_info();
                    info.typ == egui::WidgetType::Button
                        && (matches!(
                            self.role,
                            ButtonRole::Tool | ButtonRole::ToolChoice | ButtonRole::Tab
                        ) || info.label.is_none())
                }) {
                    output.events.remove(index);
                }
            });
            output.response.widget_info(|| {
                if matches!(self.role, ButtonRole::Tool | ButtonRole::Tab) {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::RadioButton,
                        ui.is_enabled(),
                        self.selected,
                        label,
                    )
                } else {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
                }
            });
            if icon_only {
                register_icon_only_label(ui.ctx(), label, output.response.rect);
            }
        }
        if selected_mode && !in_menu {
            // Selection outlines the complete native target. The halo shares
            // the clip/channel treatment and never changes layout or hit areas.
            let rect = output.response.rect;
            if ui.is_enabled() {
                crate::paint_selection_glow(
                    ui.painter(),
                    rect,
                    crate::tokens::RADIUS,
                    if colored { purpose } else { colors.accent },
                );
            }
            ui.painter().rect_stroke(
                rect,
                crate::tokens::RADIUS,
                Stroke::new(
                    crate::tokens::FOCUS_WIDTH,
                    ui.style()
                        .interact_selectable(&output.response, true)
                        .fg_stroke
                        .color,
                ),
                egui::StrokeKind::Inside,
            );
        }
        if let (Some(icon), Some(rect)) = (icon, output.rect(icon_id)) {
            let color = if output.response.enabled() {
                foreground
            } else {
                colors.muted
            };
            icon.paint(ui.painter(), rect, color);
        }
        if self.disclosure
            && let Some(rect) = output.rect(disclosure_id)
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

#[cfg(test)]
mod tests {
    use super::Icon;

    #[test]
    fn labels_resolve_to_consistent_icons() {
        for (label, icon) in [
            ("Arranger", Icon::Arranger),
            ("Show in Arranger", Icon::Arranger),
            ("Mixer", Icon::Mixer),
            ("Show in Mixer", Icon::Mixer),
            ("Add bus", Icon::Add),
            ("Delete track", Icon::Delete),
            ("Export…", Icon::Export),
            ("Cancel", Icon::Close),
        ] {
            assert_eq!(Icon::for_label(label), Some(icon), "{label}");
        }
        // Menu-bar titles and digits stay text.
        for label in ["File", "Edit", "123"] {
            assert_eq!(Icon::for_label(label), None, "{label}");
        }
    }
}
