These original test tones contain 0.12 seconds (5,760 frames) of stereo audio at
48 kHz: a 440 Hz sine at amplitude 0.4 on the left and an 880 Hz sine at amplitude
0.25 on the right. They contain no third-party recordings. The fixtures are
checked in so tests do not require FFmpeg or network access.

Generation with FFmpeg:

```sh
ffmpeg -f lavfi -i 'aevalsrc=0.4*sin(2*PI*440*t)|0.25*sin(2*PI*880*t):s=48000:d=0.12' -c:a pcm_f32le tone.wav
ffmpeg -i tone.wav -c:a libmp3lame -b:a 128k -write_xing 1 -id3v2_version 3 tone.mp3
ffmpeg -i tone.wav -c:a libmp3lame -b:a 128k -write_xing 0 -id3v2_version 0 -map_metadata -1 tone-cbr-no-tags.mp3
ffmpeg -i tone.wav -c:a flac -sample_fmt s32 -bits_per_raw_sample 24 tone.flac
ffmpeg -i tone.wav -c:a libvorbis -q:a 5 tone.ogg
```
