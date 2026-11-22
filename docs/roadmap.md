# Triton roadmap

Status: draft, November 2026. Source of truth for the look: the **Triton Shell** design canvas (Material 3 Futura, 30 boards, tokens in `project/triton/tokens.json`). This file decides *what* Triton does and *in what order*. The canvas decides how it looks.

## 1. What Triton is

Triton is a desktop environment built on niri. It is not a bar plus some widgets you bolt onto a compositor. It owns the session from greeter to power-off: the shell surfaces, the lock, login, notifications, theming, the settings app, and niri's configuration. It is one Rust/iced codebase and one blessed set of system services.

It does **not** ship its own apps. Files, terminal, editor and viewers are stock GNOME apps, re-skinned (never re-laid out) through matugen-style `gtk.css`, as the Adwaita board shows.

Rule for scope: **if GNOME Shell or gnome-control-center does it and niri does not, Triton probably does it. If a GNOME app does it, Triton does not.**

**Unification (ADR-0006):** one design source (`tokens.json` → `triton-design`), one component set, shared by every process and every generated theme. A surface composes; it never styles.

## 2. The stack

One choice per job. Triton talks to each of these directly (D-Bus, Wayland, niri IPC) and does not shell out in hot paths.

| Job | Provided by | Triton's part |
|---|---|---|
| Compositor, blur, overview, screencast backend | niri ≥ 26.04 (`background-effect`, `optional=true` includes) | Generated include files (ADR-0004), niri IPC event stream |
| Session start | `niri-session` + systemd user units | `triton.service` bound to `graphical-session.target`; XDG autostart via systemd's generator |
| Login | greetd | `triton-greeter` |
| Lock | ext-session-lock-v1 | `triton-lock` |
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
| Colors | `material-colors` crate (the engine matugen uses), in-process | Scheme from wallpaper; writes GTK3/GTK4 CSS, gsettings `color-scheme`, niri colors |
| Fonts | Google Sans Flex, Jost, JetBrains Mono, Material Symbols Rounded | Packaged with Triton |
| X11 apps | xwayland-satellite | Nothing |

**Apps:** Nautilus, Ptyxis or Console (the canvas shows Console), Text Editor, Calculator, Loupe, Papers, Showtime, Decibels, GNOME Software (updates). Users can install anything; these are the ones the theme is tested against.

## 3. Processes

| Binary | What | Why separate |
|---|---|---|
| `triton` | Bar, panels, OSD, notifications, launcher, clipboard, wallpaper, power menu, capture UI, polkit agent, idle, night light, theming | The long-running daemon (ADR-0003, ADR-0007) |
| `triton-lock` | Lock screen | A lock client that crashes must leave the session locked, not take the shell down with it |
| `triton-greeter` | greetd greeter, run in its own niri instance | Runs as the greeter user before login |
| `triton-settings` | Settings app, a normal floating window | Short-lived, opened on demand |
| `triton msg …` | CLI that niri binds call (`triton msg launcher toggle`) | Binds stay plain `spawn` lines in our generated `binds.kdl` |

## 4. Feature set

The canvas boards are the spec. A feature listed here is built to its board. "Later" means it comes after 1.0.

### Bar (`BarStates`, `MediaBar`)
- Two Glass islands. Left: wave launcher button, numbered workspaces, focused app and title. Right: clock, status group, bell.
- Modes: autohide with a 2 px reveal edge (default), pinned full-width Mica with an exclusive zone, hidden (Super+B only). Fullscreen uses a 300 ms dwell instead.
- Status group goes warm or red with state: charging, low battery, offline, DND. Recording and mic-in-use get their own chips.
- Hover popovers: a workspace preview (its window list), background apps (SNI tray), VPN, updates.
- Media pill: a center island that exists only while an MPRIS player is playing. Hover widens it; clicking opens a popover with wavy progress and a player picker.

### Panels
- **Quick settings** (`QuickSettings`): Wi-Fi, Bluetooth, sound output, night light, volume and brightness sliders, DND, power saver, dark style. Chevrons drill into pages in place. "All … settings" opens `triton-settings` on that page.
- **Notifications** (`Notifications`): toasts top right (max 4, hover pauses). The center groups by app. Card types: standard, message with inline reply, critical, progress, incoming call, screenshot. DND. The lock screen shows sender only.
- **OSD** (`OSD`): volume, mute, brightness, mic off, keyboard layout, workspace (from a shortcut only), track change, low battery with a saver action. One at a time, morphing in place.

