# Triton Roadmap

Status: active, November 2026.

This document is the single orientation map for every session: **what exists today, how the crates fit together, what we build next, and in what order.**
For exact visual tokens, materials, radii, springs, and resolved board conflicts, see `docs/design-spec.md`. For coding standards, see `docs/rust-conventions.md`. For architecture decisions, see `docs/adr/` (especially ADR-0006 and ADR-0008).

*(Note: The retired iced implementation is archived untouched on branch `archive/iced-shell`. We are architecting Triton cleanly from scratch on `triton-engine`.)*

---

## 1. What Triton is

Triton is a desktop environment built on **niri**. It owns the session from greeter to power-off: the shell surfaces, lock screen, login greeter, notifications, theming, settings app, and niri's configuration.

It does **not** ship its own apps. Files, terminal, editor, and viewers are stock GNOME apps re-skinned (never re-laid out) via generated matugen-style `gtk.css`.

**Scope rule:** if GNOME Shell or `gnome-control-center` does it and niri does not, Triton does it. If a GNOME app does it, Triton does not.

---

## 2. Workspace Architecture (4-Layer Stack)

Strict one-way dependency flow — never invert it:

```
crates/triton-design   (0 runtime deps: tokens, schemes, springs, wave, fonts)
        │
        ▼
crates/triton-engine   (shell-agnostic Wayland + GLES3 UI engine, 0 widgets)
        │
        ▼
crates/triton-kit      (shared Material 3 Futura component library, 0 D-Bus/tokio)
        │
        ▼
Binaries: triton, triton-lock, triton-greeter, triton-settings
```

| Crate | Status | Responsibility | Hard Boundaries |
|---|---|---|---|
| `triton-design` | **Done** (30 tests) | Generated constants from `tokens.json`, `Color`, `Scheme`, `Material`, `Spring`, `Wave`, embedded fonts | Zero runtime dependencies |
| `triton-engine` | **Done** (60 tests) | SCTK 0.21 + calloop + dynamic EGL/GLES3 SDF renderer, Figma 256×256 corner LUT, 9-lobe wave SDF, textured image pass (crossfade/blur/desaturate), `LayerCache` FBOs, `EGL_BUFFER_AGE_KHR` damage, `Animator`, `etagere` LRU glyph atlas, `TimerScheduler`, `StableExtent` | Zero Triton-specific widgets; `unsafe` quarantined to `platform/egl.rs`, `platform/wl_egl.rs`, `gpu/`, and `text/` uploads |
| `triton-kit` | **Scaffolded** | Shared Material 3 Futura components (`panel`, `island`, `button`, `switch`, `slider`, `text_input`, `toggle_tile`, `list_group`, `toast`, `wavy_progress`, etc.) + `showcase` example | 100% safe Rust; zero D-Bus, tokio, or niri IPC; tested headlessly via `DrawList` |
| `triton` | **Phase 1B** | Main shell daemon (bar, panels, OSD, notifications, launcher, clipboard, wallpaper, power, capture, polkit, idle, night light, theming, `triton msg` CLI) | Composes `triton-kit`; never hand-styles |
| `triton-lock` | **Phase 1D** | Separate `ext-session-lock-v1` + PAM binary (crash-isolated from shell) | Reuses `triton-kit` login components; no D-Bus bloat |
| `triton-settings` | **Phase 3** | Floating `xdg-shell` settings app (1100×760, Mica sidebar + Solid pane) | Reuses `triton-kit` controls/lists |
| `triton-greeter` | **Phase 4** | Separate `greetd` login greeter | Reuses `triton-kit` login components |

*(Extract a shared `triton-config` crate only when a second consumer—e.g. `triton-settings`—actually needs it, per `docs/rust-conventions.md` §3.)*

---

## 3. System Stack

One choice per job. Triton talks to each directly (D-Bus, Wayland, niri IPC) and never shells out in hot paths.

