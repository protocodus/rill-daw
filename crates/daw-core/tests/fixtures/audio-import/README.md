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

Additional container/codec fixtures use the same original `tone.wav`:

```sh
ffmpeg -i tone.wav -map_metadata -1 -c:a pcm_s24be tone.aiff
ffmpeg -i tone.wav -map_metadata -1 -c:a pcm_f32be -f aiff tone.aifc
ffmpeg -i tone.wav -map_metadata -1 -c:a aac -profile:a aac_low -b:a 128k -f adts tone.aac
ffmpeg -i tone.wav -map_metadata -1 -c:a aac -profile:a aac_low -b:a 128k -movflags +faststart tone-aac.m4a
ffmpeg -i tone.wav -map_metadata -1 -c:a alac -sample_fmt s32p -bits_per_raw_sample 24 -movflags +faststart tone-alac.m4a
ffmpeg -i tone.wav -map_metadata -1 -c:a pcm_s24le tone.caf
ffmpeg -i tone.wav -map_metadata -1 -c:a alac -sample_fmt s32p -bits_per_raw_sample 24 tone-alac.caf
```

AIFF, CAF PCM, and ALAC use 24-bit integer samples; AIFF-C uses 32-bit floating
point samples. Both AAC fixtures use AAC-LC, not HE-AAC. The ADTS file includes
encoder priming/padding and reports approximately 0.1493 seconds. FFmpeg writes
an inconsistent CAF ALAC packet table: it declares 8,192 valid frames even though its two packets contain 4,096 + 1,664 = 5,760 decoded frames.
That unmodified file is retained as `tone-alac-invalid-frames.caf`, a negative
fixture. `tone-alac.caf` corrects only the signed 64-bit big-endian valid-frame
count in its `pakt` chunk to 5,760. It changes no encoded audio. The fixture
correction can be reproduced after running the command above:

```python
from pathlib import Path
p = Path("tone-alac.caf")
b = bytearray(p.read_bytes())
p.with_name("tone-alac-invalid-frames.caf").write_bytes(b)
position = 8
while position < len(b):
    size = int.from_bytes(b[position + 4:position + 12], "big", signed=True)
    if b[position:position + 4] == b"pakt":
        b[position + 20:position + 28] = (5760).to_bytes(8, "big")
        break
    position += 12 + size
p.write_bytes(b)
```

The fixtures do not imply every codec is gapless or every malformed encoder
output is accepted.