### Launch and switch
- **Launcher** (`Launcher`): one field for apps, windows, files and commands, with `>` commands, `=` math and `/` files. A top-hit card carries actions. Files come from recently-used and localsearch when it is present.
- **Clipboard** (`Clipboard`, `ClipboardStates`): Super+V. Filters, pins, preview pane, sensitive items masked and wiped, 24 h history, plain-text paste, clear-with-confirm.
- **Window switcher** (`WindowSwitcher`, `WindowSwitcherStates`): MRU across workspaces, tap-swaps without drawing, hold for 120 ms to show. Same-app cycling, compact tiles at 9+ windows, private windows blurred, type-to-find. *Live previews depend on niri window capture (see risks).*
- **Overview** (`Overview`): niri's own overview, with Triton chrome on top: workspace labels, window name pills, the type-to-launch field. We style niri. We do not rebuild it.

### Session
- **Lock** (`Lock`, `LoginStates`): clock, date, next event, hidden-content notifications, avatar wave, password and fingerprint, media controls, status.
- **Greeter** (`Greeter`, `LoginStates`): user picker, session picker, keyboard layout, network, accessibility, power.
- **Power menu** (`Power`, `PowerStates`): Lock is focused first. Updates pending on restart. A 5 s countdown. Unsaved-work interception through inhibitors (we implement the `org.gnome.SessionManager` Inhibit subset GTK apps already call).
- **Polkit agent**: the same field and wave, in a solid dialog.
- **Idle**: dim, then lock, then suspend, respecting inhibitors.

### Appearance
- **Wallpaper** (`Wallpaper`, `WallpaperStates`): a background-layer renderer per output. The picker is a Glass shelf that previews live and reverts on Esc. Sources: Pictures folder, Triton set, dynamic day/dusk/night sets, plain colors. Per-display is optional. A light image offers the light scheme. Adaptive Mica tint on bright wallpapers. Applies to the lock and greeter too.
- **Color pipeline**: wallpaper → scheme → shell theme + GTK4/libadwaita + GTK3 + gsettings + niri, applied live with a toast listing which targets finished.
- **Materials**: Solid, Mica, Glass, Thick Glass. iced paints the tint, niri blurs it through layer and window rules we generate. Fall back to Solid on battery saver.

### Capture (`Screenshot`, `ScreenshotStates`)
- Region, window and screen modes, remembers the last mode, Shift+Print for the full screen. Shell surfaces never appear in the image, and private windows are blacked out. Saves to Pictures/Screenshots and copies. A preview card appears after capture.
- *Later:* screen recording (countdown, audio options, bar chip) and the markup window. Both are real projects of their own: recording is a PipeWire and encoder pipeline, and markup is a small drawing app.

### Settings app (`Settings`, `SettingsStates`)
- iced, built to the board's two-pane adaptive layout: Mica sidebar, solid pane, grouped rows, search across every page, live apply, collapses below 860 px.
- **1.0 pages:** Appearance, Bar & Shell, Windows, Keyboard (layouts + shortcuts with conflict detection), Mouse & touchpad, Displays, Notifications, Sound, Network (Wi-Fi/Ethernet/VPN on/off and lists), Bluetooth, Power, Apps (default apps, autostart), About.
- **Later:** Users (AccountsService: avatar, name, password), deep network editing (hand off to `nm-connection-editor` until then).

### Not in scope
Our own files, terminal, editor or viewer apps. Qt and Kvantum theming, and kitty/zed templates (later, a user template folder can cover these). Disk-unlock UI (that is a Plymouth theme, separate from the Rust code). Sync of any kind. An app store. X11 sessions. Plugins and widgets.

## 5. Phases

Each phase ends with something usable and `mise run check` green.

