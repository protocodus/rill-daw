#!/usr/bin/env python3
"""Read-only round3 baseline versus three fresh cycles; write a versioned summary."""
from pathlib import Path
import collections,hashlib,json,statistics,time
ROOT=Path('/workspace/rill-daw');OUT=ROOT/'artifacts/usability-2026-10-31-rounds4-6'
KEYS=['easy_to_use','how_cool_it_looks','how_fast_it_is','how_confusing_it_is','feature_completeness']
def digest(path):
 h=hashlib.sha256()
 with Path(path).open('rb') as f:
  for b in iter(lambda:f.read(1048576),b''):h.update(b)
 return h.hexdigest()
def read(path):return json.loads(Path(path).read_text())
def composite(r):return (r['easy_to_use']+r['how_cool_it_looks']+r['how_fast_it_is']+r['feature_completeness']+6-r['how_confusing_it_is'])/5
aggregates={};sources={};reports=[];rounds=[];immutable={}
for n in [3,4,5,6]:
 root=ROOT/f'artifacts/usability-2026-10-31-round{n}';path=root/'aggregate.json'
 if n>=4 and not (root/'completed-evidence.json').exists():raise SystemExit(f'Round{n} evidence not closed; refusing final comparison')
 a=read(path);aggregates[n]=a;sources[f'round{n}']={'aggregate':str(path),'aggregate_sha256':digest(path),'setup_manifest':str(root/'setup-manifest.json'),'setup_manifest_sha256':digest(root/'setup-manifest.json')}
 means={k:statistics.mean(p['ratings'][k] for p in a['participants']) for k in KEYS}
 if means!=a['mean_ratings'] or len(a['participants'])!=5:raise RuntimeError(f'Round{n} rating/sample mismatch')
 setup=read(root/'setup-manifest.json');kinds=collections.Counter()
 for name in ['freeze-manifest.json','source-hashes.json','protocol.json','participant-prompts.json','participant-prompt-provenance.json','technical/isolated-midi/mix-component-verification.json']:
  evidence=root/name
  if evidence.exists():sources[f'round{n}'][name]={'path':str(evidence),'sha256':digest(evidence)}
 for p in a['participants']:
  report=root/f'participant-{p["participant"]}/report.json'
  if digest(report)!=p['report_sha256']:raise RuntimeError(f'Changed participant report: {report}')
  log=root/f'participant-{p["participant"]}/mouse-actions.jsonl'
  if digest(log)!=p['mouse_log_sha256']:raise RuntimeError(f'Changed participant mouse log: {log}')
  data=read(report)
  if data['ratings']!=p['ratings']:raise RuntimeError('Participant rating differs from aggregation')
  kinds.update(f.get('kind',f.get('type','unspecified')) for f in data['findings'])
  for kind in ['project','wav','midi']:
   if p.get(kind):immutable[p[kind]['path']]=p[kind]['sha256']
  if n>=4:reports.append({'round':n,'participant':p['participant'],'piece_name':p['piece_name'],'ratings':p['ratings'],'report':str(report),'report_sha256':p['report_sha256'],'project':p['project'],'wav':p['wav'],'midi':p['midi'],'factual_success':p['factual_success'],'audio_and_vst_contributions_verified':p['wav_audio_and_vst_components_verified'],'elapsed_seconds':p['elapsed_seconds_first_to_last_input'],'input_events':p['mouse_input_events'],'screenshots':p['screenshots']})
 for old in a.get('original_piece_preservation',[]):immutable[old['path']]=old['expected_sha256']
 rounds.append({'round':n,'is_comparison_baseline':n==3,'sample_size':5,'mean_ratings':means,'descriptive_composite':round(composite(means),6),'summary':a['summary'],'finding_kind_counts':dict(kinds),'frozen_application_sha256':setup['binaries']['rill']['sha256'],'frozen_helper_sha256':setup['binaries']['daw-plugin-host']['sha256'],'source_file_count':setup.get('source_file_count',len(read(root/'source-hashes.json'))),'mouse_driver_sha256':setup['mouse_driver_sha256'],'viewport':setup['viewport'],'audio':setup['audio']})
