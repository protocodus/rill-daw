from pathlib import Path
import json, re, hashlib, subprocess
base=Path('/workspace/scratch/rill-audio-diagnostics')
def digest(p):return hashlib.sha256(p.read_bytes()).hexdigest()
rows=[]
for name in ['default-1024-1','default-1024-stall60','named-1024-stall60','default-4096-stall60']:
 folder=base/name;manifest=json.loads((folder/'manifest.json').read_text());log=(folder/'probe.log').read_text()
 callback=re.search(r'first_callback_samples=(\d+)',log);done=re.search(r'done callbacks=(\d+) errors=(\d+)',log)
 errors=[{'elapsed_seconds':float(t),'count':int(n),'kind':kind,'raw':raw} for t,n,kind,raw in re.findall(r'error elapsed=([\d.]+) count=(\d+) kind=(\w+) raw=(.*)',log)]
 pid=manifest['server_pid'];stat=Path(f'/proc/{pid}/stat')
 same_process_alive=stat.exists() and stat.read_text().rsplit(')',1)[1].split()[19]==manifest['server_start_ticks']
 rows.append({'name':name,'manifest':str(folder/'manifest.json'),'manifest_sha256':digest(folder/'manifest.json'),'log':str(folder/'probe.log'),'log_sha256':digest(folder/'probe.log'),'selection':manifest['selection'],'sample_rate_hz':48000,'channels':2,'requested_buffer_frames':manifest['frames'],'pulse_latency_environment_msec':60,'injected_stall_msec':int(manifest.get('extra_probe_arguments',[0])[0]),'stall_injections_observed':log.count('injected_callback_stall'),'callback_frames_observed':int(callback.group(1))//2 if callback else None,'observation_seconds':manifest['seconds'],'callbacks_total':int(done.group(1)),'error_callbacks_total':int(done.group(2)),'raw_errors_first_20':errors,'start_utc':manifest['start_utc'],'finish_utc':manifest['finish_utc'],'cleanup_original_server_identity_alive':same_process_alive,'source_sha256':manifest['source_sha256'],'probe_binary_sha256':manifest['probe_sha256']})
summary={
 'purpose':'Bounded technical reproduction of round 7 ALSA output failure outside RILL. This is not a fresh usability study or physical-device benchmark.',
 'method':'Independent fresh PulseAudio null sink per probe; same round 7 audio helper, ALSA bridge,48 kHz stereo f32, requested 1024 frames and PULSE_LATENCY_MSEC=60 unless explicitly changed. A standalone CPAL 0.18.2 program writes zeros; no RILL code, UI, audio renderer, VST, MIDI, or project is linked. Scheduled stalls are artificial delays in the callback, not measurements of participant stalls.',
 'compiler':subprocess.check_output(['/workspace/.rill-tools/cargo/bin/rustc','+1.99.1','-V'],text=True).strip(),
 'cpal_version':'0.18.2','cpal_rlib':'/workspace/rill-daw/target/debug/deps/libcpal-d91c41d47461545a.rlib',
 'cpal_rlib_sha256':digest(Path('/workspace/rill-daw/target/debug/deps/libcpal-d91c41d47461545a.rlib')),
 'round7_audio_helper_sha256':digest(Path('/workspace/scratch/rill-usability-round7/audio.py')),
 'runs':rows,
 'findings':[
  {'id':'A1','status':'observed','finding':'Default output with requested 1024 frames and no artificial stalls completed 20 seconds with 1280 callbacks and zero errors.'},
  {'id':'A2','status':'reproduced','finding':'A 60 ms delay every 64 callbacks causes an Xrun followed immediately by the identical snd_pcm_avail_delay I/O error (5) without RILL code. CPAL later resumes its callbacks, but can emit thousands of BackendError callbacks during recovery. The first recorded error and first 20 raw errors are preserved.'},
  {'id':'A3','status':'reproduced','finding':'Selecting the named Rill lab paced output also produces the same failure under the same artificial delays. Its CPAL driver identifier and the default device both resolve to ALSA PCM default.'},
  {'id':'A4','status':'observed','finding':'The requested 4096-frame run failed at startup before reaching its first scheduled delay. Only 23 callbacks were observed in 20 seconds. Increasing requestedbuffer alone was not demonstrated to solve the failure under this fixed Pulse configuration.'},
  {'id':'A5','status':'observed','finding':'ALSA negotiated 682-frame callbacks for a 1024-frame request and 2730-frame callbacks for a 4096-frame request. CPAL documents nearest supported periods; study reports should continue to distinguish requested from observed buffer size.'},
 ],
 'limits':[
  'Only one 20-second run per listed condition; no estimate of failure rate, reliability, or physical hardware behavior.',
  'The unstalled1024 probe is v1; v2 adds one first callback size log plus optional controlled stalls. Both sources and hashes are preserved.',
  'The actual cause, duration, and thread scheduling of each participant failure were not captured. The independent reproduction establishes a backend recovery path, not that every participant failure was caused by the same event.',
  'The observed temporary recovery is not permission to ignore arbitrary EIO/BackendError events. Other device disconnects and recording clock invalidation remain possible.',
  'PULSE_LATENCY_MSEC=60 is the configured environment request; actual end-to-end latency was not measured.',
  'Named/default differences in one participant recovery do not establish that choosing the named output fixes the fault; changing the output also recreates the stream.'
 ],
 'recommendation':'Preserve fatal handling for unknown/BackendError, explicit Retry/Audio setup, and raw diagnostics. Fix the separately evidenced mixer geometry jumps. Keep round 8 lab settings unchanged for comparison. Do not claim device selection or a larger buffer is a verified fix. Retain this bounded backend limitation and consider a separate ALSA/Pulse/CPAL recovery investigation with platform-specific stream state before changing error classification.',
 'production_files_changed':False,
 'participant_files_changed':False,
 'all_original_probe_server_identities_stopped':all(not r['cleanup_original_server_identity_alive'] for r in rows)
}
(base/'results.json').write_text(json.dumps(summary,indent=2)+'\n')
files=[p for p in base.rglob('*') if p.is_file() and p.name not in {'evidence-index.json'}]
(base/'evidence-index.json').write_text(json.dumps([{'path':str(p.relative_to(base)),'bytes':p.stat().st_size,'sha256':digest(p)} for p in sorted(files)],indent=2)+'\n')
print(json.dumps({'runs':[{'name':r['name'],'callbacks':r['callbacks_total'],'errors':r['error_callbacks_total'],'actual_callback_frames':r['callback_frames_observed'],'injected_stalls':r['stall_injections_observed']} for r in rows],'all_probe_servers_stopped':summary['all_original_probe_server_identities_stopped']},indent=2))
