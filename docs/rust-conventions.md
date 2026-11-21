# Rust conventions

Concise and adoptable. Each convention has a rule, a one-line **why**, and a **source**. Written for a long-running desktop process where resident memory and idle CPU matter, on Rust 1.100 and edition 2024 (pinned in `mise.toml`). Where we copied from a project, we looked at its manifest: niri, cosmic-applets, cosmic-panel.

Everything here must agree with the mise tasks: `mise run check` is fmt check, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`. If a convention needs a different command, change the mise task in the same commit.

## 1. Toolchain

- **The toolchain is pinned in `mise.toml` (`rust = { version = "1.100.0", components = "rustfmt,clippy" }`). We do not add `rust-toolchain.toml`.** Why: two pins drift apart, and mise is already the single entry point for every task (a `rust-toolchain.toml` would also make rustup try to install toolchains behind mise's back). Add one only if a contributor tool that ignores mise (some editor setups) becomes a real problem. Source: [mise rust](https://mise.jdx.dev/lang/rust.html), [rustup overrides](https://rust-lang.github.io/rustup/overrides.html).
- **`rust-version` in `[workspace.package]` equals the mise pin (`"1.100"`); bumping the pin and `rust-version` is one commit.** Why: this is an application, so we track the toolchain we test and carry no compatibility shims; with `resolver = "3"` Cargo also prefers dependency versions that support that Rust. Source: [Cargo rust-version](https://doc.rust-lang.org/cargo/reference/rust-version.html), [resolver v3](https://doc.rust-lang.org/cargo/reference/resolver.html#resolver-versions).
- **Native libraries come from `nx`, never from system `-devel` packages; a crate that probes pkg-config gets a link-only `.pc` file in `dev/pkgconfig/`.** Why: Fedora Atomic is immutable, and `mise.toml` already sets `PKG_CONFIG_PATH` and `LIBRARY_PATH` for this. Source: AGENTS.md.

## 2. Workspace layout and Cargo.toml

**Virtual root manifest, member crates under `crates/`, `members = ["crates/*"]`.** Why: one obvious place for shared config; new crates are picked up automatically. Source: [Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html), [cosmic-applets Cargo.toml](https://github.com/pop-os/cosmic-applets/blob/master/Cargo.toml).

**Centralise versions in `[workspace.dependencies]`, metadata in `[workspace.package]`, lints in `[workspace.lints]`; members opt in with `x.workspace = true` and `[lints] workspace = true`.** Why: one version of each dependency and one edit to bump it. Workspace lints are not inherited automatically, so every member must opt in (a review check, or a CI grep for `[lints]`). Source: [Cargo workspace.lints](https://doc.rust-lang.org/cargo/reference/workspaces.html#the-lints-table).

**Workspace dependencies use `default-features = false`; members re-enable what they need.** Why: a member that needs no `tokio/full` or TLS does not get it, keeping compile time and binary size honest. Caveat: when `default-features = false` is set in the workspace table, a member cannot turn defaults back on by omission, so it lists features explicitly. Source: [Cargo features](https://doc.rust-lang.org/cargo/reference/features.html), [workspace inheritance](https://doc.rust-lang.org/cargo/reference/workspaces.html#the-dependencies-table).

**Commit `Cargo.lock`.** Why: reproducible builds, and `cargo deny` audits what ships. Source: [Cargo FAQ](https://doc.rust-lang.org/cargo/faq.html#why-have-cargolock-in-version-control).

**Pin with `=` only for a stated reason, in a comment.** Existing example, keep it: `winit-core` and `winit-common` are `=0.31.0-beta.2` because beta.4 added a `NativeKeyCode` variant that breaks `iced_exdevtools` (pulled in by iced_layershell 0.19.1). Drop the pin when waycrate releases a fix. Why: an unexplained pin is never removed. Source: the comment in `crates/triton/Cargo.toml`.

Root `Cargo.toml` to grow into (the current file is the starting point; today it has no `[workspace.dependencies]` or lints yet):

```toml
[workspace]
resolver = "3"
members = ["crates/*"]
default-members = ["crates/triton"]   # add triton-lock when it exists; keep triton-settings out of plain `cargo build`

[workspace.package]
edition = "2024"
license = "MIT"
rust-version = "1.100"                # equals mise.toml
publish = false                       # nothing goes to crates.io

