#!/bin/sh
# shot.sh <board.html> <out.png> <width> <height>
# Renders a design board in headless Chromium at device scale 2, matching the
# 2x physical pixels iced_test's Simulator produces.
set -eu
exec /usr/lib64/chromium-browser/headless_shell \
  --no-sandbox --disable-gpu --hide-scrollbars \
  --force-device-scale-factor=2 \
  --virtual-time-budget=20000 \
  --window-size="$3,$4" \
  --screenshot="$2" \
  "file://$1"
