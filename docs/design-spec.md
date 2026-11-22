# Material 3 Futura Design Spec

Actionable summary of Triton's visual and interaction spec. Token values are generated from `crates/triton-design/tokens.json` (single source of truth, ADR-0006).

## Resolved Board Conflicts

1. **Shortcuts:** `Super+V` opens Clipboard history; `Super+Shift+V` toggles floating window.
2. **Bar autohide timing:** Reveal delay defaults to `120 ms` (configurable via slider); fullscreen reveal delay is `300 ms`; hide delay is `600 ms`.
3. **Quick Settings panel:** `420 px` wide, `r32` corner radius, using the `Main` header (battery readout + settings, lock, and power icon buttons).
4. **Pinned bar height:** `56 px` (not `32 px`).
5. **Floating bar island offset:** `8 px` top offset everywhere.
6. **Media popover:** Anchored below the media pill with a clean gap; never overlaps the bar pill.

## Materials

Rules: no glass on glass; if text contrast drops below 4.5:1, fall back to Solid; Mica falls back to Solid on battery saver; Mica uses `xray: true` (samples wallpaper only). On bright wallpapers (mean relative luminance > 0.6), Mica adds `+0.08` alpha (dark) / `+0.06` alpha (light), recomputed on wallpaper change.

| Material | Base Fill | Dark (alpha / blur / sat / bright) | Light (alpha / blur / sat / bright) | `xray` | Highlight & Shadow | Use |
|---|---|---|---|---|---|---|
| **Solid** | `surface_container_low..highest` | `1.00` / `0` / — / — | `1.00` / `0` / — / — | — | None | Text, code, lists, media |
| **Mica** | `surface_container_low` | `0.72` / `48` / `1.4` / `0.80` | `0.78` / `48` / `1.2` / `1.05` | `true` | None | Pinned bar, header bars, sidebars, terminal |
| **Glass** | `surface_container_high` | `0.70` / `32` / `1.5` / `0.85` | `0.82` / `32` / `1.3` / `1.05` | `false` | `1 px` top inset (`#fff` 14%), `floating` | Bar islands, Quick Settings, menus, OSD, toasts |
| **Thick Glass** | `surface_container_high` | `0.88` / `40` / `1.4` / `0.80` | `0.90` / `40` / `1.2` / `1.05` | `false` | `1 px` top inset (`#fff` 14%), `floating_large` | Launcher, switcher, clipboard, big modal panels |

Controls sitting on Glass, Thick Glass, or a scrim use Veil fills (`#fff` at `0.10` / `0.16` dark; `#191C20` at `0.06` / `0.10` light) rather than solid containers.

## Elevation & Shadows

| Token | Value | Use |
|---|---|---|
| `floating` | `0 16px 40px rgb(0 0 0 / 0.45)` | Glass surfaces (bar islands, popovers, Quick Settings, OSD) |
| `floating_large` | `0 30px 80px rgb(0 0 0 / 0.55)` | Thick Glass modal panels (launcher, switcher) |
| `raised` | `0 2px 6px rgb(0 0 0 / 0.50)` | Raised controls and thumbs |
| `window_outline` | `0 0 0 1px rgb(255 255 255 / 0.12)` | Subtle outer rim on floating surfaces |

## Shape & Radii

In-shell shapes use Figma-style corner smoothing (`0.6`). Floating panels and windows stay circular to match niri's window corners. The 9-lobe wave is for identity only (launcher button, avatars, album art, swatches, loading).

| Token | Logical px | Use |
|---|---|---|
| `tiled` | `0` | Tiled windows (no gaps, square corners) |
| `xs` | `8` | Compact chips, badges, key caps |
| `sm` | `12` | Small controls, inner cards |
| `md` | `16` | Default cards, toggled icon buttons |
| `lg` | `20` | Medium containers |
| `xl` / `floating_window` | `28` | Floating windows, large panels |
| `r32` (Quick Settings) | `32` | Quick Settings panel container |
| `full` | `9999` | Pills, circles (clamped to half height) |
| `list_outer` / `list_inner` | `24` / `6` | Grouped list outer ends vs. inner row joins |
| `connected_end` / `connected_inner` | `22` / `8` | Connected button group outer ends vs. inner joins |

## Motion & Springs

Springs replace fixed durations. Spatial springs may overshoot slightly; effects springs are critically damped (`1.0`). Under `reduce_motion`, all springs swap to `120 ms` linear fades.

| Token | Stiffness | Damping Ratio | Use |
|---|---|---|---|
| `spatial_fast` | `800` | `0.6` | Snappy control state morphs, micro-interactions |
| `spatial_default` | `380` | `0.8` | Panels opening, bar islands dropping in, layout shifts |
| `spatial_slow` | `200` | `0.8` | Large surface transitions |
| `effects_fast` | `3800` | `1.0` | Quick opacity/color state layers (hover, press) |
| `effects_default` | `1600` | `1.0` | Standard color and opacity transitions |
| `effects_slow` | `800` | `1.0` | Scrims, crossfades |

## Required Empty States Checklist

Include explicit empty states when building each panel:

- [ ] **Notification Center** — no notifications.
- [ ] **Launcher** — no search results matching query.
- [ ] **Quick Settings (Wi-Fi)** — Wi-Fi turned off, and Wi-Fi on with no networks found.
- [ ] **Wallpaper picker** — empty wallpaper folder.
