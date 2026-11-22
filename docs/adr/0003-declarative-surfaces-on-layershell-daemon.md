# 0003. The UI declares surfaces; the host reconciles them on iced_layershell's `daemon`

Status: superseded by ADR-0008 (November 2026). `triton-engine` replaces `iced_layershell` while preserving declarative surface specs.

## Context

The shell needs a bar and wallpaper on every output that survive hotplug, plus panels created when opened and destroyed when closed (product principles 1 and 6). iced_layershell 0.19.1's single-window `application` builds one surface, and `StartMode::AllScreens` does not tell the application which output a window is on. Its multi-window API (`build_pattern::daemon`, `#[to_layer_message(multi)]`, `NewLayerShell` with `output_option: OutputName(..)`, `RemoveWindow`, `*Change` messages, `window::close_events()`) can do it, but `OutputName` falls back silently to the compositor's choice if the output is not yet known to layershellev, and closing the last surface drops the GPU compositor.

## Decision

`triton-ui::Shell::surfaces()` returns the desired `SurfaceSpec`s keyed by `(Role, OutputName)`, free of layer-shell types. The `triton` host runs `daemon` with `StartMode::Background`, a pure `plan(desired, actual) -> Vec<SurfaceOp>`, and a `SurfaceTable` mapping keys to `window::Id`. `Create` uses the macro's `layershell_open` helper (id allocated synchronously), `Reconfigure` uses the narrowest `*Change` message, `Destroy` uses `RemoveWindow`, and a compositor close arrives as `close_events` and is reconciled again. Surfaces are only requested for outputs niri has reported; placement is asserted in the preview with niri's `Request::Layers`. A panel stays listed while it animates closed, and disappears when settled.

## Consequences

- Lifecycle, hotplug and destroy-on-close are a pure function of two sets and are tested without Wayland.
- The layer-shell glue is private to `host/`. If the daemon path proves unreliable, the fallback (`AllScreens` for Bar and Wallpaper, plus an upstream patch) changes only `SurfaceTable`.
- We are tied to iced_layershell's multi-window design and its winit pin (`winit-core`/`winit-common` `=0.31.0-beta.2`). Replacing it with raw sctk would be a rewrite of `host/` and the iced integration.
- Open items for the first spike: behaviour at zero surfaces, unplug during animation, creation before first configure.
