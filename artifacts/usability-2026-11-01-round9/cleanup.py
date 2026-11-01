#!/usr/bin/env python3
"""Stop only recorded recorded process identities; preserve all session evidence."""
from pathlib import Path
import argparse, json, os, signal, time

BASE=Path(__file__).resolve().parent

def identity(pid):
    try:
        fields=Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split()
        return fields[19],fields[0]
    except FileNotFoundError:return None,None

def main():
    p=argparse.ArgumentParser()
    p.add_argument('--apps-only',action='store_true',help='Leave virtual displays and audio servers running')
    args=p.parse_args()
    names=['app'] if args.apps_only else ['app','pulse','xvfb']
    rows=[]
    for i in range(1,6):
        record=BASE/f'session-{i}/processes.json'
        if not record.exists():continue
        processes=json.loads(record.read_text())
        for name in names:
            if name not in processes:continue
            pid=processes[name];expected=processes['process_start_ticks'][name]
            tick,state=identity(pid)
            row={'participant':i,'process':name,'pid':pid}
            if tick is None or state=='Z':row['result']='already exited'
            elif tick!=expected:row['result']='identity mismatch; untouched'
            else:
                try:os.kill(pid,signal.SIGTERM);row['result']='SIGTERM sent'
                except ProcessLookupError:row['result']='already exited'
            rows.append(row)
    deadline=time.monotonic()+5
    while time.monotonic()<deadline:
        live=[r for r in rows if r['result']=='SIGTERM sent' and identity(r['pid'])[1] not in [None,'Z']]
        if not live:break
        time.sleep(.1)
    for row in rows:
        if row['result']=='SIGTERM sent':row['exited']=identity(row['pid'])[1] in [None,'Z']
    report={'at_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'apps_only':args.apps_only,'processes':rows}
    path=BASE/f'cleanup-{time.time_ns()}.json';path.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report,indent=2))

if __name__=='__main__':main()
