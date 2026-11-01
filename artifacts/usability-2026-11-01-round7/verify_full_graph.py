#!/usr/bin/env python3
"""Coordinator-only source contribution proof preserving the complete saved graph."""
from pathlib import Path
import json,os,subprocess,time,zipfile
import numpy as np
from evidence_utils import digest,wav_evidence
from verify_mix_components import samples
BASE=Path(__file__).resolve().parent
OUT=BASE/'technical/full-graph-ablation'

def render(participant,mode):
    i=participant['participant'];source=Path(participant['project']['path'])
    target=OUT/f'participant-{i}-{mode}.rill';wav=target.with_suffix('.wav');log=target.with_suffix('.log')
    if any(p.exists() for p in [target,wav,log]):raise RuntimeError('Refusing to overwrite an earlier analysis attempt')
    with zipfile.ZipFile(source) as src:
        manifest=json.loads(src.read('manifest.json'));changed=[]
        if mode!='full':
            kind='Audio' if mode=='audio-muted' else 'Midi'
            for track in manifest['project']['tracks']:
                if track['kind']==kind and not track.get('mute',False):track['mute']=True;changed.append(track['id'])
        with zipfile.ZipFile(target,'w',zipfile.ZIP_DEFLATED) as dst:
            for name in src.namelist():dst.writestr(name,json.dumps(manifest).encode() if name=='manifest.json' else src.read(name))
    config=OUT/f'config-{i}-{mode}';cache=OUT/f'cache-{i}-{mode}';config.mkdir();cache.mkdir()
    (config/'settings.json').write_text(json.dumps({'sample_rate':participant['wav']['sample_rate_hz'],'buffer_frames':1024})+'\n')
    env=os.environ.copy();env.update(RILL_CONFIG_DIR=str(config),RILL_CACHE_DIR=str(cache),VST3_PATH=str(BASE/'plugins'))
    started=time.monotonic()
    with log.open('w') as stream:
        result=subprocess.run([str(BASE/'baseline/rill'),'--render',str(target),str(wav)],env=env,stdout=stream,stderr=subprocess.STDOUT,timeout=90)
    row={'mode':mode,'analysis_copy':str(target),'analysis_copy_sha256':digest(target),'changed_track_mutes':changed,'log':str(log),'exit_code':result.returncode,'elapsed_seconds':round(time.monotonic()-started,3)}
    if result.returncode:raise RuntimeError(f'Analysis render failed: {log}')
    row['wav']=wav_evidence(wav);return row

def align(path,n):
    values=samples(path);return np.pad(values,(0,max(0,n-len(values))))[:n]

def main():
    if OUT.exists():raise SystemExit('Refusing to overwrite retained verification; investigate the existing attempt')
    OUT.mkdir(parents=True)
    aggregate=json.loads((BASE/'aggregate.json').read_text());frozen=json.loads((BASE/'freeze-manifest.json').read_text())
    for record in frozen['binaries'].values():
        if digest(Path(record['path']))!=record['sha256']:raise RuntimeError('Frozen renderer/helper changed')
    plugin=BASE/'plugins/RillLabSine.vst3/Contents/x86_64-linux/RillLabSine.so'
    if digest(plugin)!=frozen['plugin']['sha256']:raise RuntimeError('Frozen native VST changed')
    rows=[];routing=[]
    for participant in aggregate['participants']:
        i=participant['participant'];project=participant.get('project');wav=participant.get('wav')
        proof={'participant':i,'method':'Full saved routing graph with source-mute ablations; no linearity assumption','audio_and_vst_components_verified':False}
        if not project or not wav:
            proof['limitation']='Missing saved project or WAV';rows.append(proof);continue
        proof.update(original_project=project['path'],original_project_sha256=project['sha256'],original_wav_sha256=wav['sha256'])
        source=Path(project['path']);original_wav=Path(wav['path'])
        try:
            if digest(source)!=project['sha256'] or digest(original_wav)!=wav['sha256']:raise RuntimeError('Original output changed after aggregate; no render attempted')
            with zipfile.ZipFile(source) as archive:saved=json.loads(archive.read('manifest.json'))['project']
            routing.append({'participant':i,'tracks':[{k:t.get(k) for k in ['id','name','kind','mute','solo','volume','pan','output_bus','parent_folder','inserts','sends','automation']} for t in saved['tracks']]})
            full=render(participant,'full');proof['full_render']=full
            original=samples(original_wav);base=align(full['wav']['path'],len(original));error=float(np.max(np.abs(original-base)))
            proof['full_render_max_abs_difference_to_original']=error;proof['reproduction_tolerance']=1e-6
            if not np.isfinite(base).all() or error>1e-6:raise RuntimeError('Full frozen render does not reproduce original within tolerance; export range, tails, state or nondeterminism require investigation')
            effects={}
            for kind in ['audio','midi']:
                result=render(participant,f'{kind}-muted');proof[f'{kind}_muted_render']=result;pcm=align(result['wav']['path'],len(original));delta=base-pcm
                effects[kind]={'all_samples_finite':bool(np.isfinite(pcm).all()),'max_abs_difference':float(np.max(np.abs(delta))),'rms_difference':float(np.sqrt(np.mean(delta**2))),'nonzero_difference_samples':int(np.count_nonzero(np.abs(delta)>1e-7))}
            proof['source_contribution_differences']=effects
            proof['audio_and_vst_components_verified']=bool(all(v['all_samples_finite'] and v['max_abs_difference']>1e-6 and v['nonzero_difference_samples']>0 for v in effects.values()) and participant['factual_success']['has_midi_with_vst3_assignment'])
            proof['interpretation']='Audio and actual VST sources both causally contribute to the independently reproduced saved mix.' if proof['audio_and_vst_components_verified'] else 'Both contributions were not established; inspect saved mutes, solo, routes, source windows and processing. This is not itself proof of an app defect.'
        except Exception as error:
            proof['limitation']=str(error);proof['interpretation']='Contribution proof inconclusive; retain original outcome and report the evidence limitation.'
        proof['original_project_unchanged']=digest(source)==project['sha256'];proof['original_wav_unchanged']=digest(original_wav)==wav['sha256']
        if not proof['original_project_unchanged'] or not proof['original_wav_unchanged']:raise RuntimeError('Original output unexpectedly changed')
        rows.append(proof);(OUT/'results.json').write_text(json.dumps(rows,indent=2)+'\n')
        print(json.dumps({'participant':i,'verified':proof['audio_and_vst_components_verified'],'limitation':proof.get('limitation')}),flush=True)
    (OUT/'routing-review.json').write_text(json.dumps(routing,indent=2)+'\n');(OUT/'results.json').write_text(json.dumps(rows,indent=2)+'\n')
    # Aggregate consumes this canonical evidence index regardless of proof method.
    index=BASE/'technical/isolated-midi/mix-component-verification.json';index.parent.mkdir(parents=True,exist_ok=True)
    if index.exists():raise RuntimeError('Do not replace an earlier contribution index')
    index.write_text(json.dumps(rows,indent=2)+'\n')

if __name__=='__main__':main()
