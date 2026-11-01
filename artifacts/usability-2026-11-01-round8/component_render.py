"""Coordinator-only read-only source checks and isolated analysis-copy rendering."""
from pathlib import Path
import json, os, subprocess, time, zipfile
from evidence_utils import digest, wav_evidence

BASE=Path(__file__).resolve().parent
OUT=BASE/'technical/isolated-midi'

def render_component(participant,kind):
    OUT.mkdir(parents=True,exist_ok=True)
    i=participant['participant'];project=participant.get('project');wav=participant.get('wav')
    row={'participant':i,'component':kind,'success':False}
    if not project or not wav:return dict(row,error='Participant project or WAV evidence missing')
    source=Path(project['path']);original_wav=Path(wav['path'])
    row.update(original_project=str(source),original_project_sha256=digest(source),original_wav_sha256=digest(original_wav))
    if row['original_project_sha256']!=project['sha256'] or row['original_wav_sha256']!=wav['sha256']:
        return dict(row,error='Participant file changed after aggregate; no render attempted')
    frozen=json.loads((BASE/'freeze-manifest.json').read_text())
    for item in frozen['binaries'].values():
        if digest(Path(item['path']))!=item['sha256']:raise RuntimeError('Frozen renderer/helper hash mismatch')
    fixture=OUT/f'participant-{i}-{kind}-only.rill';render=OUT/f'participant-{i}-{kind}-only.wav'
    log=OUT/f'participant-{i}-{kind}.log'
    if any(path.exists() for path in [fixture,render,log]):raise RuntimeError('Analysis output already exists; preserve the earlier attempt before rerunning')
    with zipfile.ZipFile(source) as src:
        manifest=json.loads(src.read('manifest.json'))
        manifest['project']['tracks']=[track for track in manifest['project']['tracks'] if track['kind']==kind.capitalize()]
        manifest['project']['loop_enabled']=False
        used_assets={clip['content']['Audio']['path'] for track in manifest['project']['tracks'] for clip in track['clips'] if 'Audio' in clip['content']}
        with zipfile.ZipFile(fixture,'w',zipfile.ZIP_DEFLATED) as dst:
            dst.writestr('manifest.json',json.dumps(manifest).encode())
            for asset in sorted(used_assets):dst.writestr(asset,src.read(asset))
    config=OUT/f'config-{kind}-{i}';cache=OUT/f'cache-{kind}-{i}'
    config.mkdir(exist_ok=True);cache.mkdir(exist_ok=True)
    (config/'settings.json').write_text(json.dumps({'sample_rate':wav['sample_rate_hz'],'buffer_frames':1024})+'\n')
    env=os.environ.copy();env.update(RILL_CONFIG_DIR=str(config),RILL_CACHE_DIR=str(cache),VST3_PATH=str(BASE/'plugins'))
    started=time.monotonic()
    try:
        with log.open('w') as output:
            result=subprocess.run([str(BASE/'baseline/rill'),'--render',str(fixture),str(render)],env=env,stdout=output,stderr=subprocess.STDOUT,timeout=90)
        row['exit_code']=result.returncode
        if result.returncode==0:row[f'{kind}_only_render']=wav_evidence(render);row['success']=True
    except Exception as error:row['error']=str(error)
    row.update(analysis_copy=str(fixture),analysis_copy_sha256=digest(fixture),renderer=str(BASE/'baseline/rill'),elapsed_seconds=round(time.monotonic()-started,3),log=str(log),transformation=f'Analysis-only ZIP copy retaining {kind.capitalize()} tracks and their referenced assets; looping disabled. Original project and WAV remain untouched. {('MIDI track processing, notes and native VST assignment retained.' if kind=='midi' else 'Audio track processing and referenced assets retained.')}')
    row['original_project_unchanged']=digest(source)==row['original_project_sha256']
    row['original_wav_unchanged']=digest(original_wav)==row['original_wav_sha256']
    if not row['original_project_unchanged'] or not row['original_wav_unchanged']:raise RuntimeError('Unexpected participant file modification detected')
    return row
