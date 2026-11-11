//! A note-local draft: retained UMP data is never rewritten merely by viewing it.
use crate::app::{DawApp, PopupKind};
use anyhow::{Context, Result, ensure};
use daw_core::{ClipContent, Midi2ChannelVoice, Midi2NoteData, MidiNoteExpression, Note};
use eframe::egui;

pub(crate) struct MidiExpressionDraft {
    track: u64,
    clip: u64,
    session: u64,
    original: Note,
    note: Note,
    start: f64,
    length: f64,
    offset: f64,
    rate: f64,
    controls: daw_ui::MidiExpressionControls,
    rows: Vec<daw_ui::MidiExpressionPoint>,
    dirty: bool,
    checked_revision: u64,
    stale: bool,
    error: Option<String>,
}

impl MidiExpressionDraft {
    fn refresh_rows(&mut self) {
        self.rows = self
            .note
            .midi2
            .as_ref()
            .into_iter()
            .flat_map(|data| &data.expressions)
            .filter_map(|point| {
                let message = point.packet.midi2_channel_voice()?;
                Some(daw_ui::MidiExpressionPoint {
                    id: point.id,
                    beat: point.beat / self.rate,
                    kind: daw_ui::MidiExpressionKind::from_status(message.status),
                    controller: message.detail & 127,
                    value: message.data,
                })
            })
            .collect();
        self.rows
            .sort_by(|a, b| a.beat.total_cmp(&b.beat).then(a.id.cmp(&b.id)));
        self.dirty = self.note != self.original;
    }

    fn matches(&self, app: &DawApp) -> bool {
        self.session == app.song_session.id && app.project.clip(self.track,self.clip).is_some_and(|clip| {
            clip.start == self.start && clip.length == self.length && clip.offset == self.offset && clip.source_rate == self.rate
                && matches!(&clip.content,ClipContent::Midi{notes} if notes.iter().any(|note|note == &self.original))
        })
    }

    fn action(&mut self, action: daw_ui::MidiExpressionAction) -> Result<()> {
        use daw_ui::MidiExpressionAction as Action;
        match action {
            Action::Select(id) => {
                let point = self
                    .rows
                    .iter()
                    .find(|point| point.id == id)
                    .context("Expression point is missing")?;
                self.controls.selected = Some(id);
                if let Some(kind) = point.kind {
                    self.controls.kind = kind;
                }
                self.controls.controller = point.controller;
                self.controls.beat = point.beat;
                self.controls.value = point.value;
            }
            Action::New => {
                self.controls.selected = None;
                self.controls.value = self.controls.kind.initial_value();
            }
            Action::Remove => {
                let id = self
                    .controls
                    .selected
                    .context("Select an expression point")?;
                let data = self
                    .note
                    .midi2
                    .as_mut()
                    .context("Expression point is missing")?;
                data.expressions.retain(|point| point.id != id);
                self.controls.selected = None;
                self.refresh_rows();
            }
            Action::Add | Action::Update => {
                let mut beat = self.controls.beat * self.rate;
                if action == Action::Update
                    && let Some(point) = self.note.midi2.as_ref().and_then(|data| {
                        data.expressions
                            .iter()
                            .find(|point| Some(point.id) == self.controls.selected)
                    })
                    && self.controls.beat == point.beat / self.rate
                {
                    beat = point.beat;
                }
                ensure!(
                    beat.is_finite() && (0.0..=daw_core::MAX_BEAT).contains(&beat),
                    "Expression time is outside the supported note source"
                );
                if self.note.midi2.is_none() {
                    self.note.midi2 = Some(Midi2NoteData::from_legacy(&self.note));
                }
                let data = self.note.midi2.as_mut().unwrap();
                let point = if action == Action::Update {
                    let id = self
                        .controls
                        .selected
                        .context("Select an expression point")?;
                    let index = data
                        .expressions
                        .iter()
                        .position(|point| point.id == id)
                        .context("Expression point is missing")?;
                    let mut point = data.expressions[index].clone();
                    let old = point
                        .packet
                        .midi2_channel_voice()
                        .context("Unsupported raw expression")?;
                    ensure!(
                        daw_ui::MidiExpressionKind::from_status(old.status).is_some(),
                        "This raw expression is retained unchanged"
                    );
                    point.beat = beat;
                    point.packet.words[1] = self.controls.value;
                    if old.status != self.controls.kind.status() {
                        point.packet.words[0] = (point.packet.words[0] & !0x00f0_0000)
                            | (u32::from(self.controls.kind.status()) << 20);
                        point.packet.words[0] &= !0x7f;
                    }
                    if self.controls.kind.has_controller()
                        && (old.detail & 127 != self.controls.controller
                            || old.status != self.controls.kind.status())
                    {
                        point.packet.words[0] =
                            (point.packet.words[0] & !0x7f) | u32::from(self.controls.controller);
                    }
                    point
                } else {
                    ensure!(
                        data.expressions.len() < daw_core::MAX_NOTE_EXPRESSIONS,
                        "This note has reached the expression point limit"
                    );
                    let packet = Midi2ChannelVoice {
                        group: data.group,
                        channel: data.channel,
                        status: self.controls.kind.status(),
                        index: data.note_index,
                        detail: if self.controls.kind.has_controller() {
                            self.controls.controller
                        } else {
                            0
                        },
                        data: self.controls.value,
                    }
                    .packet()?;
                    MidiNoteExpression::new(beat, packet)
                };
                point.validate()?;
                if action == Action::Update {
                    data.expressions.retain(|other| other.id != point.id);
                }
                self.controls.selected = Some(point.id);
                // Keep every other retained packet in its original relative
                // order; inserting/moving this point does not normalize them.
                let index = data
                    .expressions
                    .iter()
                    .position(|other| other.beat > point.beat)
                    .unwrap_or(data.expressions.len());
                data.expressions.insert(index, point);
                self.refresh_rows();
            }
        }
        Ok(())
    }
}

