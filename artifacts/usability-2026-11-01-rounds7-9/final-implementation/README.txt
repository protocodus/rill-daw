Final implementation snapshot after round 9

This directory is separate from every rated study snapshot. Its executable was
not rated by participants. Round 9 scores remain attached to the original round 9
freeze manifest; later Add send chooser polish receives independent validation.

freeze_final.py performs metadata reads and immutable copies only. It refuses to
overwrite an existing snapshot. The manifest records baseline Git revision,
reconstructible source diff/untracked archive, source and executable hashes,
compiler/profile, validation-summary copies and round 9 preservation checks.

Publication should include only the small source/provenance files. Binaries and
all original study media remain in the local archive.
