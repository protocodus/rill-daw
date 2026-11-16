//! System CPU meter and its scrollable logical-core detail.
use crate::{Icon, IconButton, theme, tokens};
use egui::{Align, Layout, Popup, Response, Ui};

#[derive(Debug, Default)]
pub enum CpuLoad {
    #[default]
    Measuring,
    Unavailable,
    Sample {
        total: u8,
        cores: Vec<u8>,
    },
}

/// A complete sampling interval near saturation gets a warning on both the
/// affected meter and the always-visible summary.
pub const CPU_OVERLOAD_PERCENT: u8 = 95;

pub fn cpu_load_indicator(ui: &mut Ui, load: &CpuLoad) -> Response {
    let colors = theme::palette(ui.ctx());
    let button = match load {
        CpuLoad::Sample { total, cores }
            if *total >= CPU_OVERLOAD_PERCENT
                || cores.iter().any(|p| *p >= CPU_OVERLOAD_PERCENT) =>
        {
            IconButton::with_icon(Icon::Warning, format!("CPU {total}% · Overload"))
                .label_color(colors.warning)
        }
        CpuLoad::Sample { total, .. } => IconButton::new(format!("CPU {total}%")),
        CpuLoad::Measuring | CpuLoad::Unavailable => IconButton::new("CPU —"),
    };
    let response = ui
        .add(button.quiet())
        .on_hover_text("System CPU usage · Show logical cores");
    let popup = Popup::from_toggle_button_response(&response)
        .align(egui::RectAlign::BOTTOM_END)
        .layout(Layout::top_down(Align::Min))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    crate::show_menu(popup, |ui| {
        crate::text::section_heading(ui, "CPU load");
        match load {
            CpuLoad::Measuring => {
                ui.label("Measuring CPU usage…");
            }
            CpuLoad::Unavailable => {
                ui.label("CPU usage unavailable");
            }
            CpuLoad::Sample { total, cores } => {
                load_row(ui, "Total", *total);
                ui.separator();
                for (index, percent) in cores.iter().enumerate() {
                    load_row(ui, &format!("Core {}", index + 1), *percent);
                }
            }
        }
    });
    response
}

fn load_row(ui: &mut Ui, label: &str, percent: u8) {
    let colors = theme::palette(ui.ctx());
    let overload = percent >= CPU_OVERLOAD_PERCENT;
    ui.horizontal(|ui| {
        ui.add_sized(
            [tokens::SPACE_XL * 2.0, tokens::COMPACT_HEIGHT],
            egui::Label::new(label),
        );
        let color = if overload {
            colors.warning
        } else {
            colors.accent
        };
        ui.add(
            egui::ProgressBar::new(f32::from(percent) / 100.0)
                .desired_width(tokens::SELECT_WIDTH)
                .desired_height(tokens::SPACE_SM)
                .fill(color),
        );
        if overload {
            ui.colored_label(color, format!("{percent}% · Overload"));
        } else {
            ui.label(format!("{percent}%"));
        }
    });
}
