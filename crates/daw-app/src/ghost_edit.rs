//! A link-breaking edit remains pending until the musician chooses its scope.
use crate::app::DawApp;
use anyhow::{Result, ensure};
use daw_core::SongSession;
use eframe::egui;
use std::collections::HashSet;

type Placement = (u64, u64);
type Apply = Box<dyn FnOnce(&mut DawApp, GhostChoice, &[Placement]) -> Result<()>>;

pub(crate) use daw_ui::GhostChoice;

pub(crate) struct GhostChange {
    session: SongSession,
    peers: Vec<Placement>,
    label: String,
    apply: Apply,
}

impl DawApp {
    pub(crate) fn ghost_peers(&self, targets: &[Placement]) -> Vec<Placement> {
        let targets: HashSet<_> = targets.iter().copied().collect();
        let mut media = HashSet::new();
        let mut automation = HashSet::new();
        for &(track, clip) in &targets {
            if let Some(group) = self.project.clip(track, clip).and_then(|c| c.link_group) {
                media.insert(group);
            } else if let Some(group) = self
                .project
                .automation_clip(track, clip)
                .and_then(|c| c.link_group)
            {
                automation.insert((track, group));
            }
        }
        if media.is_empty() && automation.is_empty() {
            return Vec::new();
        }
        let mut peers = Vec::new();
        for track in &self.project.tracks {
            peers.extend(
                track
                    .clips
                    .iter()
                    .filter(|c| c.link_group.is_some_and(|g| media.contains(&g)))
                    .map(|c| (track.id, c.id))
                    .filter(|pair| !targets.contains(pair)),
            );
            if let Some(lane) = &track.automation {
                peers.extend(
                    lane.clips
                        .iter()
                        .filter(|c| {
                            c.link_group
                                .is_some_and(|g| automation.contains(&(track.id, g)))
                        })
                        .map(|c| (track.id, c.id))
                        .filter(|pair| !targets.contains(pair)),
                );
            }
        }
        peers
    }

    /// The callback implements propagation and the original edit. The common
    /// transaction owns deletion, validation and one Undo entry.
    pub(crate) fn queue_ghost_change(
        &mut self,
        targets: &[Placement],
        label: &str,
        apply: impl FnOnce(&mut DawApp, GhostChoice, &[Placement]) -> Result<()> + 'static,
    ) -> bool {
        let peers = self.ghost_peers(targets);
        if peers.is_empty() {
            return false;
        }
        self.flush_pending_edit();
        self.ghost_change = Some(GhostChange {
            session: self.song_session,
            peers,
            label: label.to_owned(),
            apply: Box::new(apply),
        });
        true
    }

    pub(crate) fn confirm_unlink(&mut self, track: u64, clip: u64) -> bool {
        self.queue_ghost_change(
            &[(track, clip)],
            "Make independent",
            move |app, choice, _| {
                if choice != GhostChoice::Propagate {
                    if app.project.automation_clip(track, clip).is_some() {
                        app.project.make_automation_clip_independent(track, clip)?;
                    } else {
                        app.project.make_clip_independent(track, clip)?;
                    }
                }
                Ok(())
            },
        )
    }

    fn apply_ghost_choice(&mut self, change: GhostChange, choice: GhostChoice) -> Result<()> {
        ensure!(
            change.session == self.song_session,
            "The project changed. Choose the edit again."
        );
        self.begin_edit();
        let result: Result<()> = (|| {
            if choice == GhostChoice::DeleteCopies {
                for &(track, clip) in &change.peers {
                    if self.project.automation_clip(track, clip).is_some() {
                        self.project.remove_automation_clip(track, clip)?;
                    } else {
                        self.project.delete_clip(track, clip)?;
                    }
                }
            }
            (change.apply)(self, choice, &change.peers)?;
            self.project.validate()
        })();
        if let Err(error) = result {
            if let Some(before) = self.before_edit.take() {
                self.project = before;
            }
            if let Some(selection) = self.before_selection.take() {
                self.restore_selection_snapshot(selection);
            }
            return Err(error);
        }
        if self
            .before_edit
            .as_ref()
            .is_some_and(|before| before != &self.project)
        {
            self.edit_label = Some(change.label);
            self.changed();
        }
        self.flush_pending_edit();
        Ok(())
    }

    pub(crate) fn ghost_change_frame(&mut self, ctx: &egui::Context) {
        let Some(change) = &self.ghost_change else {
            return;
        };
        let count = change.peers.len();
        egui::CentralPanel::default()
            .frame(daw_ui::inset(ctx).inner_margin(0))
            .show(ctx, |ui| self.last_scene.paint(ui));
        if let Some(decision) = daw_ui::ghost_change_dialog(ctx, &change.label, count) {
            let change = self.ghost_change.take().unwrap();
            if let daw_ui::GhostDecision::Apply(choice) = decision
                && let Err(error) = self.apply_ghost_choice(change, choice)
            {
                self.message(Err(error), "");
            }
            ctx.request_repaint();
        }
    }
}
