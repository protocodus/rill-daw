#!/usr/bin/env python3
"""Launch separate native visual-review sessions only after coordinator approval."""
from pathlib import Path
import argparse
import ctypes as c
import hashlib
import json
import os
import subprocess
import time

from audio import start_audio

BASE = Path(__file__).resolve().parent
APP = Path('/workspace/rill-daw/target/debug/rill')
PLUGINS = Path('/workspace/scratch/rill-usability-round7/plugins')


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def start_ticks(pid):
    return Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[19]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--launch-native-preview', action='store_true', required=True)
    parser.parse_args()
    manifest = json.loads((BASE / 'prepare-manifest.json').read_text())
    if (BASE / 'setup-manifest.json').exists():
        raise SystemExit('Preview already launched; preserve its evidence')
    if sha(manifest['source_project']) != manifest['source_project_sha256']:
        raise SystemExit('Original project changed since preparation')
    for name, record in manifest['helpers'].items():
        if sha(BASE / name) != record['sha256']:
            raise SystemExit(f'Copied helper changed: {name}')
    for session in manifest['sessions']:
        sd = BASE / f"session-{session['session']}"
        if (sd / 'processes.json').exists() or list((sd / 'cache').iterdir()):
            raise SystemExit(f'Session {sd} is not fresh')
        if sha(session['project']) != session['project_sha256']:
            raise SystemExit('Preview project changed before launch')
        if sha(sd / 'config/settings.json') != session['settings_sha256']:
            raise SystemExit('Preview settings changed before launch')
        number = session['display'][1:]
        if Path(f'/tmp/.X11-unix/X{number}').exists() or Path(f'/tmp/.X{number}-lock').exists():
            raise SystemExit(f"Display {session['display']} is occupied")
    lib = c.CDLL('libX11.so.6')
    lib.XOpenDisplay.argtypes = [c.c_char_p]
    lib.XOpenDisplay.restype = c.c_void_p
    lib.XDefaultRootWindow.argtypes = [c.c_void_p]
    lib.XDefaultRootWindow.restype = c.c_ulong
    lib.XQueryTree.argtypes = [c.c_void_p, c.c_ulong, c.POINTER(c.c_ulong), c.POINTER(c.c_ulong), c.POINTER(c.POINTER(c.c_ulong)), c.POINTER(c.c_uint)]
    lib.XResizeWindow.argtypes = [c.c_void_p, c.c_ulong, c.c_uint, c.c_uint]
    lib.XMoveWindow.argtypes = [c.c_void_p, c.c_ulong, c.c_int, c.c_int]
    lib.XFree.argtypes = [c.c_void_p]
    lib.XFlush.argtypes = [c.c_void_p]
    lib.XCloseDisplay.argtypes = [c.c_void_p]
    manifest.update(status='launching', launched_at_utc=time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()))
    manifest['binary'] = {'path':str(APP), 'sha256':sha(APP)}
    manifest['plugin_host'] = {'path':str(APP.with_name('daw-plugin-host')), 'sha256':sha(APP.with_name('daw-plugin-host'))}
    manifest['playback_requested'] = False
    (BASE / 'setup-manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    for session in manifest['sessions']:
        i = session['session']
        sd = BASE / f'session-{i}'
        display = session['display']
        processes = {'display':display, 'process_start_ticks':{}}

        def record(name, process):
            processes[name] = process.pid
            processes['process_start_ticks'][name] = start_ticks(process.pid)
            (sd / 'processes.json').write_text(json.dumps(processes, indent=2) + '\n')

        pulse, env = start_audio(sd)
        record('pulse', pulse)
        env.update(DISPLAY=display, RILL_CONFIG_DIR=str(sd / 'config'), RILL_CACHE_DIR=str(sd / 'cache'), VST3_PATH=str(PLUGINS), LIBGL_ALWAYS_SOFTWARE='1', XDG_DATA_DIRS='/workspace/.rill-tools/sysroot/usr/share:/usr/local/share:/usr/share', GSETTINGS_SCHEMA_DIR='/workspace/.rill-tools/sysroot/usr/share/glib-2.0/schemas')
        xvfb = subprocess.Popen(['Xvfb', display, '-screen', '0', '1440x1000x24', '-ac', '-nolisten', 'tcp'], env=env, stdout=(sd / 'xvfb.log').open('w'), stderr=subprocess.STDOUT, start_new_session=True)
        record('xvfb', xvfb)
        deadline = time.monotonic() + 5
        d = None
        while time.monotonic() < deadline:
            if xvfb.poll() is not None:
                raise RuntimeError((sd / 'xvfb.log').read_text())
            d = lib.XOpenDisplay(display.encode())
            if d:
                break
            time.sleep(.05)
        if not d:
            raise RuntimeError(f'Display {display} not ready')
        app = subprocess.Popen([str(APP), session['project'], '--view', 'mixer'], cwd=sd, env=env, stdout=(sd / 'rill.log').open('w'), stderr=subprocess.STDOUT, start_new_session=True)
        record('app', app)
        deadline = time.monotonic() + 15
        found = False
        while time.monotonic() < deadline:
            if app.poll() is not None:
                raise RuntimeError((sd / 'rill.log').read_text())
            root = lib.XDefaultRootWindow(d)
            r, parent, n = c.c_ulong(), c.c_ulong(), c.c_uint()
            children = c.POINTER(c.c_ulong)()
            lib.XQueryTree(d, root, c.byref(r), c.byref(parent), c.byref(children), c.byref(n))
            for j in range(n.value):
                lib.XResizeWindow(d, children[j], 1440, 1000)
                lib.XMoveWindow(d, children[j], 0, 0)
                found = True
            if children:
                lib.XFree(children)
            if found:
                break
            time.sleep(.1)
        lib.XFlush(d)
        lib.XCloseDisplay(d)
        if not found:
            raise RuntimeError(f'No native app window appeared for session {i}')
        time.sleep(1)
        subprocess.run(['python3', str(BASE / 'mouse.py'), str(i), 'shot', str(sd / 'mixer.png')], check=True)
        session['processes'] = processes
        (BASE / 'setup-manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
        print(json.dumps({'session':i, 'theme':session['appearance'], 'display':display}), flush=True)
    if sha(manifest['source_project']) != manifest['source_project_sha256']:
        raise RuntimeError('Original project changed during preview launch')
    manifest['status'] = 'two isolated native mixer sessions ready'
    (BASE / 'setup-manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')


if __name__ == '__main__':
    try:
        main()
    except Exception:
        subprocess.run(['python3', str(BASE / 'cleanup.py')], check=False)
        raise
