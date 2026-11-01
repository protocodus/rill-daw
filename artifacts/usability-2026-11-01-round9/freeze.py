#!/usr/bin/env python3
"""Coordinator-only: freeze the build and reconstructible source after validation."""
from pathlib import Path
import argparse, hashlib, json, os, shutil, subprocess, tarfile, time

BASE=Path(__file__).resolve().parent
REPO=Path('/workspace/rill-daw')

def sha(path):
    h=hashlib.sha256()
    with Path(path).open('rb') as source:
        for chunk in iter(lambda:source.read(1024*1024),b''):h.update(chunk)
    return h.hexdigest()

def git(*args):
    return subprocess.check_output(['git',*args],cwd=REPO)

def main():
    p=argparse.ArgumentParser()
    p.add_argument('--freeze-validated-build',action='store_true',required=True)
    p.add_argument('--expected-rill-sha')
    args=p.parse_args()
    if args.expected_rill_sha and sha(REPO/'target/debug/rill')!=args.expected_rill_sha:raise SystemExit('Validated application binary changed')
    baseline=BASE/'baseline'
    if baseline.exists():raise SystemExit('Refusing to overwrite a frozen build')
    if git('diff','--name-only','--diff-filter=U').strip():raise SystemExit('Resolve all source conflicts first')
    for name in ['rill','daw-plugin-host']:
        if not (REPO/'target/debug'/name).is_file():raise SystemExit(f'Missing binary: {name}')
    baseline.mkdir(mode=0o700)
    binaries={}
    for name in ['rill','daw-plugin-host']:
        destination=baseline/name
        source=REPO/'target/debug'/name
        digest=sha(source)
        cache=Path('/workspace/scratch/rill-usability-binary-cache')/digest
        cache.mkdir(parents=True,exist_ok=True,mode=0o700)
        immutable=cache/name
        if not immutable.exists():
            temporary=cache/(name+'.pending')
            shutil.copy2(source,temporary)
            if sha(temporary)!=digest:raise SystemExit('Binary changed during freeze')
            temporary.chmod(0o500)
            temporary.rename(immutable)
        if sha(immutable)!=digest:raise SystemExit('Immutable binary cache was modified')
        os.link(immutable,destination)
        binaries[name]={'path':str(destination),'sha256':digest,'storage':'hard link to immutable content-addressed copy; never linked to Cargo output','immutable_cache':str(immutable)}
    (BASE/'source-status.txt').write_bytes(git('status','--short'))
    # HEAD includes staged resolutions as well as unstaged final corrections.
    (BASE/'source-diff.patch').write_bytes(git('diff','--binary','HEAD','--','crates','Cargo.toml','Cargo.lock'))
    names=git('ls-files','--cached','--others','--exclude-standard','-z','--','crates','Cargo.toml','Cargo.lock').decode().split('\0')
    hashes={name:sha(REPO/name) for name in sorted(set(names)) if name and (REPO/name).is_file()}
    (BASE/'source-hashes.json').write_text(json.dumps(hashes,indent=2)+'\n')
    untracked=git('ls-files','--others','--exclude-standard','-z','--','crates','Cargo.toml','Cargo.lock').decode().split('\0')
    with tarfile.open(BASE/'untracked-frozen-source.tar.gz','w:gz') as archive:
        for name in sorted(set(untracked)):
            if name and (REPO/name).is_file():archive.add(REPO/name,arcname=name)
    manifest={
        'status':'frozen; not launched',
        'frozen_at_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),
        'baseline_git_sha':git('rev-parse','HEAD').decode().strip(),
        'origin_main_sha':git('rev-parse','origin/main').decode().strip(),
        'working_tree_status':'source-status.txt','source_diff':'source-diff.patch',
        'source_hashes':'source-hashes.json','source_file_count':len(hashes),'source_scope':'All existing Git-tracked and untracked non-ignored files under crates plus Cargo.toml and Cargo.lock, without extension filtering.','untracked_source':'untracked-frozen-source.tar.gz',
        'source_reconstruction':'Checkout baseline_git_sha, apply source-diff.patch, extract untracked-frozen-source.tar.gz; verify source-hashes.json.',
        'binaries':binaries,
        'plugin':{'name':'Rill Lab Sine','path':str(BASE/'plugins/RillLabSine.vst3'),'sha256':sha(BASE/'plugins/RillLabSine.vst3/Contents/x86_64-linux/RillLabSine.so')},
        'viewport':[1440,1000],
        'build_conditions':json.loads((BASE/'protocol.json').read_text()).get('build_conditions',{}),
        'audio':{'sink_rate_hz':48000,'application_buffer_request_frames':1024,'virtual_backend_buffering_ms':60,'isolation':'Separate timer-paced PulseAudio null sink, ALSA pulse bridge, config and cache per participant'},
        'protocol_sha256':sha(BASE/'INSTRUCTIONS.txt'),
        'structured_protocol_sha256':sha(BASE/'protocol.json'),
        'report_schema_sha256':sha(BASE/'REPORT_SCHEMA.json'),
        'participant_prompt_hashes':{path.name:sha(path) for path in sorted(BASE.glob('participant-*-prompt.txt'))+([BASE/'participant-prompts.json'] if (BASE/'participant-prompts.json').exists() else [])},
        'lab_script_sha256':{p.name:sha(p) for p in sorted(BASE.glob('*.py'))},
        'mouse_driver_sha256':sha(BASE/'mouse.py'),
        'sessions':[],
    }
    (BASE/'freeze-manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    print(json.dumps(manifest,indent=2))

if __name__=='__main__':main()
