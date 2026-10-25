# RILL identity

A sea-glass teal ribbon forms an R on a charcoal tile, suggesting a flowing stream and an audio signal. The wordmark uses Inter SemiBold with open tracking. Keep the image's transparent padding and proportions intact.

- `rill-icon-source.png`: original generated artwork, 1254 × 1254, with alpha.
- `rill-icon.png`: 1024 × 1024 master for application distribution.
- `rill-icon-256.png` / `rill-icon-64.png`: native window, Linux launcher and compact UI assets.
- `rill.icns`: macOS icon with standard and Retina representations up to 1024 pixels.
- `rill.ico`: Windows icon with 16, 24, 32, 48, 64, 128 and 256 pixel PNG representations.
- `rill-logo-light.svg` / `rill-logo-dark.svg`: self-contained logo lockups, with outlined type and embedded icon; no installed font is needed.
- `logo-concept.png`: generated light-background exploration. The outlined SVGs are the production exports.

The small in-app identity and About view use the shared `daw_ui::brand_lockup` component. Icon textures are cached per egui context; lettering remains native Inter text and follows the active theme. Native window icon data is decoded once. Normal builds use checked-in assets and do not invoke an image generator or conversion tools.

To regenerate sizes, containers and outlined logos on macOS, install Python fontTools and run `python3 scripts/build-brand-assets.py`. This uses system `sips`/`iconutil` and the bundled Inter SemiBold font. It preserves the original generated artwork. The existing font notices in `assets/fonts` apply; the icon art is distributed with Rill. See [PROMPTS.md](PROMPTS.md) for the exact imagegen prompts.

Package scripts verify icon metadata and actual resources on every platform. Windows verifies the embedded PE icon payloads; Linux includes a launcher/icon and an optional per-user installer; macOS declares and embeds the ICNS resource.
