from pathlib import Path
import sys, os, json, hashlib, subprocess, time
sys.path.insert(0,'/workspace/scratch/rill-usability-round7')
from audio import start_audio
base=Path('/workspace/scratch/rill-audio-diagnostics')
name,selection,frames,seconds=sys.argv[1:]
folder=base/name
folder.mkdir()
server,env=start_audio(folder)
def ticks(pid):return Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split()[19]
record={'start_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'selection':selection,'frames':int(frames),'seconds':int(seconds),'server_pid':server.pid,'server_start_ticks':ticks(server.pid),'initial_load':Path('/proc/loadavg').read_text().strip(),'probe_sha256':hashlib.sha256((base/'cpal_probe').read_bytes()).hexdigest(),'source_sha256':hashlib.sha256((base/'cpal_probe.rs').read_bytes()).hexdigest()}
try:
 with (folder/'probe.log').open('w') as log:
  result=subprocess.run([str(base/'cpal_probe'),selection,frames,seconds],env=env,stdout=log,stderr=subprocess.STDOUT,timeout=int(seconds)+10)
 record['exit_code']=result.returncode
finally:
 if server.poll() is None and ticks(server.pid)==record['server_start_ticks']:
  server.terminate()
  try:server.wait(timeout=5)
  except subprocess.TimeoutExpired:server.kill();server.wait(timeout=5)
 record['final_load']=Path('/proc/loadavg').read_text().strip()
 record['finish_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime())
 (folder/'manifest.json').write_text(json.dumps(record,indent=2)+'\n')
print((folder/'probe.log').read_text())