[workspace.dependencies]
# internal
triton-design   = { path = "crates/triton-design" }
triton-services = { path = "crates/triton-services" }
triton-ui       = { path = "crates/triton-ui" }
triton-config   = { path = "crates/triton-config" }
# external
anyhow      = "1"
thiserror   = "2"
tracing     = "0.1"
tracing-subscriber = { version = "0.3", default-features = false, features = ["env-filter", "fmt", "ansi"] }
tokio       = { version = "1", default-features = false }
serde       = { version = "1", features = ["derive"] }
kdl         = "6.7"                   # KDL v2 document model; round-trips comments and formatting
insta       = "1"
# Pinned: beta.4 breaks iced_exdevtools. Drop once iced_layershell is fixed.
winit-core   = "=0.31.0-beta.2"
winit-common = "=0.31.0-beta.2"

[workspace.lints.rust]
unsafe_code = "deny"                  # not "forbid": forbid cannot be overridden, and the PAM/PipeWire FFI wrappers need a scoped allow
unused_must_use = "deny"
missing_debug_implementations = "warn"
rust_2018_idioms = { level = "warn", priority = -1 }

[workspace.lints.clippy]
all      = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
# pedantic rules that fight UI and pixel math
module_name_repetitions  = "allow"
must_use_candidate       = "allow"
missing_errors_doc       = "allow"
missing_panics_doc       = "allow"
cast_precision_loss      = "allow"
cast_possible_truncation = "allow"
cast_sign_loss           = "allow"
similar_names            = "allow"
# restriction lints we do want (they are off by default, so they are listed explicitly)
dbg_macro            = "deny"
todo                 = "warn"
unwrap_used          = "warn"         # clippy.toml exempts tests
await_holding_lock   = "deny"
large_futures        = "warn"
disallowed_methods   = "deny"         # encodes product principle 1, see clippy.toml
```

Profiles are in section 11. A member manifest:

```toml
[package]
name = "triton-services"
version = "0.1.0"
edition.workspace = true
license.workspace = true
rust-version.workspace = true
publish.workspace = true

[lints]
workspace = true

[dependencies]
tokio = { workspace = true, features = ["rt", "sync", "time", "net", "macros"] }
```

Why these lint choices: pedantic catches real mistakes but is noisy for UI code, so the noisy members are named and allowed once, centrally; `unwrap_used` and `dbg_macro` are restriction lints that are off by default and cheap to adopt; `disallowed_methods` is a style-group lint (on by default), listed anyway so it survives a group reshuffle. Sources: [Clippy lint groups](https://doc.rust-lang.org/clippy/lints.html), [Clippy usage, lint levels](https://doc.rust-lang.org/clippy/usage.html).

## 3. When to split crates and modules

**Default to modules. Add a crate only when it earns the boundary: (a) a second consumer exists today, (b) it is a separate binary or process, (c) a heavy dependency set should stay out of fast-iteration builds, (d) the compiler should forbid a dependency edge.** Why: crates cost manifests, version friction and `pub` everywhere; splitting by feature ("a crate per widget") creates shallow modules. Sources: [matklad, Large Rust Workspaces](https://matklad.github.io/2021/08/22/large-rust-workspaces.html), niri splits `niri-ipc` for outside consumers. Our list and the triggers for splitting further are in `architecture.md` section 9.

**A module's interface is a few items re-exported at the crate or module root; everything else is `pub(crate)` or private.** Why: the interface is what callers and tests may depend on, so the smaller it is the deeper the module is. Source: [Rust API Guidelines, future-proofing](https://rust-lang.github.io/api-guidelines/future-proofing.html).

**`#[non_exhaustive]` on event, command and error enums that cross a crate boundary.** Why: adding a variant must not silently break downstream matches; the downstream `_ =>` arm is then a deliberate act. Source: [Rust reference, non_exhaustive](https://doc.rust-lang.org/reference/attributes/type_system.html#the-non_exhaustive-attribute).

**No trait with a single implementation just for a mock.** Why: a seam with one adapter is indirection; introduce a trait when a real second adapter (a fake or a second backend) exists. Today's real seams: `Services` (real and fake), `Validator` (niri CLI and fake), the `SurfaceSpec` planner (layer-shell and test). Source: codebase-design skill, "one adapter means a hypothetical seam".

