# Alternative UI/rendering stacks for Triton

Survey of Rust UI stacks that could replace or underlie the current
`iced 0.14 + iced_layershell 0.19.1 + wgpu 27` combination. Compiled from
primary sources (repos, crates.io, docs.rs, upstream issues/PRs) in
November 2026.

## Requirements recap

From the design and ADR-0003/0005/0007:

- `zwlr_layer_shell_v1` surfaces: bar, panels, OSD, lock screen.
- Multiple surfaces, one process or a few daemons.
- "Thick Glass": large translucent panels, compositor blur-behind,
  rounded/smoothed corners, custom vector paths.
- Springs/animations at 90 Hz on a 2880x1800 @1.75 output (≈5040x3150
  physical px).
- Sharp vector edges — current failure is MSAA-dependent tessellation in
  `iced_wgpu` (MSAA is unreliable on radv/Vulkan; see iced issues
  [#834](https://github.com/iced-rs/iced/issues/834),
  [#2750](https://github.com/iced-rs/iced/issues/2750), and the old
  alpha-blending fix [#1367](https://github.com/iced-rs/iced/pull/1367)).
- Software fallback that can keep up; iced's tiny-skia path cannot hold
  90 Hz on large translucent surfaces.

### The blur question is orthogonal

niri 26.04 (April 2026) ships background blur for **both windows and
layer-shell surfaces** via `ext-background-effect-v1`
([niri release notes](https://github.com/niri-wm/niri/discussions/3899),
[Window Effects docs](https://niri-wm.github.io/niri/Window-Effects.html)).
Clients request/shape blur through the protocol, or the user forces it
with a `layer-rule` on `namespace`. Shaped blur behind *client-drawn*
rounded corners needs the client to set a blur region via
`ext-background-effect` — smithay-client-toolkit added support in
[0.21.0](https://github.com/smithay/client-toolkit/blob/HEAD/CHANGELOG.md)
and winit 0.31 supports it too. So "blurred panels" cost the same on
every stack: draw with alpha + optionally speak one small protocol.
Nothing here can or needs to do its own blur.

## Comparison table

| Stack | License | Maturity / activity | Layer-shell? | Multi-surface? | Renderer | Design fit | Verdict |
|---|---|---|---|---|---|---|---|
| **gpui** (Zed) | Apache-2.0 | Pre-1.0, very active; powers Zed | **Yes, upstream** (`WindowKind::LayerShell`, Oct 2025) | Yes | wgpu on Linux (Blade removed Feb 2026), SDF quads + glyph atlas | Strong | **Top contender** |
| **rune** (anomalyco) | MIT | Weeks old, extremely active; pre-release, git-pin only | **Yes** (own `layer_shell` platform path) | Yes (windows + overlays) | wgpu 30, instanced SDF uber-quads + Parley text | Strongest aesthetic match | **High-risk, high-reward** |
| **egui + SCTK + wgpu** | MIT/Apache | Very mature, huge ecosystem | DIY (proven: wayapp, wlr-chooser) | DIY (SCTK gives it free) | egui-wgpu tessellation; glow/EGL possible | Medium (AA quality, no damage) | **Safe middle path** |
| **COSMIC pop-os/iced + libcosmic** | iced fork MIT; libcosmic MPL-2.0 | Ships a whole DE | **Yes** (sctk runtime, `Action::LayerShell`) | Yes (`iced::daemon`, multi-window) | Same wgpu + tiny-skia | Same rendering pain as today | Fallback "stay close" option |
| **Custom wgpu + SCTK** | — | SCTK 0.21.1 active | Native | Native | Yours (vello / SDF quads / cosmic-text / taffy) | Full control incl. damage | Best long-term fit, most work |
| slint | GPL-3.0 / Royalty-free / Commercial | Mature, company-backed | No (winit toplevels; custom platform possible) | Limited | femtovg GL/wgpu, Skia, software | Medium | License + layer-shell gap |
| floem (lapce) | MIT | 0.2.0 Nov 2024; main alive, no release since | No (old winit 0.29 fork) | Young | vger/vello/Skia on wgpu + tiny-skia | Good on paper | Interesting but stalled cadence |
| xilem / masonry | Apache-2.0 | 0.4.0 Oct 2025, alpha | No (winit) | Initial | Vello (wgpu compute) + parley | Good architecture | Watch; too early |
| makepad | MIT/Apache | Active; not on crates.io | No (own Wayland client, no layer-shell) | Yes | Own Vulkan/GLES + shader DSL | Medium | Wrong shape for a shell |
| vizia | MIT | Active | No (winit/baseview) | Limited | Skia (GL; Vulkan/DX12/Metal in PR) | Medium | Not for this |
| Ribir | MIT | 0.4.0-alpha, small team | No (winit) | Limited | wgpu | Medium | Too small |
| blitz / dioxus-native | MIT/Apache | Active beta | No (winit) | Limited | AnyRender: vello / skia / vello-cpu | Medium | Web-style DOM, wrong shape |
| freya, kas, azul, cushy | MIT / Apache | Varying | No | No/limited | Skia / wgpu | — | Not for this |

---

## gpui — zed-industries/zed

**What**: Zed's GPU-accelerated UI framework. Hybrid immediate/retained,
pure-Rust views (`impl Render`, `div().flex()...`). Apache-2.0,
`gpui` 0.2.2 on crates.io (updated 2026-10-30, ~310k recent downloads,
163 dependents). Pre-1.0 with regular breaking changes; docs are thin —
you read the source and the `gpui.rs` site.
([crates.io](https://crates.io/crates/gpui),
[README](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md))

**Wayland/layer-shell**: Native Wayland backend over
`wayland-client`/`wayland-protocols(-wlr)` + calloop (not winit).
**Layer-shell support was merged upstream** in
[zed PR #35610](https://github.com/zed-industries/zed/pull/35610)
(Oct 2025): `WindowKind::LayerShell` with
`gpui::layer_shell::LayerShellOptions { namespace, layer, anchor,
exclusive_zone, exclusive_edge, margin, keyboard_interactivity }` — the
full wlr protocol surface. Shipping examples exist
(`examples/layer_shell.rs`, visible in the community forks
[gpui-ce](https://docs.rs/crate/gpui-ce/latest/source/examples/layer_shell.rs)).
Multi-window is first-class (`cx.open_window`). Known wrinkle:
`cx.displays()` is empty at startup, which currently breaks layer-shell
output selection if you need a specific monitor
([zed#46378](https://github.com/zed-industries/zed/issues/46378)).

**Rendering**: Quad-scene model with SDF corner radii drawn in-shader —
the right architecture for crisp rounded translucent panels *without*
MSAA. Glyph atlas text. Linux renderer was **rewritten from Blade onto
wgpu** ([zed PR #46758](https://github.com/zed-industries/zed/pull/46758),
merged Feb 2026) — same GPU stack we already target; macOS uses Metal.
`WindowBackgroundAppearance` supports transparent windows; blur would be
niri-side per above. Path/`canvas` primitives exist for custom vector
work. Animation primitives (keyframes, easing) exist; springs are DIY.

**Text/widgets**: cosmic-text + font-kit on Linux. No "widget library"
in the iced sense — you compose `div`s and element traits; Zed shows it
scales to a huge real app.

**Footprint**: heavy dependency tree (ashpd, font-kit, cosmic-text,
blade leftovers on macOS); binary size comparable to a full toolkit.

**Verdict**: The most pragmatic upgrade path. Upstream layer-shell,
wgpu, SDF-quality corners, real multi-window, production-scale text.
Costs: Zed-centric API churn, thin docs, one known output-selection bug,
and you buy the whole platform layer (fine — it replaces both winit and
iced_layershell). Pin a git rev like all gpui consumers do.

---

## anomalyco/rune

**What**: New native GPU-rendered Rust UI framework from Anomaly (the
opencode/SST company) — `github.com/anomalyco/rune`. It **is** a real
Rust UI framework (the question was whether it might be a terminal/other
project). MIT. Repo created Oct 2026, already ~7.7k stars, hundreds of
issues and rapid churn on `dev`. **Pre-release: pin a rev; not published
to crates.io.** Explicitly designed to be written and *checked* by coding
agents: every app runs headless, renders PNGs, dumps the element tree,
takes scripted input.
([repo](https://github.com/anomalyco/rune),
[PLAN.md](https://github.com/anomalyco/rune/blob/dev/PLAN.md),
[crates/rune README](https://github.com/anomalyco/rune/blob/dev/crates/rune/README.md))

**Wayland/layer-shell**: Two windowing paths: winit 0.30 for normal
windows, plus an **own Wayland overlay subsystem that speaks
`zwlr_layer_shell_v1` directly** via raw `wayland-client` +
`wayland-protocols-wlr` (`crates/rune/src/platform/layer_shell.rs`,
`layer_shell_machine.rs`). `OverlayOptions` exposes placement
(edge-margined, rect, fullscreen), display target (primary/under-pointer/
id), layer level, keyboard policy, input policy including **click-through
/ empty input region**, and prewarm. On compositors without layer-shell
(GNOME) it falls back to an undecorated xdg toplevel with a capability
report ([issue #20](https://github.com/anomalyco/rune/issues/20)); X11
gets override-redirect windows. Already used in production by
[anomalyco/hex's dictation HUD](https://github.com/anomalyco/hex/blob/67e0bc41/src/linux/rune_hud.rs)
as an overlay-layer layer-shell surface. Multiple windows + overlays per
process.

**Rendering**: The most aligned with "Thick Glass" of anything surveyed:
one instanced SDF "uber-quad" pass (rounded rects, shadows, soft-edge
masks on *every* clip in-shader), glyph atlas for text, Slug for
large/animated text, `vello_gpu` sparse strips for complex vector paths,
wgpu ~30 on every platform (plus an opt-in custom Metal backend on Apple).
Analytic springs (`Spring::new(duration, bounce)`), tweens, decay —
retargetable tracks, first-class in the API (`.motion(motion::SNAPPY)`).
OKLCH token theming. Taffy 0.15 layout, Parley text, accesskit.
No software renderer — wgpu-only (lavapipe works but won't fix the
90 Hz software-fallback requirement).

**Verdict**: The design brief reads like Rune's feature list — SDF
corners, translucency, springs, layer-shell overlays, and a headless
verification loop that matches this repo's agent workflow. The catch is
maturity: weeks old, fast-breaking `dev` branch, small team, no release
discipline yet. Ideal for a spike branch to validate the *look and frame
budget* even if we don't adopt it.

---

## egui (+ eframe/egui-wgpu, or SCTK directly)

**What**: Mature immediate-mode GUI. `egui`/`eframe`/`egui-wgpu` 0.37.0
(Oct 2026, wgpu 30), MIT/Apache, enormous ecosystem, MSRV 1.95.
([crates.io/eframe](https://crates.io/crates/eframe))

**Wayland/layer-shell**: eframe = winit → toplevels only. But egui is
integration-friendly: `egui::FullOutput` (clipped primitives + textures)
can be painted onto **any** surface. Proven layer-shell integrations:

- [`wayapp`](https://crates.io/crates/wayapp) 0.4.1 (Oct 2026): a small
  SCTK-0.21 + wgpu-30 app wrapper, deliberately winit-free, with working
  layer-surface and multi-surface examples.
- [`wlr-chooser`](https://docs.rs/wlr-chooser/latest/src/wlr_chooser/shell.rs.html):
  a real rofi-like overlay daemon, egui_glow on EGL directly bound to a
  layer surface, reusable host across overlays.
- [phisch/sctk_egui](https://github.com/phisch/sctk_egui): WIP, stalled.

So: no upstream support, but a ~600-line shell loop you own outright,
which is also exactly what damage control, per-surface present timing,
and `ext-background-effect` want.

**Rendering**: Tessellated shapes with vertex-alpha AA — decent, not
SDF-crisp; fine at 1.75x for rects but custom vector work is weaker than
vello/SDF. Translucency is trivial (alpha in `Color32`, transparent
surface). Full-frame tessellation+paint whenever anything repaints;
immediate mode means *you* decide when to `request_repaint` — springs are
just `ctx.request_repaint()` + a clock. No partial-present; at our sizes
that's a real cost unless we add scissor/damage logic ourselves in the
wgpu glue (possible; `egui-wgpu` takes arbitrary `ClipRect`s).

**Text/widgets**: Own shaping (now with system-font provider + color
emoji in 0.37); rich built-in widget set; widgets are immediate, so
"widgets fight the design" is less of an issue — but polished
custom visuals mean painting `Shape`s yourself anyway.

**Footprint**: smallest of the serious options; compiles fast.

**Verdict**: The boring, low-risk option. All maturity gains are real,
but we inherit writing the shell loop *and* accept full-frame repaints
and mediocre vector AA. Best paired with the mindset that egui is
prototyping/interim, not the endgame for a 90 Hz glass design.

---

## System76 COSMIC: pop-os/iced fork + libcosmic

**What**: A whole shipping desktop (COSMIC Epoch) built on a
[System76 fork of iced](https://github.com/pop-os/iced) (MIT), plus
[`libcosmic`](https://github.com/pop-os/libcosmic) (MPL-2.0): app/applet
framework, themed widgets, cosmic-config, theming.

**What they changed**: per
[mmstick](https://github.com/pop-os/cosmic-epoch/issues/175): "We forked
iced to implement a custom sctk runtime for the Wayland integrations that
we need for applets, as well as various modifications to implement
features that we need in libcosmic." Concretely: a smithay-client-toolkit
platform path instead of winit on Wayland (`iced/wayland` feature),
layer surfaces + xdg popups + subsurfaces via
`iced_runtime::platform_specific::wayland::*` and
[`cosmic::surface::Action::LayerShell`](https://pop-os.github.io/libcosmic/src/cosmic/app/cosmic.rs.html),
multi-window/multi-surface daemons (`iced::daemon` + `multi-window`
feature), cosmic-time animations, cosmic-client-toolkit extensions.
([libcosmic Wayland integration](https://deepwiki.com/pop-os/libcosmic/8.1-wayland-integration),
[multi-window](https://deepwiki.com/pop-os/libcosmic/9.1-multi-window-applications))

**Rendering**: unchanged `iced_wgpu` + `iced_tiny_skia` — the same
tessellation+MSAA and software-raster limits we already hit. COSMIC's
look is mostly opaque rounded panels, which that renderer handles fine;
it does not fix our actual pain.

**Verdict**: Proof that iced-as-shell-runtime is viable at DE scale, and
a rich source of layer-shell/applet patterns (their `Action` system is a
better multi-surface API than `iced_layershell`'s). But adopting it keeps
the renderer we want to escape, adds MPL-2.0 into the tree, and makes us
track two forks. If we stay on iced, we'd rather *study* pop-os/iced
(and waycrate's iced_layershell) than adopt libcosmic wholesale.

---

## Custom: wgpu + smithay-client-toolkit (no framework)

SCTK 0.21.1 (July 2026) already ships everything a shell client needs:
`shell::wlr_layer`, xdg popups, fractional-scale, and —
[added in 0.21.0](https://github.com/smithay/client-toolkit/blob/HEAD/CHANGELOG.md)
— `ext-background-effect-v1` for shaped blur-behind. On top of that the
modern stack is off-the-shelf:

- **Vector**: `vello` 0.12 (wgpu compute; very active — 6 releases in
  2026, [crates.io](https://crates.io/crates/vello)), `vello_cpu` for the
  software fallback that tiny-skia can't reach, or a modest custom SDF
  quad pipeline in the gpui/rune style (a few hundred lines of WGSL).
- **Text**: `parley` (+ `swash`) — same as rune/masonry/blitz — or
  `cosmic-text` (gpui's Linux pick).
- **Layout**: `taffy`, or hand-rolled for a shell's small trees.
- **Animation**: analytic springs are ~30 lines.

Damage/partial present, per-surface frame callbacks, input regions,
output-specific surfaces (per-monitor bars) — all directly controllable.
This is also the only option that cleanly gives *each surface its own
present loop*, which a multi-surface daemon wants anyway. Precedent:
wayapp/wlr-chooser (above) and every wgpu+sctk example do exactly this
composition.

A middle path also exists: keep `iced`'s widget tree and
`iced_layershell` runtime but implement a **custom
`iced_core::Renderer`** over wgpu/SDF — the trait is public; cost is the
size of the primitive surface (borders, text, images, shadows, meshes).

**Verdict**: Highest ceiling and the truest fix for damage/AA/perf —
replace precisely the layer that's failing. Costs a widget framework
(no button/scrollable for free) — mitigated because a shell's widget
vocabulary is small and `triton-design` already owns the look. Serious
candidate if gpui/rune don't survive a spike.

---

## Also surveyed

**slint** — Mature, company-backed, `.slint` DSL + Rust. winit backend
only: *"No layer-shell, xdg-activation, or idle-inhibit support — Slint
targets ordinary application toplevels via winit, not panels/overlays"*
([platform study](https://sparkles-docs.pages.dev/research/window-system-integration/slint)).
A custom `slint::platform` backend over SCTK is architecturally possible
(the software renderer does proper dirty-region partial rendering —
attractive for big translucent surfaces), but that's a bespoke port.
License: GPLv3 **or** Royalty-free requiring an AboutSlint/badge
attribution **or** paid commercial — real friction for an MIT project
([LICENSE.md](https://github.com/slint-ui/slint/blob/refs/heads/master/LICENSE.md)).
**Out on license + layer-shell; revisit only if the software-renderer
requirement dominates.**

**floem** (lapce) — Signal-reactive, Taffy, nice API; renderers vger/
vello/Skia(wgpu) + tiny-skia fallback. MIT. But: last release 0.2.0
(Nov 2024), maintainer says "use main; 1.0 unlikely in 2-4 years"
([issue #1050](https://github.com/lapce/floem/issues/1050)); windowing
is a **fork of winit 0.29** (old) → no layer-shell path except replacing
their windowing layer. **Skip for now; re-check if 0.3 ships.**

**xilem / masonry** (Linebender) — The best long-term architecture:
Vello + parley + accesskit, retained widget tree with serious
invalidation/damage work, multi-window *just* landed in 0.4.0 (Oct 2025)
— "alpha-quality... major breaking changes" per the
[release notes](https://github.com/linebender/xilem/releases/tag/v0.4.0).
winit shell → no layer-shell; you'd write a masonry_sctk driver
(the shell boundary is clean). **Watch — this is where the ecosystem is
heading — but too early to build a shell on.**

**makepad** — MIT, 1.0 widgets May 2025, *not on crates.io*
([maintainer rationale](https://github.com/makepad/makepad/issues/1026)).
Remarkable: its own Wayland client implementation and own Vulkan/GLES —
a completely self-owned platform stack, so adding layer-shell would mean
patching their (moving-fast) platform code, not wrapping it. Shader-DSL
immediate-ish model; built for creative apps/studio. **Wrong shape; note
as prior art for "own the platform layer."**

**vizia** — Declarative ECS-style toolkit, Skia (GL) renderer with
Vulkan/DX12/Metal backends in PR [#704](https://github.com/vizia/vizia/pull/704),
winit 0.30. Active (CI through Nov 2026). No layer-shell. **Skip.**

**Ribir** — Declarative "data-driven" GUI, wgpu (29) backend,
0.4.0-alpha.65 (Apr 2026). Active but single-digit-adopter scale, winit,
no layer-shell. **Skip.**

**blitz / dioxus-native** — DOM/CSS (stylo + taffy + parley) rendered by
AnyRender over vello/vello-cpu/vello-hybrid/Skia, winit shell, hot
development pace (0.3.0-beta.4 Oct 2026). The `anyrender` abstraction is
the interesting part (one paint tree over four renderers). No
layer-shell; DOM model is wrong for a shell. **Skip; anyrender worth
stealing ideas from.**

**freya** — Skia-rendered reactive GUI (own reactive core since 0.4),
0.5.3 stable / 0.6 alpha, winit, no layer-shell. **Skip.**

**kas** — Apache-2.0 stateful-widget toolkit, wgpu 28 + `kas-soft`
software backend, 0.17.1 (Feb 2026). Serious engineering, but winit-only
and a conventional desktop-app model. **Skip.**

**GTK4 + gtk4-layer-shell (gtk-rs)** — the non-pure-Rust escape hatch:
mature, layer-shell via a dedicated library, GL renderer with fine
damage handling; but C FFI, GObject lifecycle, and a different design
language. Mentioned for completeness; out of scope for a Rust-native
goal.

## Recommendation

1. **Spike gpui first.** It is the only mature framework that already
   does wlr layer-shell upstream, runs on wgpu on Linux, and renders
   SDF corners that don't depend on broken MSAA. Risk is API churn and
   Zed-specific assumptions — acceptable for a shell that's mostly
   custom-drawn anyway. Watch item: `displays()` empty at startup
   (zed#46378) affects per-output bars.
2. **Spike rune in parallel for the look.** Its SDF uber-quad + analytic
   springs + layer-shell overlays + headless PNG/element-tree checks are
   the closest thing to "the design, as a library." Age is the only
   argument against it; a spike costs little and validates the visual
   target at 90 Hz.
3. **If both fail, build the thin custom shell:** SCTK 0.21
   (layer-shell + ext-background-effect) + wgpu + vello (+ vello_cpu
   software path) + parley + taffy, with per-surface present loops and
   explicit damage. Optionally egui on top for a fast first pass
   (wayapp proves the glue).
4. **Do not adopt libcosmic/slint/floem/... :** either they keep the
   renderer that's failing (COSMIC iced), can't make layer surfaces
   (slint, floem, vizia, makepad, xilem, blitz), or have license
   friction (slint).

Whatever is chosen, add `ext-background-effect-v1` (SCTK has it; gpui /
rune / iced_layershell need a small protocol addition) so niri 26.04+
draws correctly-shaped blur behind the rounded panels instead of relying
on user-side `layer-rule` blur.
