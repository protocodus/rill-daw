# Embedded interface fonts

These unmodified font files are compiled into Rill; no system fonts or network
downloads are required at runtime. Their SIL Open Font License 1.1 notices must
accompany every binary distribution.

| File | Upstream version and source | SHA-256 |
| --- | --- | --- |
| Inter-Regular.ttf | [Inter 4.1, Rasmus Andersson](https://github.com/rsms/inter/releases/tag/v4.1), `extras/ttf` in the official release archive | `40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82` |
| Inter-SemiBold.ttf | Same release | `78a843fade9d4612a5567302fb595b56976eb5fcebf4fea5a5912d638bafcde3` |
| NotoSansCJKjp-Regular.otf | [Noto Sans CJK 2.004, Adobe/Google](https://github.com/notofonts/noto-cjk/tree/Sans2.004), `Sans/OTF/Japanese` | `68a3fc98800b2a27b371f2fb79991daf3633bd89309d4ffaa6946fd587f375b5` |

Inter supplies Latin, Greek and Cyrillic interface text; Noto Sans CJK supplies
Chinese, Japanese and Korean fallback glyphs with Japanese regional Han forms.
The CJK file adds approximately 16 MiB before binary compression. We retain
egui's bundled Hack for aligned numeric readouts and Noto Emoji for monochrome
emoji. Their notices are included by the Cargo dependency license collector.

Font coverage does not imply full Unicode shaping. The current egui renderer
does not promise bidirectional/complex-script shaping, colored emoji, or joined
emoji sequences such as flags, skin tones and family ZWJ sequences. Unsupported
characters use egui's replacement glyph; project strings remain unchanged.

`Inter-OFL.txt` and `NotoSansCJK-OFL.txt` contain the upstream licenses. Release
archives and the macOS application bundle include this directory's Markdown and
license text, alongside dependency notices.
