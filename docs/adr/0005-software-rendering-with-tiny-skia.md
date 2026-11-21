# 0005. The shell renders in software with tiny-skia, not wgpu

Status: accepted, November 2026.

## Context

The memory target is under ~80 MB resident at idle (product.md). iced 0.14 can render with wgpu (GPU) or tiny-skia (CPU, presented through softbuffer). We measured the same bar in the headless preview (release build, one 2560×1440 output, two workspaces, a window title, the clock):

| Renderer | RSS |
|---|---|
| wgpu (Vulkan/GL context, shader pipelines) | 78 MB |
| tiny-skia, with wgpu still compiled in | 15.7 MB |
| tiny-skia only (wgpu feature removed) | 14.5 MB |

The screenshots were pixel-identical at bar scale: rounded corners, text and colours. Idle CPU was zero with both, because iced only redraws when there's a reason to.

## Decision

The `triton` binary enables only iced's `tiny-skia` feature. It does not link wgpu.

## Consequences

- Memory falls by about 5×. The 80 MB budget now leaves room for the wallpaper, panels and services.
- Every redraw costs CPU, in proportion to the damaged area. A bar is cheap. Large animated panels (launcher, control center, dashboard) at 2880×1800 scale 1.75 and 90 Hz are the risk. Measure frame time when the first big panel lands. If it drops frames, the options in order are: smaller surfaces sized to content (already the plan, architecture section 3), fewer animated pixels, then wgpu for that panel's process only.
- The wallpaper is a full-output image. It is a static buffer redrawn only on a change or crossfade, so it suits software rendering. Confirm when it lands.
- `triton-settings` is short-lived and may still choose wgpu. `triton-lock` uses tiny-skia.
- Blur stays off. It wasn't wanted anyway (target-setup.md), and it would be costly on the CPU.
- softbuffer 0.4 gives Wayland buffers with no alpha channel, so transparent pixels came out black (the floating bar's corners). We patch it to use `Argb8888` (`patches/softbuffer`, see `patches/README.md`). Drop the patch when iced moves to a softbuffer with alpha support.
