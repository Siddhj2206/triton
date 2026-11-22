# 0007. The shell renders with wgpu, with tiny-skia as the fallback

Status: superseded by ADR-0008 (November 2026). Kept for its memory measurements and niri blur findings.

## Context

ADR-0005 chose tiny-skia (software rendering) for its memory use. It was measured on a static bar: 14.5 MB RSS against 78 MB for wgpu. The design has since become Glass panels, springs and 90 Hz motion on every surface. Spike 1 (`crates/triton/examples/spike_glass.rs`, numbers in ADR-0005's update sections) measured what that costs at 2880×1800, scale 1.75, 90 Hz:

- On tiny-skia, one full-screen translucent fill costs 13.4 ms, over the 11.1 ms frame budget before any content is drawn. On a panel-sized surface (720 × 540), a moving selection is cheap (1.4 ms), but whole-panel transitions run at 74 to 86 fps (5 to 10 ms a frame).
- On wgpu, every scenario held 90 fps at under 1 ms of CPU a frame, on panel-sized and full-screen surfaces alike.
- Making tiny-skia fast enough would mean patching `iced_tiny_skia`'s damage handling, then fighting every later full-screen effect (wallpaper crossfade, overview chrome, lock screen) the same way.

Shells with animated, translucent chrome (GNOME Shell, Plasma, Quickshell, Noctalia v5) all render on the GPU. The compositor already uses it, so the driver is loaded and the hardware is awake.

## Decision

- `triton` builds iced's wgpu renderer (`iced_renderer` features `wgpu-bare` and `wgpu`) and keeps tiny-skia as the fallback. iced tries wgpu first and uses tiny-skia if no adapter works.
- Vulkan is the default backend: `main` sets `WGPU_BACKEND=vulkan` unless the user set it. Left alone, wgpu would also set up GL, and wgpu's GL backend used 20 to 30 MB more than Vulkan.
- MSAA ×4 is on (`antialiasing: true`). wgpu antialiases paths (the wave, smoothed corners) only through MSAA. Quads and text are smooth without it.
- Spike 1's rules still hold, because they cost memory and frames under wgpu too:
  - Surfaces are sized to their content. A full-screen wgpu surface costs 60 to 70 MB more GPU memory than a panel-sized one, for its swapchain images.
  - niri draws blur and shadows (layer rules, `Surface::LayerPanel`). iced draws no large shadows.
- Snapshot tests stay on tiny-skia (`ICED_TEST_BACKEND=tiny-skia` in `mise.toml`): they are deterministic and need no GPU. `triton-ui` itself depends only on tiny-skia, so its own builds and tests stay small.

## Consequences

- Memory: the bar alone, measured in the preview (release build, MSAA on), uses PSS 47 MB plus 54 MB of GPU memory with Vulkan. On the integrated GPU that GPU memory is system RAM, and some of it may also be counted in RSS, so about 100 MB is an upper bound. With GL it is PSS 59 MB plus 69 MB. The old 80 MB idle target is replaced by about 100 MB for the shell process. `triton-lock` and `triton-greeter` each cost about the same while they run.
- CPU per frame drops from 5 to 500 ms to under 1 ms, so animations no longer have to be designed around damage area.
- **Resolved: the "MSAA has no effect on Vulkan" symptom was niri, not rendering.** The jagged paths came from the `geometry-corner-radius` + `blur` layer-rule combination binarizing the surface's interior alpha — identical on Vulkan, GL and tiny-skia. MSAA ×4 resolves correctly on radv (verified at raw wgpu and iced level; `docs/research/niri-layer-blur-alpha.md`). Vulkan stays the default; the follow-up is routing blur through `ext-background-effect` instead of the niri rule.
- Next, measure idle, then tune: swapchain image count, present mode, and how much the glyph atlas and staging buffers grow.
- softbuffer's alpha patch (`patches/softbuffer`) stays for the fallback.
