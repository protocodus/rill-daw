#!/usr/bin/env python3
"""Publish selected evidence verbatim; retain the complete local originals."""
from pathlib import Path
import hashlib
import json
import shutil
import time

REPO = Path('/workspace/rill-daw')
SCRATCH = Path('/workspace/scratch')

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def publish(root, destination, paths, scope):
    destination.mkdir(parents=True, exist_ok=True)
    entries = []
    for source in sorted(set(paths)):
        if not source.is_file():
            raise RuntimeError(f'Missing source: {source}')
        relative = source.relative_to(root)
        parts = list(relative.parts)
        if parts[0].startswith('session-'):
            parts[0] = parts[0].replace('session-', 'participant-')
        target = destination.joinpath(*parts)
        digest = sha(source)
        target.parent.mkdir(parents=True, exist_ok=True)
        if target.exists() and sha(target) != digest:
            raise RuntimeError(f'Refusing to overwrite different published evidence: {target}')
        shutil.copyfile(source, target)
        assert sha(target) == digest
        entries.append(dict(source=str(source), path=str(target.relative_to(REPO)),
                            bytes=source.stat().st_size, sha256=digest))
    record = dict(created_utc=time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
                  scope=scope, file_count=len(entries),
                  total_bytes=sum(e['bytes'] for e in entries), files=entries)
    (destination/'publication-manifest.json').write_text(json.dumps(record, indent=2)+'\n')
    return {k: record[k] for k in ['file_count', 'total_bytes']}

def publish_round(number, selected):
    root = SCRATCH/f'rill-usability-round{number}'
    destination = REPO/'artifacts'/f'usability-2026-11-01-round{number}'
    names = ['aggregate.json', 'completed-evidence.json', 'output-preservation.json',
             'verification-summary.json', 'feedback-resolution.json',
             'prepared-manifest.json', 'freeze-manifest.json', 'setup-manifest.json',
             'source-status.txt', 'source-diff.patch', 'source-hashes.json',
             'untracked-frozen-source.tar.gz', 'protocol.json', 'INSTRUCTIONS.txt',
             'REPORT_SCHEMA.json', 'participant-prompts.json', 'lab_sine_plugin.rs']
    paths = [root/name for name in names]
    for name in ['feedback-resolution-draft.json', 'feedback-coverage-audit.json',
                 'feedback-resolution-audit.json',
                 'screenshot-capture-inventory.json',
                 'verification-helper-amendment.json', 'controlled-comparison-check.json']:
        if (root/name).exists():
            paths.append(root/name)
    for pattern in ['*.py', 'participant-*-prompt.txt', 'cleanup-*.json', 'feedback-*.json',
                    'coordinator-logs/*', 'aggregate-versions/*.json',
                    'technical/full-graph-ablation/*.log']:
        paths.extend(p for p in root.glob(pattern) if p.is_file())
    paths.extend(root/p for p in ['technical/full-graph-ablation/results.json',
                                 'technical/full-graph-ablation/routing-review.json',
                                 'technical/isolated-midi/mix-component-verification.json'])
    for participant in range(1, 6):
        folder = root/f'session-{participant}'
        paths.extend(folder/name for name in ['report.json', 'reactions.txt',
                     'mouse-actions.jsonl', 'mouse-state.json', 'rill.log', 'pulse.log',
                     'processes.json', 'final.png'] + selected.get(str(participant), []))
    return publish(root, destination, paths,
        'Verbatim reports, actions, hashes, verification and selected screenshots. '
        'Original absolute paths refer to the complete local archive. Executables, '
        'VST binaries, music projects/WAV/MIDI, analysis media and other screenshots '
        'remain local. The small tar archive contains only untracked source files '
        'needed to reconstruct the frozen implementation, not generated media.')

if __name__ == '__main__':
    import sys
    number = int(sys.argv[1])
    selected = json.loads(Path(sys.argv[2]).read_text())
    print(json.dumps(publish_round(number, selected), indent=2))
