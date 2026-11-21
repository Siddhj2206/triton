# Triton

A desktop shell for the niri compositor, written in Rust (iced + iced_layershell). The project is being restarted with a new design and direction; the old Dux-derived implementation was removed. What remains are the tech decisions: `docs/adr/` and `docs/rust-conventions.md`.

## Development loop

Everything runs through mise. Run `mise tasks` to see the full list.

| Task | What it does |
|---|---|
| `mise run setup` | One time: installs native libraries with `nx` (the nix frontend; Fedora Atomic has no -devel packages) |
| `mise run build` | Build all crates |
| `mise run check` | fmt check, clippy with `-D warnings`, tests. Must pass before you call work done |

Never run triton directly against the real session. It would collide with the user's running shell.

Native libraries: install with `nx add <pkg>`. nx provides libraries but not their pkg-config files, so add a link-only `.pc` file to `dev/pkgconfig/` when a crate probes for one. `mise.toml` sets `PKG_CONFIG_PATH` and `LIBRARY_PATH`.

## Agent skills

### Issue tracker

Issues live in GitHub Issues for Siddhj2206/triton, used through the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default vocabulary: needs-triage, needs-info, ready-for-agent, ready-for-human, wontfix. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `GLOSSARY.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.
