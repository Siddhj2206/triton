# Patched dependencies

Wired in through `[patch.crates-io]` in the root `Cargo.toml`. Each patch is the
upstream crate with the smallest possible change; drop it when upstream fixes it.

## softbuffer 0.4.8

`src/backends/wayland/buffer.rs`: `wl_shm::Format::Xrgb8888` → `Argb8888`.

iced's tiny-skia renderer (ADR-0005) presents through softbuffer, which always
allocates Wayland buffers without alpha, so transparent pixels (the corners of the
floating bar, any rounded panel) show as black. tiny-skia already writes
premultiplied ARGB, which is exactly what `Argb8888` means, and every Wayland
compositor must support that format. Upstream softbuffer is adding alpha support in a
later release; iced 0.14 pins 0.4.
