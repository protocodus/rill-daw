//! Source-specific output and send commands shared by the mixer.
use crate::app::{DawApp, PopupKind};
use daw_core::TrackKind;
use eframe::egui;

impl DawApp {
    pub(crate) fn mixer_add_bus(&mut self, ctx: &egui::Context) {
        self.begin_edit();
        let name = self.available_bus_name();
        self.selected_track = Some(self.project.add_bus(name));
        ctx.data_mut(|data| {
            data.insert_temp(
                egui::Id::new("mixer_reveal_channel"),
                self.selected_track.unwrap(),
            )
        });
        self.selected_clip = None;
        self.selected_notes.clear();
        self.edit_label = Some("Add bus".into());
        self.changed();
    }

    pub(crate) fn mixer_add_send_menu(&mut self, ui: &mut egui::Ui) {
        let source = self.selected_track.filter(|id| {
            self.project.track(*id).is_some_and(|track| {
                matches!(
                    track.kind,
                    TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
                )
            })
        });
        let mut destination = None;
        ui.add_enabled_ui(source.is_some(), |ui| {
            daw_ui::action_menu(ui, "Add send…", |ui| {
                destination = source.and_then(|source| self.mixer_send_choices(ui, source));
            })
            .response
            .on_hover_text(source.and_then(|id| self.project.track(id)).map_or_else(
                || "Select a channel to add a send".to_owned(),
                |track| format!("Add a post-fader send from {}", track.name),
            ));
        });
        if let Some((source, destination)) = source.zip(destination) {
            self.mixer_create_send(source, destination);
        }
    }

    fn mixer_route_choices(
        &self,
        ui: &mut egui::Ui,
        source: u64,
        kind: daw_ui::MixerRouteKind,
        details: bool,
    ) -> Option<daw_ui::MixerRouteChoice> {
        let track = self.project.track(source)?;
        let buses = self.project.eligible_output_buses(source).ok()?;
        let destinations: Vec<_> = buses
            .iter()
            .filter(|bus| {
                kind == daw_ui::MixerRouteKind::Output
                    || !track.sends.iter().any(|send| send.target_bus == bus.id)
            })
            .map(|bus| daw_ui::MixerDestination {
                id: bus.id,
                name: &bus.name,
            })
            .collect();
        daw_ui::mixer_route_choices(
            ui,
            &track.name,
            kind,
            track.output_bus,
            &destinations,
            details,
        )
    }

    pub(crate) fn mixer_send_choices(&self, ui: &mut egui::Ui, source: u64) -> Option<Option<u64>> {
        match self.mixer_route_choices(ui, source, daw_ui::MixerRouteKind::Send, false)? {
            daw_ui::MixerRouteChoice::Bus(id) => Some(Some(id)),
            daw_ui::MixerRouteChoice::NewBus => Some(None),
            _ => None,
        }
    }

    /// Attach after releasing the mutable channel-strip borrow. The response
    /// captures its source; browsing does not retarget the selected channel.
    pub(crate) fn mixer_output_menu(
        &mut self,
        ui: &mut egui::Ui,
        source: u64,
        response: &egui::Response,
    ) {
        let choice = egui::Popup::menu(response)
            .show(|ui| self.mixer_route_choices(ui, source, daw_ui::MixerRouteKind::Output, true))
            .and_then(|popup| popup.inner);
        match choice {
            Some(daw_ui::MixerRouteChoice::Master) => self.mixer_set_output(source, None),
            Some(daw_ui::MixerRouteChoice::Bus(bus)) => self.mixer_set_output(source, Some(bus)),
            Some(daw_ui::MixerRouteChoice::NewBus) => self.mixer_create_routed_bus(&[source]),
            Some(daw_ui::MixerRouteChoice::Details) => self.mixer_open_routing(source),
            None => {}
        }
        let _ = ui;
    }

