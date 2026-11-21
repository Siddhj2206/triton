# niri `geometry-corner-radius` + `blur` destroys interior alpha on layer surfaces

Status: fixed, November 2026. This resolves the "MSAA has no
effect on Vulkan" open item in ADR-0007: the jagged paths were never a
renderer, wgpu, MSAA or driver problem. The shipped fix is in
[the fix path](#the-fix-path-ext-background-effect-v1): the islands request
pill-shaped blur through `ext-background-effect`, and the generated rule is
radius + shadow only.

## Symptom

The bar's vector edges — the wave launcher, smoothed corners — came out with
hard, single-pixel steps in the preview, byte-identical whether iced's MSAA
was on or off, and identical across `WGPU_BACKEND=vulkan`, `WGPU_BACKEND=gl`
and `ICED_BACKEND=tiny-skia`. Text and quad borders were smooth, so it looked
exactly like "mesh edges are not antialiased".

## What the experiments showed

1. **The buffers triton submits are correctly antialiased.** A shm buffer was
   dumped straight out of the running tiny-skia process (`/proc/*/fd` memfd):
   the wave's edge is a proper alpha ramp (a ≈ 0.70 → 0.89 → 0.98 → 1.0) and
   the buffer is 98×98 for the 56×56 logical surface (scale 1.75 via
   `wp_fractional_scale_v1` + `wp_viewport` — correct). The hard steps are
   introduced *after* the buffer leaves triton.
2. **Raw wgpu MSAA resolves correctly on radv** (`examples/spike_msaa.rs`,
   offscreen ×4 → ×1 → readback): diagonal edge gets intermediate coverage,
   on `Rgba8Unorm`, `Rgba8UnormSrgb`, and `Bgra8UnormSrgb`, on radv, on GL,
   and on lavapipe.
3. **iced's real MSAA path antialiases on radv** (`examples/spike_wave.rs`,
   the fallback renderer's headless wgpu mode, which hardcodes MSAA ×4): a
   canvas `Path::circle` resolves to graded coverage (0/137/188/225/255).
4. **The jaggedness tracks the niri layer-rule, not the renderer:**

   | `layer-rule` for `triton-bar` | wave edge |
   |---|---|
   | none | smooth |
   | `blur` only | smooth |
   | `geometry-corner-radius` only | smooth |
   | `geometry-corner-radius` + `shadow` | smooth |
   | `geometry-corner-radius` + `blur` | **binary, jagged** |
   | `geometry-corner-radius` + `shadow` + `blur` (shipped rule) | **binary, jagged** |

   Toggling the generated `~/.config/niri/triton/layer-rules.kdl` and touching
   `config.kdl` reloads niri live; the effect flips immediately.

So niri's `geometry-corner-radius` + `background-effect { blur }` combination
on a layer surface re-renders the surface through a path that quantizes its
alpha — every interior edge becomes binary. (`geometry-corner-radius` is
documented to only shape the *shadow* on its own; the wiki notes that with
rule-driven blur it also shapes the blur mask, which is presumably where the
surface gets routed through the effect/clip path. The exact shader mechanism
wasn't pinned down — worth confirming against niri source if we file
upstream.)

## The fix path: `ext-background-effect-v1`

niri 26.04 advertises `ext_background_effect_manager_v1` (the preview's
`wl_registry` lists it). Per niri's own blur docs: *"blur enabled through
`ext-background-effect` will exactly follow the shape requested by the
window"* — clients with their own rounded corners are expected to set a blur
shape through the protocol instead of relying on `geometry-corner-radius`.

Implemented this way, and verified in the preview:

- `ext_background_effect_manager_v1.get_background_effect(wl_surface)` →
  `ext_background_effect_surface_v1`, then `set_blur_region(region)` with the
  pill banded into rectangles (`triton_design::shape::rounded_rect_bands`;
  `wl_region` is rect unions only). `host::blur_region` sends it on create and
  re-sends on resize, since the region is surface-local.
- The generated rule drops `background-effect` and keeps
  `geometry-corner-radius` + `shadow`. Confirmed in the preview: rule blur +
  protocol blur is fine together, the edge stays smooth, and the blur hugs the
  pill — no rectangular halo.

Integration: `iced_layershell` 0.19.1 exposes no `wl_surface`, so the three
vendored patches in `patches/` carry it — `layershellev` owns the `Dispatch`
impls, `iced_layershell` binds the manager at `BindProvide` and handles the
`SetBlurRegion` action, `iced_layershell_macros` maps the message. Worth
upstreaming; drop the patches if it lands. Quickshell, Vicinae, Ghostty and
DankMaterialShell implement the same protocol (niri PR #3483 discussion).

## What this means for the renderer decision

- **Keep wgpu/Vulkan as the default (ADR-0007 stands).** MSAA ×4 works on
  radv — the open item that motivated "maybe GL or analytic AA" is resolved.
  GL offers nothing over Vulkan here (+~28 MB).
- tiny-skia's output was equally affected; its edges were always fine.
- Remaining true jaggedness risk: custom `Primitive` draws (the "Shader" path
  in `iced_wgpu::present`) bypass the MSAA pass by design — if we add custom
  primitives for the smoothed corners, they must do their own AA (SDF or
  alpha-to-coverage), because MSAA will never cover them.

## Diagnostic tools left in the tree

- `crates/triton/examples/spike_msaa.rs` — raw wgpu MSAA probe, no surface:
  `MSAA`, `SPIKE_FORMAT` (rgba8unorm|srgb|bgra|bgra-srgb), `WGPU_BACKEND`,
  `VK_DRIVER_FILES` all honored; prints adapter list and edge coverage.
- `crates/triton/examples/spike_wave.rs` — iced headless `Path::circle` through
  `iced_renderer`'s fallback (`ICED_BACKEND`), prints coverage and can write a
  PPM (`SPIKE_PPM`).
- `crates/triton-ui/tests/wave_edge.rs` — `iced_test` snapshot of the real
  `wave_shape`; `WAVE_SNAPSHOT` sets the output path, `ICED_TEST_BACKEND` the
  renderer (tiny-skia only, since `triton_ui::Renderer` is tiny-skia).
