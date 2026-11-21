# Patched dependencies

Wired in through `[patch.crates-io]` in the root `Cargo.toml`. Each patch is the
upstream crate with the smallest possible change; drop it when upstream fixes it.

## softbuffer 0.4.8

`src/backends/wayland/buffer.rs`: `wl_shm::Format::Xrgb8888` → `Argb8888`.

iced's tiny-skia renderer (ADR-0005) presents through softbuffer, which always
allocates Wayland buffers without alpha, so transparent pixels (the corners of the
floating bar, any rounded panel) show as black. tiny-skia already writes
premultiplied ARGB, which is exactly what `Argb8888` means, and every Wayland
compositor must support that format. Upstream softbuffer is adding alpha support in a
later release; iced 0.14 pins 0.4.

## layershellev 0.19.1

`src/lib.rs`: `delegate_noop!` for `ExtBackgroundEffectManagerV1` and
`ExtBackgroundEffectSurfaceV1` (wayland-protocols `staging` + `client` were
already enabled). iced_layershell's `BindProvide` hands out the `GlobalList` and
`QueueHandle`, but only `WindowState<T>`'s owner can implement `Dispatch` for
new objects, so the impls live here.

## iced_layershell 0.19.1

`src/actions.rs`, `src/event.rs`, `src/multi_window.rs`: a `SetBlurRegion`
action (same `ActionCallback` shape as `SetInputRegion`) that creates one
`ext_background_effect_surface_v1` per surface and fills the blur region from a
fresh `WlRegion`. The manager is bound in `BindProvide`; the compositor and
`QueueHandle` are stored so actions can build regions later. Needed while
upstream exposes no `wl_surface` or background-effect API; also the niri fix
for docs/research/niri-layer-blur-alpha.md.

## iced_layershell_macros 0.19.1

`src/lib.rs`: `SetBlurRegion` variants and `TryInto` arms in the generated
code — the macro hardcodes every layer action, so a new action has to be added
here too.
