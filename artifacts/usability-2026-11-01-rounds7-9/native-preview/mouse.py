#!/usr/bin/env python3
"""Mouse-only native input. Held gestures span commands so feedback can be inspected."""
import argparse, ctypes as c, json, os, subprocess, time
from pathlib import Path

p=argparse.ArgumentParser()
p.add_argument('session',type=int,choices=range(1,6))
p.add_argument('action',choices=['click','double','move','down','up','drag','drag_hold','scroll','shot'])
p.add_argument('args',nargs='*')
a=p.parse_args()
base=Path(__file__).resolve().parent
sd=base/f'session-{a.session}'
processes=json.loads((sd/'processes.json').read_text())
display=processes['display'];os.environ['DISPLAY']=display
statefile=sd/'mouse-state.json'
state=json.loads(statefile.read_text()) if statefile.exists() else {'held_button':None}
held=state['held_button'];args=a.args
counts={'click':(2,3),'double':(2,3),'move':(2,2),'down':(2,3),'up':(0,1),'drag':(4,5),'drag_hold':(4,5),'scroll':(3,3),'shot':(0,1)}
lo,hi=counts[a.action]
if not lo<=len(args)<=hi:p.error(f'{a.action} requires {lo}–{hi} arguments')
if held is not None and a.action in ['click','double','down','drag','drag_hold','scroll']:
    p.error(f'Button {held} remains held; use move/shot to inspect, then up before another gesture')
lib=c.CDLL('libX11.so.6');xt=c.CDLL('libXtst.so.6')
lib.XOpenDisplay.argtypes=[c.c_char_p];lib.XOpenDisplay.restype=c.c_void_p
lib.XFlush.argtypes=[c.c_void_p];lib.XSync.argtypes=[c.c_void_p,c.c_int];lib.XCloseDisplay.argtypes=[c.c_void_p]
xt.XTestFakeMotionEvent.argtypes=[c.c_void_p,c.c_int,c.c_int,c.c_int,c.c_ulong]
xt.XTestFakeButtonEvent.argtypes=[c.c_void_p,c.c_uint,c.c_int,c.c_ulong]
d=lib.XOpenDisplay(display.encode())
if not d:raise SystemExit(f'Cannot open {display}')

def move(x,y):
    xt.XTestFakeMotionEvent(d,-1,int(x),int(y),0);lib.XFlush(d)
def button(b,down):
    xt.XTestFakeButtonEvent(d,int(b),int(down),0);lib.XFlush(d)
def click(x,y,b=1):
    move(x,y);time.sleep(.06);button(b,True);time.sleep(.06);button(b,False)

started=time.time()
if a.action in ['click','double']:
    x,y=map(float,args[:2]);b=int(args[2]) if len(args)>2 else 1
    click(x,y,b)
    if a.action=='double':time.sleep(.09);click(x,y,b)
elif a.action=='move':move(*map(float,args[:2]))
elif a.action=='down':
    x,y=map(float,args[:2]);b=int(args[2]) if len(args)>2 else 1
    move(x,y);time.sleep(.06);button(b,True);held=b
elif a.action=='up':
    b=int(args[0]) if args else held or 1
    button(b,False)
    if b==held:held=None
elif a.action in ['drag','drag_hold']:
    x,y,tx,ty=map(float,args[:4]);b=int(args[4]) if len(args)>4 else 1
    move(x,y);time.sleep(.05);button(b,True);time.sleep(.1)
    for i in range(1,31):
        move(x+(tx-x)*i/30,y+(ty-y)*i/30);time.sleep(.018)
    if a.action=='drag':button(b,False)
    else:held=b
elif a.action=='scroll':
    x,y,amount=map(float,args[:3]);move(x,y)
    for _ in range(abs(int(amount))):
        b=4 if amount>0 else 5;button(b,True);button(b,False);time.sleep(.04)
lib.XSync(d,False);time.sleep(.2);lib.XCloseDisplay(d)
state['held_button']=held;statefile.write_text(json.dumps(state)+'\n')
entry={'time':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'started_unix':started,'action':a.action,'args':args,'held_button_after':held}
with (sd/'mouse-actions.jsonl').open('a') as log:log.write(json.dumps(entry)+'\n')
if a.action=='shot':
    path=Path(args[0]) if args else sd/f'screenshot-{time.time_ns()}.png'
    if not path.is_absolute():p.error('Screenshot output must be an absolute path')
    if not path.resolve().is_relative_to(sd.resolve()):p.error('Keep screenshots inside your session directory')
    subprocess.run(['import','-display',display,'-window','root',str(path)],check=True)
    print(path)
else:print(json.dumps(entry))
