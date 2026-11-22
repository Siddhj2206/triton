#!/bin/sh
# dev/nested/session.sh — isolated nested-niri session for triton surfaces.
#
# Launches `niri` as a window inside the host session with a fully private
# environment: its own XDG_RUNTIME_DIR, scratch HOME and XDG dirs (nothing ever
# touches the real $HOME or $HOME/.config/niri). Run under a private session
# bus:
#
#   REPO=$PWD dbus-run-session -- sh dev/nested/session.sh target/debug/triton
#
# NEVER `niri --session` — that would export this environment to systemd and
# the real session bus.

set -eu

BIN="${1:?usage: session.sh <path-to-binary> [args...]}"
shift || true
REPO="${REPO:-$PWD}"
ROOT="$REPO/target/nested-niri"
RT="$ROOT/runtime"
SCRATCH_HOME="$ROOT/home"

# Resolve the host compositor socket to an ABSOLUTE path before overriding
# XDG_RUNTIME_DIR — after that, bare names like `wayland-1` would resolve
# inside the private runtime dir and niri would not be a nested client.
if [ -z "${WAYLAND_DISPLAY:-}" ]; then
    echo "dev/nested: WAYLAND_DISPLAY is not set — run inside a Wayland session" >&2
    exit 1
fi
case "$WAYLAND_DISPLAY" in
    /*) HOST_SOCKET="$WAYLAND_DISPLAY" ;;
    *)  HOST_SOCKET="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/$WAYLAND_DISPLAY" ;;
esac
if [ ! -S "$HOST_SOCKET" ]; then
    echo "dev/nested: host Wayland socket $HOST_SOCKET does not exist" >&2
    exit 1
fi

# Private runtime + scratch XDG dirs. Everything lives under target/ so a
# `cargo clean`-style wipe removes all trace; nothing reads or writes the
# host's real HOME or ~/.config/niri.
#
# The real runtime dir lives at a SHORT path under the host runtime dir:
# Unix socket paths are limited to SUN_LEN (108 bytes), and deep checkouts
# (worktrees under ~/.t3) push $ROOT/runtime past that — niri's IPC socket
# would fail to bind. $ROOT/runtime is a symlink to it so `mise run msg` and
# everything else still reaches the session through target/nested-niri.
HOST_RT="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
REAL_RT="$HOST_RT/triton-nested-$(printf '%s' "$ROOT" | cksum | cut -d' ' -f1)"
rm -rf "$REAL_RT"
rm -rf "$RT"
mkdir -p -m 700 "$REAL_RT"
mkdir -p "$ROOT"
ln -s "$REAL_RT" "$RT"
mkdir -p \
    "$SCRATCH_HOME/.config/niri" \
    "$SCRATCH_HOME/.local/share" \
    "$SCRATCH_HOME/.cache" \
    "$SCRATCH_HOME/.local/state"

cleanup() {
    rm -rf "$RT"
    rm -rf "$REAL_RT"
}
trap cleanup EXIT HUP INT TERM

cp "$REPO/dev/nested/niri.kdl" "$SCRATCH_HOME/.config/niri/config.kdl"

export HOME="$SCRATCH_HOME"
export XDG_RUNTIME_DIR="$REAL_RT"
export XDG_CONFIG_HOME="$SCRATCH_HOME/.config"
export XDG_DATA_HOME="$SCRATCH_HOME/.local/share"
export XDG_CACHE_HOME="$SCRATCH_HOME/.cache"
export XDG_STATE_HOME="$SCRATCH_HOME/.local/state"
export WAYLAND_DISPLAY="$HOST_SOCKET"

# Keep every GSettings/dconf read and write in-process: `triton --real-services`
# inside the nested child may exercise the theme pipeline's `gsettings` calls,
# and the memory backend guarantees the host dconf database is never touched.
export GSETTINGS_BACKEND=memory

# Never inherit the host's niri IPC socket or real session D-Bus. When launched
# under `dbus-run-session` (TRITON_NESTED_PRIVATE_BUS=1), keep that private bus;
# otherwise (e.g. Fedora Atomic with dbus-broker), detach from the host bus.
unset NIRI_SOCKET
if [ "${TRITON_NESTED_PRIVATE_BUS:-0}" != "1" ]; then
    unset DBUS_SESSION_BUS_ADDRESS
fi

# Foreground child + `wait`, never `exec`: `exec` would replace this shell so
# the EXIT trap above would never run and $RT/$REAL_RT would leak.
niri -c "$XDG_CONFIG_HOME/niri/config.kdl" -- "$BIN" "$@" &
pid=$!
wait "$pid"
