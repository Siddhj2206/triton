# 0008. Owned UI engine on SCTK and GLES3

Status: accepted, November 2026.

## Context

Earlier prototypes used `iced` (first tiny-skia, then wgpu with MSAA ×4). That carried two compounding costs:

- **Footprint:** the bar alone used ~100 MB (47 MB PSS + 54 MB GPU memory) under Vulkan with MSAA ×4. Each additional shell process (`triton-lock`, `triton-greeter`) paid the same baseline.
- **Framework friction:** iced's retained widget tree and redraw model fought the shell's motion and surface design. Spring-animated corner morphs, per-surface shadow gutters, exit animations on unmounting panels, and continuous 90–144 Hz motion all required working around iced rather than expressing them directly.

Two spikes compared alternatives (`t3/spike-custom-sdf-engine` vs `t3/spike-gpui-glass-bar`). An owned Wayland engine built on Smithay's Client Toolkit (SCTK 0.21), calloop, and a GLES3 SDF uber-quad renderer delivered razor-sharp fractional-scale smoothed corners, glass highlights, shadows, and sub-millisecond frames at a fraction of the memory footprint, with no framework impedance mismatch.

## Decision

- **`crates/triton-engine` is Triton's owned UI engine.** It combines:
  - Wayland / layer-shell via SCTK 0.21 and `calloop`.
  - Dynamic EGL + GLES3 SDF rendering via `glow`.
  - Flexbox/grid layout via Taffy.
  - Text shaping and rasterization via `cosmic-text` and `swash`.
  - Design tokens from `triton-design` (ADR-0006 still stands).
- **`unsafe` is strictly quarantined.** All `unsafe` code is confined to `platform/egl.rs`, `platform/wl_egl.rs`, `gpu/`, and texture upload in `text/`. `ui/` and `anim/` (and `crates/triton-kit`) are 100% safe Rust.
- **First-class animation:**
  - An `Animator` registry keyed by `NodeKey` tracks animated values across frames.
  - `Presence` handles enter and exit transitions without dropping unmounting subtrees prematurely.
  - `animate_layout` interpolates layout changes cleanly.
  - Time is passed deterministically via `ctx.now`.
  - Frame pacing binds automatically to `wl_surface.frame` callbacks while animations are active and drops to 0% idle CPU once settled.
  - Global `reduce_motion` swaps springs for 120 ms fades.
- **Headless testing:** `DrawList` snapshot tests replace `iced_test`, verifying engine layout, animation state, and widgets without a Wayland display or GPU.

## Consequences

- Full control over every shell-specific visual primitive: Figma smoothed corners (256×256 LUT), 9-lobe wave SDF/clip, translucent shadow cutouts, FBO layers, `EGL_BUFFER_AGE_KHR` partial damage, and niri `ext-background-effect-v1` blur regions.
- Triton owns its shared component library in `crates/triton-kit` (keeping `triton-engine` strictly shell-agnostic), while `triton-design` remains the single source of truth for colors, materials, shapes, typography, and motion (ADR-0006).
- Memory and startup overhead drop sharply compared to iced + wgpu + MSAA ×4, and no external UI framework upgrade cycle can break layer-shell or animation behavior.
