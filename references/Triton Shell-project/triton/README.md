# Triton design system (Material 3 Futura)

Triton is the niri shell for the Neptuno OS image. Material 3 Futura is its design language: Material 3 Expressive shape and spring language, space-age livery, tasteful blur.

Everything here was extracted from the canvas boards. The boards stay the visual source of truth; these files are the implementable form.

| File | What it is |
| --- | --- |
| `tokens.json` | All tokens: color roles (dark, light), state layers, shape, type, space, sizes, materials, elevation, springs, niri rules, the wave path. Feed this to the iced theme. |
| `tokens.css` | The same as CSS variables, prefix `--t-`. |
| `sync.py` | Injects the two CSS files into every board and normalizes materials. |
| `components.css` | Shell-wide components, prefix `t-`. Reference implementation of every control in the Component index board. |

## Principles
1. **Real estate first.** Tiled windows are square with no gaps. Radius, shadow and blur belong to things that float.
2. **Three materials, one rule.** Solid for content, Mica for chrome, Glass for floating things. No glass on glass. Thick Glass for big panels over busy content.
3. **Color from the wallpaper.** Every role comes from matugen. Primary is action and focus, tertiary is warmth, containers carry state.
4. **The wave is identity.** Launcher button, avatars, album art, swatches, loading. Never decoration on controls.
5. **Springs, not durations.** Spatial springs may overshoot a little. Effects springs never do. Reduce motion swaps both for 120 ms fades.
6. **Re-skin, never re-layout** GNOME and libadwaita apps through `gtk.css` and matugen.

## Using it in iced
- Build one `Theme` struct from `tokens.json` with the 28 color roles for the active scheme. Expose `Theme::glass()`, `mica()`, `solid()` as container styles.
- **Blur is the compositor's job, not iced's.** Draw the translucent fill in iced and let niri blur behind the layer surface: `layer-rule { background-effect { blur true } }` for Mica, add `xray false` for Glass.
- Shell surfaces (bar, panels, OSD, launcher) are layer-shell surfaces. Floating windows are normal windows with an `open-floating` rule.
- The wave is a `canvas::Path` built from `shape.wave.path` (100 x 100 viewBox, scale to size). Wavy progress is a polyline: linear amplitude 3 px, wavelength 24 px; circular 9 lobes, amplitude about 1.1 px at 34 px.
- Shape morph (toggle round to square, press tightens to 12 px) is an animated corner radius on the widget style. Drive it with the spring tokens.
- Icons: Material Symbols Rounded, FILL 0 or 1, wght 500, opsz 24.

## Pages in the canvas
Overview, Foundations, Components, Bar and panels, Launch and switch, Session, Settings and capture.

## Review pass (2026-11-21)
All 30 boards were rendered at real size in Chromium and checked by eye and by script (overflow, clipped text, missing icons, failed images). Contrast was measured on the real wallpaper: blurred and filtered the way each material does it, then tinted.

| Material | Worst case on this wallpaper (on-surface / on-surface-variant / primary) |
| --- | --- |
| Dark Mica .72 | 8.7 / 6.6 / 6.6 |
| Dark Glass .70 (was .62: 5.5 / 4.2 / 4.2) | 6.5 / 4.9 / 4.9 |
| Dark Thick .88 | 9.2 / 7.0 / 7.0 |
| Light Mica .78, no dimming | 10.0 / 5.4 / 3.7 |
| Light Glass .82, no dimming | 9.7 / 5.2 / 3.5 |
| Light Thick .90, no dimming | 11.6 / 6.3 / 4.2 |

Rule from this: in the light scheme, primary is for fills only, never text on a material.

## Consistency pass (2026-11-21)
- **Icon buttons:** one family, `.t-ibtn`, with `filled`, `tonal`, `outlined`, `neutral` (unselected toggle), `veil` (on Glass) and sizes `s` and `l`. Controls, Main, Screenshot, Clipboard and Quick settings now use it.
- **Connected groups:** the Screenshot toolbar uses `.t-seg l veil` (48 px, veil fill on Glass) instead of its own 52 px segments.
- **Chips vs pills vs buttons:** filter chips are `.t-chip` (36 px, 10 px corners), including the Launcher. Non-interactive floating labels (Overview window names, capture hints) are `.t-pill`. The Greeter session and keyboard menus, and the Discard/Copy/Save/Edit actions after a capture, were styled as chips but are buttons, so they are `.t-btn` now.
- **Lists:** the Containers board, which defines lists, now shows the same segmented `.t-li` rows that Settings uses, with `.t-li.on` for selection. Rows sit one tone above what is behind them.
- **The wave stays identity.** App icons in the Launcher results and top hit, the Lock and Notifications toasts, and the compact switcher row moved from waves to circles. Avatars, album art, the launcher button and loading keep the wave. In the compact switcher the selected app goes from a circle to a rounded square: shape carries state.
- **Materials:** 18 panels marked Glass but painted Thick are now `.t-thick`. The clipboard "Clear history" dialog was already solid (no glass on glass); it gained a scrim and shared buttons.
- **Bar:** Media controls rebuilt its bar from Bar states: same launcher wave, numbered workspaces, `21:41 Sat 21 Nov` clock, status pill and bell.
- **Headers:** the six doc boards with a small `Title: subtitle` header use the eyebrow and 72 px Jost title like the rest. Those boards grew 60 px; canvas rows below moved down to keep 120 px gaps.
- **New tokens:** `--t-veil`, `--t-veil-hi`, `--t-mica-adaptive`.

## Known gaps
- **Light scheme hexes are still hand-picked.** Regenerate them on your machine with `matugen image wall.jpg --json hex --dry-run` (tonal-spot) and paste the light roles into `tokens.json` and `tokens.css`.
- **Springs** now use the Material 3 Expressive scheme (spatial 800/0.6, 380/0.8, 200/0.8; effects 3800, 1600, 800 at 1.0). Checked against community ports of the Compose tokens, not Google's own token page. The curves drawn on the Type board were not redrawn for the new stiffness.
- **Component migration (done).** Every board uses the shared classes for icons, materials, labels, buttons, icon buttons, connected groups, chips, switches, list rows and icon containers. The only local controls left are the calculator keys in Main, which are app content inside a re-skinned GNOME app, and one-off layout classes.
- **Placeholder copy:** battery saver saving is `[x] W`. Adaptive Mica (the wallpaper picker's "8% more tint") is now a token: `--t-mica-adaptive` and `material.mica_adaptive` in tokens.json.
- `sync.py` rewrites material colors everywhere in a board, code samples included. Code samples use GTK syntax (`alpha(#191C20, 0.72)`) so they are not touched.