### Phase 0: Foundations and spikes
- ✅ `triton-design` (ADR-0006): constants generated from `tokens.json` (both schemes, state layers, radii, space, sizes, type scale, materials, shadows, springs, wave path), material fills, closed-form springs with velocity-preserving retarget and reduce-motion fades, wave outline, wavy linear and circular progress. 30 behavior tests.
- ◐ `triton-ui`: a Triton `Theme` that implements iced's widget `Catalog` traits with component classes, so surfaces pick a component variant and cannot style widgets themselves. Done: buttons (4 kinds × 4 sizes, veil, disabled), icon buttons (6 kinds, toggled), chips, nav items, connected groups (incl. large veil on Glass), materials and floating panels, island, pill, card, dialog, grouped list rows, icon disc, key cap, tag, eyebrow, type scale, icons, concentric corners for tight wrappers (`wrap`, outer = inner + gap). `settings()` is the one iced setup every process uses. Gallery snapshot tests in both schemes (`crates/triton-ui/tests/snapshots/`). Also done: switch, checkbox, radio, the slider (our own widget: iced's cannot draw the handle gaps or split radii), search and filled text fields, menus with concentric rows, toast, tooltip label, scrollbar, dividers, wavy linear and circular progress, flat progress, the wave shape. Also done: our own `Panel` and `Pressable` widgets paint every surface and control through one painter (`paint.rs`): the Glass/Thick top highlight, smoothed corners on in-shell shapes (`shape.corner_smoothing` 0.6; floating panels stay circular to match niri), hover and press layers that fade, corners that spring between shapes (press, toggled icon buttons), and a switch thumb that slides and grows. Plain, pill and floating shapes stay iced quads; smoothed outlines are cached paths; widgets request frames only while animating. Our own `Scroll` widget draws the 4 px thumb widening to 10 px on the fast spatial spring while hovered or dragged (iced's scrollable fixes the rail width at build time), with wheel, drag and band-press scrolling; and our `FilledField` wraps the input and paints the label itself so it turns primary on focus and error on error — a sibling widget cannot see the input's focus. `index.rs` reproduces the canvas's Component index board itself (the 1646×1620 table page) with our type scale, colors and spacing; `mise run pixel` renders `ComponentIndex.dc.html` in headless Chromium (dev/boards, 2× like the simulator) and diffs it against the committed snapshot, passing when the share of pixels over a small per-channel tolerance stays under 5% — currently ~2.5%, all of it font rasterization (Skia vs cosmic-text, the tracking iced cannot draw). Still to do: checking the motion live in `mise run dev`. Painting cost is measured with spike 1. Later, only when a surface needs them: tabs, navigation rail, badges, range and centered sliders.
- ◐ `triton` host (ADR-0003): one `iced_layershell` daemon that starts with no surface. A thread follows niri's event stream and hands the app snapshots (outputs are re-queried on `WorkspacesChanged`, since niri sends no output events). `shell::desired` maps outputs to surface specs, `host::plan` turns the difference into create, reconfigure and destroy operations (unit-tested without Wayland), and `SurfaceTable` sends them as layer-shell messages. The bar lives on every output: launcher, live workspaces, focused title, clock, status group and bell — floating islands while the bar is up (modes below). niri blurs and shadows a layer surface as a whole rectangle (checked on the design wallpaper: one full-width surface came out as a frosted band). So each island is its own surface, sized to the island's measured width (`triton_ui::measure`). Each island asks niri for pill-shaped blur through `ext-background-effect` (`host::blur_region`), and the layer rule's `geometry-corner-radius` + `shadow` round and lift it from the tokens — rule-side blur is never combined with the radius because it destroys surface alpha; each rule does carry `xray false` so the client-requested blur sees windows, not just the wallpaper (niri defaults it to xray — the Mica strip keeps that default). Autohide reserves nothing: the islands use exclusive zone −1 so they stay at the edge, and the pinned strip's own zone does the reserving. Verified in `mise run preview` (with `dev/preview/wallpaper.jpg`, the canvas wallpaper). iced_futures needs its `tokio` feature; without a backend, subscriptions silently never run. Modal panels follow the launcher's shape: a `Role::Launcher` Overlay surface (centered, exclusive keyboard, niri rounds and shadows it by layer rule) plus a transparent `Role::Scrim` under it that takes a press anywhere outside as a dismiss; Esc does the same and the field focuses on open. `triton msg launcher toggle` / `notifications toggle` is the command socket niri binds call; the `Desired`/`plan` tests cover surface create/destroy per output. The bell opens the notification center (`Role::Notifications`): a 440 px Glass column at the top-right on the same scrim/overlay plumbing as the launcher, sized to its content by `measure_height` and starting below the bar — `desired` takes per-output bar visibility so the top edge pulls to the output's top once autohide can hide the bar. It carries a live DND toggle (the bell glyph follows it); groups land with the notification server in Phase 1. The bar runs the `BarStates` board: `BarMode::AutoHide` (the default) hides everything but two invisible 2 px `Role::RevealStart`/`RevealEnd` edges, one over each island's footprint — 120 ms of hover wakes that island, 600 ms off its chrome puts it away (`bar()` + `Task::perform` + `tokio::time::sleep`), and the two ends are independent (`bar_shown`/`bar_hover` are keyed by `SurfaceKey`). Nothing is reserved, so windows use the whole screen and the Glass islands float over them. `BarMode::Pinned` maps one square full-width `Role::Strip` (Mica, exclusive zone = its height) instead of the islands. `BarMode::Hidden` maps nothing until `triton msg bar toggle` wakes it — the command also covers `bar mode autohide|pinned|hidden`. The fullscreen 300 ms dwell waits on a fullscreen flag in niri's IPC (v26.4 has none). Status services run through `services::statuses`: one thread per D-Bus service (`zbus::block_on`, no runtime of ours) — UPower's display device for the battery (kind 2 only, warning-level or ≤10 % while discharging counts as low) and NetworkManager's `Connectivity` (full is online; none, portal and limited are offline) — merged into `Statuses` snapshots on a subscription. The status group shows wifi/wifi_off and the battery, and tints by `StatusTone`: blue when all is well, tertiary "rust" when something is off (charging, offline or DND), error red when urgent (low battery). Unless `--real-services` is passed the fake backend runs instead — a dev session must never touch real power or network state and the preview has no system bus; it cycles four canned states so every tone shows. Still to do: real launcher results (the field, scopes, groups and hints render; search is Phase 1).
- ✅ niri include files (ADR-0004): `niri_config::install` writes `triton/index.kdl` and `layer-rules.kdl` (pill clip, shadow and `xray false` for `triton-bar`, `triton-launcher` and `triton-notifications`; blur is client-requested), skipping unchanged files, and appends the one `include optional=true "triton/index.kdl"` line to the user's `config.kdl` when it is missing. That append is the only write the user's file ever gets; every later change — the settings app included — just regenerates the owned files, which niri already watches and reloads. No staged-validate/swap/rollback transaction: `niri validate` stays a dev-time check.
- **Spikes** (each answers yes/no before the phase that needs it):
  1. ✅ (answer: no, not by repainting the whole surface) A full-screen Thick Glass surface with a spring at 2880×1800 @1.75, 90 Hz on tiny-skia: frame time. Measured by `crates/triton/examples/spike_glass.rs` in the preview (now 90 Hz). Numbers are in ADR-0005. The decision that followed is in ADR-0007: the shell renders with wgpu on Vulkan, with tiny-skia as the fallback. In short: one full-screen translucent fill is 13.4 ms on its own, over the 11.1 ms budget. iced's own path made it far worse (an iced shadow on a big panel costs about 200 ms a frame, and every damaged region re-rasterizes every layer that touches it). A still panel with a small moving part held 84 fps at 7 ms. Rules from it: big surfaces take their shadow from niri (`Surface::LayerPanel`), big panels are sized to their content, and only small parts move. Also found: a message a widget publishes during a redraw waits until some other event wakes `iced_layershell`, so frame-driven app state uses `window::frames()`.
  2. ✅ Fonts: cosmic-text renders a variable font's `wght`, but only *selects* a family's face on an exact weight match, and fontdb sees a variable font as one weight. So every family ships as static instances at 400/500/600, and icons as two static fonts (FILL 0 and 1) cut to `icons.txt`. Built by `mise run fonts` (fonttools in a container), embedded by `triton-design::fonts`, about 1.1 MB in all.
  3. ✅ (answer: yes, unpatched) `ext-session-lock` from iced. `iced_sessionlock` 0.19.1 rides our iced 0.14 stack straight from the registry — no vendored patch needed (`default-features = false` drops the mundy D-Bus theme watcher, same as `iced_layershell`). `crates/triton/examples/spike_lock.rs` locks the session, paints our `Theme` on the output, and unlocks itself on a timer. Verified in the preview end to end: niri binds `ext_session_lock_manager_v1`, configures the lock surface to the full output, gives it exclusive keyboard and pointer, and sends `locked`; the `#[to_session_message]` macro's `UnLock` variant maps to `unlock_and_destroy` and niri logs `locking session`/`unlocking session` six seconds apart. Two niri behaviors worth knowing: while locked it renders only lock surfaces (every layer and window vanishes — the spike's screenshot gap proved it, `screenshot-screen` returns success but writes no file), and a lock client that dies without unlocking leaves the session locked, which is why section 2 keeps `triton-lock` a separate process.
  4. ✅ (answer: yes, in the next niri release) niri per-window capture gates switcher previews. #3942 was superseded: `ext-image-copy-capture` for outputs and cursors merged in niri-wm/niri#4554 (Sep 12) and toplevel capture in niri-wm/niri#4599 (Nov 5). Both landed after v26.04 (Apr 25), so they ship in the release after our pinned one. Phase 5 builds icon-card switcher tiles to the same layout and swaps in live previews once a release carries it.