| Job | Provider | Triton's part |
|---|---|---|
| Compositor, blur, overview, screencast | niri ≥ 26.04 (`ext-background-effect-v1`, `optional=true` includes) | Generated include files (ADR-0004), niri IPC event stream |
| Session start | `niri-session` + systemd user units | `triton.service` bound to `graphical-session.target` |
| Login / Lock / Auth | `greetd`, `ext-session-lock-v1`, PAM, `fprintd` | `triton-greeter`, `triton-lock`, shared `triton-kit` auth field |
| Privilege prompts | `polkit` | `triton` acts as the polkit authentication agent |
| Audio | PipeWire + WirePlumber | Volume, devices, mic-in-use, OSD |
| Network / Bluetooth | NetworkManager, BlueZ | Toggles, lists, connect/disconnect |
| Power / Battery / Brightness | UPower, `power-profiles-daemon`, `systemd-logind` | Indicators, power saver, brightness, power menu, inhibitors |
| Night light / Idle | `wlr-gamma-control`, `ext-idle-notify-v1` | Gamma ramp + dim → lock → suspend |
| Clipboard / Media / Tray | `wlr/ext-data-control`, MPRIS, StatusNotifierItem | Clipboard history, media pill/popover, tray overflow |
| Notifications | `org.freedesktop.Notifications` | `triton` is the notification server |
| Colors | `material-colors` crate (in-process) | Wallpaper → scheme → shell + GTK3/4 CSS + gsettings + niri colors |
| Fonts | Google Sans Flex, Jost, JetBrains Mono, Material Symbols Rounded | Embedded in `triton-design` (static weights) |

---

## 4. Phased Build Plan

Every milestone ends with `mise run check` green (fmt + clippy `-D warnings` + tests).

### Phase 0 — Foundations ✅ Complete
- ✅ `triton-design`: zero-dep token crate generated from `tokens.json` (30 tests).
- ✅ `triton-engine`: complete Wayland + GLES3 SDF engine with first-class `Animator`, corner LUT, wave SDF, image pass, FBO `LayerCache`, buffer-age damage, `etagere` glyph atlas, `TimerScheduler` (60 tests).
- ✅ Design audit: all 30 boards mapped and conflicts resolved in `docs/design-spec.md`.
- ✅ Workspace cleaned: `triton-kit` extracted, zero `iced` deps anywhere.

### Phase 1A — `triton-kit` Component Library & Visual Runner
Build the complete Material 3 Futura component library in `crates/triton-kit`, backed by headless `DrawList` tests and a live `showcase` gallery:
- **Surfaces & layout:** `panel`, `island`, `card`, `dialog`, `scrim`, `empty_state`, `list_group` (24/6 radii), `scroll_area`, `divider`.
- **Controls:** `button` (4 kinds × 4 sizes + veil), `icon_button`, `chip`, `toggle_tile` (`r36↔r20` morph), `segmented`, `switch`, `checkbox`, `radio`, `slider` (standard, stepped, on-glass).
- **Inputs:** `text_input`, `search_field` (mode wave-disc + keycap hint), `password_field` (masked dots, eye-reveal, submit disc, wrong-password shake, caps-lock chip).
- **Chrome & indicators:** `pill`, `icon_disc`, `wave_avatar`, `wavy_progress`, `wavy_ring`, `level_meter` (3-bar equalizer), `badge`, `keycap`, `hint_bar`, `menu`, `tooltip`, `toast_card`.
- **Isolated visual runner:** Re-introduce a clean nested-niri dev runner (`mise run dev` / `mise run preview`) targeting `triton-kit`'s `showcase` and `triton` so every visual change can be verified on real niri blur/shadows without touching the host session.

### Phase 1B — `triton` Core Daemon, Bar, OSD & Wallpaper
Stand up `crates/triton` cleanly from scratch:
- Single-process `ShellApp` host with declarative surface reconciliation (`(Role, OutputName)` → `SurfaceSpec`), niri IPC event thread, `triton msg` CLI socket, and owned niri config writer (`triton/index.kdl`, `binds.kdl`, `colors.kdl`, `layer-rules.kdl`).
- **Bar:** autohide (2 px reveal edge, 120 ms / 300 ms fullscreen / 600 ms hide), pinned 56 px Mica strip, hidden (`Super+B`), workspaces, focused title (edge-faded), clock, status pill, bell.
- **OSD:** morph-in-place Glass pill (1.2 s hold, pointer-transparent) for volume, brightness, mic, layout, workspace, low battery.
- **Wallpaper & Color Pipeline:** per-output background surface with 200 ms crossfade, adaptive Mica luminance check, `material-colors` extraction → live shell theme + GTK3/4 CSS + `colors.kdl`.
- **Systemd user unit** (`triton.service`) + `--real-services` vs fake dev backend flag.

