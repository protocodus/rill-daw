use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{sync::{Arc,atomic::{AtomicUsize, Ordering}},time::{Duration,Instant}};
fn main() {
 let args:Vec<_>=std::env::args().collect();
 let select=args.get(1).map(String::as_str).unwrap_or("default");
 let frames:u32=args.get(2).map(|v|v.parse().unwrap()).unwrap_or(1024);
 let seconds:u64=args.get(3).map(|v|v.parse().unwrap()).unwrap_or(20);
 let host=cpal::default_host();
 let device=if select=="default" {host.default_output_device().unwrap()} else {host.output_devices().unwrap().find(|d|d.description().is_ok_and(|v|v.name()=="Rill lab paced output")).unwrap()};
 println!("selection={select} device={:?}",device.description().unwrap());
 let mut supported:Vec<_>=device.supported_output_configs().unwrap().filter(|r|r.channels()>0 && r.min_sample_rate()<=48000 && r.max_sample_rate()>=48000).collect();
 supported.sort_by_key(|r|(r.channels()!=2,r.sample_format()!=cpal::SampleFormat::F32));
 let range=supported.first().unwrap();
 println!("supported={range:?}");
 assert_eq!(range.sample_format(),cpal::SampleFormat::F32);
 let mut config=range.with_sample_rate(48000).config();config.buffer_size=cpal::BufferSize::Fixed(frames);
 println!("config={config:?}");
 let calls=Arc::new(AtomicUsize::new(0));let errors=Arc::new(AtomicUsize::new(0));
 let cc=Arc::clone(&calls);let ec=Arc::clone(&errors);let started=Instant::now();
 let stream=device.build_output_stream(config,move|data:&mut[f32],_|{data.fill(0.0);cc.fetch_add(1,Ordering::Relaxed);},move|e|{let count=ec.fetch_add(1,Ordering::Relaxed)+1;if count<=20 {eprintln!("error elapsed={:.6} count={count} kind={:?} raw={e}",started.elapsed().as_secs_f64(),e.kind());}},None).unwrap();
 stream.play().unwrap();
 for i in 1..=seconds {std::thread::sleep(Duration::from_secs(1));println!("tick={i} elapsed={:.6} callbacks={} errors={}",started.elapsed().as_secs_f64(),calls.load(Ordering::Relaxed),errors.load(Ordering::Relaxed));}
 drop(stream);println!("done callbacks={} errors={}",calls.load(Ordering::Relaxed),errors.load(Ordering::Relaxed));
}