    pub(crate) fn mixer_send_menu(
        &mut self,
        ui: &mut egui::Ui,
        source: u64,
        response: &egui::Response,
    ) {
        let has_sends = self
            .project
            .track(source)
            .is_some_and(|track| !track.sends.is_empty());
        let choice = egui::Popup::menu(response)
            .show(|ui| {
                self.mixer_route_choices(ui, source, daw_ui::MixerRouteKind::Send, has_sends)
            })
            .and_then(|popup| popup.inner);
        match choice {
            Some(daw_ui::MixerRouteChoice::Bus(bus)) => self.mixer_create_send(source, Some(bus)),
            Some(daw_ui::MixerRouteChoice::NewBus) => self.mixer_create_send(source, None),
            Some(daw_ui::MixerRouteChoice::Details) => self.mixer_open_routing(source),
            _ => {}
        }
        let _ = ui;
    }

    fn mixer_open_routing(&mut self, source: u64) {
        self.selected_track = Some(source);
        if self.selected().is_none() {
            self.selected_clip = None;
        }
        self.open_popup(PopupKind::Routing);
    }

    fn mixer_set_output(&mut self, source: u64, destination: Option<u64>) {
        if self
            .project
            .track(source)
            .is_some_and(|track| track.output_bus == destination)
        {
            return;
        }
        let mut candidate = self.project.clone();
        let result = candidate.set_output(source, destination);
        if result.is_ok() {
            self.begin_edit();
            self.project = candidate;
            self.edit_label = Some("Route output".into());
            self.changed();
        }
        self.message(result, "Output routing updated");
    }

    /// Insert a new group before the common downstream destination, retaining
    /// its effects. A multi-channel group with different outputs goes to Master.
    /// Construct and validate the complete graph before creating one undo edit.
    pub(crate) fn mixer_create_routed_bus(&mut self, sources: &[u64]) {
        let result = (|| -> anyhow::Result<daw_core::Project> {
            anyhow::ensure!(!sources.is_empty(), "Select a channel to route");
            let mut downstream = None;
            let mut common = true;
            for (index, source) in sources.iter().enumerate() {
                let track = self
                    .project
                    .track(*source)
                    .ok_or_else(|| anyhow::anyhow!("Source channel not found"))?;
                anyhow::ensure!(
                    matches!(
                        track.kind,
                        TrackKind::Audio | TrackKind::Midi | TrackKind::Bus
                    ),
                    "Only audio, MIDI and bus channels can be routed"
                );
                if index == 0 {
                    downstream = track.output_bus;
                } else {
                    common &= downstream == track.output_bus;
                }
            }
            let mut candidate = self.project.clone();
            let bus = candidate.add_bus(self.available_bus_name());
            candidate.set_output(bus, if common { downstream } else { None })?;
            for source in sources {
                candidate.set_output(*source, Some(bus))?;
            }
            candidate.validate()?;
            Ok(candidate)
        })();
        match result {
            Ok(candidate) => {
                self.begin_edit();
                self.project = candidate;
                self.edit_label = Some("Create group bus".into());
                self.changed();
                self.message(Ok(()), "Bus created and output routed");
            }
            Err(error) => self.message(Err(error), ""),
        }
    }

    pub(crate) fn mixer_create_send(&mut self, source: u64, destination: Option<u64>) {
        let mut candidate = self.project.clone();
        let bus = destination.unwrap_or_else(|| candidate.add_bus(self.available_bus_name()));
        let result = candidate.add_send(source, bus, 0.25, false);
        if result.is_ok() {
            self.begin_edit();
            self.project = candidate;
            self.edit_label = Some(
                if destination.is_none() {
                    "Add bus send"
                } else {
                    "Add send"
                }
                .into(),
            );
            self.changed();
        }
        self.message(result, "Send added");
    }

    pub(crate) fn available_bus_name(&self) -> String {
        for index in 1.. {
            let name = format!("Bus {index}");
            if !self.project.tracks.iter().any(|track| track.name == name) {
                return name;
            }
        }
        unreachable!()
    }
}
