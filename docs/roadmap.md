# Triton roadmap

Status: draft, November 2026 — rewritten for the `triton-engine` rebuild (ADR-0008). The old iced stack lived in `crates/triton`, `crates/triton-ui`, `crates/triton-lock`; it is preserved untouched on branch **`archive/iced-shell`** and should not be consulted for new work except to salvage logic (launcher search ranking, PAM flow, niri IPC event handling, services D-Bus code).

Source of truth for the look: the **Triton Shell** design canvas (Material 3 Futura, 30 boards). Token values live in `crates/triton-design` (ADR-0006); the resolved board conflicts, materials, shadows, radii and spring tokens are in `docs/design-spec.md`. This file decides *what* Triton does and *in what order*. The canvas decides how it looks.

## 1. What Triton is

Triton is a desktop environment built on niri. It owns the session from greeter to power-off: the shell surfaces, the lock, login, notifications, theming, the settings app, and niri's configuration. It does **not** ship its own apps: files, terminal, editor and viewers are stock GNOME apps, re-skinned (never re-laid out) through matugen-style `gtk.css`.

Rule for scope: **if GNOME Shell or gnome-control-center does it and niri does not, Triton probably does it. If a GNOME app does it, Triton does not.**

## 2. The stack

One choice per job. Triton talks to each of these directly (D-Bus, Wayland, niri IPC) and does not shell out in hot paths.

| Job | Provided by | Triton's part |
|---|---|---|
| **UI engine** | **`triton-engine`** (ADR-0008): SCTK 0.21 + calloop + dynamic EGL/GLES3 SDF + Taffy + cosmic-text/swash | All surfaces, widgets, animation |
| Design tokens | `triton-design` (`tokens.json` → generated constants) | Every crate depends on it; nothing restates design values |
| Compositor, blur, overview, screencast backend | niri ≥ 26.04 (`background-effect`, `optional=true` includes) | Generated include files, niri IPC event stream |
| Session start | `niri-session` + systemd user units | `triton.service` bound to `graphical-session.target` |
| Login | greetd | `triton-greeter` |
| Lock | ext-session-lock-v1 | `triton-lock` (rebuilt on triton-engine; PAM logic salvaged from archive) |
| Auth | PAM, fprintd (optional) | Lock, greeter and polkit agent share one field |
| Privilege prompts | polkit | Triton is the polkit authentication agent |
| Audio | PipeWire + WirePlumber | Volume, devices, mic-in-use, OSD |
| Network | NetworkManager | Wi-Fi, Ethernet, VPN toggles and lists |
| Bluetooth | BlueZ | Power, pair, connect |
| Battery, power profiles | UPower, power-profiles-daemon | Indicators, power saver |
| Brightness, suspend, power-off, inhibitors | systemd-logind | Brightness via `SetBrightness`, power menu |
| Night light | wlr-gamma-control (niri), geoclue for sunset | Triton drives gamma itself |
| Idle | ext-idle-notify-v1 | Dim, lock, suspend timeouts |
| Clipboard | wlr/ext-data-control | History panel |
| Media | MPRIS | Media pill and popover |
| Tray | StatusNotifierItem | Background-apps (⋯) popover |
| Notifications | `org.freedesktop.Notifications` | Triton is the server |
| Portals | xdg-desktop-portal-gnome + -gtk | Nothing; niri implements the Mutter screencast API they use |
| Secrets | gnome-keyring (prompts via gcr) | Unlocked at login through PAM |
| Colors | `material-colors` crate in-process | Scheme from wallpaper → GTK3/4 CSS, gsettings, niri colors |
| Fonts | Google Sans Flex, Jost, JetBrains Mono, Material Symbols Rounded | Shipped in `triton-design` (static instances) |
| X11 apps | xwayland-satellite | Nothing |

## 3. The engine (`crates/triton-engine`, complete)

Everything below exists, is tested headlessly, and is green under `mise run check`. See `src/lib.rs` doc header for the module map and ADR-0008 for the decision.

