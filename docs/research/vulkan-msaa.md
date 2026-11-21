# Vulkan MSAA appears to be a no-op (radv), while GL antialiases

Status: **resolved**, November 2026 — the premise was wrong. The jagged edges
were introduced by niri's `geometry-corner-radius` + `blur` layer-rule
path, which binarizes a layer surface's interior alpha on *every* renderer
(wgpu/Vulkan, wgpu/GL and tiny-skia are byte-identical). MSAA works correctly
at every level tested: raw wgpu resolve on radv (unorm and sRGB formats, GL
and lavapipe too), and iced's real MSAA pipeline in headless mode. The
"identical to MSAA-off" observation was true only *after* niri destroyed the
alpha gradations. See `niri-layer-blur-alpha.md` for the experiment matrix and
the `ext-background-effect` fix path. The code archaeology below remains
accurate and is kept for reference.

Related: ADR-0007 ("Open: MSAA has no effect on Vulkan in the preview").

Symptom: `antialiasing: true` in iced settings (→ MSAA ×4) leaves vector paths —
the wave shape, Figma-smoothed corners — jagged on `WGPU_BACKEND=vulkan`
(Mesa radv in a podman container, `/dev/dri` passthrough), pixel-for-pixel
identical to `antialiasing: false`. The ×4 attachment is allocated (+9 MB GPU
memory). The same build on `WGPU_BACKEND=gl` is smooth.

Stack: iced 0.14.0 (`iced_wgpu` 0.14.0, `iced_graphics` 0.14.0,
`iced_renderer` 0.14.0), `iced_layershell` 0.19.1, wgpu 27.0.1
(wgpu-core 27.0.3, wgpu-hal 27.0.4), lyon 1.0 (Cargo.lock).

---

## TL;DR — most likely root cause

Everything iced and wgpu emit on Vulkan is a **textbook, correct MSAA setup**:
an ×4 color attachment, an ×1 resolve attachment wired through
`pResolveAttachments` of a single-subpass render pass, a mesh pipeline with
`rasterizationSamples = 4`, then a 1:1 nearest-sample blit of the resolved
texture to the swapchain. We traced every layer of the code below — there is
**no code path in iced or wgpu that can produce "resolve silently skipped but
output identical to a ×1 render"**. Unsupported configurations (format without
`MULTISAMPLE_RESOLVE`, sample-count mismatches) are hard validation errors,
not silent fallbacks.

Because output is byte-identical to MSAA-off, the ×4 pass must be producing
per-pixel-uniform coverage — i.e. either the driver did not really rasterize
multisampled, or its resolve picks a single sample. **That points below wgpu:
the Mesa/radv build in the container** (version, or a bug in the
MSAA-resolve path — note the precedent, wgpu#725: Mesa had a bug where an
sRGB MSAA resolve silently disabled MSAA; iced renders to an *sRGB* surface
format on both backends).

Second most likely: an environmental wrinkle — e.g. the ICD inside the
container is not actually radv, or the comparison ran through a different
code path than assumed.

**How to verify locally (15 min):** run a minimal wgpu-only MSAA test —
×4 `Rgba8UnormSrgb` texture + ×1 resolve, one diagonal triangle, read back the
resolved pixels (sketch in §5). It needs no surface, so it works headless in
the container. Three outcomes:

- Edge pixels all 0%/100% coverage → **driver bug**; report to Mesa, check
  Mesa version, try `RADV_DEBUG=nodcc`.
- Edge pixels graded but triton still jagged → **app/iced-layer bug**; next
  capture a frame in RenderDoc and check the pass's attachment sample counts.
- Also cheap: `RUST_LOG=info` — `iced_wgpu` logs `Available adapters`,
  `Selected`, `Selected format` and `alpha_mode` at info level
  (`iced_wgpu-0.14.0/src/window/compositor.rs:68-145`).

---

## 1. How iced_wgpu 0.14 sets up MSAA

### Setting plumbing

