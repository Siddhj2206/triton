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

## Steal — backlog (next up)

- **3-layer config** (hand config / GUI overrides in state dir / app-owned
  runtime state; section-scoped change sets for hot reload). Design now while
  we have almost no config; retrofitting is expensive.
- **One IPC schema** driving `msg` + `--help` + completions; caller-cwd
  forwarding so relative paths resolve against the client.
- **Idle grace fade** (pre-lock warning window) + `dpms-off` → `dpms-on` pair.
- **Toast hover-pause + stable slots**; notification history.
- **Serialized last-wins pipeline queue** for wallpaper→scheme re-requests.
- **Click-shield surface** for outside-dismiss (DMS `EdgeExclusion`); unload
  heavy panels with a grace timer for memory discipline.
- **`status` JSON** dump; `config get/set` dotted paths.
- **Plugin slot design** later — Noctalia's manifest-first (typed entry kinds
  + settings schema + API version, unsandboxed-by-policy) is the reference.
- **PAM helper binary** for the greeter phase (Noctalia splits PAM into a
  minimal setgid-able helper, length-prefixed stdio protocol).

## Don't copy

- In-process session lock; god objects / raw-pointer service lifetimes.
- Two-socket subscribe + capability re-dial dance (DMS).
- Runtime config writes from the UI layer.
- Compositor abstraction tax (they support 8; niri-only is our feature).
