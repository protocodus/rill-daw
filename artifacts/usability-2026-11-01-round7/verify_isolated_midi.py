#!/usr/bin/env python3
"""Render MIDI-only analysis copies with the frozen executable and actual native VST."""
import json
from component_render import BASE,OUT,render_component

def main():
    OUT.mkdir(parents=True,exist_ok=True)
    aggregate=json.loads((BASE/'aggregate.json').read_text());rows=[]
    for participant in aggregate['participants']:
        try:row=render_component(participant,'midi')
        except Exception as error:row={'participant':participant['participant'],'success':False,'error':str(error)}
        rows.append(row);(OUT/'results.json').write_text(json.dumps(rows,indent=2)+'\n')
        print(json.dumps({'participant':row['participant'],'success':row['success'],'peak':row.get('midi_only_render',{}).get('peak'),'error':row.get('error')}),flush=True)

if __name__=='__main__':main()
