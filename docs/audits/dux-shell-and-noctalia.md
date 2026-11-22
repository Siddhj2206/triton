# Architecture audit: Dux Modular Shell (DMS) + Noctalia

Subagent audits of `/tmp/shell-audit/{dux-shell,noctalia}` (Nov 2026). DMS =
QML UI + Go core monorepo; `noctalia` = the new native C++23 shell (the legacy
QML `noctalia-shell` repo was replaced upstream by the same codebase — only
the native architecture was auditable).

## What both confirm we already do right

- **One daemon owning all surfaces.** Noctalia's `reconcileOutputSurfaces` is
  structurally identical to our reconcile — and theirs is a ~450-line god
  object ours avoids.
- **niri include files** for colors/binds/layer-rules; ours via a Rust writer
  is safer than DMS's QML `sh -c` writes.
- **`triton msg`** ≈ `dms ipc` / `noctalia msg`.
- **Separate `triton-lock` process** — both run `ext-session-lock` in-process
  and both carry fragile "shell crashed → session unlocked" retry machinery
  (DMS `handleLockLost`/3 retries). Ours is strictly better.
- **Worker-threads → snapshot services** beats DMS's mutable-manager+channels
  and Noctalia's raw-pointer service lifetimes (their own ARCHITECTURE.md
  warns about it).
- **In-process Material You pipeline** beats DMS's matugen subprocess +
  FileView watch.

## Steal — done

- `add-wants niri.service` + `Type=notify`/`BusName=` unit (`sd_notify` on ready)
- `optional=true` includes, gated on `niri validate` capability probe
- `mise run install` deploy task
- `triton doctor` diagnostics
- Suspend waits for session-lock *secure* (locked pid file written by
  triton-lock on first mapped surface)
- Persisted settings (`~/.config/triton/settings.toml`: bar mode, DND,
  variant, night — Noctalia's "GUI overrides" layer)
- Toast hover-pause (`mouse_area` enter/exit → `Store::pause_toast`/`resume`)
- Serialized last-wins wallpaper pipeline (`pipeline_busy`/`wallpaper_pending`)
- `triton msg status` JSON dump + request/reply over the msg socket
- **Consume niri where it already serves** (verified live + against
  `niri-ipc` 26.4): real keyboard-layout events
  (`KeyboardLayoutSwitched`/`KeyboardLayoutsChanged` → layout OSD),
  DPMS via `PowerOffMonitors`/`PowerOnMonitors` (`triton msg dpms off|on`),
  `ShowHotkeyOverlay` bound to `Mod+Slash` — no need for our own
  wlr-output-power client or a config-parsing cheatsheet.
- **Removed** `inhibit.rs` after verification: niri owns
  `org.freedesktop.ScreenSaver` itself (Inhibit/UnInhibit only) and gates
  `ext-idle-notify` internally via `is_fdo_idle_inhibited` — inhibitors hold
  every idle stage for free. Serving the API would collide; consuming is
  impossible (no query surface). `doctor` now reports who owns
  Notifications/ScreenSaver on the bus.

## Steal — backlog (next up)

- **3-layer config** (hand config / GUI overrides in state dir / app-owned
  runtime state; section-scoped change sets for hot reload). `settings.toml`
  is the overrides layer; a hand-editable `config.toml` and inotify hot
  reload are still open.
- **One IPC schema** driving `msg` + `--help` + completions; caller-cwd
  forwarding so relative paths resolve against the client.
- **Idle grace fade** (pre-lock warning window) + `dpms-off` → `dpms-on` pair.
- **Toast stable slots** + notification history file.
- **Click-shield surface** for outside-dismiss (DMS `EdgeExclusion`); unload
  heavy panels with a grace timer for memory discipline.
- **`config get/set`** dotted paths over the reply-capable msg socket.
- **Plugin slot design** later — Noctalia's manifest-first (typed entry kinds
  + settings schema + API version, unsandboxed-by-policy) is the reference.
- **PAM helper binary** for the greeter phase (Noctalia splits PAM into a
  minimal setgid-able helper, length-prefixed stdio protocol).

## Don't copy

- In-process session lock; god objects / raw-pointer service lifetimes.
- Two-socket subscribe + capability re-dial dance (DMS).
- Runtime config writes from the UI layer.
- Compositor abstraction tax (they support 8; niri-only is our feature).
