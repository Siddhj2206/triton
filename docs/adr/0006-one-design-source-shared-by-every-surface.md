# 0006. One design source, shared by every surface

Status: accepted, November 2026.

## Context

Triton draws in several processes (`triton`, `triton-lock`, `triton-greeter`, `triton-settings`) and themes things it doesn't draw (GTK 3 and 4 apps through generated CSS, niri through generated rules for blur, radius and colors). The design canvas is about 90% final and already ships its values as `tokens.json`. If each process or generator restates colors, radii, alphas or springs, they drift, and every drift is a visible inconsistency.

## Decision

- `crates/triton-design/tokens.json` is the canvas's token file, copied verbatim. It is the only place a design value is written down. `build.rs` generates Rust constants from it. The struct definitions live in the crate, so a role missing from the file, or one the crate doesn't know, fails the build.
- `triton-design` is pure (`iced_core` only): tokens, materials, springs, wave geometry. Every binary and every theme generator depends on it, and none defines its own design values.
- Derived values are computed, not stored: material fills are a scheme role at the material's alpha, list and connected-group corners are functions of position, and niri blur rules come from `MaterialSpec`.
- One component set (the canvas's Component index board) is shared by every iced surface. It lives in one UI crate. A surface composes components; it does not style widgets itself.
- Tests check the design's own claims (AA contrast of on-roles, effects springs never overshoot, the wave has nine lobes) rather than restating numbers.

## Consequences

- Updating the design is: copy the new `tokens.json`, build, look at the preview.
- A one-off style in a surface is a review failure. If a surface needs something new, it becomes a token or a component first.
- The canvas's hand-picked light scheme now has a test (on-roles at 4.5:1). It passes today; the wallpaper pipeline will replace the reference schemes at runtime.
