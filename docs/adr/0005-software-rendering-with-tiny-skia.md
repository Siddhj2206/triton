# 0005. The shell renders in software with tiny-skia, not wgpu

Status: superseded by ADR-0007 (November 2026). Kept for its measurements: spike 1 below is why the shell moved to wgpu.

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
- iced never blurs. The design's Glass and Thick materials do blur, but niri does it (layer and window rules, ADR-0004); iced paints only the translucent tint.
- softbuffer 0.4 gives Wayland buffers with no alpha channel, so transparent pixels came out black (the floating bar's corners). We patch it to use `Argb8888` (`patches/softbuffer`, see `patches/README.md`). Drop the patch when iced moves to a softbuffer with alpha support.

## Update: spike 1, large animated panels (November 2026)

Measured with `crates/triton/examples/spike_glass.rs` in the headless preview (2880×1800 at scale 1.75, 90 Hz, release build, 12-core laptop CPU). A full-screen overlay surface ran four scenarios, each kept in motion by a spring for 6 s:

| Scenario | What moves | CPU per frame | fps |
|---|---|---|---|
| grow | full-screen Thick panel, 60% ↔ 100% | 242 ms (3.1 s with the iced shadow) | 4 |
| slide | full-size panel's content, 240 px | 535 ms | 2 |
| select | a selection tile on a still full-size panel | 7.0 ms | 84 |
| sized | a 720 × 540 panel, 92% ↔ 100% | 22 ms | 40 |

Where the time goes (bisected offline):

- **The floor.** One full-screen translucent fill takes 13.4 ms in tiny-skia (a quarter-screen fill takes 3.5 ms). Any frame that repaints the whole output misses 90 Hz before content is drawn.
- **iced shadows.** A shadow on a big quad costs about 200 ms. Big panels must not draw one.
- **Damage regions multiply the work.** `iced_tiny_skia` redraws each damage region separately. For every layer that touches the region, it clears and refills a full-size clip mask. Then it rasterizes every quad that touches the region in full, with the mask applied. So a full-screen panel is painted once per region, and moving content that splits into many regions (slide) is the worst case.

Decisions:

- A big panel is its own layer surface and takes its shadow from niri (`Surface::LayerPanel`, like the bar islands). niri also clips the blur to its corner radius.
- Big panels are sized to their content, not full-screen with a transparent margin. Their animations move small parts (selection, rows, chevrons), not the whole surface.
- Open and close transitions of big panels must not resize or slide the whole surface every frame. Use a short fade or no transition until one of the options below lands.
- Still open, decide before the launcher and quick settings: (a) patch `iced_tiny_skia` (in `patches/`, like softbuffer) so each damage region rasterizes only its own pixels and the mask is not rebuilt per layer. By the fill rates above, a 720 × 540 panel transition would be about 3 ms. (b) Use wgpu for the process that shows big panels, which costs the memory measured above.
- The wallpaper crossfade is also a full-output repaint (at least 13 ms a frame). Run it at a lower rate or let niri do it.

### Renderers compared (same spike, `--features spike-wgpu`)

The same four scenarios, on a full-screen surface and on a 720 × 540 surface sized to the panel. Memory is in MiB, taken after the scenarios. GTT is GPU memory the driver allocates for the process. On this iGPU it comes out of system RAM, and part of it may also show in RSS when it is CPU-mapped, so PSS + GTT is an upper bound on the real cost.

| Renderer | Surface | PSS | GTT | Worst scenario | CPU per frame |
|---|---|---|---|---|---|
| tiny-skia | panel | 15 | 0 | slide, 74 fps | 1.4 to 9.7 ms |
| tiny-skia | full screen | 35 | 0 | slide, 2 fps | 7 to 498 ms |
| wgpu, Vulkan | panel | 44 | 55 | all 90 fps | 0.6 ms |
| wgpu, Vulkan | full screen | 45 | 122 | all 90 fps | 0.6 ms |
| wgpu, GL | panel | 55 | 65 | all 90 fps | 0.7 ms |
| wgpu, GL | full screen | 56 | 127 | all 90 fps | 0.7 ms |

- wgpu holds 90 fps in every scenario, at under 1 ms of CPU a frame. The cost is about 80 MiB more memory per process with Vulkan, and about 105 MiB with GL. GL is heavier than Vulkan here.
- tiny-skia on a panel-sized surface is close but not there. A selection is fine (1.4 ms). Whole-panel transitions drop frames (5 to 10 ms a frame, 74 to 86 fps), and the per-region repaint described above is still the multiplier.
- A full-screen wgpu surface costs 60 to 70 MiB more GTT than a panel-sized one (its swapchain images). Surfaces sized to content matter for both renderers.
