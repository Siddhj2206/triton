# Software rendering on iced 0.14 — can a patched tiny-skia hold 90 Hz?

Research for the renderer decision in ADR-0005/ADR-0007. Versions under test:
iced_tiny_skia 0.14.1, iced_graphics 0.14.0, tiny-skia 0.11.4, softbuffer 0.4.8
(patched), iced_layershell 0.19.1.

**Verdict up front:** the damage path is *not* architecturally blocked. iced 0.14's
`iced_tiny_skia` already computes correct damage rects and clips all *writes* to
them; the measured blowups come from three local, patchable inefficiencies — a
full-window `memset` per clip-mask update, a per-pixel shadow SDF evaluated over
the entire shadow bounds on every damaged frame, and (probably) a whole-buffer
present that forwards no damage to the compositor. A bounded patch set
(~200–400 lines inside `iced_tiny_skia`, vendored like `patches/softbuffer`)
makes per-frame cost proportional to damaged area. Under the ADR-0007 rules —
content-sized surfaces, niri-owned shadows and blur, small moving parts — a
patched tiny-skia should plausibly hold 90 Hz on panel-sized surfaces (≈3 ms
for a 720×540 @1.75 transition by the measured ~2.6 ns/px fill rate). No CPU
rasterizer fixes the 13.4 ms full-screen fill floor, so the "no animated
full-screen surfaces" rule is permanent for a software path. The bigger
strategic caveat: upstream iced has already deleted tiny-skia on master in
favour of `vello_cpu` — a patched tiny-skia is a dead end beyond iced 0.14,
while iced 0.15's `iced_vello_cpu` ships SIMD + multithreading and a *better*
damage model out of the box.

## 1. How iced_tiny_skia 0.14.1 actually handles damage