- The dev loop (section 7) lands first, before any surface.

**Done when:** a headless preview renders the Component index board pixel-close.

**Known deviations and deferrals** (reviewed, kept; revisit in the named phase):
- *Board deviations on purpose:* the scrim is transparent (the `Launcher` board's 30% dim was dropped — the panel floats over live windows), and the reveal edge is per-island over each island's footprint, not the board's full-width strip.
- *Roadmap/board conflict:* DND warms the whole status pill (roadmap §4); the `BarStates` board restyles only the bell. Follow the roadmap until the boards are re-cut.
- *Waiting on niri IPC:* urgency-pulse reveals and the fullscreen 300 ms reveal dwell — v26.4's `Window` has no urgent or fullscreen flag.
- *Phase 1:* launcher search and real result groups; `LAUNCHER_SIZE` is fixed at 900×700 until they drive the height; live-motion check in `mise run dev` (hover field label, scrollbar widen, springs).
- *Polish, unscheduled:* focused title hard-clips at `max_width(360)`; the board wants an edge fade. Naming: `triton_ui::Surface` (a paint class) vs. the glossary's layer-shell *Surface*, `button::Size` vs. `iced_core::Size`, and `Status`/`StatusTone`/`Statuses` — rename only if they start confusing. `Measure`/`FilledField`/`Scroll` hand-write iced's child-delegation boilerplate (only `Panel` uses `forward!`); a shared macro is fine if a sixth widget joins.

### Phase 1: Daily-drivable
Bar (all modes, workspaces, title, clock, status, bell), wallpaper renderer and color pipeline, notifications (toasts and center), OSD, quick settings (with pages), launcher (apps, windows, math, commands), lock screen, power menu (basic), polkit agent, idle, night light, systemd unit and generated `binds.kdl`/`rules.kdl`.

- ✅ Launcher engine (`launch/`): `Index` scans XDG data dirs (flatpak roots included), `Hidden` masks same-id shadows, `NoDisplay`/`TryExec`/`Type` filtered, desktop actions parsed. Fuzzy `search` ranks prefix > word-start > substring > subsequence across apps, open windows (niri snapshot) and a `>`-scope shell command palette; `=` forces `math` (`+ - * / % ^`, parens, `×`/`÷`, thousand separators), `/` is recognized but has no backend. Hits spawn detached (`setsid`), `Terminal=true` via `$TERMINAL` or a known emulator; windows focus through `niri::focus_window`; commands run as `triton msg`; math copies via `wl-copy`. The panel renders the board: field (focused on open, Enter takes the top hit), scope chips, top-hit card with the app's actions as buttons, grouped list rows that press to activate, key-hint footer. Verified in the preview: real Foot entries and the palette both render.
- ✅ Notifications (`notify.rs`): the `org.freedesktop.Notifications` server on the session bus (spec 1.2: Notify/Close/GetCapabilities/GetServerInformation plus `NotificationClosed`/`ActionInvoked` signals through a `SignalEmitter` channel). The store groups by app (newest first), toasts expire (default 6 s, never for critical), DND hides non-critical toasts, and the center (`Role::Notifications`, 440 px Glass, content-measured) shows app groups with expand and Clear-all; the bell's badge count and DND toggle are live.
- ✅ OSD (`Role::Osd`): content-measured Overlay surface, pointer-pass-through and non-focusable, holds ~1.2 s past the last change and replaces instead of queueing (`osd_gen` invalidates stale hide timers). Kinds: volume, brightness, mic mute, keyboard layout, low battery (auto on the UPower `low` transition; the board's Saver button is dropped — the surface must stay pass-through), workspace (`Mod+1..9` binds route `triton msg workspace N` → `niri::focus_workspace_idx` + the pill). `triton msg osd volume|brightness|mic|layout|battery` drives it; the volume/brightness/mic binds call through, and mutes toggle both ways.
- ✅ Quick settings (`Role::QuickSettings`): the board's root (2×2 tiles + brightness/volume sliders + footer) and Wifi/Bluetooth/Sound pages, all service-fed. Toggles write back: wifi power + rescan on open, AP rows join networks (`nmcli dev wifi connect`), BT power, device connect/disconnect from rows (battery % in the row), night light, dark/light, power saver, DND, mic mute toggle (hidden when no source). The `… settings` footers are placeholders until the settings app exists (Phase 3).
- ✅ Power menu (`Centered::Power`): five actions on a scrim, arrow/digit/Enter keyboard control, greeting + uptime header. Lock spawns `triton-lock`, suspend/reboot/poweroff go through logind, logout quits niri (`Action::Quit`).
- ✅ Lock screen (`triton-lock` crate): `iced_sessionlock` + `ext-session-lock`, glass veil, huge clock, password field with visibility toggle. Authentication is `libpam` loaded by `libloading` (no `-lpam` link), service `triton-lock` if installed else `login`, the `pam_conv` callback answers echo-off prompts from the field. `--demo` skips PAM for previews. The shell spawns it as a sibling binary or from `PATH` — a crash there cannot take the shell down (roadmap §3); an idle lock or `triton msg lock` fires it. The board's notifications line shows under the clock ("3 new · Chat, Calendar" — the shell leaves the count + senders in `$XDG_RUNTIME_DIR/triton/lock-notifs` at spawn).
- ✅ Polkit agent (`polkit.rs`): the listener serves `org.freedesktop.PolicyKit1.AuthenticationAgent` on the **system** bus (polkitd calls back the registration sender; no well-known name needed) and registers `unix-session:$XDG_SESSION_ID`. `BeginAuthentication` maps to `Centered::Polkit` + scrim; the field's Enter spawns `polkit-agent-helper-1` and runs the `PAM_PROMPT_*` line protocol; only `SUCCESS` sends `AuthenticationAgentResponse2(u s a{sv})`; cancel/kill and Esc clean up `SESSIONS` (a `Pending` reservation wins a race with cancel).
- ✅ Idle + night light (`wayl.rs`): one Wayland side connection owns `ext-idle-notify` (per-seat watches, re-registered on rebind, dead watches reaped) and `zwlr_gamma_control` (linear ramps, Tanner-Helland whitepoint, table written via an unlinked file in `XDG_RUNTIME_DIR`). Three stages on `IDLE_TIMEOUTS` in `app.rs`: 4 min dims through gamma (folds into the same table as the temperature), 5 min locks, 10 min suspends, resume undims and resets. Idle inhibitors are honored at the compositor: niri owns `org.freedesktop.ScreenSaver` (and the Wayland idle-inhibit protocol) and gates `ext-idle-notify` itself — no shell-side service needed. `triton msg nightlight toggle` flips 4500 K ⇄ 6500 K.
- ✅ Services (`services.rs`): `State { statuses, net, bt, audio, brightness, profile }` merged from five worker threads — UPower (battery icon incl. charging/warning/low), NetworkManager (one `GetManagedObjects` pass: wifi on/SSID/APs strongest-first-8, ethernet, vpn, connectivity; signal-burst refetch), BlueZ (adapter power, devices w/ battery), audio (**transitional `wpctl`/`pactl` spawn backend** — see deferrals), backlight (`/sys/class/backlight`, `raw` preferred) + power-profiles-daemon. Commands route real/fake on `static REAL`: `set_wifi`, `wifi_rescan`, `bt_power`, `bt_(dis)connect`, `set_volume` (0–150), `set_(mic_)mute`, `set_brightness` (logind `SetBrightness`, sysfs fallback), `set_profile`, `suspend`/`reboot`/`power_off` via logind. The fake backend keeps the canned laptop + 8 s pill cycle and makes command mutations visible immediately.
- ✅ Wallpaper + colors (`pipeline.rs`): configured `~/.config/triton/wallpaper` else newest image in `~/Pictures` — decode once → `extract_color` → `SchemeTonalSpot` dark + light mapped onto all 26 `Scheme` roles → `apply` writes matugen-style `@define-color` gtk-3.0/4.0 css for the active variant and `gsettings color-scheme`, `niri_config::install` regenerates `colors.kdl`. The `Role::Wallpaper` background surface (pointer-pass-through) shows the image `Cover`; luminance feeds `bright_backdrop` so Mica brightens on light wallpapers; `Variant` flips the live `Theme` and re-renders both file sets on change. `triton msg wallpaper <path>` drives it; boot runs the pipeline for the configured file.
- ✅ niri + systemd: `binds.kdl` (launcher, notifications, quicksettings, bar toggle/mode, lock, power, volume/mute/mic, brightness, workspace `Mod+1..9` — all via `triton msg`), `colors.kdl` from the scheme, `layer-rules.kdl` rounding/shadow on the bar and the floating namespaces (no rule-side blur), `index.kdl` includes the leaves, and the include line is appended to `config.kdl` exactly once. `~/.config/systemd/user/triton.service` (`PartOf`/`WantedBy graphical-session.target`, `Restart=on-failure`) installs on first run with `ExecStart=<the installing binary's absolute path> --real-services` — a real session gets real services and the polkit agent; the fake backend stays a dev/preview opt-in — and a `systemctl --user daemon-reload` runs when the unit or its link changed; `triton-lock` installs as its `Exec` sibling on PATH.
- ◐ Surface plumbing (ADR-0003): `Role::{Wallpaper,Osd,Toasts,QuickSettings,PowerMenu,Polkit}` + `Role::Scrim` under every modal, content-sized for measured panels (`measure`/`measure_height`), `SurfaceSpec` grows `blur_radius` (client-side `ext-background-effect`) and `input_passthrough` (wallpaper, OSD). `shell::ambient` supplies the always-on surfaces; modals merge into `shell::desired`.

**Known deviations and deferrals** (reviewed, kept; named phase to revisit):
- *Audio is `wpctl`/`pactl` spawn, not a PipeWire/Pulse FFI client* — neither library is linkable in the current dev environment (`nx` has no libpulse-devel/libpipewire). `pactl subscribe` provides the watcher. `TODO(pipewire)` in `services.rs`; replace when the deps exist (a decision-board item).
- *Files scope `/`* is classified by `scope()` but has no search backend — the card shows nothing. Phase 2 at the earliest.
- *Brightness watcher:* `backlight` is read once; `set_brightness` pushes a hint through `Update::Brightness`. sysfs has no notification mechanism; poll or inotify later.
- *Toast stable slots* — toasts reflow when one above them expires instead of holding a slot.
- *Fullscreen 300 ms dwell* (BarStates) still waits on a fullscreen flag in niri's IPC (v26.4 has none) — carried over from Phase 0.
- *Multi-seat:* the first `wl_seat` wins idle watches; re-register happens if the seat reappears.
- *Screen-lock D-Bus state:* niri's `org.freedesktop.ScreenSaver` exposes only `Inhibit`/`UnInhibit` (no `GetActive`/`ActiveChanged` — niri issue #4753). Apps that lock-on-screen-blank (Bitwarden, KeePassXC, Chromium) can't see a locked session until niri implements those; track upstream.
- *Polkit retry UX:* a failed authentication closes the dialog (`Event::Cancelled`); a `Failed` event for wrong-password retries needs app support (polkit.rs documents the seam). `PAM_ERROR_MSG`/`TEXT_INFO` are logged, not shown.
- *`OnlyShowIn` ignored* on desktop entries — triton isn't a registered DE name and honoring it would hide apps (documented in `launch/index.rs`).
- *Notification `image-data` hint* (raw pixmap bytes) is ignored; `app_icon`/`image-path` icons only.
- *Wi-Fi join covers known and open networks:* `nmcli dev wifi connect` fails a secured network with no saved profile — the password prompt is the settings app's (Phase 3).
- *`layout` OSD is demo-fed:* `msg osd layout` shows a canned value — niri emits no keyboard-layout events. (Workspace and battery OSDs are real: the `Mod+1..9` binds route through `triton msg workspace`, and UPower's `low` transition fires the battery pill.)
- *Lock-board extras:* next event and media wait for the calendar/media backends (Phases 2–3); the notifications line landed.
- *Ranking deviation:* subsequence beats scattered-subsequence per the launch agent's review (strictly stronger match) — kept; flag if the spec's order was literal.
- *Lock screen veil* uses the `Glass` material rather than the board's tuned scrim alpha — visual only, easy to revisit.
- *Light scheme* remains partially hand-tuned where the engine's tones disagreed with the board (`signal_warn`/`signal_info` keep the reference values; `success_*` GTK vars are approximations of libadwaita defaults — documented in `gtk_css`).

**Done when:** you can log in through GDM or a TTY into `niri-session`, use Triton all day, and never need another shell component.

### Phase 2: Complete shell
Media pill and popover, clipboard history, screenshot (three modes plus the post-capture card), wallpaper picker shelf and dynamic sets, SNI tray, recording and mic chips, notification extras (inline reply, progress, calls), power-state countdown and the unsaved-work flow, bar popovers.

### Phase 3: Settings
`triton-settings` with the 1.0 pages above. The quick-settings hand-off lands on the right page.

### Phase 4: Login
`triton-greeter` under greetd, fingerprint, session picker, wallpaper and scheme shared with the greeter (copied to a system-readable location on change). Triton now owns the session end to end.

### Phase 5: Switch and overview → 1.0
The window switcher (with live previews if spike 4 says yes, else app-icon cards to the same layout), overview chrome, an accessibility pass (screen reader announcements for OSD, reduce motion everywhere), a memory soak against the 80 MB budget. **1.0.**

### After 1.0
Screen recording, markup editor, Users page, per-display wallpaper polish, user theme templates.

## 6. Risks and conflicts to resolve

- **niri `geometry-corner-radius` + `blur` destroys layer-surface alpha** (resolved the "MSAA on Vulkan" scare — MSAA was fine all along): the rule combination binarizes interior alpha on every renderer. Fixed: the generated rule keeps `geometry-corner-radius` + `shadow` only, and the islands request their own pill-shaped blur through `ext-background-effect` (`patches/iced_layershell`'s `SetBlurRegion` action; drop the patch if upstream adds it). Verified in the preview. See `docs/research/niri-layer-blur-alpha.md`.
- **Memory budget** is now about 100 MB for the shell process (ADR-0007), measured in the preview. Re-measure on the real session once the shell runs there.
- **The Settings board says "ship as a real libadwaita app".** This roadmap builds it in iced instead, to keep one stack. It still follows the board's layout.
- **The canvas says "matugen".** We use the same engine (the `material-colors` crate) in-process instead of shelling out. The visible behavior is the same.
- **Placeholder copy in the canvas** (`Bar.qml`, `PanelWindow`, `quickshell` folders in the Main and Materials boards) is mock content only. Triton does not use Quickshell.
- **Window previews** depend on a niri release carrying toplevel capture: merged upstream (niri-wm/niri#4599) but after v26.04, so it is not in our pinned version yet (spike 4).
- **Workspace OSD "only from a shortcut":** niri events don't say what caused a switch. Route through `triton msg` binds so we know.
- **Light-scheme hexes are hand-picked** in the tokens (the canvas README's own known gap). Generate them from the pipeline in Phase 1 and diff against the boards.
- ADRs reference `product.md` and `architecture.md`, which were removed. This roadmap stands in for product.md until those are rewritten.

## 7. Dev loop

No VM. One small rootless podman container (`dev/preview/`, Fedora 45 with niri 26.04, sway, dbus, mesa). niri nests: inside any Wayland session it runs its winit backend as a normal window. Its headless backend (`src/backend/headless.rs`) is only used by niri's own tests and has no CLI flag, so headless runs nest niri inside a headless sway. The container exists because sway ships with file capabilities that `nx` can't unpack, and it also gives isolation for free. Plain podman, not toolbox or distrobox: those share your home, runtime dir and session bus.

Three tiers, cheapest first:

| Tier | Command | What runs | Use for |
|---|---|---|---|
| 1. Widgets | `cargo test -p triton-design` | `iced_test` on tiny-skia, no Wayland | Components, theme, springs; snapshot diffs |
| 2. Nested | `mise run dev` | container: niri as a window on your desktop (host socket mounted) → triton | Interactive work: hover, keys, animation, blur |
| 3. Headless | `mise run preview`, `preview:msg -- …`, `preview:shot`, `preview:stop` | container: headless sway at 2880×1800 → nested niri at scale 1.75 → triton | Agents and CI: drive with `niri msg action …`, capture with niri's own `screenshot-screen` (layer surfaces and blur included) to `target/preview/shot.png` |

`TRITON_CMD=/work/target/debug/triton` sets what niri starts. The repo is mounted read-only at `/work`; only `target/preview` is writable. `target/` is a symlink into the mbx cache, so its real path is mounted read-only too.

Isolation rules (all three tiers enforce them in the mise task, not by convention):

- **Never `niri --session`.** That flag exports the environment to systemd and the session bus.
- **Private runtime dir.** `XDG_RUNTIME_DIR` points at a temp dir, so wayland sockets, the niri socket and triton's own socket can't collide with the real session. The nested tier keeps the host display with an absolute `WAYLAND_DISPLAY=/run/user/$UID/wayland-1`, which libwayland accepts.
- **Private config.** `XDG_CONFIG_HOME` and friends point under `target/preview/home`, so the ADR-0004 writer edits a scratch `niri/config.kdl`, never yours. niri runs with `-c` on that file.
- **Private session bus** (`dbus-run-session`), so triton's notification server, SNI watcher and portal names don't fight the real shell.
- **System-bus services are faked** (`services::statuses(false)` → `follow_fake`): NetworkManager, BlueZ, UPower and logind live on the system bus, and a dev session must never toggle your real Wi-Fi or suspend the laptop. The fake is a runtime default, not a feature — the preview runs the same `target/debug/triton` — and `--real-services` opts into the read-only watchers.

Verified on 2026-11-21: nested niri gets hardware EGL on the AMD iGPU (`/dev/dri` passed through), output reports 2880×1800 @1.75 (logical 1645×1028; niri floors, the canvas rounds to 1646×1029), and a floating translucent window with `background-effect { blur true; xray false }` shows real blur of the window behind in `screenshot-screen` output. Not yet checked: llvmpipe for CI without a GPU. Starts in about a second.