### Phase 1C — Interactive Panels & Services
- **Services layer** (one `current_thread` tokio runtime, bounded channels, fake + real backends): NetworkManager, BlueZ, UPower, `power-profiles-daemon`, `systemd-logind`, audio (PipeWire/WirePlumber).
- **Quick Settings:** 420 px r32 Glass panel (`Main` header), 2×2 toggle tiles, sliders, in-place Wi-Fi/Bluetooth/Sound sub-pages, empty/offline states.
- **Notifications:** `org.freedesktop.Notifications` server, top-right toasts (max 4, 6 s hover-pause, "N more" pill), grouped Notification Center (440 px), DND, empty state.
- **Launcher:** Thick Glass modal, fuzzy app/window/command (`>`) / math (`=`) search, scope chips, top-hit card, keycap footer, no-results state.
- **Power Menu:** connected button group (focused cell grows to labeled pill), keyboard nav (`←→`, `1–5`, `Enter`, `Esc`).

### Phase 1D — Session Security (`triton-lock`, Polkit, Idle, Night Light)
- **Engine extension:** add `ext-session-lock-v1` surface support to `triton-engine::platform`.
- **`crates/triton-lock`:** separate lock binary using `triton-kit` auth components + PAM, blurred wallpaper, clock, sender-only notifications, wrong-password shake.
- **Polkit agent** (`org.freedesktop.PolicyKit1.AuthenticationAgent`), **idle** (`ext-idle-notify-v1` dim → lock → suspend), **night light** (`wlr-gamma-control`).

**Phase 1 done when:** you can log into `niri-session` and daily-drive Triton end to end.

### Phase 2 — Complete Shell
Media pill + popover (MPRIS), clipboard history (`Super+V`, sensitive masking + 30 s TTL), screenshot overlay (region/window/screen + post-capture preview card), wallpaper bottom-shelf picker + dynamic day/dusk/night sets, SNI tray popover, recording & mic-in-use bar chips, rich notifications (inline reply, progress, call), power countdown wavy ring + `org.gnome.SessionManager` unsaved-work interception, workspace preview popover.

### Phase 3 — Settings (`triton-settings`)
- **Engine extension:** add `xdg-shell` floating toplevel window support to `triton-engine::platform`.
- **`crates/triton-settings`:** 1100×760 floating window, Mica sidebar + Solid pane, global search, live apply, all 1.0 pages (Appearance, Bar & Shell, Windows, Keyboard with conflict detection, Mouse & touchpad, Displays with drag-to-arrange, Notifications, Sound, Network, Bluetooth, Power, Apps, About).

### Phase 4 — Login (`triton-greeter`)
- **`crates/triton-greeter`** under `greetd`: user picker (150 px wave avatar), session & keyboard pickers, fingerprint, power controls, shared wallpaper/scheme.

### Phase 5 — Switcher, Overview & 1.0 Polish
- **Window Switcher:** MRU across workspaces, tap-swap / 120 ms hold, same-app mode, compact tiles (9+), private-window blur, type-to-find (live previews once niri ships `ext-image-copy-capture` toplevel; icon-card fallback otherwise).
- **Overview chrome:** workspace labels, window pills, type-to-launch bar over niri's overview.
- **1.0 gate:** accessibility pass (AT announcements for OSD/toasts, `reduce_motion` sweep), memory & idle-CPU soak.

### After 1.0
Screen recording pipeline, screenshot markup editor, Users page (AccountsService), per-display wallpaper polish, user theme templates.

---

## 5. Known Risks & Open Items

- **Window live previews** depend on a niri release carrying `ext-image-copy-capture` toplevel (merged upstream post-26.04); icon-card fallback is designed in.
- **Audio native deps:** check whether `libpipewire` / `libpulse` are available via `nx` when starting Phase 1C so we don't fall back to shelling out.
- **Light-scheme hexes** in `tokens.json` are hand-picked; regenerate via `material-colors` in Phase 1B and verify contrast against the boards.
- **Blur correctness:** never combine niri's `geometry-corner-radius` with rule-side `blur` (see `docs/research/niri-layer-blur-alpha.md`); `triton-engine` requests shaped blur via `ext-background-effect-v1` directly.
