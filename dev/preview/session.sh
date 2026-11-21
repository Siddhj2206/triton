#!/bin/sh
# Entry point inside the container, already under dbus-run-session (a private session bus).
#   session.sh headless   headless sway -> nested niri
#   session.sh nested     nested niri as a window on the host desktop (/run/host-wayland)
# TRITON_CMD, if set, is what niri starts (e.g. /work/target/debug/triton).
set -eu
mode=${1:-headless}

export LANG=C.UTF-8 XDG_RUNTIME_DIR=/run/triton-preview
rm -rf "$XDG_RUNTIME_DIR" && mkdir -p -m 700 "$XDG_RUNTIME_DIR"

# Fresh scratch home on every start: deterministic, and never the user's config.
export HOME=/preview/home
rm -rf "$HOME"
export XDG_CONFIG_HOME="$HOME/.config" XDG_DATA_HOME="$HOME/.local/share" \
       XDG_CACHE_HOME="$HOME/.cache" XDG_STATE_HOME="$HOME/.local/state"
mkdir -p "$XDG_CONFIG_HOME/niri" "$XDG_DATA_HOME" "$XDG_CACHE_HOME" "$XDG_STATE_HOME" \
         /preview/screenshots
cp /work/dev/preview/niri.kdl "$XDG_CONFIG_HOME/niri/config.kdl"
mkdir -p "$XDG_CONFIG_HOME/foot"   # foot is the blur probe: translucent, so blur shows
printf '[colors-dark]\nalpha=0.55\n' > "$XDG_CONFIG_HOME/foot/foot.ini"

case "$mode" in
headless)
    WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 sway -c /work/dev/preview/sway.conf \
        >"$XDG_RUNTIME_DIR/sway.log" 2>&1 &
    i=0
    until sock=$(ls "$XDG_RUNTIME_DIR" | grep -m1 '^wayland-[0-9]*$'); do
        i=$((i + 1)); [ "$i" -lt 100 ] || { cat "$XDG_RUNTIME_DIR/sway.log"; exit 1; }
        sleep 0.1
    done
    export WAYLAND_DISPLAY="$sock"
    ;;
nested)
    export WAYLAND_DISPLAY=/run/host-wayland
    ;;
*) echo "unknown mode: $mode" >&2; exit 2 ;;
esac

# Never --session: that would export this environment to systemd and the session bus.
if [ -n "${TRITON_CMD:-}" ]; then
    exec niri -c "$XDG_CONFIG_HOME/niri/config.kdl" -- $TRITON_CMD
else
    exec niri -c "$XDG_CONFIG_HOME/niri/config.kdl"
fi
