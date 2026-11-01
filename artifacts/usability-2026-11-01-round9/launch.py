#!/usr/bin/env python3
"""Coordinator-only: launch fresh native sessions from the immutable frozen build."""
from pathlib import Path
import argparse, ctypes as c, json, os, subprocess, time
from audio import start_audio
from freeze import sha

BASE=Path(__file__).resolve().parent
ROUND=int(BASE.name.rsplit('round',1)[1])
DISPLAY_BASE=ROUND*100+80

def start_ticks(pid):
    return Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split()[19]

def main():
    p=argparse.ArgumentParser()
    p.add_argument('--launch-frozen-build',action='store_true',required=True)
    p.parse_args()
    manifest=json.loads((BASE/'freeze-manifest.json').read_text())
    if (BASE/'setup-manifest.json').exists():raise SystemExit('Round already launched')
    for item in manifest['binaries'].values():
        if sha(item['path'])!=item['sha256']:raise SystemExit('Frozen executable hash mismatch')
    if sha(BASE/'mouse.py')!=manifest['mouse_driver_sha256']:raise SystemExit('Mouse driver changed after freeze')
    if sha(BASE/'INSTRUCTIONS.txt')!=manifest['protocol_sha256']:raise SystemExit('Instructions changed after freeze')
    if sha(BASE/'protocol.json')!=manifest['structured_protocol_sha256']:raise SystemExit('Structured protocol changed after freeze')
    if sha(BASE/'REPORT_SCHEMA.json')!=manifest['report_schema_sha256']:raise SystemExit('Report schema changed after freeze')
    for name,expected in manifest.get('participant_prompt_hashes',{}).items():
        if sha(BASE/name)!=expected:raise SystemExit('Participant prompt changed after freeze')
    for i in range(1,6):
        sd=BASE/f'session-{i}'
        if (sd/'processes.json').exists() or list((sd/'config').iterdir()) or list((sd/'cache').iterdir()):raise SystemExit(f'Session {i} is not fresh')
        if Path(f'/tmp/.X11-unix/X{DISPLAY_BASE+i}').exists() or Path(f'/tmp/.X{DISPLAY_BASE+i}-lock').exists():raise SystemExit(f'Display {DISPLAY_BASE+i} occupied')
    lib=c.CDLL('libX11.so.6')
    lib.XOpenDisplay.argtypes=[c.c_char_p];lib.XOpenDisplay.restype=c.c_void_p
    lib.XDefaultRootWindow.argtypes=[c.c_void_p];lib.XDefaultRootWindow.restype=c.c_ulong
    lib.XQueryTree.argtypes=[c.c_void_p,c.c_ulong,c.POINTER(c.c_ulong),c.POINTER(c.c_ulong),c.POINTER(c.POINTER(c.c_ulong)),c.POINTER(c.c_uint)]
    lib.XResizeWindow.argtypes=[c.c_void_p,c.c_ulong,c.c_uint,c.c_uint]
    lib.XMoveWindow.argtypes=[c.c_void_p,c.c_ulong,c.c_int,c.c_int]
    lib.XFree.argtypes=[c.c_void_p];lib.XFlush.argtypes=[c.c_void_p];lib.XCloseDisplay.argtypes=[c.c_void_p]
    manifest.update(status='launching',launched_at_utc=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()))
    for i in range(1,6):
        sd=BASE/f'session-{i}';display=f':{DISPLAY_BASE+i}'
        (sd/'config/settings.json').write_text(json.dumps({'sample_rate':48000,'buffer_frames':1024})+'\n')
        processes={'display':display,'process_start_ticks':{}}
        def record(name,process):
            processes[name]=process.pid
            processes['process_start_ticks'][name]=start_ticks(process.pid)
            (sd/'processes.json').write_text(json.dumps(processes,indent=2)+'\n')
        pulse,env=start_audio(sd);record('pulse',pulse)
        env.update(DISPLAY=display,RILL_CONFIG_DIR=str(sd/'config'),RILL_CACHE_DIR=str(sd/'cache'),VST3_PATH=str(BASE/'plugins'),LIBGL_ALWAYS_SOFTWARE='1',XDG_DATA_DIRS='/workspace/.rill-tools/sysroot/usr/share:/usr/local/share:/usr/share',GSETTINGS_SCHEMA_DIR='/workspace/.rill-tools/sysroot/usr/share/glib-2.0/schemas')
        xvfb=subprocess.Popen(['Xvfb',display,'-screen','0','1440x1000x24','-ac','-nolisten','tcp'],env=env,stdout=(sd/'xvfb.log').open('w'),stderr=subprocess.STDOUT,start_new_session=True);record('xvfb',xvfb)
        deadline=time.monotonic()+5
        d=None
        while time.monotonic()<deadline:
            if xvfb.poll() is not None:raise RuntimeError((sd/'xvfb.log').read_text())
            d=lib.XOpenDisplay(display.encode())
            if d:break
            time.sleep(.05)
        if not d:raise RuntimeError(f'Display {display} not ready')
        app=subprocess.Popen([str(BASE/'baseline/rill')],cwd=sd,env=env,stdout=(sd/'rill.log').open('w'),stderr=subprocess.STDOUT,start_new_session=True);record('app',app)
        time.sleep(1.5)
        if app.poll() is not None:raise RuntimeError((sd/'rill.log').read_text())
        root=lib.XDefaultRootWindow(d);r=c.c_ulong();parent=c.c_ulong();children=c.POINTER(c.c_ulong)();n=c.c_uint()
        lib.XQueryTree(d,root,c.byref(r),c.byref(parent),c.byref(children),c.byref(n))
        for j in range(n.value):lib.XResizeWindow(d,children[j],1440,1000);lib.XMoveWindow(d,children[j],0,0)
        if children:lib.XFree(children)
        lib.XFlush(d);lib.XCloseDisplay(d);time.sleep(.3)
        subprocess.run(['python3',str(BASE/'mouse.py'),str(i),'shot',str(sd/'initial.png')],check=True)
        manifest['sessions'].append({'participant':i,'directory':str(sd),**processes})
        (BASE/'setup-manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
        print(json.dumps({'session':i,'display':display,'directory':str(sd)}),flush=True)
    manifest['status']='five fresh native sessions ready'
    (BASE/'setup-manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')

if __name__=='__main__':main()
