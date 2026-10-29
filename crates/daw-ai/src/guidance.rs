//! Curated musical briefs for the assistant. These are editable starting points,
//! not commands: the host inserts them into a draft and the user chooses Send.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptCategory {
    Compose,
    Develop,
    Mix,
    Learn,
}

impl PromptCategory {
    pub const ALL: [Self; 4] = [Self::Compose, Self::Develop, Self::Mix, Self::Learn];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Compose => "Compose",
            Self::Develop => "Develop",
            Self::Mix => "Mix",
            Self::Learn => "Learn",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PromptExample {
    pub id: &'static str,
    pub category: PromptCategory,
    pub title: &'static str,
    pub description: &'static str,
    pub prompt: &'static str,
}

/// Examples use the current-song tool catalog. They deliberately describe
/// musical intent rather than embedding stale IDs, revisions, or plug-in IDs.
pub const PROMPT_EXAMPLES: &[PromptExample] = &[
    PromptExample {
        id: "compose_song",
        category: PromptCategory::Compose,
        title: "Start a song",
        description: "Chords, arpeggio and bass in one musical idea.",
        prompt: "Create an eight-bar idea in D natural minor at the cursor: warm sustained chords, a quiet eighth-note arpeggio and a root/fifth bass. Use degrees 1, 6, 3, 7 for two bars each, with smooth chord voicings. Derive bar lengths from the current meter and keep the tempo unchanged. Create new parts without replacing existing clips, preferably as one coordinated composition edit. Discover installed VST3 instruments before assigning sounds; if none are available, create the MIDI and tell me which tracks still need instruments.",
    },
    PromptExample {
        id: "compose_melody",
        category: PromptCategory::Compose,
        title: "Write a melody",
        description: "A short motif with space and a clear ending.",
        prompt: "Write a four-bar melody in A minor at the cursor on a new MIDI track. Keep it between C4 and C6, use mostly eighth and quarter notes with rests, repeat a short opening motif, and resolve the last note to A. Use the current meter and tempo. Preserve the existing arrangement. Discover an installed melodic instrument for the new track, or clearly report that the MIDI needs an instrument.",
    },
    PromptExample {
        id: "compose_pulse",
        category: PromptCategory::Compose,
        title: "Build a pulse",
        description: "An evenly spaced rhythm with subtle accents.",
        prompt: "Create a four-bar rhythmic MIDI part at the cursor using a Euclidean rhythm with five hits across sixteen equal steps per bar. Use C3 for every hit, accent the first hit, and keep notes short. Derive each step length from the current meter and preserve the tempo and existing clips. Create a new track, discover an installed instrument suitable for a pitched pulse, and report if it still needs a sound. Do not assume a drum mapping.",
    },
    PromptExample {
        id: "develop_variation",
        category: PromptCategory::Develop,
        title: "Create a variation",
        description: "Keep the original and develop its ending.",
        prompt: "Inspect the selected MIDI clip, then create an independent variation immediately after it on the same track. Preserve the original clip, its instrument and the first three quarters of the phrase. Change the final quarter with a small rhythmic or melodic answer that fits the existing pitches and register. Do not overwrite another clip; if there is no free space, ask where to place the variation. Summarize what changed.",
    },
    PromptExample {
        id: "develop_humanize",
        category: PromptCategory::Develop,
        title: "Humanize a phrase",
        description: "Gentle timing and velocity variation.",
        prompt: "Humanize the selected notes in the selected MIDI clip; if no notes are selected, use all notes in that clip. Apply deterministic randomization with seed 7, velocity variation of plus or minus 8 and onset variation of at most 0.02 quarter-note beats. Keep pitches, note lengths, instrument and clip placement unchanged. Do this as one undoable phrase edit and describe its scope.",
    },
    PromptExample {
        id: "develop_legato",
        category: PromptCategory::Develop,
        title: "Connect chord voices",
        description: "Sustain each voice into the next chord.",
        prompt: "Inspect the selected MIDI clip and connect its chord voices with top-to-bottom legato: match notes by high-to-low voice rank at successive onsets. Change note lengths only, preserving pitches, onsets, velocities and the instrument. Use the selected notes if there is a note selection; otherwise use the clip. Explain the result and leave other clips unchanged.",
    },
    PromptExample {
        id: "mix_headroom",
        category: PromptCategory::Mix,
        title: "Check headroom",
        description: "Measure an excerpt before suggesting changes.",
        prompt: "Inspect this song's routing, then analyze an excerpt of the mix using local audio measurements. Prefer the selected range, limited to 30 seconds and ending within the first 120 seconds; if that range cannot be analyzed, explain the limit and use a suitable excerpt near the song start. Report peak level, RMS, clipping and stereo measurements, with two practical next steps. Distinguish measured facts from suggestions. Do not change channel settings, routing or clips.",
    },
    PromptExample {
        id: "mix_balance",
        category: PromptCategory::Mix,
        title: "Make room in the mix",
        description: "Lower source channels while keeping their balance.",
        prompt: "Inspect the song's source channels, buses and gain automation. Lower each unmuted audio or MIDI source channel by 3 dB relative to its current gain in one mix edit; leave silent channels, buses, Master, pan, mute, solo and sends unchanged. If a source channel's gain is automated, leave it unchanged and report it rather than overriding its automation. Summarize the channels changed. This is a starting gain adjustment, not a claim that the mix has been perceptually balanced.",
    },
    PromptExample {
        id: "mix_fade",
        category: PromptCategory::Mix,
        title: "Automate a fade",
        description: "A smooth four-bar channel fade with held endpoints.",
        prompt: "Add volume automation to the selected audio, MIDI or bus channel, fading from its current level at the cursor to 12 dB below that level over four bars of the current meter. Use a spline segment with a gentle exponential shape, and hold the starting value before the fade and the final value afterward. Leave clips, routing and other controls unchanged. Inspect existing volume automation first; if a lane already exists, explain it and ask before replacing points or changing an override. Use the tool's documented gain normalization.",
    },
    PromptExample {
        id: "learn_harmony",
        category: PromptCategory::Learn,
        title: "Explain the harmony",
        description: "Understand the notes already in your phrase.",
        prompt: "Inspect the selected MIDI clip and explain its likely tonal center, chord tones and any notes that add tension. Use actual note data and the composition theory tools where useful. Treat the key as a hypothesis if the phrase is ambiguous. Suggest two possible continuations with note names, but do not edit the song.",
    },
    PromptExample {
        id: "learn_colors",
        category: PromptCategory::Learn,
        title: "Explore modal color",
        description: "Compare major and Dorian with concrete notes.",
        prompt: "Use the composition theory tools to compare C major and C Dorian. Show the scale notes, explain which notes change, and give a short diatonic chord progression for each with spelled chord tones. Suggest how their different colors could shape a melody. This is a theory explanation only; do not add tracks or edit the song.",
    },
    PromptExample {
        id: "learn_arrangement",
        category: PromptCategory::Learn,
        title: "Plan the arrangement",
        description: "Turn the current material into a section plan.",
        prompt: "Inspect the current song's tracks, clip lengths, repetitions and instrument assignments. Suggest a concise intro, main section, contrasting section and ending using the material already present. Give section lengths in bars of the current meter, identify which existing parts to introduce or remove, and distinguish structural observations from musical suggestions. Explain missing ingredients if the song is empty. Do not edit or play the song yet.",
    },
];

/// Examples for the authority shown in the assistant's Scope line. Unknown
/// contexts deliberately have no examples rather than falling back to song-wide
/// operations. The host supplies the canonical context name and enforces scope.
pub fn scoped_prompt_examples(context: &str) -> &'static [PromptExample] {
    match context {
        "song" => PROMPT_EXAMPLES,
        "midi_clip" => &[
            PromptExample {
                id: "clip_humanize",
                category: PromptCategory::Develop,
                title: "Humanize this phrase",
                description: "Small timing and velocity changes inside this piano roll.",
                prompt: "Inspect the MIDI clip in the stated scope. Humanize its notes with seed 7, velocity variation of plus or minus 8 and onset variation of at most 0.02 quarter-note beats. Keep pitches, lengths and clip placement unchanged. If the clip is linked, explain that it must be made independent before changing its source notes. Edit only this clip and summarize the result.",
            },
            PromptExample {
                id: "clip_legato",
                category: PromptCategory::Develop,
                title: "Connect these voices",
                description: "Join note lengths while retaining the phrase's pitches.",
                prompt: "Inspect the scoped MIDI clip and connect its chord voices with top-to-bottom legato. Change note lengths only; keep pitches, onsets, velocities, the instrument and all other clips unchanged. If this is a linked clip, explain the need to make it independent before editing its notes. Summarize the changes.",
            },
            PromptExample {
                id: "clip_harmony",
                category: PromptCategory::Learn,
                title: "Explain this harmony",
                description: "Read this clip's actual notes before suggesting ideas.",
                prompt: "Inspect only the MIDI clip in the stated scope. Explain its likely tonal center, chord tones and notes that add tension, using actual note data and theory queries. Treat the key as a hypothesis if ambiguous. Suggest two ways to refine the phrase, but make no edits.",
            },
        ],
        "audio_clip" => &[
            PromptExample {
                id: "audio_measure",
                category: PromptCategory::Learn,
                title: "Measure this clip",
                description: "Inspect local audio measurements without changing the region.",
                prompt: "Analyze only the audio clip in the stated scope. Report its measured peak, RMS and any clipping, explaining which region was analyzed and any analysis limits. Suggest two practical adjustments without editing. Describe these as local measurements, not as having listened to the audio.",
            },
            PromptExample {
                id: "audio_normalize",
                category: PromptCategory::Mix,
                title: "Normalize this clip",
                description: "Set this region's peak without changing the channel mix.",
                prompt: "Inspect and normalize only the audio clip in the stated scope to a peak of -1 dBFS using analysis-backed clip processing. Preserve its placement, length, source file, channel gain, effects and all other clips. Report the applied clip gain or explain why no change was needed.",
            },
            PromptExample {
                id: "audio_fades",
                category: PromptCategory::Develop,
                title: "Soften clip edges",
                description: "Add short fades at this clip's boundaries.",
                prompt: "Inspect the scoped audio clip and its existing fades. Add a short fade-in and fade-out of 0.02 quarter-note beats each, shortened if necessary to fit the region. Preserve longer existing fades, placement, source window, clip gain and other clips. Explain the changed boundaries without claiming to have listened.",
            },
        ],
        "instrument" => &[
            PromptExample {
                id: "instrument_explain",
                category: PromptCategory::Learn,
                title: "Explore this instrument",
                description: "Discover the assigned instance's actual controls.",
                prompt: "Inspect the instrument in the stated scope and discover its real parameter metadata. Explain a few useful controls with their names, units, ranges and current values. Do not guess parameter IDs, replace the instrument or edit any settings.",
            },
            PromptExample {
                id: "instrument_shape",
                category: PromptCategory::Develop,
                title: "Shape this sound",
                description: "Find a focused change within the opened instrument.",
                prompt: "Inspect the scoped instrument's real parameters and existing automation. Suggest one small parameter adjustment that could soften its attack or brightness, naming the actual control and value. Explain uncertainty if the plug-in provides no suitable metadata. Do not apply the suggestion yet or change notes, effects or the channel mix.",
            },
        ],
        "effect" => &[
            PromptExample {
                id: "effect_explain",
                category: PromptCategory::Learn,
                title: "Understand this effect",
                description: "Read this insert's controls and automation.",
                prompt: "Inspect only the effect in the stated scope. Discover its real parameter names, ranges, units and current values, then explain its useful controls and existing automation. Make no edits and do not infer a sound from parameter values alone.",
            },
            PromptExample {
                id: "effect_bypass",
                category: PromptCategory::Mix,
                title: "Bypass this effect",
                description: "Turn off the opened insert while retaining its settings.",
                prompt: "Bypass only the effect in the stated scope, preserving its position, parameters, automation and every other insert. If it is already bypassed, leave it unchanged. Report its final bypass state so I can compare the result with the normal transport.",
            },
        ],
        "effect_chain" => &[
            PromptExample {
                id: "chain_review",
                category: PromptCategory::Learn,
                title: "Review this chain",
                description: "Explain the selected channel's insert order.",
                prompt: "Inspect only the effect chain in the stated scope. List its inserts in signal order, including bypass states and known parameter information. Explain one possible order change as a suggestion, without editing, and distinguish general signal-flow advice from measured facts.",
            },
            PromptExample {
                id: "chain_bypass",
                category: PromptCategory::Mix,
                title: "Bypass this chain",
                description: "Disable this channel's inserts in one undoable edit.",
                prompt: "Bypass all inserts in the effect chain in the stated scope as one atomic edit. Keep their order, parameters and automation, and leave the instrument, channel mix, routing, clips and other effect chains unchanged. Report which inserts changed.",
            },
        ],
        "channel" => &[
            PromptExample {
                id: "channel_headroom",
                category: PromptCategory::Mix,
                title: "Lower this channel",
                description: "Reduce its gain by 3 dB without changing the balance elsewhere.",
                prompt: "Inspect the channel in the stated scope and its gain automation. If gain is not automated, lower its current gain by 3 dB, preserving pan, mute, solo, sends and all other channels. If gain is automated, explain its state and make no change rather than overriding it. Report the exact resulting level.",
            },
            PromptExample {
                id: "channel_pan",
                category: PromptCategory::Mix,
                title: "Center this channel",
                description: "Set the selected channel's pan to the center.",
                prompt: "Inspect the scoped channel's pan and automation. If pan is not automated, set it to the center and leave every other control unchanged. If pan is automated, explain the lane and its override state without changing it. Report the exact pan value.",
            },
        ],
        "routing" => &[
            PromptExample {
                id: "routing_explain",
                category: PromptCategory::Learn,
                title: "Explain these routes",
                description: "Trace the opened channel's output and sends.",
                prompt: "Inspect routing for the channel in the stated scope. Explain its output destination and each send's destination, level and pre/post-fader state. Identify any disabled sends. Make no changes and do not inspect unrelated clips or effects.",
            },
            PromptExample {
                id: "routing_quiet_sends",
                category: PromptCategory::Mix,
                title: "Lower these sends",
                description: "Reduce only this channel's send levels by 3 dB.",
                prompt: "Inspect sends belonging to the channel in the stated routing scope. Reduce each nonzero send gain by 3 dB in one edit, leaving zero sends unchanged. If a send level is automated, leave that send unchanged and report it. Preserve destinations, pre/post state, output, channel mix and all other channels.",
            },
        ],
        "automation" => &[
            PromptExample {
                id: "automation_explain",
                category: PromptCategory::Learn,
                title: "Explain this curve",
                description: "Read this lane's points, shapes and override state.",
                prompt: "Inspect only the automation lane in the stated scope. Explain its target, point values and beat positions, segment interpolation and slopes, and whether it is overridden. Explain how the first value holds before the first point and the last value holds after the last point. Make no edits.",
            },
            PromptExample {
                id: "automation_smooth",
                category: PromptCategory::Develop,
                title: "Smooth this lane",
                description: "Use spline segments while retaining its control points.",
                prompt: "Inspect the scoped automation lane, then set its segments to spline interpolation with neutral slope. Keep every point's beat and value, the lane's target and override state, and all other lanes unchanged. Explain what changed and retain held endpoint values outside the point range.",
            },
            PromptExample {
                id: "automation_resume",
                category: PromptCategory::Mix,
                title: "Resume this automation",
                description: "Release the manual override for this lane only.",
                prompt: "Inspect the automation lane in the stated scope. If it is manually overridden, resume automation for this lane only. Preserve its points, interpolation, target and all other lanes. Report whether an override was released or it was already following automation.",
            },
        ],
        _ => &[],
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AssistantTip {
    pub title: &'static str,
    pub body: &'static str,
}

pub const ASSISTANT_TIPS: &[AssistantTip] = &[
    AssistantTip {
        title: "Name the scope",
        body: "Check the Scope line before sending. The menu-bar AI can edit the current song; AI for this context is pinned to the opened clip, instrument, effect or other editor. Selecting something else does not retarget it. Browse Tools to see what this scope permits.",
    },
    AssistantTip {
        title: "Give a musical brief",
        body: "Include the key, number of bars, rhythm, register and desired role. For example: four bars of sparse A-minor bass, below C3, with a rest before each chord change.",
    },
    AssistantTip {
        title: "Audition and refine",
        body: "Start with a short phrase, play it with the transport, then ask for one specific change: fewer notes, a stronger ending or a quieter accompaniment. Local AI edits its stated context. Use global AI when a request needs new tracks or clips elsewhere.",
    },
    AssistantTip {
        title: "Keep control",
        body: "Ask for advice only when you want ideas before edits. Stop prevents further assistant work; committed changes remain. Edit → Undo reverses the latest song edit. Contextual AI cannot run global history actions, so use the Edit menu to undo local results.",
    },
    AssistantTip {
        title: "Choose the sound",
        body: "MIDI needs an assigned VST3 instrument to be heard. Global AI can assign installed instruments; instrument-scoped AI works with the opened instance's parameters. Rill does not include bundled instruments or effects.",
    },
    AssistantTip {
        title: "Connect your provider",
        body: "Set your provider, model and API key in AI settings. Prompts and song context go to that provider; audio analysis supplies local measurements, not audio listening. Provider charges and usage limits apply.",
    },
];
