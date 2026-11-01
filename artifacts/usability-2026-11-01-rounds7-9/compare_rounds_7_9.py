#!/usr/bin/env python3
"""Recompute all scores and check report hashes against the three aggregates."""
from pathlib import Path
import argparse
import hashlib
import json
import statistics
import time

parser = argparse.ArgumentParser()
parser.add_argument('--repo-root', type=Path, help='Read published participant folders instead of local sessions')
parser.add_argument('--output', type=Path, default=Path('/workspace/scratch/rill-study-iteration/comparison-rounds7-9.json'))
args = parser.parse_args()
keys = ['easy_to_use', 'how_cool_it_looks', 'how_fast_it_is',
        'how_confusing_it_is', 'feature_completeness']
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
rounds = {}
for number in [7, 8, 9]:
    if args.repo_root:
        root = args.repo_root/'artifacts'/f'usability-2026-11-01-round{number}'
        prefix = 'participant-'
    else:
        root = Path('/workspace/scratch')/f'rill-usability-round{number}'
        prefix = 'session-'
    aggregate = json.loads((root/'aggregate.json').read_text())
    reports = [root/f'{prefix}{i}/report.json' for i in range(1, 6)]
    values = [json.loads(p.read_text()) for p in reports]
    means = {key: statistics.mean(r['ratings'][key] for r in values) for key in keys}
    assert means == aggregate['mean_ratings']
    assert aggregate['summary']['wav_audio_and_vst_components_verified'] == 5
    for i, path in enumerate(reports, 1):
        row = next(p for p in aggregate['participants'] if p['participant'] == i)
        assert row['report_sha256'] == sha(path)
        assert row['ratings'] == values[i-1]['ratings']
    rounds[str(number)] = dict(
        aggregate_sha256=sha(root/'aggregate.json'), mean_ratings=means,
        summary=aggregate['summary'],
        participant_ratings=[dict(participant=i, report_sha256=sha(path),
                                 ratings=values[i-1]['ratings'])
                             for i,path in enumerate(reports, 1)])
comparison = []
for key in keys:
    a,b,c = [rounds[str(n)]['mean_ratings'][key] for n in [7,8,9]]
    comparison.append(dict(category=key, round7=a, round8=b, round9=c,
                           delta8_minus7=round(b-a,10), delta9_minus8=round(c-b,10),
                           delta9_minus7=round(c-a,10),
                           improves_when='decreases' if key=='how_confusing_it_is' else 'increases'))
result = dict(
    status='complete; recomputed from fifteen unchanged original reports',
    generated_utc=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),
    scale='1–5; lower confusion is better; ordinal simulated judgments, not human research or a performance benchmark.',
    rounds=rounds, comparison_rows=comparison,
    controlled_conditions='Same five personas/task, mouse driver, 1440×1000 viewport, laboratory VST, 48 kHz/1024-frame request/60 ms virtual audio and Rust1.99.1 build profile. Fresh independent agents each round.',
    implementation_sequence={
        '7_to_8':'Stable mixer geometry, retained knob release movement, shared exact-value mouse keypad.',
        '8_to_9':'Piano-roll gesture origin uses primary press event, including original target/pitch/onset and outside-canvas rejection.'},
    interpretation_limits=[
        'Earlier rounds did not explicitly require mixer tasks and have different build conditions; use round7 as this cycle baseline.',
        'Original ratings precede the subsequent fixes and are never retroactively changed.',
        'Score changes cannot isolate causal effects of a particular fix or predict human usability.',
        'Session durations and input counts include tool/reporting overhead; no hardware-speed claim.',
        'File and source-ablation verification establishes both exported components, not continuous playback or heard musical quality.',
        'Playback failure remains unresolved; standalone CPAL reproduction does not establish each participant stall cause.'])
args.output.parent.mkdir(parents=True,exist_ok=True)
args.output.write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(comparison,indent=2))