impl DawApp {
    pub(crate) fn open_midi_expression(&mut self, track: u64, clip: u64, note: u64) {
        let draft = (|| -> Result<MidiExpressionDraft> {
            let clip_data = self
                .project
                .clip(track, clip)
                .context("MIDI clip is missing")?;
            let ClipContent::Midi { notes } = &clip_data.content else {
                anyhow::bail!("Select a MIDI note");
            };
            let original = notes
                .iter()
                .find(|candidate| candidate.id == note)
                .context("Select one MIDI note")?
                .clone();
            let controls = daw_ui::MidiExpressionControls {
                velocity: original.velocity16(),
                release_velocity: original.release_velocity16(),
                pitch_semitones: original.pitch_semitones(),
                selected: None,
                kind: daw_ui::MidiExpressionKind::Pressure,
                controller: 0,
                beat: 0.0,
                value: 0,
            };
            let mut draft = MidiExpressionDraft {
                track,
                clip,
                session: self.song_session.id,
                note: original.clone(),
                original,
                start: clip_data.start,
                length: clip_data.length,
                offset: clip_data.offset,
                rate: clip_data.source_rate,
                controls,
                rows: Vec::new(),
                dirty: false,
                checked_revision: self.revision,
                stale: false,
                error: None,
            };
            draft.refresh_rows();
            Ok(draft)
        })();
        match draft {
            Ok(draft) => {
                self.open_popup(PopupKind::MidiExpression);
                self.midi_expression = Some(draft);
            }
            Err(error) => self.message(Err(error), ""),
        }
    }

