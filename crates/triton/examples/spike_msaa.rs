//! Spike: does wgpu's MSAA resolve produce coverage on this backend/adapter?
//!
//! iced draws jagged paths on Vulkan even with `antialiasing: true` (MSAA x4),
//! while GL is smooth. This isolates the driver/wgpu layer from iced: it draws
//! one triangle whose hypotenuse runs diagonally through a 256x256 image, into
//! a multisampled texture (`MSAA` env, default 4), resolves it, reads the
//! pixels back and reports the coverage values along the edge. Working MSAA
//! shows intermediate values (64/128/191 for x4); a broken resolve shows only
//! 0 and 255.
//!
//! It also lists every adapter wgpu sees, which answers why iced falls back to
//! tiny-skia under lavapipe: run with
//! `VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json WGPU_BACKEND=vulkan`.
//!
//! Runs headless (no surface), so it works in the preview container:
//!
//! ```sh
//! cargo build --release --example spike_msaa
//! podman run --rm --device /dev/dri --security-opt label=disable \
//!   -v "$PWD:/work:ro" -v "$(readlink -f target):$(readlink -f target):ro" \
//!   -e WGPU_BACKEND=vulkan localhost/triton-preview:latest \
//!   /work/target/release/examples/spike_msaa
//! ```

use iced_futures::futures::executor::block_on;
use iced_renderer::wgpu::wgpu;

const SIZE: u32 = 256;

const SHADER: &str = r"
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    // Right triangle covering the top-left half; the hypotenuse is the
    // anti-diagonal through the image centre.
    var pos = array<vec2f, 3>(
        vec2f(-1.0, -1.0),
        vec2f(1.0, -1.0),
        vec2f(-1.0, 1.0),
    );
    return vec4f(pos[i], 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4f {
    return vec4f(1.0, 1.0, 1.0, 1.0);
}
";

#[expect(
    clippy::too_many_lines,
    reason = "a spike: enumerating, rendering and reporting in one pass is clearer"
)]
fn main() {
    let sample_count: u32 = std::env::var("MSAA")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());

    println!("adapters:");
    for adapter in instance.enumerate_adapters(wgpu::Backends::all()) {
        let info = adapter.get_info();
        println!(
            "  {:?} {:?} | {} | driver {} {}",
            info.backend, info.device_type, info.name, info.driver, info.driver_info
        );
    }

    let Some(adapter) = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::None,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .ok() else {
        println!("request_adapter: none");
        return;
    };
    let info = adapter.get_info();
    println!(
        "using {:?} {:?} | {} | driver {} {}",
        info.backend, info.device_type, info.name, info.driver, info.driver_info
    );

    let (device, queue) = match block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
    {
        Ok(pair) => pair,
        Err(e) => {
            println!("request_device: {e}");
            return;
        }
    };

    // iced's Wayland surfaces are sRGB (e.g. Bgra8UnormSrgb); its MSAA targets
    // use the surface format, so `SPIKE_FORMAT` mirrors that. Mesa has a
    // precedent for MSAA silently breaking on sRGB resolves (wgpu#725).
    let format = match std::env::var("SPIKE_FORMAT").as_deref() {
        Ok("bgra-srgb") => wgpu::TextureFormat::Bgra8UnormSrgb,
        Ok("srgb") => wgpu::TextureFormat::Rgba8UnormSrgb,
        Ok("bgra") => wgpu::TextureFormat::Bgra8Unorm,
        _ => wgpu::TextureFormat::Rgba8Unorm,
    };
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: None,
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState {
            count: sample_count,
            ..wgpu::MultisampleState::default()
        },
        multiview: None,
        cache: None,
    });

    let resolved = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let msaa = (sample_count > 1).then(|| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
    });
    let resolved_view = resolved.create_view(&wgpu::TextureViewDescriptor::default());
    let msaa_view = msaa.map(|t| t.create_view(&wgpu::TextureViewDescriptor::default()));
    let (view, resolve_target) = match &msaa_view {
        Some(view) => (view, Some(&resolved_view)),
        None => (&resolved_view, None),
    };

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..wgpu::RenderPassDescriptor::default()
        });
        pass.set_pipeline(&pipeline);
        pass.draw(0..3, 0..1);
    }

    let bytes_per_row = SIZE * 4;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(bytes_per_row) * u64::from(SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &resolved,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(SIZE),
            },
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);

    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .expect("poll");
    let pixels = slice.get_mapped_range().to_vec();
    readback.unmap();

    // The hypotenuse crosses each row once. Report the distinct coverage
    // values and the middle row's edge neighbourhood.
    let value = |x: u32, y: u32| pixels[((y * SIZE + x) * 4) as usize];
    let mut distinct: Vec<u8> = (0..SIZE * SIZE)
        .map(|i| pixels[(i * 4) as usize])
        .collect::<std::collections::BTreeSet<u8>>()
        .into_iter()
        .collect();
    distinct.sort_unstable();
    let mid = SIZE / 2;
    let run: Vec<u8> = (mid.saturating_sub(8)..mid + 8)
        .map(|x| value(x, mid))
        .collect();
    println!("sample_count {sample_count}");
    println!("distinct pixel values: {distinct:?}");
    println!("middle row x={}..{}: {run:?}", mid - 8, mid + 8);
    let intermediate = distinct.iter().filter(|&&v| v > 0 && v < 255).count();
    println!(
        "verdict: {}",
        if intermediate > 0 {
            "MSAA resolve works (intermediate coverage values)"
        } else {
            "NO antialiasing (only 0/255)"
        }
    );
}
