#!/usr/bin/env python3
"""Metadata-only final snapshot; never builds, runs apps, or changes rated studies."""
from pathlib import Path
import argparse,hashlib,json,os,shutil,subprocess,tarfile,time
BASE=Path(__file__).resolve().parent
REPO=Path('/workspace/rill-daw')
SCOPE=['crates','vendor','scripts','.cargo','Cargo.toml','Cargo.lock','rust-toolchain.toml','rust-toolchain','build.rs','rustfmt.toml','.rustfmt.toml']

def digest(path):
    h=hashlib.sha256()
    with Path(path).open('rb') as stream:
        for chunk in iter(lambda:stream.read(1024*1024),b''):h.update(chunk)
    return h.hexdigest()

def git(*args):return subprocess.check_output(['git',*args],cwd=REPO)
def listed(*args):return sorted(set(x for x in git(*args).decode().split('\0') if x))
def source_entry(name):
    path=REPO/name
    if path.is_symlink():
        target=os.readlink(path)
        return {'kind':'symlink','sha256':hashlib.sha256(os.fsencode(target)).hexdigest(),'target':target}
    if path.is_file():return {'kind':'file','sha256':digest(path),'bytes':path.stat().st_size}
    return None

def capture_source():
    names=listed('ls-files','--cached','--others','--exclude-standard','-z','--',*SCOPE)
    return {name:item for name in names if (item:=source_entry(name)) is not None}

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--freeze-validated-final-build',action='store_true',required=True);parser.add_argument('--validation-summary',action='append',default=[])
    args=parser.parse_args()
    if (BASE/'manifest.json').exists() or (BASE/'binaries').exists():raise SystemExit('Refusing to overwrite an existing final implementation snapshot')
    if git('diff','--name-only','--diff-filter=U').strip():raise SystemExit('Unresolved source conflicts')
    head=git('rev-parse','HEAD').decode().strip();source=capture_source()
    binaries={name:REPO/'target/debug'/name for name in ['rill','daw-plugin-host']}
    before={name:digest(path) for name,path in binaries.items()}
    nine=Path('/workspace/scratch/rill-usability-round9');rated=json.loads((nine/'freeze-manifest.json').read_text());originals=json.loads((nine/'completed-evidence.json').read_text())
    preserved={'rated_binaries':{name:digest(item['path'])==item['sha256'] for name,item in rated['binaries'].items()},'rated_helpers':{name:digest(nine/name)==expected for name,expected in rated['lab_script_sha256'].items()},'participant_evidence':{item['path']:digest(item['path'])==item['sha256'] for item in originals['files']}}
    if not all(all(rows.values()) for rows in preserved.values()):raise SystemExit('Rated round9 evidence changed; investigate before snapshot')
    (BASE/'binaries').mkdir(mode=0o700);records={}
    for name,path in binaries.items():
        sha=before[name];cache=Path('/workspace/scratch/rill-usability-binary-cache')/sha;cache.mkdir(parents=True,exist_ok=True,mode=0o700);immutable=cache/name
        if not immutable.exists():
            temporary=cache/(name+'.pending')
            if temporary.exists():raise RuntimeError('Previous unfinished immutable copy requires review')
            shutil.copy2(path,temporary)
            if digest(temporary)!=sha:raise RuntimeError('Build output changed during immutable copy')
            temporary.chmod(0o500);temporary.rename(immutable)
        if digest(immutable)!=sha:raise RuntimeError('Immutable binary cache mismatch')
        target=BASE/'binaries'/name;os.link(immutable,target)
        records[name]={'path':str(target),'sha256':sha,'bytes':target.stat().st_size,'immutable_cache':str(immutable),'storage':'Hard link to independent immutable content-addressed copy; never linked to Cargo output'}
    (BASE/'source-status.txt').write_bytes(git('status','--short'))
    (BASE/'source-diff.patch').write_bytes(git('diff','--binary','HEAD','--',*SCOPE))
    (BASE/'source-hashes.json').write_text(json.dumps({name:item['sha256'] for name,item in source.items()},indent=2)+'\n')
    (BASE/'source-entries.json').write_text(json.dumps(source,indent=2)+'\n')
    untracked=listed('ls-files','--others','--exclude-standard','-z','--',*SCOPE)
    with tarfile.open(BASE/'untracked-source.tar.gz','w:gz') as archive:
        for name in untracked:
            path=REPO/name
            if path.exists() or path.is_symlink():archive.add(path,arcname=name,recursive=False)
    validation=[];folder=BASE/'validation-provenance';folder.mkdir()
    for index,name in enumerate(args.validation_summary,1):
        original=Path(name);destination=folder/f'{index}-{original.name}';shutil.copy2(original,destination)
        validation.append({'original_path':str(original),'retained_copy':str(destination),'sha256':digest(destination)})
    compiler=subprocess.check_output(['rustc','+1.99.1','--version','--verbose'],text=True).strip()
    if git('rev-parse','HEAD').decode().strip()!=head or capture_source()!=source or any(digest(path)!=before[name] for name,path in binaries.items()):raise RuntimeError('Source or binaries changed during snapshot; do not publish this attempt')
    for item in originals['files']:
        if digest(item['path'])!=item['sha256']:raise RuntimeError('Round9 original changed during snapshot')
    manifest={'kind':'final implementation after round9; distinct from every rated executable','frozen_at_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'rated_by_participants':False,'rating_scope':'Round9 ratings apply only to round9/freeze-manifest.json. This final snapshot contains subsequent per-strip Add send chooser polish and has automated/native validation, with no fourth study or invented participant score.','baseline_git_sha':head,'origin_main_sha':git('rev-parse','origin/main').decode().strip(),'branch':git('branch','--show-current').decode().strip(),'source_scope':SCOPE,'source_file_count':len(source),'source_entries':'source-entries.json','source_hashes':'source-hashes.json','source_diff':'source-diff.patch','untracked_source':'untracked-source.tar.gz','source_reconstruction':'Checkout baseline_git_sha, apply source-diff.patch, extract untracked-source.tar.gz. Verify source-hashes.json (regular-file bytes or symlink-target bytes); source-entries.json records kinds.','binaries':records,'compiler':compiler,'build_configuration':{'toolchain':'1.99.1','profile':'dev','CARGO_PROFILE_DEV_DEBUG':'0','CARGO_PROFILE_TEST_DEBUG':'0','CARGO_INCREMENTAL':'0','provenance':'Root FINALBUILD signal confirms these established build settings, explicit workspace build, latest affected tests, strict all-targets Clippy, formatting and diff checks. This mechanism itself performs metadata reads and immutable copies only.'},'validation_summaries':validation,'round9_rated_manifest':str(nine/'freeze-manifest.json'),'round9_preservation':{'binaries_unchanged':all(preserved['rated_binaries'].values()),'helpers_unchanged':all(preserved['rated_helpers'].values()),'participant_evidence_count':len(originals['files']),'participant_evidence_unchanged':True},'publication_scope':'Small source/provenance files and validation-summary copies may be published. Immutable application/helper binaries remain in the local archive.'}
    manifest['snapshot_file_hashes']={p.name:digest(p) for p in BASE.iterdir() if p.is_file() and p.name!='manifest.json'}
    (BASE/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    print(json.dumps({'manifest':str(BASE/'manifest.json'),'rated_by_participants':False,'source_file_count':len(source),'binaries':{k:v['sha256'] for k,v in records.items()},'round9_preserved':manifest['round9_preservation']},indent=2))

if __name__=='__main__':main()