- **Rendering (`gpu/`)**: GLES3 SDF uber-quad — Figma smooth corners via a 256×256 corner LUT, per-corner radii, inner borders, Glass 1px top highlight, soft drop shadows with translucent-caster cutout, edge-fade masks, the analytic 9-lobe **wave** SDF, textured image quads (rounded/wave clip, Cover/Contain/Fill, 2-source crossfade, 13-tap blur, desaturate), instanced glyphs, offscreen FBO `LayerCache` (`Element::layer(key)`).
- **Animation (`anim/`)**: engine-owned `Animator` keyed by `NodeKey` — scalar/color/corners/layout-rect springs, `Presence` enter/exit, one-shot shake, velocity-seeded springs, auto-pruning, global `reduce_motion`, deterministic `ctx.now`, automatic `wl_surface.frame` pacing (0% idle CPU when settled).
- **Wayland (`platform/`)**: SCTK + calloop multi-surface host; `FitContent`, `StableExtent`, `Fixed`, `Stretch` sizing; `ext-background-effect-v1` blur regions; per-frame input-region updates; `EGL_BUFFER_AGE_KHR` partial damage via `DamageTracker` + `DrawList::diff_bounds`; `TimerScheduler` (schedule/cancel/pause/resume) and `AppCommand::SetReduceMotion`.
- **Text (`text/`)**: cosmic-text + swash; `etagere` LRU glyph atlas with cold-slot eviction.
- **Safety**: `unsafe` only in `platform/egl.rs`, `platform/wl_egl.rs`, `gpu/`, and `text/` uploads. `ui/`, `anim/`, and `crates/triton-kit` are 100% safe Rust.

## 4. Processes

| Binary | What | Why separate |
|---|---|---|
| `triton` | Bar, panels, OSD, notifications, launcher, clipboard, wallpaper, power menu, capture UI, polkit agent, idle, night light, theming | The long-running daemon, built on `triton-engine` |
| `triton-lock` | Lock screen | A lock client that crashes must leave the session locked |
| `triton-greeter` | greetd greeter | Runs as the greeter user before login |
| `triton-settings` | Settings app, a normal floating window | Short-lived, opened on demand |
| `triton msg …` | CLI that niri binds call (`triton msg launcher toggle`) | Binds stay plain `spawn` lines in `binds.kdl` |

## 5. Feature set

The canvas boards are the spec. A feature listed here is built to its board. "Later" means after 1.0. Full surface inventory and the empty-states checklist: `docs/design-spec.md`.

### Bar (`BarStates`, `MediaBar`)
Two Glass islands + media pill. Modes: autohide (2 px reveal edge, 120 ms delay / 300 ms fullscreen dwell / 600 ms hide), pinned 56 px Mica strip, hidden (Super+B). Hover popovers: workspace preview, background apps, tray. Media pill widens on hover, popover on click.

### Panels
- **Quick settings** (420 px, r32, `Main` header): toggles with `r36↔r20` morph, sliders, in-place sub-pages, hand-off to `triton-settings`.
- **Notifications**: toasts top-right (max 4, 6 s hover-paused), grouped center (440 px), card types: standard / message+reply / critical / progress / call / screenshot. DND.
- **OSD**: morph-in-place Glass pill, 1.2 s hold, pointer-transparent; volume/brightness/mic/layout/workspace/track/battery-low.

### Launch and switch
- **Launcher** (`>` commands, `=` math, `/` files, scope chips, top-hit card, keycap footer).
- **Clipboard** (Super+V): filters, pins, preview, sensitive masking + TTL, 24 h retention, plain-text paste.
- **Window switcher**: MRU, tap-swaps, 120 ms hold opens; same-app mode, compact tiles, private-window blur, type-to-find. *Live previews need a niri release carrying toplevel capture (merged post-26.04).*
- **Overview**: niri's own overview + Triton chrome (workspace labels, window pills, search field).

### Session
- **Lock**: clock, notifications (sender only), avatar wave, password + fingerprint, media, status. Wrong-password shake.
- **Greeter**: user picker (150 px wave avatar), session picker, layout, network, a11y, power.
- **Power menu**: connected group whose focused cell grows to a labeled pill; 5 s countdown wavy ring; unsaved-work interception via `org.gnome.SessionManager` Inhibit.
- **Polkit agent**: same field and wave, solid dialog.
- **Idle**: dim → lock → suspend, honoring inhibitors.