    pub(crate) fn midi_expression_popup(&mut self, ctx: &egui::Context) {
        if !self.midi_expression_open {
            ctx.data_mut(|data| {
                data.remove::<serde_json::Value>(egui::Id::new("midi_expression_diagnostics"))
            });
            return;
        }
        let Some(mut draft) = self.midi_expression.take() else {
            self.close_popup();
            return;
        };
        if draft.checked_revision != self.revision || draft.session != self.song_session.id {
            draft.stale = !draft.matches(self);
            draft.checked_revision = self.revision;
        }
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        let title = format!(
            "Note expression · {}",
            self.project
                .clip(draft.track, draft.clip)
                .map_or("MIDI", |clip| if clip.name.is_empty() {
                    "MIDI"
                } else {
                    clip.name.as_str()
                })
        );
        daw_ui::page(title).id(egui::Id::new("midi_expression")).open(&mut open).width(daw_ui::tokens::POPUP_WIDTH_MD).show(ctx,|window_ui| {
            let mut footer = daw_ui::popup_footer(window_ui,"midi_expression_footer");
            let mut geometry = None;
            let mut body_clip = egui::Rect::NOTHING;
            daw_ui::scroll_body(footer.body(),"midi_expression_body",|ui| {
                body_clip = ui.clip_rect();
                let audible_start = draft.start + (draft.note.start - draft.offset)/draft.rate;
                daw_ui::text::note(ui,format!("Note begins at song beat {:.6} · point times are beats after the note",audible_start));
                if let Some(data) = &draft.note.midi2 { daw_ui::text::note(ui,format!("Group {} · Channel {} · Wire note {}",data.group+1,data.channel+1,data.note_index)); }
                daw_ui::text::note(ui,"Expression playback depends on instrument support. Retained raw packets stay unchanged.");
                daw_ui::section_gap(ui);
                let maximum = draft.rows.iter().map(|point|point.beat).fold(draft.note.length/draft.rate,f64::max);
                let response = ui.add_enabled_ui(!draft.stale,|ui|daw_ui::midi_expression_editor(ui,egui::Id::new(("note_expression_controls",draft.track,draft.clip,draft.note.id)),&mut draft.controls,&draft.rows,maximum)).inner;
                if response.velocity_changed { draft.note.set_velocity16(draft.controls.velocity); draft.refresh_rows(); draft.error=None; }
                if response.release_changed { draft.note.set_release_velocity16(draft.controls.release_velocity); draft.refresh_rows(); draft.error=None; }
                if response.pitch_changed {
                    match draft.note.set_pitch_semitones(draft.controls.pitch_semitones) { Ok(())=>{draft.controls.pitch_semitones=draft.note.pitch_semitones();draft.refresh_rows();draft.error=None;},Err(error)=>draft.error=Some(format!("{error:#}")) }
                }
                if let Some(action) = response.action {
                    match draft.action(action) { Ok(())=>draft.error=None, Err(error)=>draft.error=Some(format!("{error:#}")) }
                }
                geometry = Some(response);
                if draft.stale { daw_ui::text::note(ui,"The source note or clip timing changed. Cancel and reopen to keep those edits."); }
                if let Some(error) = &draft.error { daw_ui::text::note(ui,error); }
            });
            let action_rects = footer.show(window_ui,|ui|daw_ui::dialog_actions(ui,|ui| {
                let response = ui.add_enabled(!draft.stale && draft.dirty && draft.error.is_none(),daw_ui::dialog_primary_widget(ui,"Apply"));
                apply = response.clicked();
                let cancel_response = daw_ui::dialog_button(ui,"Cancel");
                cancel = cancel_response.clicked();
                [response.rect,cancel_response.rect]
            },|ui| { daw_ui::text::note(ui,"One Undo step"); })).inner.inner.0;
            let rect = |r:egui::Rect|serde_json::json!({"min":[r.min.x,r.min.y],"max":[r.max.x,r.max.y]});
            if let Some(response) = geometry {
                ctx.data_mut(|data|data.insert_temp(egui::Id::new("midi_expression_diagnostics"),serde_json::json!({
                    "open":true,"track":draft.track,"clip":draft.clip,"note":draft.note.id,"dirty":draft.dirty,"stale":draft.stale,"point_count":draft.rows.len(),"selected_point":draft.controls.selected,
                    "velocity16":draft.note.velocity16(),"release_velocity16":draft.note.release_velocity16(),"pitch_semitones":draft.note.pitch_semitones(),"source_rate":draft.rate,"note_song_beat":draft.start+(draft.note.start-draft.offset)/draft.rate,
                    "expression_value":draft.controls.value,"expression_beat":draft.controls.beat,"expression_kind":draft.controls.kind.label(),"body":rect(body_clip),
                    "velocity":rect(response.velocity_rect),"release_velocity":rect(response.release_rect),"pitch":rect(response.pitch_rect),"time":rect(response.time_rect),"value":rect(response.value_rect),"normalized":rect(response.normalized_rect),"controller":rect(response.controller_rect),
                    "rows":response.rows.into_iter().map(|(id,r)|serde_json::json!({"id":id,"rect":rect(r)})).collect::<Vec<_>>(),"apply":rect(action_rects[0]),"cancel":rect(action_rects[1]),"error":draft.error,
                })));
            }
        });
        if apply {
            match self.apply_midi_expression(&draft) {
                Ok(()) => {
                    self.close_popup();
                    return;
                }
                Err(error) => {
                    draft.error = Some(format!("{error:#}"));
                }
            }
        }
        if !open || cancel {
            self.close_popup();
        } else {
            self.midi_expression = Some(draft);
        }
    }

    fn apply_midi_expression(&mut self, draft: &MidiExpressionDraft) -> Result<()> {
        self.flush_pending_edit();
        ensure!(
            draft.matches(self),
            "The source note changed; cancel and reopen its expression editor"
        );
        if draft.note == draft.original {
            return Ok(());
        }
        let mut candidate = self.project.clone();
        let note = candidate
            .midi_notes_mut(draft.track, draft.clip)
            .context("MIDI clip is missing")?
            .iter_mut()
            .find(|note| note.id == draft.note.id)
            .context("MIDI note is missing")?;
        *note = draft.note.clone();
        candidate.synchronize_linked_clip(draft.track, draft.clip)?;
        candidate.validate()?;
        self.begin_edit();
        self.project = candidate;
        self.edit_label = Some("Edit note expression".into());
        self.changed();
        self.flush_pending_edit();
        Ok(())
    }
}
