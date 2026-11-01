#!/usr/bin/env python3
"""Read-only evidence aggregation; never modifies participant files or old studies."""
from pathlib import Path
import collections, json, statistics
from evidence_utils import wav_evidence, midi_evidence, project_evidence, digest, stamp

BASE=Path(__file__).resolve().parent
REPO=Path('/workspace/rill-daw')
RATING_KEYS=['easy_to_use','how_cool_it_looks','how_fast_it_is','how_confusing_it_is','feature_completeness']
ALLOWED_INPUTS={'click','double','drag','drag_hold','scroll','move','down','up'}


def read_json(path):return json.loads(path.read_text())

def main():
    round_number=int(BASE.name.rsplit('round',1)[1])
    def previous_aggregate(n):
        candidates=[REPO/('artifacts/usability-2026-10-31' if n==1 else f'artifacts/usability-2026-10-31-round{n}')/'aggregate.json', REPO/f'artifacts/usability-2026-11-01-round{n}'/'aggregate.json', Path(f'/workspace/scratch/rill-usability-round{n}/aggregate.json')]
        for candidate in candidates:
            if candidate.is_file():return read_json(candidate)
        raise FileNotFoundError(f'Missing original round{n} aggregate')
    previous={f'round{n}':previous_aggregate(n) for n in range(1,round_number)}
    component_file=BASE/'technical/isolated-midi/mix-component-verification.json'
    component_rows={x['participant']:x for x in read_json(component_file)} if component_file.exists() else {}
    participants=[]
    for i in range(1,6):
        folder=BASE/f'session-{i}'
        report=read_json(folder/'report.json')
        actions=[json.loads(x) for x in (folder/'mouse-actions.jsonl').read_text().splitlines() if x.strip()]
        inputs=[x for x in actions if x['action']!='shot']
        ratings=report.get('ratings',report.get('ratings_1_to_5'))
        if not isinstance(ratings,dict) or set(ratings)!=set(RATING_KEYS):raise ValueError(f'Participant {i}: ratings require exactly {RATING_KEYS}')
        if not all(isinstance(v,(int,float)) and not isinstance(v,bool) and 1<=v<=5 for v in ratings.values()):raise ValueError(f'Participant {i}: ratings must be numeric 1–5')
        paths=report.get('artifacts',{})
        def find_file(key,extension):
            path=paths.get(key,paths.get('audio') if key=='wav' else None)
            if path:
                candidate=Path(path)
                if not candidate.is_absolute():candidate=folder/candidate
                if candidate.is_file():return candidate.resolve()
            matches=list(folder.glob('*'+extension))
            return matches[0].resolve() if len(matches)==1 else None
        project=find_file('project','.rill');wav=find_file('wav','.wav');midi=find_file('midi','.mid')
        evidence_errors=[]
        def inspect(kind,path,reader):
            if path is None:return None
            try:return reader(path)
            except Exception as error:
                evidence_errors.append({'type':kind,'path':str(path),'sha256':digest(path),'error':str(error)})
                return None
        pe=inspect('project',project,project_evidence)
        we=inspect('wav',wav,wav_evidence)
        me=inspect('midi',midi,midi_evidence)
        success={
            'exact_30_second_wav':bool(we and we['duration_seconds']==30.0),
            'nonzero_finite_wav':bool(we and we['all_samples_finite'] and we['nonzero_samples']>0),
            'has_audio_assets':bool(pe and pe['audio_clip_count']>0 and all(t['audio_assets_present'] for t in pe['tracks'])),
            'has_midi_with_vst3_assignment':bool(pe and pe['midi_clip_count']>0 and pe['notes_overlapping_clip_source_window']>0 and pe['actual_vst3_assignments']),
            'saved_project_wav_and_midi':all(x is not None for x in [pe,we,me]),
            'midi_notes_match_project_window':bool(pe and me and pe['notes_overlapping_clip_source_window']==me['note_on_count']),
        }
        component=component_rows.get(i,{})
        verified=bool(component.get('audio_and_vst_components_verified') and pe and we and component.get('original_project_sha256')==pe['sha256'] and component.get('original_wav_sha256')==we['sha256'])
        elapsed=(stamp(inputs[-1]['time'])-stamp(inputs[0]['time'])).total_seconds() if inputs else None
        participants.append({
            'participant':i,'persona':report.get('persona','Unspecified simulated beginner'),'piece_name':report.get('piece_name'),
            'report':str(folder/'report.json'),'report_sha256':digest(folder/'report.json'),'ratings':ratings,'mixer_outcome':report.get('mixer_outcome',{}),
            'mouse_log':str(folder/'mouse-actions.jsonl'),'mouse_log_sha256':digest(folder/'mouse-actions.jsonl'),
            'first_input_utc':inputs[0]['time'] if inputs else None,'last_input_utc':inputs[-1]['time'] if inputs else None,
            'elapsed_seconds_first_to_last_input':elapsed,'mouse_input_events':len(inputs),
            'screenshots':sum(x['action']=='shot' for x in actions),
            'input_event_counts':dict(collections.Counter(x['action'] for x in inputs)),
            'unexpected_input_actions':sorted({x['action'] for x in inputs}-ALLOWED_INPUTS),
            'held_button_at_end':actions[-1].get('held_button_after') if actions else None,
            'project':pe,'wav':we,'midi':me,'evidence_errors':evidence_errors,'factual_success':success,
            'wav_audio_and_vst_components_verified':verified,
        })
    means={key:statistics.mean(p['ratings'][key] for p in participants) for key in RATING_KEYS}
    preservation=[]
    for round_name,old in previous.items():
        for participant in old['participants']:
            for kind in ['project','wav','midi']:
                entry=participant.get(kind)
                if not entry:continue
                path=Path(entry['path']);current=digest(path) if path.is_file() else None
                preservation.append({'round':round_name,'participant':participant['participant'],'type':kind,'path':str(path),'expected_sha256':entry['sha256'],'current_sha256':current,'unchanged':current==entry['sha256']})
    elapsed=[p['elapsed_seconds_first_to_last_input'] for p in participants if p['elapsed_seconds_first_to_last_input'] is not None]
    aggregate={
        'study':'Five fresh independent simulated beginner mouse-only RILL composition attempts after latest-main rebase',
        'round':round_number,'sample_size':5,'protocol':read_json(BASE/'protocol.json'),'setup_manifest':str(BASE/'setup-manifest.json'),
        'participants':participants,'mean_ratings':means,
        'previous_mean_ratings':{name:value['mean_ratings'] for name,value in previous.items()},
        'rating_changes_vs_previous_rounds':{name:{key:means[key]-value['mean_ratings'][key] for key in means} for name,value in previous.items()},
        'summary':{
            'completed_exact_30_second_audio_vst_midi_exports':sum(all(p['factual_success'].values()) for p in participants),
            'saved_project_wav_and_midi':sum(p['factual_success']['saved_project_wav_and_midi'] for p in participants),
            'wav_within_half_second_of_30':sum(bool(p['wav'] and abs(p['wav']['duration_seconds']-30)<=0.5) for p in participants),
            'wav_audio_and_vst_components_verified':sum(p['wav_audio_and_vst_components_verified'] for p in participants),
            'elapsed_seconds_mean':statistics.mean(elapsed) if elapsed else None,
            'total_mouse_input_events':sum(p['mouse_input_events'] for p in participants),
            'total_screenshots':sum(p['screenshots'] for p in participants),
            'prior_pieces_checked':len(preservation),'all_prior_pieces_unchanged':all(p['unchanged'] for p in preservation),
        },
        'original_piece_preservation':preservation,
        'comparison_limits':['Rounds7and8 add an explicit mixer task absent from earlier rounds; only round7→8 is directly comparable.','Simulated participants, not human research.','Elapsed time includes screenshots and reporting; input events include new held-gesture actions and are not equivalent to physical gesture counts.','Round1 did not explicitly encourage a chosen project name; later rounds did.','Each round evaluates its frozen build; ratings are never retroactively applied to later fixes.'],
    }
    encoded=json.dumps(aggregate,indent=2)+'\n'
    versions=BASE/'aggregate-versions';versions.mkdir(exist_ok=True)
    version=versions/f'aggregate-{len(list(versions.glob("aggregate-*.json")))+1:03d}.json'
    with version.open('x') as stream:stream.write(encoded)
    (BASE/'aggregate.json').write_text(encoded)
    print(json.dumps({'mean_ratings':means,'summary':aggregate['summary'],'participants':[{'id':p['participant'],'seconds':p['elapsed_seconds_first_to_last_input'],'actions':p['mouse_input_events'],'wav_duration':p['wav']['duration_seconds'] if p['wav'] else None,'success':p['factual_success']} for p in participants]},indent=2))

if __name__=='__main__':main()