## 4. Naming

- **Crates are `triton-*` kebab-case; modules `snake_case`; directories match modules.** Why: Cargo and RFC 430 convention. Source: [API Guidelines, naming](https://rust-lang.github.io/api-guidelines/naming.html).
- **Types name the domain term (`Scheme`, `Snapshot`, `Intent`, `SurfaceSpec`), not the pattern; `Manager`, `Handler`, `Service` suffixes need a reason.** Why: names should match `architecture.md` so prose and code stay greppable together.
- **Layer-shell namespaces are `triton-<role>` (`triton-bar`, `triton-osd`).** Why: niri `layer-rule`s match on namespace, so they are user-facing API.
- **Binaries: `triton`, `triton-lock`, `triton-settings`.** Why: they appear in `ps`, niri `spawn` lines and systemd units.

## 5. Feature flags

**Features are additive and few.** None today — dev-only backends are runtime flags, not features: the shell runs its fake services backend unless `--real-services` is passed, so the same binary serves preview and session. Later: `tracy` for profiling. No feature per service until a service's dependency weight shows in `cargo build --timings`. Why: features multiply the test matrix, and Cargo unifies them across the workspace, so a non-additive feature breaks other crates. Source: [Cargo features](https://doc.rust-lang.org/cargo/reference/features.html).

## 6. Error handling

- **Library crates: `thiserror` 2 enums, one per module where a caller can act on the difference; otherwise one opaque module error with a `source`.** Why: callers can match, and libraries do not hide types behind `anyhow`. Source: [thiserror](https://docs.rs/thiserror), [API Guidelines, error types](https://rust-lang.github.io/api-guidelines/interoperability.html#error-types-are-meaningful-and-well-behaved-c-good-err).
- **Binaries: `anyhow` with `.context("...")` at task and `main` roots.** Why: the output is a log line, and context chains are what a log line needs. Source: [anyhow](https://docs.rs/anyhow).
- **Across the services seam, errors become state (`Availability::Unavailable(Reason)`), not `Result`.** Why: the shell must keep running when bluez is missing. `architecture.md` 2.2.
- **Config problems are `Diagnostic`s with line and column, not early returns** (`TritonConfig::parse` always yields a config plus diagnostics). Why: one typo must not discard the whole file, and the settings app needs the positions. Source: `kdl` errors implement [`miette::Diagnostic`](https://docs.rs/kdl/latest/kdl/).
- **No `unwrap()` outside tests (`clippy::unwrap_used`); `expect("why this cannot fail")` for real invariants.** Why: a panic in a daemon is a visible outage. Source: [clippy unwrap_used](https://rust-lang.github.io/rust-clippy/master/index.html#unwrap_used).

## 7. Logging

**`tracing` everywhere; `tracing-subscriber` with `EnvFilter` installed once in each binary's `main`; default filter `triton=info,warn`, `RUST_LOG` overrides (mise sets `triton=debug,info` for development).** Why: structured events with per-service spans (`info_span!("service", name = "audio")` at task roots). We log to stderr only, no `ansi` when stderr is not a terminal: under systemd stderr already reaches journald, so `tracing-journald` is extra dependency for little gain (revisit if we want journald fields). Install a panic hook that logs through `tracing`. Log at `debug` or `trace` in anything that runs per event or per frame; an idle shell logs nothing. Source: [tracing](https://docs.rs/tracing), [tracing-subscriber EnvFilter](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html).

## 8. Testing

- **Behaviour tests cross the module's interface (`crates/<x>/tests/*.rs`, or `#[cfg(test)]` next to a pure function with no better seam); delete tests of private helpers once an interface test covers them.** Why: tests that describe behaviour survive refactors. Source: [The Book, test organization](https://doc.rust-lang.org/book/ch11-03-test-organization.html), codebase-design "replace, don't layer".
- **Runner is `cargo test --workspace` (what `mise run test` runs).** Why: one fewer tool, and doc-tests run (nextest does not run them). Adopt `cargo nextest` only if a D-Bus test needs per-test process isolation or retries, and then add it to `mise.toml` in the same change. Source: [nextest, doctests](https://nexte.st/docs/running/#doctests).
- **`insta` snapshots for structured data (surface plans, parsed config, rendered KDL, event sequences); `iced_test` screenshots on tiny-skia for UI.** Why: reviewable diffs, deterministic across machines. CI sets `INSTA_UPDATE=no`. Source: [insta](https://insta.rs/), [iced_test 0.14](https://docs.rs/iced_test/0.14.0/iced_test/).
- **Fixtures live in `tests/fixtures/`: recorded niri `.jsonl` streams, sample desktop entries, sample KDL (including a hand-written niri config full of comments).** Why: real protocol shapes and real user files catch what invented ones do not.
- **No sleeping; time and randomness are arguments.** Why: determinism and speed; state machines take `Instant`.
- **D-Bus tests use a private bus (`dbus-run-session`), never the user's session; the preview harness has its own D-Bus too.** Why: a test must not talk to the real desktop. AGENTS.md: never run triton against the real session.
- **UI changes are verified by looking: `mise run dev`, read `target/preview/dev.png`.** Why: tests cannot say a layout looks right. Source: AGENTS.md.

## 9. Formatting and linting

`rustfmt.toml` (stable options only, so no nightly):

```toml
# edition and style edition follow Cargo.toml (2024)
use_field_init_shorthand = true
use_try_shorthand = true
```

Why: the defaults (width 100, edition-2024 style) are what the ecosystem reads; two small options remove noise. Source: [rustfmt configuration](https://rust-lang.github.io/rustfmt/).

`clippy.toml`:

```toml
allow-unwrap-in-tests = true
allow-expect-in-tests = true
allow-dbg-in-tests = true
allow-print-in-tests = true
disallowed-methods = [
  { path = "std::thread::sleep", reason = "no sleeping or polling; wait on an event (product principle 1)" },
  { path = "std::process::Command::output", reason = "no subprocess in hot paths; spawn detached from a service, or use a service" },
  { path = "std::process::Command::status", reason = "same as output" },
]
```

Why: this turns "no polling timers, no subprocesses in hot paths" into a compile-time check. A legitimate use (the `niri validate` call in `NiriCli`, a spawn on user action) takes `#[expect(clippy::disallowed_methods, reason = "...")]`, which documents the exception and fails if it becomes unnecessary. Source: [Clippy configuration](https://doc.rust-lang.org/clippy/lint_configuration.html), [`#[expect]`](https://doc.rust-lang.org/reference/attributes/diagnostics.html#the-expect-attribute).

Gate: `mise run check`. Proposed additions once the workspace has more than a few crates (as separate mise tasks, not yet in `mise.toml`): `cargo deny check` and `cargo machete`, installed through mise's `cargo:` backend.

## 10. Dependency hygiene

- **`cargo-deny` with a committed `deny.toml`.** Why: iced, wgpu and zbus bring hundreds of crates; advisories, licences (project is MIT; avoid GPL and AGPL) and duplicates should fail loudly. Source: [cargo-deny](https://embarkstudios.github.io/cargo-deny/).

```toml
[advisories]
unmaintained = "workspace"
yanked = "deny"
[bans]
multiple-versions = "warn"
wildcards = "deny"
deny = [{ crate = "openssl" }, { crate = "openssl-sys" }]
[licenses]
allow = ["MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib", "MPL-2.0", "Unicode-3.0", "CC0-1.0"]
confidence-threshold = 0.93
[sources]
unknown-registry = "deny"
unknown-git = "deny"
```

(Check against the installed cargo-deny's schema when adding it; the config keys have changed between 0.14 and 0.18.)

- **`cargo-machete` for unused dependencies; false positives go in `[workspace.metadata.cargo-machete] ignored`.** Why: cheap, regex-based, catches leftovers after a refactor. Source: [cargo-machete](https://github.com/bnjbvr/cargo-machete).
- **Git dependencies only with a pinned `rev` and a comment saying why; prefer crates.io.** Why: reproducibility and licence audit. Source: [Cargo, specifying dependencies from git](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#specifying-dependencies-from-git-repositories).
- **Check weight before adding a crate: `cargo tree -d`, `cargo build --timings`.** Why: one heavy dependency in the wrong crate defeats the split; the lock binary and `triton-config` stay free of zbus and iced.
- **`kdl` (v2 document model) is the only KDL dependency in the workspace.** Why: it is what makes comment-preserving edits possible, and one parser means one set of quirks. Source: [kdl-rs](https://github.com/kdl-org/kdl-rs).

## 11. Profiles

The product is a process that runs for weeks, so small resident memory and "one service panics, the shell lives" matter more than the last 3% of speed. The repo already has `[profile.dev.package."*"] opt-level = 2`; keep it.

```toml
[profile.dev]
opt-level = 1                       # iced is sluggish at 0, even for our own crates
debug = "line-tables-only"         # smaller binaries, faster links, backtraces still work
split-debuginfo = "unpacked"        # Linux: faster incremental links

[profile.dev.package."*"]
opt-level = 2                       # already in the repo

[profile.release]
opt-level = 3
lto = "fat"                         # single developer, build time is cheap; drop to "thin" if it annoys
codegen-units = 1
panic = "unwind"                    # keep: a service panic is caught as a task JoinError and restarted
strip = "debuginfo"                 # keep symbol names so panic backtraces stay readable
```

Why each: LTO and one codegen unit shrink code and improve inlining across the iced and wgpu graph; `panic = "abort"` (used by cosmic-applets) saves size but a panic in one service would end the whole shell, and the panic strategy is per-profile, not per-package. Measure binary size and RSS, do not guess. Sources: [Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html), [min-sized-rust](https://github.com/johnthagen/min-sized-rust), manifests of cosmic-applets and niri.

**Allocator.** glibc malloc can fragment in a long-lived multithreaded process; Dux tuned jemalloc for this (`research/dux-analysis.md` 1.1, 8). Do not add jemalloc or mimalloc speculatively: first measure the RSS slope with `mise run preview:status` over a soak, then try fewer threads (the single services runtime) and `MALLOC_ARENA_MAX`, and change the allocator last. Source: [glibc malloc tunables](https://www.gnu.org/software/libc/manual/html_node/Memory-Allocation-Tunables.html).

**Fast iteration.** Use `mise run dev` for the real thing and `cargo test -p triton-ui` (no zbus or PipeWire linked) while editing views. A faster linker (`mold` or `lld` via a `.cargo/config.toml` `rustflags = ["-C", "link-arg=-fuse-ld=mold"]`) is optional and must be installed through `nx` first; do not add it speculatively. Source: [Bevy setup guide](https://bevy.org/learn/quick-start/getting-started/setup/).

## 12. Async and concurrency

- **Services share one `current_thread` runtime; there is no `tokio::time::interval` for polling.** Waiting for a deadline with `sleep_until` is fine; a periodic interval needs a review comment saying why no event exists. Why: idle CPU is a product goal. Source: [tokio time](https://docs.rs/tokio/latest/tokio/time/index.html).
- **Never hold a `std::sync::Mutex` guard across `.await`** (`clippy::await_holding_lock` is `deny`). Why: deadlocks and `!Send` futures. Source: [clippy](https://rust-lang.github.io/rust-clippy/master/index.html#await_holding_lock).
- **Bounded channels; coalesce latest-wins state at the sender.** Why: a growing queue is a memory leak, and flat memory is principle 1. Source: [tokio mpsc](https://docs.rs/tokio/latest/tokio/sync/mpsc/index.html).
- **Blocking work (the niri socket if using `niri_ipc::socket::Socket`, directory scans, image decode, `niri validate`) runs on `spawn_blocking` or a named thread, never on the services reactor.** Name every thread with `std::thread::Builder::name`, as the spike does. Why: one blocked reactor stalls every service. Source: [tokio, CPU-bound and blocking code](https://docs.rs/tokio/latest/tokio/index.html#cpu-bound-tasks-and-blocking-code).
- **Config and settings files are written atomically (temp file in the same directory, `fsync`, `rename`) and watched by directory.** Why: editors and our own writers replace files; a half-written file would reload as garbage, and a watch on the file itself loses the inode. Source: [`std::fs::rename`](https://doc.rust-lang.org/std/fs/fn.rename.html), [inotify(7)](https://man7.org/linux/man-pages/man7/inotify.7.html).
- **Every `unsafe` block has a `// SAFETY:` comment, and the module that contains it has `#![allow(unsafe_code)]` with a reason.** Why: `unsafe_code = "deny"` workspace-wide means every use is visible in review. Source: [Rust unsafe guidelines](https://rust-lang.github.io/api-guidelines/documentation.html#unsafe-functions-document-safety-considerations-c-failure).
