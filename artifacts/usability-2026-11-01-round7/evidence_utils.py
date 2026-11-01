from pathlib import Path
import json,struct,array,math,statistics,zipfile,hashlib,collections,datetime
BASE=Path(__file__).resolve().parent

def digest(p): return hashlib.sha256(p.read_bytes()).hexdigest()
def stamp(t): return datetime.datetime.fromisoformat(t.replace('Z','+00:00'))

def wav_evidence(p):
 b=p.read_bytes(); assert b[:4]==b'RIFF' and b[8:12]==b'WAVE'; pos=12; fmt=None; payload=None
 while pos+8<=len(b):
  tag=b[pos:pos+4]; size=struct.unpack_from('<I',b,pos+4)[0]; chunk=b[pos+8:pos+8+size]
  if tag==b'fmt ': fmt=chunk
  if tag==b'data': payload=chunk
  pos+=8+size+(size%2)
 codec,channels,sr,rate,align,bits=struct.unpack_from('<HHIIHH',fmt)
 subtype=struct.unpack_from('<I',fmt,24)[0] if codec==65534 else codec
 assert subtype==3 and bits==32,(codec,subtype,bits)
 samples=array.array('f');samples.frombytes(payload)
 finite=all(math.isfinite(x) for x in samples)
 return dict(path=str(p),bytes=len(b),sha256=digest(p),encoding='32-bit IEEE float',sample_rate_hz=sr,channels=channels,frames=len(payload)//align,duration_seconds=len(payload)/rate,all_samples_finite=finite,nonzero_samples=sum(x!=0 for x in samples),peak=max(abs(x) for x in samples),rms=math.sqrt(sum(x*x for x in samples)/len(samples)))

def midi_evidence(p):
 b=p.read_bytes();assert b[:4]==b'MThd';hs=struct.unpack_from('>I',b,4)[0];fmt,ntracks,division=struct.unpack_from('>HHH',b,8);pos=8+hs; counts=[]
 def vlq(buf,j):
  v=0
  while True:
   c=buf[j];j+=1;v=(v<<7)|(c&127)
   if c<128:return v,j
 for ti in range(ntracks):
  assert b[pos:pos+4]==b'MTrk';sz=struct.unpack_from('>I',b,pos+4)[0];t=b[pos+8:pos+8+sz];pos+=8+sz;j=0;tick=0;running=None;ons=[];offs=0;tempos=[];max_tick=0
  while j<len(t):
   delta,j=vlq(t,j);tick+=delta;max_tick=max(max_tick,tick);status=t[j]
   if status>=128:j+=1
   else:assert running is not None;status=running
   if status==255:
    kind=t[j];j+=1;size,j=vlq(t,j);data=t[j:j+size];j+=size
    if kind==81:tempos.append({'tick':tick,'microseconds_per_quarter':int.from_bytes(data,'big')})
    continue
   if status in (240,247):size,j=vlq(t,j);j+=size;continue
   running=status;kind=status>>4;n=1 if kind in (12,13) else 2;data=t[j:j+n];j+=n
   if kind==9 and data[1]>0:ons.append({'tick':tick,'pitch':data[0],'velocity':data[1]})
   elif kind==8 or kind==9 and data[1]==0:offs+=1
  counts.append({'track':ti,'note_on_count':len(ons),'note_off_count':offs,'last_event_tick':max_tick,'tempos':tempos,'pitch_set':sorted({n['pitch'] for n in ons})})
 return {'path':str(p),'bytes':len(b),'sha256':digest(p),'format':fmt,'track_count':ntracks,'ticks_per_quarter':division,'note_on_count':sum(x['note_on_count'] for x in counts),'tracks':counts,'parse_complete':pos==len(b)}

def project_evidence(p):
 with zipfile.ZipFile(p) as z:
  manifest=json.loads(z.read('manifest.json'));project=manifest['project'];assets=z.namelist()
 tracks=project['tracks'];summary=[]
 for t in tracks:
  clips=t['clips'];aud=[c for c in clips if 'Audio' in c['content']];mid=[c for c in clips if 'Midi' in c['content']];stored=0;audible=0;pitchset=set();ends=[]
  for c in mid:
   notes=c['content']['Midi']['notes'];stored+=len(notes);start=c.get('offset',0);end=start+c['length']*c.get('source_rate',1)
   for n in notes:
    pitchset.add(n['pitch'])
    if n['start']<end and n['start']+n['length']>start:audible+=1
  summary.append({'name':t['name'],'kind':t['kind'],'audio_clip_count':len(aud),'midi_clip_count':len(mid),'stored_note_count':stored,'notes_overlapping_clip_source_window':audible,'pitch_set':sorted(pitchset),'instrument':t['instrument'],'end_beat_zero_based':max([c['start']+c['length'] for c in clips],default=0),'audio_assets_present':all(c['content']['Audio']['path'] in assets for c in aud)})
 end=max([x['end_beat_zero_based'] for x in summary],default=0)
 return {'path':str(p),'bytes':p.stat().st_size,'sha256':digest(p),'manifest_version':manifest['version'],'tempo_bpm':project['tempo'],'beats_per_bar':project['beats_per_bar'],'arrangement_end_beat':end,'arrangement_seconds':end*60/project['tempo'],'loop_enabled':project['loop_enabled'],'loop_start':project['loop_start'],'loop_end':project['loop_end'],'audio_clip_count':sum(t['audio_clip_count'] for t in summary),'midi_clip_count':sum(t['midi_clip_count'] for t in summary),'stored_note_count':sum(t['stored_note_count'] for t in summary),'notes_overlapping_clip_source_window':sum(t['notes_overlapping_clip_source_window'] for t in summary),'actual_vst3_assignments':[t['instrument']['Vst3'] for t in summary if isinstance(t['instrument'],dict) and 'Vst3' in t['instrument']],'tracks':summary}

