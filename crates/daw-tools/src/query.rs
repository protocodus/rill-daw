//! Bounded, path-redacted read models shared by song tools and pinned scopes.
use crate::{MAX_QUERY_ITEMS, SongToolHost, ToolError, parse_arguments};
use daw_core::*;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct Page {
    #[serde(default)]
    pub(crate) offset: usize,
    #[serde(default = "page_limit")]
    pub(crate) limit: usize,
}
fn page_limit() -> usize {
    100
}
impl Page {
    pub(crate) fn validate(&self) -> Result<(), ToolError> {
        if self.limit == 0 || self.limit > MAX_QUERY_ITEMS {
            return Err(ToolError::new(
                "invalid_arguments",
                "limit must be between 1 and 256",
            ));
        }
        Ok(())
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TrackQuery {
    pub(crate) track_id: u64,
    #[serde(default)]
    pub(crate) offset: usize,
    #[serde(default = "page_limit")]
    pub(crate) limit: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClipQuery {
    pub(crate) track_id: u64,
    clip_id: u64,
    #[serde(default)]
    pub(crate) offset: usize,
    #[serde(default = "page_limit")]
    pub(crate) limit: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoteExpressionQuery {
    pub(crate) track_id: u64,
    clip_id: u64,
    note_id: u64,
    #[serde(default)]
    pub(crate) offset: usize,
    #[serde(default = "page_limit")]
    pub(crate) limit: usize,
}
pub(crate) fn bounded_text(value: &str) -> &str {
    let mut end = value.len().min(1024);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}
pub(crate) fn track_summary(track: &Track) -> Value {
    json!({"id":track.id,"name":bounded_text(&track.name),"name_truncated":track.name.len()>1024,"kind":track.kind,"gain":track.gain,"input_gain":track.input_gain,"instrument_output":track.instrument_output,"gain_db":if track.gain > 0.0 {Some(20.0 * f64::from(track.gain).log10())} else {None},"pan":track.pan,"mute":track.mute,"solo":track.solo,"latency_samples":track.latency_samples,"clip_count":track.clips.len(),"output_bus":track.output_bus,"parent_folder":track.parent_folder,"has_instrument":!matches!(track.instrument, Instrument::None),"midi_input_group":track.midi_input_group,"insert_count":track.inserts.len()})
}
fn note_summary(note: &Note) -> Value {
    let mut value = json!({"id":note.id,"pitch":note.pitch,"start":note.start,"length":note.length,"velocity":note.velocity});
    if let Some(data) = &note.midi2 {
        value["midi2"] = json!({"protocol":data.protocol,"group":data.group,"channel":data.channel,"note_index":data.note_index,
            "velocity":data.velocity,"release_velocity":data.release_velocity,"attribute_type":data.attribute_type,
            "attribute_data":data.attribute_data,"release_attribute_type":data.release_attribute_type,
            "release_attribute_data":data.release_attribute_data,"expression_count":data.expressions.len()});
    }
    value
}
/// Each change with its start beat, so callers need not rebuild the bar map.
pub(crate) fn meter_changes(project: &Project) -> Value {
    let meter = project.meter_map();
    json!(
        project
            .meter_changes
            .iter()
            .map(|change| json!({"bar":change.bar,"beats_per_bar":change.beats_per_bar,"start_beat":meter.bar_start(change.bar)}))
            .collect::<Vec<_>>()
    )
}
fn marker_summary(marker: &Marker) -> Value {
    json!({"id":marker.id,"name":bounded_text(&marker.name),"beat":marker.beat,"color":marker.color})
}
pub(crate) fn clip_summary(clip: &Clip) -> Value {
    json!({"id":clip.id,"name":bounded_text(&clip.name),"name_truncated":clip.name.len()>1024,"muted":clip.muted,"start":clip.start,"length":clip.length,"offset":clip.offset,"source_rate":clip.source_rate,"midi_event_count":clip.midi_events.len(),"link_group":clip.link_group,"audio_take":clip.audio_take,"kind":match clip.content {ClipContent::Midi{..}=>"midi",ClipContent::Audio{..}=>"audio"}})
}

pub(crate) fn handles(name: &str) -> bool {
    matches!(
        name,
        "get_song_context" | "list_tracks" | "get_track" | "get_clip" | "get_note_expressions"
    )
}

pub(crate) fn dispatch<H: SongToolHost + ?Sized>(
    host: &H,
    name: &str,
    arguments: Value,
) -> Result<Value, ToolError> {
    match name {
        "get_song_context" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Empty {}
            let _: Empty = parse_arguments(arguments)?;
            let p = host.project();
            let session = host.song_session();
            Ok(
                json!({"session_id":session.id,"revision":session.revision,"name":bounded_text(&p.name),"name_truncated":p.name.len()>1024,"tempo":p.tempo,"beats_per_bar":p.beats_per_bar,"meter_changes":meter_changes(p),"markers":p.markers.iter().take(MAX_QUERY_ITEMS).map(marker_summary).collect::<Vec<_>>(),"marker_count":p.markers.len(),"end_beat":p.end_beat(),"track_count":p.tracks.len(),"master":track_summary(&p.master),"loop":{"enabled":p.loop_enabled,"start":p.loop_start,"end":p.loop_end},"metronome":{"enabled":p.metronome_enabled,"gain":p.metronome_gain,"precount_bars":p.precount_bars},"timeline_units":"Quarter-note beats. Arrangement starts at 0; bars use beats_per_bar from bar 1, then each meter change from its bar; MIDI note positions use clip source beats.","limits":{"edits_per_batch":MAX_BATCH_EDITS,"notes_per_batch":MAX_BATCH_NOTES,"growth_bytes_per_batch":MAX_BATCH_GROWTH_BYTES,"query_items":MAX_QUERY_ITEMS}}),
            )
        }
        "list_tracks" => {
            let q: Page = parse_arguments(arguments)?;
            q.validate()?;
            let p = host.project();
            let s = host.song_session();
            Ok(
                json!({"session_id":s.id,"revision":s.revision,"total":p.tracks.len(),"offset":q.offset,"tracks":p.tracks.iter().skip(q.offset).take(q.limit).map(track_summary).collect::<Vec<_>>(),"master":track_summary(&p.master)}),
            )
        }
        "get_track" => {
            let q: TrackQuery = parse_arguments(arguments)?;
            Page {
                offset: q.offset,
                limit: q.limit,
            }
            .validate()?;
            let track = host
                .project()
                .track(q.track_id)
                .ok_or_else(|| ToolError::new("not_found", "Track not found"))?;
            let s = host.song_session();
            let mut value = track_summary(track);
            value["session_id"] = json!(s.id);
            value["revision"] = json!(s.revision);
            value["clips"] = json!(
                track
                    .clips
                    .iter()
                    .skip(q.offset)
                    .take(q.limit)
                    .map(clip_summary)
                    .collect::<Vec<_>>()
            );
            value["clips_offset"] = json!(q.offset);
            value["instrument_inputs"] = json!(track.instrument_inputs);
            value["sends"] = json!(track.sends.iter().take(MAX_QUERY_ITEMS).collect::<Vec<_>>());
            value["inserts"] = json!(track.inserts.iter().take(MAX_QUERY_ITEMS).map(|slot|json!({"id":slot.id,"name":bounded_text(&slot.name),"name_truncated":slot.name.len()>1024,"enabled":slot.enabled,"parameter_count":slot.parameters.len(),"inputs":slot.inputs})).collect::<Vec<_>>());
            // No opaque plugin state, VST paths, or source paths leave this API.
            if let Some(lane) = &track.automation {
                let clips: Vec<_> = lane
                    .clips
                    .iter()
                    .take(MAX_QUERY_ITEMS)
                    .map(|clip| {
                        json!({"id":clip.id,"name":bounded_text(&clip.name),"start":clip.start,"length":clip.length,"offset":clip.offset,"link_group":clip.link_group,"total_points":clip.points.len(),"points":clip.points.iter().take(MAX_QUERY_ITEMS).collect::<Vec<_>>()})
                    })
                    .collect();
                value["automation"] = json!({"target":lane.target,"interpolation":lane.interpolation,"override_value":lane.override_value,"total_points":lane.points.len(),"points":lane.points.iter().skip(q.offset).take(q.limit).collect::<Vec<_>>(),"clip_count":lane.clips.len(),"clips":clips});
            }
            Ok(value)
        }
        "get_note_expressions" => {
            let q: NoteExpressionQuery = parse_arguments(arguments)?;
            Page {
                offset: q.offset,
                limit: q.limit,
            }
            .validate()?;
            let note = host
                .project()
                .midi_notes(q.track_id, q.clip_id)
                .and_then(|notes| notes.iter().find(|note| note.id == q.note_id))
                .ok_or_else(|| ToolError::new("not_found", "MIDI note not found"))?;
            let points = note
                .midi2
                .as_ref()
                .map_or(&[][..], |data| data.expressions.as_slice());
            let session = host.song_session();
            Ok(
                json!({"session_id":session.id,"revision":session.revision,"note":note_summary(note),"total":points.len(),"offset":q.offset,"expressions":points.iter().skip(q.offset).take(q.limit).collect::<Vec<_>>()}),
            )
        }
        "get_clip" => {
            let q: ClipQuery = parse_arguments(arguments)?;
            Page {
                offset: q.offset,
                limit: q.limit,
            }
            .validate()?;
            let clip = host
                .project()
                .clip(q.track_id, q.clip_id)
                .ok_or_else(|| ToolError::new("not_found", "Clip not found"))?;
            let mut v = clip_summary(clip);
            let s = host.song_session();
            v["session_id"] = json!(s.id);
            v["revision"] = json!(s.revision);
            v["midi_events_offset"] = json!(q.offset);
            v["midi_events"] = json!(
                clip.midi_events
                    .iter()
                    .skip(q.offset)
                    .take(q.limit)
                    .collect::<Vec<_>>()
            );
            match &clip.content {
                ClipContent::Midi { notes } => {
                    v["total_notes"] = json!(notes.len());
                    v["notes_offset"] = json!(q.offset);
                    v["notes"] = json!(
                        notes
                            .iter()
                            .skip(q.offset)
                            .take(q.limit)
                            .map(note_summary)
                            .collect::<Vec<_>>()
                    );
                }
                ClipContent::Audio {
                    gain,
                    fade_in,
                    fade_out,
                    fade_in_curve,
                    fade_out_curve,
                    reverse,
                    processing,
                    envelope,
                    boundary_fades,
                    crossfade,
                    ..
                } => {
                    v["audio"] = json!({"gain":gain,"fade_in":fade_in,"fade_out":fade_out,"fade_in_curve":fade_in_curve,"fade_out_curve":fade_out_curve,"reverse":reverse,"processing":processing,"envelope":envelope,"boundary_fades":boundary_fades,"crossfade":crossfade,"warp_markers":clip.warp_markers,"source_end":clip.source_end()});
                }
            }
            Ok(v)
        }
        _ => Err(ToolError::new("unknown_tool", "Unknown song query")),
    }
}