- `iced_layershell`'s daemon maps `settings.antialiasing: bool` to
  `Some(iced_graphics::Antialiasing::MSAAx4)`:
  `iced_layershell-0.19.1/src/build_pattern/daemon.rs:679-688`
  ([docs.rs source](https://docs.rs/iced_layershell/0.19.1/src/iced_layershell/build_pattern/daemon.rs.html)).
  The identical mapping exists upstream in
  `iced_graphics-0.14.0/src/settings.rs:50`
  (`antialiasing.then_some(Antialiasing::MSAAx4)`).
- `iced_wgpu::window::Compositor::request` passes
  `settings.antialiasing` into `Engine::new`
  (`iced_wgpu-0.14.0/src/window/compositor.rs:180-187`), which builds the
  triangle pipeline (`engine.rs:38-42`).

### The MSAA machinery (`triangle/msaa.rs`)

`triangle::Pipeline::new` (`triangle.rs:275-286`):

```rust
Pipeline {
    msaa: antialiasing.map(|a| msaa::Pipeline::new(device, format, a)),
    solid: solid::Pipeline::new(device, format, antialiasing),
    gradient: gradient::Pipeline::new(device, format, antialiasing),
}
```

The `solid` and `gradient` mesh pipelines get
`multisample: wgpu::MultisampleState { count: 4, mask: !0, alpha_to_coverage_enabled: false }`
(`triangle.rs:600-608`, used at `triangle.rs:786` and `triangle.rs:951`).

`msaa::Pipeline::new` (`msaa.rs:19-132`) builds the "blit" pipeline
(`blit.wgsl`: fullscreen quad sampling a `texture_2d<f32>` through a
`NonFiltering` sampler, premultiplied-alpha blend, `multisample.count = 1`).

`Targets::new` (`msaa.rs:192-251`) allocates, sized to the render region
(targets are shared via `Arc<RwLock>` inside the Pipeline and only ever
grow, `msaa.rs:134-157`):

- `attachment`: `sample_count = 4`, format = **the surface format**,
  `usage = RENDER_ATTACHMENT` (`msaa.rs:206-215`).
- `resolve`: `sample_count = 1`, same format,
  `usage = RENDER_ATTACHMENT | TEXTURE_BINDING` (`msaa.rs:217-227`).
- a bind group pointing at the resolve view (`msaa.rs:235-242`).

### Per-frame flow

1. `triangle::State::prepare` (`triangle.rs:151-213`) calls
   `msaa::State::prepare` (`msaa.rs:300-331`): ensures `Targets`, uploads the
   `u_ratio` uniform (`region_size / targets.size`, used by the blit shader to
   sample the used sub-rect when targets are larger), and returns the mesh
   projection `Transformation::orthographic(targets.size)`.
2. `triangle::render` (`triangle.rs:289-334`): when MSAA is on, it opens
   `msaa::Pipeline::render_pass` — **attachment = ×4 view,
   `resolve_target = ×1 view`, `load = Clear(TRANSPARENT)`,
   `store = Store`** (`msaa.rs:159-181`) — draws every tessellated mesh with
   the ×4 pipelines, ends the pass (resolve executes here), then
   `msaa::State::render` (`msaa.rs:333-369`) draws the blit quad over the
   frame with `LoadOp::Load` + premultiplied blending.
3. This is sandwiched inside `Renderer::render` (`lib.rs:497-531`): the main
   ×1 pass is dropped, `self.triangle.render(...)` runs (MSAA pass + blit),
   then a fresh `Load`-op pass continues with images/text.

So with `antialiasing: true`, path pixels on screen are an **exact 1:1
nearest-sample copy of the resolved texture**. Whatever the resolve produced
is what you see.

### What wgpu does with it on Vulkan

- wgpu-core validates the pair: `color_view.samples` becomes the pass
  `sample_count` (`wgpu-core-27.0.3/src/command/render.rs:1039-1048`);
  `resolve_target` must have `samples == 1`, matching format, and the format
  must carry `TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE`
  (`render.rs:1308-1331`). Violations are **errors, not silent skips**.
- wgpu-hal builds a `VkRenderPass`: color attachment description
  `.samples(TYPE_4)`, resolve attachment `.samples(TYPE_1)`, subpass
  `.resolve_attachments(&resolve_refs)`
  (`wgpu-hal-27.0.4/src/vulkan/device.rs:83-198`), framebuffer = [msaa view,
  resolve view] (`command.rs:814-827`). Pipelines are compiled with
  `.rasterization_samples(TYPE_4)` (`device.rs:2158`); the image itself is
  created `.samples(4)` (`device.rs:728`).
- On GL the same HAL call becomes a multisampled FBO + a
  `ResolveAttachment` command (`glBlitFramebuffer`) at `end_render_pass`
  (`wgpu-hal-27.0.4/src/gles/command.rs:566-570`, `676-683`).

**Conclusion:** on a conformant driver this sequence *must* produce
multisampled coverage. iced/wgpu code paths that could silently degrade to
×1 do not exist — they would be validation errors.

## 2. Where Triton's paths actually render (atlas hypothesis: disproven)

The hypothesis "paths are rasterized into a texture atlas, so framebuffer
MSAA can't help" is **false for canvas geometry**:

- `progress.rs` `draw_wave` → `Frame::fill`; `paint.rs` → `Frame::fill` /
  `Frame::stroke` on `canvas::Cache`.
- `iced_wgpu::geometry::Frame` tessellates fills/strokes with lyon
  (`FillTessellator`/`StrokeTessellator`,
  `iced_wgpu-0.14.0/src/geometry.rs:157-255`) producing `Mesh::Solid` /
  `Mesh::Gradient` (`geometry.rs:469-579`).
- `draw_geometry` routes them via `Layer::draw_mesh_group` /
  `draw_mesh_cache` into `layer.triangles`
  (`iced_wgpu-0.14.0/src/lib.rs:865-897`, `layer.rs:219-243`) → the MSAA pass
  described above.

Caveat that *is* an atlas: **SVG images** (`Image::Vector`) are resvg-
rasterized into `iced_wgpu`'s image atlas (`src/image/vector.rs`,
`src/image/atlas.rs`) and **text** is glyphon-rasterized into a glyph atlas —
MSAA never touches either. If any visibly jagged element is an SVG or text
drawn with `Frame::stroke_text`/`fill_text` at a transform, MSAA can't help
it; the wave and smoothed-corner *paths* are not in that class.

## 3. Why "pixel-identical to MSAA off" is weird, and what it implies

For the final image to equal a ×1 render byte-for-byte, the resolve texture
must contain per-pixel values identical to a ×1 render. In this pipeline that
can only happen if:

- the ×4 attachment wasn't really ×4 (image/pipeline sample mismatch is a
  wgpu validation error — not silent), or