### Appearance
- **Wallpaper**: background-layer renderer per output; bottom-shelf picker with live preview + Esc revert; dynamic day/dusk/night sets; per-display optional; adaptive Mica.
- **Color pipeline**: wallpaper → scheme → shell theme + GTK4/libadwaita + GTK3 + gsettings + niri, applied live.
- **Materials**: Solid / Mica / Glass / Thick; niri does the blur (`xray` per `docs/design-spec.md`), engine paints the tint. Solid fallback on battery saver.

### Capture
Region/window/screen modes, 3 s record countdown, post-capture preview card, markup editor (later), screen recording (later).

### Settings app
Floating window, 1100×760, Mica sidebar + solid pane, global search, live apply. 1.0 pages: Appearance, Bar & Shell, Windows, Keyboard, Mouse & touchpad, Displays, Notifications, Sound, Network, Bluetooth, Power, Apps, About.

### Not in scope
Our own files/terminal/editor/viewer apps. Qt/Kvantum theming. Disk-unlock UI (Plymouth). Sync of any kind. App store. X11 sessions. Plugins.

## 6. Phases

### Phase 0 — done
- ✅ `triton-design` tokens, schemes, springs, wave geometry (30 tests).
- ✅ `triton-engine` core (section 3): renderer, animation, platform, text, timers — **complete, `mise run check` green**.
- ✅ Design board audit: all 30 boards mapped; conflicts resolved into `docs/design-spec.md`.

### Phase 1 — `triton-kit` component library + shell daemon, first daily-driver
- `crates/triton-kit` built from scratch on `triton-engine` + `triton-design` (`panel`, `island`, `button`, `icon_button`, `chip`, `toggle_tile`, `segmented`, `switch`, `checkbox`, `radio`, `slider`, `scroll_area`, `text_input`/`search_field`/`password_field`, `list_group`, toast/notification cards, `keycap`, `hint_bar`, `badge`, `wavy_progress`, `wavy_ring`, `level_meter`, `menu`, `dialog`, `empty_state`). The `showcase` example doubles as the live gallery.
- `crates/triton` host daemon architected from scratch on `triton-engine` + `triton-kit` (niri IPC event stream, declarative surface reconciliation, D-Bus services, launcher, notifications, OSD, quick settings, power menu, polkit agent, idle, night light, wallpaper + color pipeline, `binds.kdl`/`layer-rules.kdl` generation, systemd unit).
- `crates/triton-lock` (`ext-session-lock-v1` + PAM, sharing `triton-kit` login components).

**Done when:** you can log into `niri-session` and daily-drive Triton end to end.

### Phase 2 — complete shell
Media pill + popover, clipboard history, screenshot flow + preview card, wallpaper shelf + dynamic sets, SNI tray, recording/mic chips, notification extras (inline reply, progress, calls), power countdown + unsaved-work flow, bar popovers.

### Phase 3 — settings
`triton-settings` with the 1.0 pages.

### Phase 4 — login
`triton-greeter` under greetd; fingerprint; session picker; shared wallpaper/scheme.

### Phase 5 — switch, overview, a11y → 1.0
Window switcher (live previews when niri ships toplevel capture), overview chrome, screen-reader announcements, reduce-motion sweep, memory soak.

## 7. Dev loop

`mise run check` is the gate (fmt + clippy `-D warnings` + tests). Engine surfaces are verified headlessly through `layout_and_paint`/`DrawList` tests and the `showcase` example; the old containerized niri preview was tied to the iced binaries and is being re-scoped — an isolated nested-niri runner for `triton-engine` lands with the first real surface. `mise tasks` lists what exists.

## 8. Risks and open items

- **Window previews** depend on a niri release carrying `ext-image-copy-capture` toplevel (merged post-26.04); icon-card fallback otherwise.
- **Audio backend** was `wpctl`/`pactl` spawning in the archive (no libpipewire-devel under nx); revisit for a real client.
- **Light-scheme hexes** are hand-picked in the tokens; regenerate via the wallpaper pipeline and diff against the boards.
- **Blur correctness**: the `geometry-corner-radius` + rule-side blur alpha bug is documented in `docs/research/niri-layer-blur-alpha.md`; `triton-engine` requests pill-shaped blur via `ext-background-effect` instead — that pattern is final.
