Isolated native visual review, separate from all participant rounds and ratings.
Prepared only; launch requires the parent agent's explicit readiness signal.

Launch:
source /workspace/.rill-tools/activate.sh
python3 /workspace/scratch/rill-iteration-preview/launch.py --launch-native-preview

Inspect session-1/mixer.png (Dark) and session-2/mixer.png (Light), then use the
unchanged local mouse.py driver to right-click the visible Pan control and click
123. Capture keypad-expanded.png within the respective session directory.
No playback, project save, project mutation API, or participant session input.

Mouse example (coordinates must come from the actual screenshot):
python3 /workspace/scratch/rill-iteration-preview/mouse.py 1 click X Y 3
python3 /workspace/scratch/rill-iteration-preview/mouse.py 1 shot /workspace/scratch/rill-iteration-preview/session-1/keypad-expanded.png

Cleanup only owned recorded process identities:
python3 /workspace/scratch/rill-iteration-preview/cleanup.py

Original participant project and local project copies have SHA-256 recorded in
prepare-manifest.json. Copied helpers are byte-for-byte identical to round7.
PulseAudio helpers allocate private paths entirely inside these preview sessions
to avoid unpaced null ALSA callbacks. No playback is requested.