Damage is computed in `tiny_skia::window::compositor::present` and rasterized in
`Renderer::draw`. The pipeline (source:
[compositor.rs](https://docs.rs/iced_tiny_skia/0.14.1/src/iced_tiny_skia/window/compositor.rs.html),
[lib.rs](https://docs.rs/iced_tiny_skia/0.14.1/src/iced_tiny_skia/lib.rs.html)):

1. `buffer_mut()` hands back the softbuffer back buffer; `buffer.age()` says how
   many presents ago its contents were last written (double-buffered on Wayland,
   so age is 1–2). `Surface` keeps a `VecDeque<Frame>` of `{background, layers}`
   snapshots indexed by age, `truncate`d to `max_age`.
2. Damage = `graphics::damage::diff(last_frame.layers, renderer.layers(), …)` —
   a *primitive-level diff* against the frame that painted this buffer, added in
   PR [#1811](https://github.com/iced-rs/iced/pull/1811) and reworked by PR
   [#3032](https://github.com/iced-rs/iced/pull/3032) (merged 2025-11-29, in
   0.14.0). If the buffer is fresh or the background changed, damage is the
   whole logical viewport.
3. If damage is empty, the frame snapshot is pushed and **drawing is skipped
   entirely — but `buffer.present()` still runs**.
4. Otherwise damage rects go through `damage::group`, which merges rects whose
   union grows the covered area by ≤ **20,000 logical px²**
   ([graphics/src/damage.rs](https://docs.rs/iced_graphics/0.14.0/src/iced_graphics/damage.rs.html)).
   Scattered moving parts collapse into one bounding rect once they are dense.
5. A `tiny_skia::PixmapMut` is built over the **entire** back buffer
   (`PixmapMut::from_bytes(cast_slice_mut(&mut buffer), w, h)` — stride ==
   width, so no strided sub-views are possible).
6. `Renderer::draw` iterates **each damage rect × each layer**:
   - fills the *damage rect* with the background (`BlendMode::Source`),
   - intersects `damage ∩ layer.bounds` → `layer_bounds`, then calls
     `engine::adjust_clip_mask(clip_mask, layer_bounds)` — `clip_mask.clear()`
     **plus** a rect `fill_path`, on a `Mask` allocated at
     `configure_surface` with `Mask::new(width, height)` — i.e. a
     **full-surface memset** (1 byte/px, ~5 MB at 2880×1800),
   - then walks every quad / primitive group / image / text run in the layer.
     Each `draw_*` early-returns when its physical bounds don't intersect
     `clip_bounds`, but a primitive that *does* intersect is rasterized
     **in full**, with writes limited to the clip mask.
   - Layers whose bounds don't intersect the region are skipped.

So the answer to "damage-sized pixmap or whole-surface repaint" is *neither*:
it is a **full-size pixmap with clip-limited writes**. Blit work is proportional
to damaged area, but per-region fixed costs (mask memset) and per-primitive
rasterization (full shape) are not. Upstream issue
[#3368](https://github.com/iced-rs/iced/issues/3368) measures the mask cost
alone at ~300 MB of `memset` per frame at 1080p for a dense-text app and
documents a fix that resolved it in a real app.

Per-layer diffing detail: `Layer::damage` expands a changed quad's bounds by
`max(|shadow.offset|) + blur_radius` before reporting damage
([layer.rs](https://docs.iced.rs/src/iced_tiny_skia/layer.rs.html)), so a moving
shadowed quad damages its whole shadow footprint.

### Where the ~200 ms shadow lives

`engine::draw_quad` renders `Quad::shadow` by evaluating a **rounded-box SDF +
smoothstep per pixel** over `quad.bounds + offset ± blur_radius` — for every
damaged frame, into a freshly allocated `Vec`, wrapped as a `Pixmap` and
`draw_pixmap`'d under the clip mask
([engine.rs](https://docs.iced.rs/src/iced_tiny_skia/engine.rs.html)). Two
properties make it brutal:

- The SDF loop covers the **entire shadow bounds**, *not* intersected with
  `clip_bounds`. A full-screen shadowed quad intersects every damage region,
  so the whole multi-megapixel SDF is re-evaluated **once per damage region
  per frame** — this is the dominant term behind the spike's 535 ms "slide"
  and the 3.1 s "grow + shadow" measurements in ADR-0005.
- There is **no cache** — a static shadow re-rasterizes whenever any part of
  its quad is damaged.

By contrast, text is fine: glyphs are cached rasterized bitmaps
(`GlyphCache`/`SwashCache`, [text.rs](https://docs.iced.rs/src/iced_tiny_skia/text.rs.html)),
so damaged text re-blits cached glyph pixmaps masked to the region.

## 2. Patch feasibility

All three hot spots are inside `iced_tiny_skia` and can be fixed without
touching iced_graphics' damage model:

| Fix | Where | Size | Effect |
|---|---|---|---|
| Bound the clip-mask clear to the damaged rect | `engine::adjust_clip_mask` — `Mask::data_mut()` is public; memset only the rows of `prev ∪ new` clip rect instead of `clear()` (mask content is only ever one rect) | ~30 lines | Removes the full-surface memset per region×layer×group |
| Clip the shadow SDF to `clip_bounds` | `draw_quad`: iterate `shadow_bounds ∩ clip_bounds`, offset the produced pixmap | ~20 lines | Shadow cost ∝ damaged area instead of full shadow bounds |
| Cache shadow pixmaps | `Engine`: `FxHashMap<(w,h,radii,blur,color), Pixmap>` keyed on physical params | ~50 lines | Static shadows compute once ever |
| Forward damage to the compositor | `present`: `buffer.present_with_damage(&physical_rects)` (softbuffer 0.4 supports it; Wayland → `wl_surface.damage_buffer` on v4+) | ~30 lines | niri recomposites only damaged output; also limits niri's own damage tracking |
| (Optional) Damage-sized render pixmap | `draw`: for small regions, render into a `damage`-sized `Pixmap` with a translated `Transformation`, copy rows back | ~100 lines | Makes *rasterization* cost ∝ region; adds a copy, so only wins for small regions |

A subtler alternative to the mask fix: most `draw_*` calls already skip the mask
entirely when the primitive `is_within(clip_bounds)`; extending that pattern and
only materializing the mask for partially-clipped primitives removes most mask
traffic without any `data_mut` surgery.

Nothing in the architecture blocks this: the damage→layer→primitive loop and
the `clip_bounds` early-outs are already the right shape — the unbounded costs
are implementation details, not the model. The main risks are (a) maintaining a
vendored fork of ~3 files through iced bumps, and (b) upstream deleting the
backend (below), which caps the lifetime of any patch set.

## 3. Alternatives

### vello_cpu / iced_vello_cpu — where upstream is going

- PR [#3306](https://github.com/iced-rs/iced/pull/3306) "Goodbye tiny-skia,
  Hello vello_cpu" replaced the software backend; iced master's
  [Cargo.toml](https://github.com/iced-rs/iced/blob/master/Cargo.toml) now has
  `default = ["wgpu", "vello-cpu", …]` plus a `vello-cpu-multithreading`
  feature, on workspace version `0.15.0-dev`. tiny-skia is gone from the tree.
- `vello_cpu` is Linebender's sparse-strips CPU renderer: SIMD-friendly
  pipeline, optional `rayon` multithreading
  ([crates.io](https://crates.io/crates/vello_cpu),
  [linebender.org blog](https://linebender.org/blog/tmil-19/),
  [LaurenzV Blend2D harness](https://laurenzv.github.io/vello_chart/)). In the
  bunnymark pathological scene vello CPU MT (26 ms) beats Skia's CPU backend
  (273 ms) and approaches GPU paths
  ([vello#1427](https://github.com/linebender/vello/issues/1427)).
- Its compositor ([iced_vello_cpu lib.rs](https://docs.iced.rs/src/iced_vello_cpu/lib.rs.html))
  keeps `frames`/`max_age` like tiny-skia's but additionally computes
  **`buffer_damage`** (vs the frame at buffer age) **and `screen_damage`** (vs
  the newest frame), converts to physical `Rect`s, early-outs when nothing on
  screen changed, and presents **with** damage — a strictly better model than
  0.14's.
- Caveats: PR #3306 initially *removed* damage tracking ("always a bit
  glitchy… may re-introduce") — it was reintroduced before landing. iced 0.15
  is unreleased; iced_layershell 0.19 targets iced 0.14 and would need a bump.
  For Triton this is the natural software path *after* an iced upgrade, not
  something to backport.

### skia-safe (real Skia, CPU backend)

- No `iced_skia` renderer exists — it has only ever been a wish-list item in
  iced's old CONTRIBUTING
  ([example](https://github.com/iced-rs/iced/blob/0.1/CONTRIBUTING.md)).
- Writing one means implementing iced_graphics' full `Renderer` trait stack
  (quads, primitives, text pipeline, compositor impl) — comparable effort to
  iced_tiny_skia itself (~2.5k lines), for a rasterizer that is only ~20–100%
  faster than tiny-skia on x86-64 per the
  [tiny-skia README](https://github.com/linebender/tiny-skia/), plus a heavy
  C++ build. Not competitive with patching (~400 lines) or vello_cpu.

### femtovg

- GPU-only: OpenGL ES 2/3 and wgpu backends, no CPU renderer
  ([femtovg README](https://github.com/femtovg/femtovg)). Not an option.

### cosmic-text + swash

- Already iced's text stack inside iced_tiny_skia (`cosmic-text` 0.15 dep,
  `GlyphCache`/`SwashCache`). It is a text pipeline, not a shape rasterizer —
  not an alternative render path.

### What COSMIC actually ships

COSMIC runs a **forked iced** (`pop-os/iced`, vendored inside libcosmic) whose
*default* rendering is the same tiny-skia + softbuffer path — `wgpu` is an
opt-in feature ("By default, softbuffer is used for software rendering";
[libcosmic Cargo.toml](https://github.com/pop-os/libcosmic/blob/master/Cargo.toml)).
They maintain their own softbuffer fork but **have not patched tiny-skia's
damage model** — instead:

- Applets stay on softbuffer; heavier surfaces move to wgpu — COSMIC 1.10 moved
  the launcher to GPU rendering
  ([Phoronix](https://www.phoronix.com/forums/forum/software/desktop-linux/1662558-cosmic-epoch-1-10-released-with-launcher-now-using-gpu-accelerated-rendering));
  cosmic-term uses glyphon+wgpu with softbuffer/tiny-skia as fallback
  ([System76 blog](https://system76.com/blog/post/cosmic-the-road-to-alpha/)).
- mmstick attributes much of the remaining CPU cost to pre-0.14
  full-window redraws and expects iced 0.14's reactive rendering to cut draw
  calls ~80% ([cosmic-comp#2008](https://github.com/pop-os/cosmic-comp/issues/2008)).

The precedent is ambivalent for us: a shipped DE accepts tiny-skia for small
surfaces, but nobody — including a team that lives on software rendering — has
tried to make it fast for large animated ones. They pay with GPU instead.

## 4. Presentation path

`iced_layershell` 0.19.1 depends on `iced_graphics` and drives the standard
`Compositor` trait: per layer-shell surface it calls `ui.draw(&mut renderer,…)`
then `compositor.present(…)` inside its refresh-event handler
([iced_layershell Cargo.toml](https://docs.rs/crate/iced_layershell/latest/source/Cargo.toml),
[multi_window.rs event flow](https://deepwiki.com/waycrate/exwlshelleventloop/2.3-event-flow)).
The layer-shell `wl_surface` is handed to the backend as a raw window handle,
so the tiny-skia backend's `softbuffer::Surface` attaches `wl_shm` buffers and
commits directly — the same code path as an ordinary iced window.

Buffer damage to the compositor: softbuffer supports
`Buffer::present_with_damage(&[Rect])`, which on Wayland issues
`wl_surface.damage_buffer` per rect when the surface is ≥ v4 (else full
`damage(0,0,i32::MAX,i32::MAX)`)
([wayland backend](https://codebrowser.dev/slint/crates/softbuffer/src/backends/wayland/mod.rs.html),
[Buffer docs](https://docs.rs/softbuffer/latest/softbuffer/struct.Buffer.html)).
This is confirmed in our vendored copy: `patches/softbuffer/src/backends/wayland/mod.rs`
lines 96–137 (`damage_buffer` per rect) and 301–310 (`present()` delegates to
`present_with_damage(&[whole-buffer rect])`).
**But** `iced_tiny_skia`'s `present` appears to call plain `buffer.present()` —
PR #3032 *proposed* `present_with_damage`, while the merged code keeps the
`frames` tracking and a plain present, and no `physical_damage` helper or
`present_with_damage` call exists in the crate's indexed source (contrast with
`iced_vello_cpu` on master, which does both). So on 0.14.1 **every present
tells niri the entire buffer is dirty**, even when only a small region was
rewritten — compositor-side work (blur region, damage tracking) is unbounded
even when client-side rasterization is cheap. *Verify locally:*
`grep -n "present" ~/.cargo/registry/src/*/iced_tiny_skia-0.14.1/src/window/compositor.rs`
(the last ~10 lines of `present` — the search index truncates just before
them).

## 5. Verdict

**Could a patched tiny-skia hold 90 Hz?** Under the ADR-0007 rules — surfaces
sized to content, niri-owned shadows and blur, only small parts moving —
**yes, plausibly**, for panels. The arithmetic from our own spike:

- Translucent fill ≈ 2.6 ns/px (13.4 ms @ 2880×1800). A 720×540 panel at 1.75
  scale is ~1.19 Mpx → ~3 ms when fully damaged; a moving selection is already
  ~1.4 ms today because damage diffing works.
- The measured multipliers are the mask memset and the shadow SDF, both
  removable with the patch set above. Upstream issue #3368 shows the mask fix
  alone rescued a real app from seconds-per-keystroke to interactive.
- Forwarding `damage_buffer` also shrinks *niri's* per-frame work, which
  matters on an iGPU shared with everything else.

**What stays impossible:** any animation that damages most of a large surface.
Full-screen damage is bounded below by the ~13.4 ms fill plus whatever copy a
damage-pixmap scheme adds — no damage bookkeeping shrinks a region that is the
whole surface. Wallpaper crossfades, overview chrome, and full-output lock
screens can never run at 90 Hz on tiny-skia. Even vello_cpu single-threaded is
unlikely to fill 5.2 Mpx inside 11.1 ms; with `multithreading` it might.

**Recommendation:**

1. Keep wgpu as the primary renderer (ADR-0007) — nothing here changes that.
2. If a credible software fallback is wanted *now*, vendor `iced_tiny_skia`
   0.14.1 in `patches/` with the four small fixes (bounded mask clear, shadow
   clip + cache, `present_with_damage`). Re-run `spike_glass`; success metric:
   panel-sized surfaces hold 90 Hz, full-screen stays what it is.
3. Track iced 0.15/`vello_cpu`: when `iced_layershell` supports it, the
   software path gets a better damage model and SIMD+MT rasterization for
   free — at which point a "software as primary" experiment becomes cheap to
   run. Patching tiny-skia today should be scoped as a bridge, not a long-term
   fork.
4. The design rules are permanent for software rendering and remain good
   hygiene for wgpu (swapchain memory, niri damage): keep surfaces sized to
   content and let niri composite shadows, blur and big transitions.

## Appendix: primary sources

- iced_tiny_skia 0.14.1: [compositor.rs](https://docs.rs/iced_tiny_skia/0.14.1/src/iced_tiny_skia/window/compositor.rs.html) · [lib.rs](https://docs.rs/iced_tiny_skia/0.14.1/src/iced_tiny_skia/lib.rs.html) · [engine.rs](https://docs.iced.rs/src/iced_tiny_skia/engine.rs.html) · [layer.rs](https://docs.iced.rs/src/iced_tiny_skia/layer.rs.html) · [text.rs](https://docs.iced.rs/src/iced_tiny_skia/text.rs.html)
- iced_graphics damage model: [damage.rs](https://docs.rs/iced_graphics/0.14.0/src/iced_graphics/damage.rs.html)
- iced PRs/issues: [#1811 incremental rendering](https://github.com/iced-rs/iced/pull/1811) · [#3032 present fix](https://github.com/iced-rs/iced/pull/3032) · [#3306 vello_cpu](https://github.com/iced-rs/iced/pull/3306) · [#3360 clip text groups](https://github.com/iced-rs/iced/pull/3360) · [#3368 clip-mask performance](https://github.com/iced-rs/iced/issues/3368) · [0.14.0 release](https://github.com/iced-rs/iced/releases/tag/0.14.0)
- tiny-skia: [Mask API](https://docs.rs/tiny-skia/latest/tiny_skia/struct.Mask.html) · [README perf claims](https://github.com/linebender/tiny-skia/)
- softbuffer: [Buffer](https://docs.rs/softbuffer/latest/softbuffer/struct.Buffer.html) · [Wayland backend damage](https://codebrowser.dev/slint/crates/softbuffer/src/backends/wayland/mod.rs.html) · [PR #329 IOSurface/present cost](https://github.com/rust-windowing/softbuffer/pull/329)
- vello_cpu: [crate](https://crates.io/crates/vello_cpu) · [iced_vello_cpu source](https://docs.iced.rs/src/iced_vello_cpu/lib.rs.html) · [frontend rewrite + numbers](https://github.com/linebender/vello/pull/1701) · [Blend2D chart](https://laurenzv.github.io/vello_chart/) · [bunnymark](https://github.com/linebender/vello/issues/1427)
- COSMIC: [libcosmic Cargo.toml](https://github.com/pop-os/libcosmic/blob/master/Cargo.toml) · [launcher→GPU news](https://www.phoronix.com/forums/forum/software/desktop-linux/1662558-cosmic-epoch-1-10-released-with-launcher-now-using-gpu-accelerated-rendering) · [cosmic-comp#2008](https://github.com/pop-os/cosmic-comp/issues/2008)
- iced_layershell: [Cargo.toml](https://docs.rs/crate/iced_layershell/latest/source/Cargo.toml) · [event flow](https://deepwiki.com/waycrate/exwlshelleventloop/2.3-event-flow)
- femtovg: [backends](https://github.com/femtovg/femtovg)

### Not verified byte-exact (needs the vendored source)

- The final lines of `iced_tiny_skia 0.14.1`'s `present()` — whether it calls
  `buffer.present()` or `present_with_damage(&…)`. Evidence strongly indicates
  the former (no `physical_damage` helper; PR #3032's review comment favored a
  plain present), but confirm with the grep in §4 before writing the patch.
- Whether 0.14.1 (published 2026-09-06) includes PR #3360's text-group
  clipping; it postdates the code shown above in places.
