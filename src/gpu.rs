use std::{borrow::Cow, num::NonZeroU32};

use anyhow::{Context, Error, Result};
use bytemuck::{bytes_of, Pod, Zeroable};
use tracing::info;
use wgpu::{
    util::{BufferInitDescriptor, DeviceExt},
    Backends, BindGroupDescriptor, BindGroupEntry, BufferUsages, CommandEncoderDescriptor,
    ComputePassDescriptor, ComputePipelineDescriptor, DeviceDescriptor, ErrorFilter, Extent3d,
    ImageCopyBuffer, ImageDataLayout, Instance, Maintain, RequestAdapterOptions,
    ShaderModuleDescriptor, ShaderSource, TextureDescriptor, TextureDimension, TextureFormat,
    TextureUsages,
};

macro_rules! checked_device_op {
    ($ctx:literal, $device:expr, $op:block) => {{
        $device.push_error_scope(ErrorFilter::Validation);
        let r = $op;
        match $device.pop_error_scope().await {
            None => r,
            Some(err) => return Err(Error::msg(format!(concat!($ctx, " error {}"), err))),
        }
    }};

    (sync $ctx:literal, $device:expr, $op:block) => {{
        $device.push_error_scope(ErrorFilter::Validation);
        let r = $op;
        match Handle::current().block_on($device.pop_error_scope()) {
            None => r,
            Some(err) => return Err(Error::msg(format!(concat!($ctx, " error {}"), err))),
        }
    }};
}

#[repr(C, align(1))]
#[derive(Copy, Clone, Pod, Zeroable)]
struct ComputeScene {
    time: f32,
    freq: f32,
    count: u32,
    width: u32,
    height: u32,
}

const WIDTH: u32 = 1024; // make sure as multiply of 256
const HEIGHT: u32 = 1024;
const POINT: u32 = 8;
const FREQUENCY: u32 = 43000;

pub async fn run() -> Result<()> {
    let instance = Instance::new(if cfg!(target_family = "wasm") {
        Backends::BROWSER_WEBGPU
    } else if cfg!(windows) {
        Backends::DX12 | Backends::DX11
    } else if cfg!(target_vendor = "apple") {
        Backends::METAL
    } else if cfg!(target_os = "linux") {
        Backends::VULKAN
    } else {
        Backends::all()
    });

    let adapter = instance
        .request_adapter(&RequestAdapterOptions {
            power_preference: Default::default(),
            force_fallback_adapter: false,
            compatible_surface: None,
        })
        .await
        .ok_or_else(|| Error::msg("no adapter found"))?;

    let info = adapter.get_info();
    info!("adapter {}", info.name);
    info!("  vendor {}", info.vendor);
    info!("  device type {:?}", info.device_type);
    info!("  backend {:?}", info.backend);
    info!("  driver info {:?}", info.driver_info);

    let (device, queue) = adapter
        .request_device(
            &DeviceDescriptor {
                ..Default::default()
            },
            None,
        )
        .await
        .context("request gpu device")?;

    let limits = device.limits();
    info!(
        "  max worker size ({}, {}, {})",
        limits.max_compute_workgroup_size_x,
        limits.max_compute_workgroup_size_y,
        limits.max_compute_workgroup_size_z
    );
    let shader = checked_device_op!("create shader", device, {
        device.create_shader_module(ShaderModuleDescriptor {
            label: Some("compute.wgsl"),
            source: ShaderSource::Wgsl(Cow::Borrowed(include_str!("./shaders/compute.wgsl"))),
        })
    });

    let compute_pipe = checked_device_op!("create compute pipeline", device, {
        device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("compute:step:pipe"),
            layout: None,
            module: &shader,
            entry_point: "step",
        })
    });

    let bind_group_layout = checked_device_op!("get bind group layout", device, {
        compute_pipe.get_bind_group_layout(0)
    });
    let scene_buf = checked_device_op!("create uniform scene buf", device, {
        let scene = ComputeScene {
            time: 0.0,
            freq: FREQUENCY as f32,
            count: POINT,
            width: WIDTH,
            height: HEIGHT,
        };
        device.create_buffer_init(&BufferInitDescriptor {
            label: Some("compute:bind:scene"),
            contents: bytes_of(&scene),
            usage: BufferUsages::UNIFORM,
        })
    });
    let buf_size = WIDTH as u64 * HEIGHT as u64 * 16; // rgba 4 * f32 4 = 16
    let output_buf = checked_device_op!("create output buf", device, {
        let content = vec![0; buf_size as usize];
        device.create_buffer_init(&BufferInitDescriptor {
            label: Some("compute:bind:output:storage"),
            contents: &content,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        })
    });
    let bind_group = checked_device_op!("create bind group", device, {
        device.create_bind_group(&BindGroupDescriptor {
            label: Some("compute:pipe:bind"),
            layout: &bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: scene_buf.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: output_buf.as_entire_binding(),
                },
            ],
        })
    });

    let mut encoder = checked_device_op!("create compute command encoder", device, {
        device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("compute:step:encoder"),
        })
    });

    let mut pass = checked_device_op!("start compute pass", device, {
        encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("compute:step:pass"),
        })
    });
    pass.set_pipeline(&compute_pipe);
    pass.set_bind_group(0, &bind_group, &[]);
    pass.dispatch_workgroups(WIDTH, HEIGHT, 1);
    drop(pass);

    // create texture
    let texture = checked_device_op!("create output texture", device, {
        device.create_texture(&TextureDescriptor {
            label: Some("compute:output:texture"),
            size: Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::COPY_SRC | TextureUsages::COPY_DST,
        })
    });

    checked_device_op!("write output texture", device, {
        encoder.copy_buffer_to_texture(
            ImageCopyBuffer {
                buffer: &output_buf,
                layout: ImageDataLayout {
                    offset: 0,
                    bytes_per_row: NonZeroU32::new(WIDTH * 16),
                    rows_per_image: NonZeroU32::new(HEIGHT),
                },
            },
            texture.as_image_copy(),
            Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
        );
    });

    let index = checked_device_op!("submit compute", device, {
        queue.submit([encoder.finish()])
    });
    checked_device_op!("wait compute done", device, {
        device.poll(Maintain::WaitForSubmissionIndex(index));
    });

    Ok(())
}
