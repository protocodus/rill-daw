#!/usr/bin/env python3
"""Check exported mixes against isolated audio + native VST renders, without edits."""
from pathlib import Path
import json, struct
import numpy as np
from component_render import BASE,OUT,render_component
from evidence_utils import digest

def samples(path):
    data=Path(path).read_bytes();position=12
    while position+8<=len(data):
        tag=data[position:position+4];size=struct.unpack_from('<I',data,position+4)[0]
        if tag==b'data':return np.frombuffer(data[position+8:position+8+size],dtype='<f4').astype(np.float64)
        position+=8+size+(size%2)
    raise ValueError('No WAV data chunk')

def main():
    OUT.mkdir(parents=True,exist_ok=True)
    aggregate=json.loads((BASE/'aggregate.json').read_text())
    isolated={r['participant']:r for r in json.loads((OUT/'results.json').read_text())}
    rows=[]
    for participant in aggregate['participants']:
        i=participant['participant']
        row={'participant':i,'audio_and_vst_components_verified':False}
        try:
            midi_row=isolated.get(i,{})
            if not midi_row.get('success'):raise ValueError('MIDI-only render unavailable or failed')
            if midi_row.get('original_project_sha256')!=participant['project']['sha256'] or midi_row.get('original_wav_sha256')!=participant['wav']['sha256']:raise ValueError('Isolated MIDI evidence belongs to a different source snapshot')
            audio_row=render_component(participant,'audio');row.update(audio_row)
            if not audio_row['success']:raise ValueError('Audio-only render failed; see log')
            wav=participant['wav'];midi_meta=midi_row['midi_only_render'];audio_meta=audio_row['audio_only_render']
            if any(meta['sample_rate_hz']!=wav['sample_rate_hz'] or meta['channels']!=wav['channels'] for meta in [midi_meta,audio_meta]):raise ValueError('Render format mismatch')
            midi_path=Path(midi_meta['path'])
            if digest(midi_path)!=midi_meta['sha256']:raise ValueError('Isolated MIDI render changed')
            original=samples(wav['path']);audio=samples(audio_meta['path']);midi=samples(midi_path)
            n=len(original)
            audio=np.pad(audio,(0,max(0,n-len(audio))))[:n]
            midi=np.pad(midi,(0,max(0,n-len(midi))))[:n]
            residual=original-audio;difference=residual-midi
            maximum=float(np.max(np.abs(difference)))
            correlation=float(np.corrcoef(residual,midi)[0,1]) if np.std(residual)>0 and np.std(midi)>0 else None
            row.update(compared_stereo_samples=n,mix_minus_audio_rms=float(np.sqrt(np.mean(residual**2))),isolated_midi_rms=float(np.sqrt(np.mean(midi**2))),max_abs_mix_minus_audio_minus_midi=maximum,rms_mix_minus_audio_minus_midi=float(np.sqrt(np.mean(difference**2))),correlation_residual_to_midi=correlation,comparison_tolerance=1e-6)
            row['audio_and_vst_components_verified']=bool(maximum<=1e-6 and audio_meta['all_samples_finite'] and midi_meta['all_samples_finite'] and np.count_nonzero(audio)>0 and np.count_nonzero(midi)>0 and participant['factual_success']['has_midi_with_vst3_assignment'])
            row['interpretation']='Sample comparison supports both audio and VST contributions.' if row['audio_and_vst_components_verified'] else 'Mix contribution not established by this comparison; investigate routing, processing, export range, tails, or failed/empty components before drawing a conclusion.'
        except Exception as error:row['error']=str(error)
        rows.append(row);(OUT/'mix-component-verification.json').write_text(json.dumps(rows,indent=2)+'\n')
        print(json.dumps({key:value for key,value in row.items() if key not in ['audio_only_render','transformation']}),flush=True)

if __name__=='__main__':main()
