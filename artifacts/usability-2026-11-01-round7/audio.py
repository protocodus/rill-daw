"""Isolated timer-paced PulseAudio server and ALSA bridge for native Rill testing."""
from pathlib import Path
import json, os, subprocess, time
SYSROOT=Path('/workspace/.rill-tools/sysroot')

def audio_environment(directory):
    directory=Path(directory)
    for name in ['pulse-runtime','pulse-state','runtime']:
        (directory/name).mkdir(exist_ok=True); (directory/name).chmod(0o700)
    sock=directory/'pulse-runtime/native'
    env=os.environ.copy()
    lib=str(SYSROOT/'usr/lib/x86_64-linux-gnu')
    env['LD_LIBRARY_PATH']=lib+'/pulseaudio:'+str(SYSROOT/'usr/lib/pulse-17.0+dfsg1/modules')+':'+lib+':'+env.get('LD_LIBRARY_PATH','')
    env['PULSE_RUNTIME_PATH']=str(directory/'pulse-runtime')
    env['PULSE_STATE_PATH']=str(directory/'pulse-state')
    env['PULSE_SERVER']='unix:'+str(sock)
    env['PULSE_SINK']='rill_lab'
    env['PULSE_SOURCE']='rill_lab.monitor'
    env['PULSE_LATENCY_MSEC']='60'
    env['XDG_RUNTIME_DIR']=str(directory/'runtime')
    env['ALSA_PLUGIN_DIR']=str(SYSROOT/'usr/lib/x86_64-linux-gnu/alsa-lib')
    env['ALSA_CONFIG_PATH']=str(directory/'alsa.conf')
    (directory/'alsa.conf').write_text('pcm.!default { type pulse server "unix:'+str(sock)+'" hint { show on description "Rill lab paced output" } }\nctl.!default { type pulse server "unix:'+str(sock)+'" }\n')
    return env

def start_audio(directory):
    directory=Path(directory); env=audio_environment(directory)
    args=[str(SYSROOT/'usr/bin/pulseaudio'),'-n','--daemonize=no','--use-pid-file=no','--exit-idle-time=-1','--disable-shm=yes','--log-level=warning','--dl-search-path='+str(SYSROOT/'usr/lib/pulse-17.0+dfsg1/modules'),'--load=module-native-protocol-unix socket='+str(directory/'pulse-runtime/native')+' auth-anonymous=1','--load=module-null-sink sink_name=rill_lab rate=48000 channels=2 sink_properties=device.description=Rill_Lab_Paced_Output']
    server=subprocess.Popen(args,env=env,stdout=(directory/'pulse.log').open('w'),stderr=subprocess.STDOUT,start_new_session=True)
    deadline=time.monotonic()+10
    while time.monotonic()<deadline:
        if server.poll() is not None: raise RuntimeError((directory/'pulse.log').read_text())
        if (directory/'pulse-runtime/native').exists():
            result=subprocess.run([str(SYSROOT/'usr/bin/pactl'),'info'],env=env,capture_output=True,text=True)
            if result.returncode==0:
                (directory/'pulse-info.txt').write_text(result.stdout)
                return server,env
        time.sleep(.05)
    server.terminate(); server.wait(timeout=5)
    raise RuntimeError('Pulse server did not become ready')

if __name__=='__main__':
    import sys
    server,env=start_audio(sys.argv[1]); print(json.dumps({'pid':server.pid,'server':env['PULSE_SERVER']}))
