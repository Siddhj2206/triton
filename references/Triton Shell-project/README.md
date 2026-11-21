# Triton Shell — entire project

This project’s pages, files, and uploads as this browser tab had them at export.
Anything left out is listed below.

## Contents

### Overview

- `Main.dc.html` — Triton: desktop on the real wallpaper
- `Adwaita.dc.html` — Futura: libadwaita apps

### Foundations

- `Color.dc.html` — Foundations: color
- `Type.dc.html` — Foundations: type, shape, motion
- `Materials.dc.html` — Futura: materials

### Components

- `Controls.dc.html` — Components: controls
- `Containers.dc.html` — Components: containers
- `ComponentIndex.dc.html` — Design system: component index

### Bar and panels

- `BarStates.dc.html` — Shell: bar states
- `MediaBar.dc.html` — Shell: bar media controls
- `Notifications.dc.html` — Shell: notifications
- `OSD.dc.html` — Shell: on-screen displays
- `QuickSettings.dc.html` — Shell: quick settings hand-off

### Launch and switch

- `Launcher.dc.html` — Shell: launcher
- `Overview.dc.html` — Shell: overview
- `WindowSwitcher.dc.html` — Shell: window switcher
- `WindowSwitcherStates.dc.html` — Shell: switcher states
- `Clipboard.dc.html` — Shell: clipboard history
- `ClipboardStates.dc.html` — Shell: clipboard states

### Session

- `Lock.dc.html` — Shell: lock screen
- `Greeter.dc.html` — Shell: greeter
- `LoginStates.dc.html` — Shell: login states
- `Power.dc.html` — Shell: power menu
- `PowerStates.dc.html` — Shell: power states

### Settings and capture

- `Settings.dc.html` — Shell: settings
- `SettingsStates.dc.html` — Shell: settings pages
- `Wallpaper.dc.html` — Shell: wallpaper picker
- `WallpaperStates.dc.html` — Shell: wallpaper states
- `Screenshot.dc.html` — Shell: screenshot
- `ScreenshotStates.dc.html` — Shell: capture states

### Other files

- `canvas.json` — the canvas: its pages, where each artboard sits, its notes and shapes
- `triton/README.md`
- `triton/components.css`
- `triton/sync.py.txt`
- `triton/tokens.css`
- `triton/tokens.json`
- `assets/` — files uploaded to the design (images, fonts, media)
- `_runtime/` — what runs the pages in a browser; not part of the design

## Uploaded files

Images, fonts and media uploaded to the design are written once each under
`assets/` — 1 in this export — and the exported files refer to them there. A
reference a script puts together while the page runs (for example
`"/_blob/" + id`) is not rewritten and does not load from this folder.

A page in a subfolder may not show an upload that reaches it through a script, a data
file or an imported component, because those files link to `assets/` from a different folder.

## Viewing

To view a page, start a local web server in this folder (for example `python3 -m http.server`), then open an artboard’s `.dc.html` file through it. Opening the file directly may not work, because some browsers block its scripts.

## Left out of this export

The export never includes hidden files (files whose names start with a dot) or
files the editor keeps for its own use.