- all 4 samples always got identical coverage — impossible for correct ×4
  rasterization of diagonal/curved edges, or
- the driver's resolve effectively picks a single sample / ignores
  multisampling.

All three live **below wgpu-hal's API-visible behavior**. Precedent that this
class of bug exists in Mesa: [wgpu#725 — "Cannot combine MSAA with sRGB
resolve"](https://github.com/gfx-rs/wgpu/issues/725) (Intel Mesa: MSAA
silently didn't work for sRGB targets; listed in wgpu's
[known driver issues](https://wgpu.rs/doc/src/wgpu/documentation/internals/known_driver_issues.rs.html)).
Note iced deliberately picks an **sRGB surface format** when available
(`GAMMA_CORRECTION` → `formats.find(is_srgb)`,
`compositor.rs:107-115`; on Wayland/radv typically `Bgra8UnormSrgb`), so both
the ×4 attachment and the resolve target are sRGB — the same shape that hit
that Mesa bug family on Intel. There is no published radv bug for plain
Bgra8UnormSrgb subpass resolves, but the container's Mesa build/version is
unverified and could carry a regression or a non-radv ICD.

Secondary possibility: `Targets` is shared (`Arc<RwLock>`) across **all**
renderers of one `Engine` and is recreated whenever a surface's region
doesn't fit the current targets — with several surfaces of different sizes
the MSAA target is reallocated every frame (a perf bug, not a visual one).

## 4. Isolation plan — smallest wgpu-only test

Offscreen (no surface needed → runs headless in the container):

```rust
// cargo add wgpu bytemuck env_logger pollster
// 1. Instance::new(backends: Backends::VULKAN)   // second run: GL
// 2. request_adapter(compatible_surface: None)  -> request_device
// 3. msaa  = create_texture(Rgba8UnormSrgb, 256x256, sample_count=4, RENDER_ATTACHMENT)
//    res   = create_texture(Rgba8UnormSrgb, 256x256, sample_count=1, RENDER_ATTACHMENT|COPY_SRC)
// 4. pipeline: fullscreen-pass vertex shader; fragment returns solid color;
//    draw a big *diagonal* triangle; multisample.count = 4; target = Rgba8UnormSrgb
// 5. pass: view=msaa_view, resolve_target=res_view, Clear+Store
// 6. copy_texture_to_buffer(res) -> map -> scan rows
// 7. Assert: pixels along the diagonal take intermediate values
//    (~25/50/75% for x4). Binary-only values => driver resolve broken.
```

Even simpler, on a session with a window: `cargo run --example msaa-line` in a
wgpu 27 checkout under `WGPU_BACKEND=vulkan` vs `gl` — same test against the
surface path.

Decision tree:

- Minimal test **fails** (binary edges) → driver/env. Actions:
  `vulkaninfo --summary` (confirm `driverName`/Mesa version, ICD path);
  `RADV_DEBUG=nodcc` / `AMD_DEBUG=nodcc` (DCC/MSAA interaction — see Mesa
  envvars docs and `RADV_PERFTEST=dccmsaa` being opt-in);
  try MSAA ×2 (`Antialiasing::MSAAx2`); try a non-sRGB format in the test
  (unorm attachment+resolve) to bisect the sRGB-resolve angle; then file a
  Mesa issue with the renderpass dump.
- Minimal test **passes** → iced/layershell side. Next: RenderDoc capture of
  triton to inspect the actual render pass (attachment samples, resolve
  attachment, pipeline `rasterizationSamples`), and run an iced winit example
  (`solar_system`, `geometry` with `antialiasing: true`) on Vulkan in the
  same session — smooth there would implicate `iced_layershell` or niri's
  surface handling rather than `iced_wgpu`.
- Cheap data point either way: `RUST_LOG=info` — iced logs available/selected
  adapter, selected surface format and alpha mode
  (`compositor.rs:68-145`).

## 5. Why lavapipe is refused (and why tiny-skia kicked in)

Chain:

1. `fallback::Compositor::with_backend` tries wgpu first, tiny-skia second
   (`iced_renderer-0.14.0/src/fallback.rs:267-330`).
2. `iced_wgpu::window::Compositor::request` calls
   `instance.request_adapter` with **`compatible_surface: Some(<the wayland
   surface>)`, `power_preference: HighPerformance`,
   `force_fallback_adapter: false`**
   (`compositor.rs:80-94`).
3. wgpu-core `request_adapter`
   (`wgpu-core-27.0.3/src/instance.rs:446-496`): for each exposed adapter it
   requires `surface.get_capabilities_with_raw(adapter)` to succeed.
   On Vulkan that is `hal::Adapter::surface_capabilities`
   (`wgpu-hal-27.0.4/src/vulkan/adapter.rs:2562-2601`), which calls
   `vkGetPhysicalDeviceSurfaceSupportKHR`. **lavapipe has no Wayland WSI**
   (its only surface support is `VK_EXT_headless_surface`), so the call
   returns `VK_FALSE`/errors → `None` → adapter dropped →
   `RequestAdapterError::NotFound` → iced `Error::NoAdapterFound`
   → `Compositor::Secondary` (tiny-skia).
4. Additional gates, secondary here:
   - `force_fallback_adapter` is **false** — and note it works the other way:
     when *true* it retains only `DeviceType::Cpu` adapters
     (`instance.rs:459-473`). With it false, CPU adapters are still eligible;
     they just sort last under `HighPerformance`
     (Discrete 1 < Integrated 2 < Other 3 < VirtualGpu 4 < Cpu 5,
     `instance.rs:498-530`). Ranking is moot — the surface-compat filter
     removes lavapipe before sorting.
   - wgpu-hal hides adapters reporting `conformance_version.major == 0`
     unless `InstanceFlags::ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER`
     (`adapter.rs:1710-1727`); iced's instance sets no such flag
     (`compositor.rs:55-66`). Modern Mesa lavapipe reports conformant, so
     this shouldn't fire, but on an old/non-CTS build it would also hide it.

So: it is the **surface-compatibility filter**, not `DeviceType` scoring,
required features, or limits — `request_device` is only reached for adapters
that survive, and iced requests `Features::empty()` +
`Limits::default()`/`downlevel_defaults` (`compositor.rs:147-204`).
Lavapipe *can* be used by wgpu for offscreen rendering
(`compatible_surface: None`, optionally `VK_ICD_FILENAMES` pointing at
`lvp_icd.*.json`), but it can never present to a Wayland surface, so for
triton the tiny-skia fallback is the designed outcome.

## Source index (all inspected in the local cargo registry)

- `iced_wgpu-0.14.0/src/settings.rs` — `Settings.antialiasing`.
- `iced_wgpu-0.14.0/src/window/compositor.rs` — adapter request
  (55-94), surface format/alpha pick (98-145), `Engine::new` (180-187),
  surface config (320-339).
- `iced_wgpu-0.14.0/src/triangle.rs` — MSAA pipeline wiring (275-334),
  `multisample_state` (600-608), solid (717-797)/gradient (870-963)
  pipelines, per-mesh pixel snap (407-424).
- `iced_wgpu-0.14.0/src/triangle/msaa.rs` — whole MSAA scheme:
  `Pipeline::new` (19-132), `targets` (134-157), `render_pass` (159-181),
  `Targets::new` (192-251), `State::render` blit (333-370).
- `iced_wgpu-0.14.0/src/shader/blit.wgsl` — resolve→frame blit shader.
- `iced_wgpu-0.14.0/src/geometry.rs` — lyon tessellation of canvas paths.
- `iced_wgpu-0.14.0/src/lib.rs` — main render loop (417-671), `draw_geometry`
  (865-897).
- `iced_wgpu-0.14.0/src/layer.rs` — `draw_mesh_*` → `triangles` (198-243).
- `iced_wgpu-0.14.0/src/image/vector.rs`, `image/atlas.rs` — SVG→resvg→atlas.
- `iced_graphics-0.14.0/src/antialiasing.rs`, `settings.rs:50`,
  `color.rs` (`GAMMA_CORRECTION = true`).
- `iced_renderer-0.14.0/src/fallback.rs` — `Compositor::with_backend`
  wgpu→tiny-skia cascade (267-330).
- `iced_layershell-0.19.1/src/build_pattern/daemon.rs:679-688` — bool→MSAAx4;
  `multi_window.rs` — `C::new` compositor + `configure_surface`/`present`
  (326-338, 499-600).
- `wgpu-core-27.0.3/src/instance.rs:425-560` — adapter enumeration,
  `force_fallback_adapter` filter (459-473), surface-compat filter
  (475-494), DeviceType sort (498-530).
- `wgpu-core-27.0.3/src/command/render.rs` — pass `sample_count` from
  attachment (988-1051), resolve-target validation (1282-1343), HAL pass
  (1414-1428).
- `wgpu-hal-27.0.4/src/vulkan/device.rs` — `make_render_pass` resolve
  attachments (83-204), texture `.samples` (728), pipeline
  `.rasterization_samples` (2158).
- `wgpu-hal-27.0.4/src/vulkan/command.rs:780-836` — attachment/resolve keys.
- `wgpu-hal-27.0.4/src/vulkan/adapter.rs` — `surface_capabilities`
  (2562-2601), llvmpipe/conformance handling (1687-1737).
- `wgpu-hal-27.0.4/src/gles/command.rs:540-604,676-683` — GL MSAA resolve.

External references:

- wgpu#725 — "Cannot combine MSAA with sRGB resolve" (Mesa driver-bug
  precedent): https://github.com/gfx-rs/wgpu/issues/725
- wgpu known driver issues list:
  https://wgpu.rs/doc/src/wgpu/documentation/internals/known_driver_issues.rs.html
- iced #1367 — premultiplied-alpha blit fix (why the blit pipeline blends
  `PREMULTIPLIED_ALPHA_BLENDING`): https://github.com/iced-rs/iced/pull/1367
- iced #834 — canvas arcs AA quality tracking:
  https://github.com/iced-rs/iced/issues/834
- Mesa envvars (`RADV_DEBUG=nodcc`, `RADV_PERFTEST=dccmsaa`):
  https://docs.mesa3d.org/envvars.html
