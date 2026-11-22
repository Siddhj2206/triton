# Glossary

Terms from the design (Material 3 Futura) as they are used in code and docs.

- **Scheme**: the 28 color roles for one variant (dark or light). Normally generated from the wallpaper; `Scheme::DARK`/`LIGHT` are the canvas's reference schemes.
- **Material**: how a surface is filled. One of:
  - **Solid**: opaque. For content.
  - **Mica**: translucent, blurs the wallpaper only (niri `xray true`). For chrome: header bars, sidebars, the pinned bar.
  - **Glass**: translucent, blurs the windows below (`xray false`). For floating things: bar islands, menus, OSD, toasts.
  - **Thick** (Thick Glass): Glass at higher opacity. For big modal panels: launcher, switcher, clipboard.
- **Veil**: the faint fill for a control sitting on Glass, Thick or a scrim, in place of a solid container.
- **Island**: a 56 px pill of Glass in the bar. The bar is two islands (three while media plays).
- **Wave**: the nine-lobed identity shape. Used only for identity (launcher button, avatars, album art, swatches, loading), never on ordinary controls.
- **Wavy progress**: a progress indicator whose played part is a sine wave, linear or circular.
- **Spatial spring / effects spring**: motion for position, size and shape (may overshoot) versus color and opacity (never overshoots).
- **Reduce motion**: replaces every spring with a 120 ms linear fade.
- **Surface**: a layer-shell surface the shell owns (bar, panel, OSD). Owned by `triton-engine`'s `platform` host (ADR-0008).
