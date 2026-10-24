//! Linked instances share musical source data and a name, never arrangement
//! bounds. The stable group identity outlives whichever instance was first.
use crate::{AudioEnvelope, Clip, ClipContent, MAX_BEAT, Project, new_id};
use anyhow::{Context, Result, ensure};
use std::collections::HashMap;

/// Canonical source representation strips only instance-local audio settings.
/// MIDI note IDs identify shared events, so editing another instance does not
/// invalidate note selection or require a fragile positional correspondence.
fn shared_source(clip: &Clip) -> (String, ClipContent) {
    let mut content = clip.content.clone();
    if let ClipContent::Audio {
        envelope,
        crossfade,
        ..
    } = &mut content
    {
        *envelope = Some(AudioEnvelope {
            offset: 0.0,
            length: envelope.map_or(clip.length, |region| region.length),
        });
        *crossfade = None;
    }
    (clip.name.clone(), content)
}

// Compare borrowed source fields: validation and unrelated UI edits must not
// allocate another MIDI event vector for every linked instance.
fn same_content(a: &ClipContent, a_length: f64, b: &ClipContent, b_length: f64) -> bool {
    match (a, b) {
        (ClipContent::Midi { notes: a }, ClipContent::Midi { notes: b }) => a == b,
        (
            ClipContent::Audio {
                path: ap,
                gain: ag,
                fade_in: ai,
                fade_out: ao,
                fade_in_curve: aic,
                fade_out_curve: aoc,
                reverse: ar,
                envelope: ae,
                ..
            },
            ClipContent::Audio {
                path: bp,
                gain: bg,
                fade_in: bi,
                fade_out: bo,
                fade_in_curve: bic,
                fade_out_curve: boc,
                reverse: br,
                envelope: be,
                ..
            },
        ) => {
            ap == bp
                && ag == bg
                && ai == bi
                && ao == bo
                && aic == bic
                && aoc == boc
                && ar == br
                && ae.map_or(a_length, |region| region.length)
                    == be.map_or(b_length, |region| region.length)
        }
        _ => false,
    }
}

fn same_source(a: &Clip, b: &Clip) -> bool {
    a.name == b.name && same_content(&a.content, a.length, &b.content, b.length)
}

fn explicit_envelope(clip: &Clip) -> bool {
    !matches!(clip.content, ClipContent::Audio { envelope: None, .. })
}

fn apply_source(clip: &mut Clip, source: &(String, ClipContent)) {
    if explicit_envelope(clip)
        && clip.name == source.0
        && same_content(&clip.content, clip.length, &source.1, 0.0)
    {
        return;
    }
    let mut content = source.1.clone();
    if let (
        ClipContent::Audio {
            envelope: old_envelope,
            crossfade: old_crossfade,
            ..
        },
        ClipContent::Audio {
            envelope,
            crossfade,
            ..
        },
    ) = (&clip.content, &mut content)
    {
        // Source fades can change their shared length without moving the
        // envelope's origin within independently trimmed/split instances.
        envelope.as_mut().expect("canonical audio envelope").offset =
            old_envelope.map_or(0.0, |region| region.offset);
        *crossfade = *old_crossfade;
    }
    clip.name.clone_from(&source.0);
    clip.content = content;
}

impl Project {
    /// Create the next adjacent instance of the same source. The first clone
    /// establishes a stable group; subsequent clones reuse it, across tracks.
    pub fn clone_clip(&mut self, track_id: u64, clip_id: u64) -> Result<u64> {
        self.validate()?;
        let clip = self.clip(track_id, clip_id).context("Clip not found")?;
        ensure!(
            clip.end() + clip.length <= MAX_BEAT,
            "Clone exceeds the supported timeline"
        );
        let group = clip.link_group.unwrap_or_else(new_id);
        let source = shared_source(clip);
        let mut clone = clip.clone();
        clone.id = new_id();
        clone.link_group = Some(group);
        clone.start = clip.end();
        apply_source(&mut clone, &source);
        let id = clone.id;
        let parent = self.clip_mut(track_id, clip_id).expect("validated clip");
        parent.link_group = Some(group);
        apply_source(parent, &source);
        self.track_mut(track_id)
            .expect("validated track")
            .clips
            .push(clone);
        Ok(id)
    }

