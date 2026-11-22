# Triton

A desktop shell for the niri compositor, built on a 4-layer Rust stack: `triton-design` (zero-dep tokens), `triton-engine` (SCTK + EGL/GLES3 + Taffy), `triton-kit` (shared Material 3 Futura components), and the shell binaries. Architecture and design references live in `docs/adr/`, `docs/design-spec.md`, `docs/roadmap.md`, and `docs/rust-conventions.md`.

## Development loop

Everything runs through mise. Run `mise tasks` to see the full list.

| Task | What it does |
|---|---|
| `mise run setup` | One time: installs native libraries with `nx` (the nix frontend; Fedora Atomic has no -devel packages) |
| `mise run build` | Build all crates |
| `mise run check` | fmt check, clippy with `-D warnings`, tests. Must pass before you call work done |
| `mise run dev` | Launch `triton --real-services` inside an isolated nested-niri child session (`target/nested-niri`; scratch `HOME`/`XDG_*`, private `XDG_RUNTIME_DIR`, `GSETTINGS_BACKEND=memory`, `unset NIRI_SOCKET`) — live niri IPC and `triton/*.kdl` live-reload stay strictly inside the child |
| `mise run dev:showcase` | Same isolated session running the `triton-kit` showcase gallery |
| `mise run msg -- ...` | Forward `triton msg` to the nested session's private socket (`target/nested-niri/runtime/triton.sock`) — e.g. `mise run msg -- status`, `mise run msg -- osd volume 70` |
| `mise run snap -- <args>` | Headless `triton` snapshot: `mise run snap -- target/snap --surface all`, `--tree <path|->`, `--script <file>`, `--surface`, `--variant`, `--mode`, `--osd`, `--reveal`, `--scale`, `--size`, `--reduce-motion`, `--transparent` |

Never run triton directly against the real session. It would collide with the user's running shell and could touch the real `~/.config/niri`. The nested session keeps every write — including `--real-services` `gsettings` calls — inside `target/nested-niri`. For millisecond offscreen iteration without a compositor, use the headless flags: `triton --snap out.png --surface osd`, `triton --tree - --mode pinned`, `triton --script events.json --snap out.png` (always the fake dev backend).

Native libraries: install with `nx add <pkg>`. nx provides libraries but not their pkg-config files, so add a link-only `.pc` file to `dev/pkgconfig/` when a crate probes for one. `mise.toml` sets `PKG_CONFIG_PATH` and `LIBRARY_PATH`.

## Agent skills

### Issue tracker

Issues live in GitHub Issues for Siddhj2206/triton, used through the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default vocabulary: needs-triage, needs-info, ready-for-agent, ready-for-human, wontfix. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `GLOSSARY.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.
