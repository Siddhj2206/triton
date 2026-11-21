#!/bin/sh
# `niri msg` against the preview's niri. Run inside the container (mise run preview:msg -- ...).
set -eu
export NIRI_SOCKET=$(ls /run/triton-preview/niri.*.sock 2>/dev/null | head -n1)
[ -n "$NIRI_SOCKET" ] || { echo "preview niri is not running" >&2; exit 1; }
exec niri msg "$@"