    /// Detach an instance while retaining its current source, name and bounds.
    /// MIDI events receive independent IDs; the clip instance ID stays stable.
    pub fn make_clip_independent(&mut self, track_id: u64, clip_id: u64) -> Result<()> {
        let clip = self.clip_mut(track_id, clip_id).context("Clip not found")?;
        if clip.link_group.take().is_some()
            && let ClipContent::Midi { notes } = &mut clip.content
        {
            for note in notes {
                note.id = new_id();
            }
        }
        Ok(())
    }

    /// Publish one explicitly edited instance to its linked group. Call after
    /// direct edits to public fields, including every frame of a drag gesture.
    /// Position, visible length, source offset, envelope offset and crossfade
    /// remain local. This also makes implicit fade origins explicit again.
    pub fn synchronize_linked_clip(&mut self, track_id: u64, clip_id: u64) -> Result<()> {
        let clip = self.clip(track_id, clip_id).context("Clip not found")?;
        let Some(group) = clip.link_group else {
            return Ok(());
        };
        if self
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .filter(|member| member.link_group == Some(group))
            .all(|member| explicit_envelope(member) && same_source(clip, member))
        {
            return Ok(());
        }
        let source = shared_source(clip);
        ensure!(
            self.tracks
                .iter()
                .flat_map(|track| &track.clips)
                .filter(|clip| clip.link_group == Some(group))
                .all(|clip| std::mem::discriminant(&clip.content)
                    == std::mem::discriminant(&source.1)),
            "Linked clips must have the same source type"
        );
        for clip in self.tracks.iter_mut().flat_map(|track| &mut track.clips) {
            if clip.link_group == Some(group) {
                apply_source(clip, &source);
            }
        }
        Ok(())
    }

    /// Synchronize a completed transaction against its immediately preceding
    /// synchronized document. Conflicting simultaneous source edits are rejected
    /// without changing the document. For a multi-frame gesture use the explicit
    /// `synchronize_linked_clip` API, not its gesture-initial undo snapshot.
    pub fn synchronize_linked_clips(&mut self, before: &Project) -> Result<()> {
        let previous: HashMap<_, _> = before
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .map(|clip| (clip.id, clip))
            .collect();
        let mut sources = HashMap::new();
        for clip in self.tracks.iter().flat_map(|track| &track.clips) {
            let Some(group) = clip.link_group else {
                continue;
            };
            let source = shared_source(clip);
            if previous
                .get(&clip.id)
                .is_some_and(|old| old.link_group == Some(group) && shared_source(old) != source)
            {
                if let Some(other) = sources.get(&group) {
                    ensure!(
                        other == &source,
                        "Conflicting edits to linked clip instances"
                    );
                } else {
                    sources.insert(group, source);
                }
            }
        }
        let mut candidate = self.clone();
        for clip in candidate
            .tracks
            .iter_mut()
            .flat_map(|track| &mut track.clips)
        {
            if let Some(source) = clip.link_group.and_then(|group| sources.get(&group)) {
                apply_source(clip, source);
            }
        }
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    pub(crate) fn validate_linked_clips(&self) -> Result<()> {
        let mut groups: HashMap<u64, &Clip> = HashMap::new();
        for clip in self.tracks.iter().flat_map(|track| &track.clips) {
            let Some(group) = clip.link_group else {
                continue;
            };
            ensure!(
                group > 0 && group < u64::MAX,
                "Invalid linked clip group ID"
            );
            ensure!(
                explicit_envelope(clip),
                "Linked audio clips need an explicit source envelope"
            );
            if let Some(other) = groups.get(&group) {
                ensure!(
                    same_source(clip, other),
                    "Linked clips have inconsistent names or source data"
                );
            } else {
                groups.insert(group, clip);
            }
        }
        Ok(())
    }
}
