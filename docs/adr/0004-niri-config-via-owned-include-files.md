# 0004. niri is configured through generated include files that triton owns

Status: proposed, November 2026.

## Context

The settings app exists so users never hand-edit niri's or triton's config. But `config.kdl` is a hand-written file full of comments, and people will keep editing it. Options: edit `config.kdl` in place with the `kdl` crate's v2 document model (it round-trips comments and formatting), regenerate the whole file from a typed model (loses comments), or write files we own and `include` them. niri's own config crate is not published, so reusing its typed model is not an option. niri supports top-level `include` since 25.11, merges positionally, watches included files and live-reloads, supports `~` paths and `optional=true` since 26.04, and `niri validate -c <file>` checks a config. Dux already uses this scheme (`~/.config/niri/dms/*.kdl`).

## Decision

Triton writes `~/.config/niri/triton/*.kdl` (one file per topic: layout, input, outputs, binds, window rules, animations, colors) behind an `index.kdl`, and appends exactly one line to the end of the user's `config.kdl`: `include optional=true "triton/index.kdl"`. Every change goes through `NiriConfig::apply`: render to a staging directory, run `niri validate -c` on a temporary copy of the main file that points at the staging directory, back up, swap, then confirm with niri's `ConfigLoaded` event, with one-click rollback. Reading a user's hand-written values (provenance) uses the `kdl` document model read-only. The only in-place edit of the user's file, besides the include line, is an explicit "take over" that disables a conflicting node with KDL slashdash. Triton's own `config.kdl` is edited by patching the parsed `kdl` document, so its comments survive.

## Consequences

- The user's file is touched once and reversibly; our files are regenerated freely, so no comment preservation is needed where we write.
- Because includes are positional and ours is last, settings made in the app win over earlier hand-written values; values the user writes after the include win over ours, and the app detects and shows that.
- niri's semantics (merging, defaults) are not re-implemented; `niri validate` is the oracle, injected through a `Validator` seam (real CLI and fake), so the transaction is testable without niri.
- Requires niri 26.04 or later for `optional=true`; older niri would need a plain `include`.
- Open item for the first spike: confirm that `kdl` 6.x round-trips a slashdashed node verbatim, otherwise "take over" is a span-based text edit.