preservation=[{'path':p,'expected_sha256':expected,'current_sha256':digest(p) if Path(p).is_file() else None} for p,expected in sorted(immutable.items())]
for entry in preservation:entry['unchanged']=entry['expected_sha256']==entry['current_sha256']
if not all(e['unchanged'] for e in preservation):raise RuntimeError('An original output changed; inspect source records before comparing')
base=aggregates[3]['mean_ratings'];final=aggregates[6]['mean_ratings'];combined={k:statistics.mean(p['ratings'][k] for p in reports) for k in KEYS}
result={'created_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'study_type':'Fifteen fresh independent simulated beginner sessions, five per cycle; not human research','baseline_round':3,'new_rounds':[4,5,6],'rating_scale':'1–5; higher is more of the named quality. Lower confusion is better.','sources':sources,'rounds':rounds,'final_round6_minus_baseline_round3_points':{k:round(final[k]-base[k],6) for k in KEYS},'direction_adjusted_improvement_points':{k:round((base[k]-final[k]) if k=='how_confusing_it_is' else (final[k]-base[k]),6) for k in KEYS},'composite_definition':{'formula':'(ease + looks + speed + completeness + (6 - confusion)) / 5','interpretation':'Descriptive equal-weight summary of subjective ratings, not a validated usability metric or percentage improvement.','final_minus_baseline_points':round(composite(final)-composite(base),6)},'all15_new_sessions':{'count':len(reports),'mean_ratings':combined,'exact_30_second_exports_with_both_sources_verified':sum(all(p['factual_success'].values()) and p['audio_and_vst_contributions_verified'] for p in reports),'both_sources_verified':sum(p['audio_and_vst_contributions_verified'] for p in reports),'saved_project_wav_and_midi':sum(p['factual_success']['saved_project_wav_and_midi'] for p in reports),'mean_elapsed_seconds_including_overhead':statistics.mean(p['elapsed_seconds'] for p in reports if p['elapsed_seconds'] is not None),'total_input_events':sum(p['input_events'] for p in reports),'total_screenshots':sum(p['screenshots'] for p in reports)},'participants':reports,'original_output_preservation':{'checked_files':len(preservation),'all_unchanged':True,'files':preservation},'comparison_limits':['These are independently simulated beginner sessions, not a human usability experiment or statistical causal estimate.','Every round rates its own frozen build; post-study fixes do not change earlier ratings.','Baseline finding annotations use mixed schemas; counts preserve kind/type labels or unspecified without inferring missing labels, so these counts are not a standardized improvement metric.','The five broad personas and core task are repeated, but each agent is new; these are not paired within-person measurements.','Higher confusion is worse; all other requested category scores increase with the named quality.','Elapsed time and perceived speed include screenshot, tool and reporting overhead and virtual laboratory conditions; neither is a hardware or app performance benchmark.','Nineteen idle old laboratory services were stopped before round4, reducing background load relative to round3. No speed difference can be attributed solely to application changes.','Only the native laboratory Rill Lab Sine VST was exercised; no general third-party compatibility or heard-quality claim follows.','Current contribution proofs are tied to original file hashes; any inconclusive nonlinear/routing/state analysis stays an evidence limitation rather than being labeled an app bug.'],'collector_path':str(Path(__file__).resolve()),'collector_sha256':digest(__file__)}
OUT.mkdir(parents=True,exist_ok=True);versions=OUT/'comparison-versions';versions.mkdir(exist_ok=True);path=versions/f'comparison-{len(list(versions.glob("comparison-*.json")))+1:03d}.json';encoded=json.dumps(result,indent=2)+'\n'
with path.open('x') as f:f.write(encoded)
(OUT/'comparison.json').write_text(encoded)
(OUT/'compare_cycles.py').write_bytes(Path(__file__).read_bytes())
print(json.dumps({'summary':result['all15_new_sessions'],'round6_minus_round3_points':result['final_round6_minus_baseline_round3_points'],'comparison':str(OUT/'comparison.json')},indent=2))
