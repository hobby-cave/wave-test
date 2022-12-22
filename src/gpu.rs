use std::borrow::Cow;

use anyhow::{Context, Error, Result};
use bytemuck::{bytes_of, cast_slice, Pod, Zeroable};
use image::GrayImage;
use tokio::sync::oneshot;
use tracing::{debug, error, info};
use wgpu::{
    util::{BufferInitDescriptor, DeviceExt},
    Backends, BindGroupDescriptor, BindGroupEntry, Buffer, BufferDescriptor, BufferUsages,
    CommandEncoderDescriptor, ComputePassDescriptor, ComputePipelineDescriptor, Device,
    DeviceDescriptor, ErrorFilter, Instance, Maintain, MapMode, Queue, RequestAdapterOptions,
    ShaderModuleDescriptor, ShaderSource,
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

    let output_buf = compute(&device, &queue).await?;
    info!("compute done, start extraction.");

    let image = extract_buf(&device, &queue, output_buf).await?;
    info!("extraction done, save to file.");

    image.save("output.png").context("save image")?;

    Ok(())
}

async fn compute(device: &Device, queue: &Queue) -> Result<Buffer> {
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
    let output_buf = checked_device_op!("create output buf", device, {
        let content = vec![0; WIDTH as usize * HEIGHT as usize * 4];
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

    let index = checked_device_op!("submit compute", device, {
        queue.submit([encoder.finish()])
    });
    checked_device_op!("wait compute done", device, {
        device.poll(Maintain::WaitForSubmissionIndex(index));
    });

    Ok(output_buf)
}

async fn extract_buf(device: &Device, queue: &Queue, buf: Buffer) -> Result<GrayImage> {
    debug_assert_eq!(buf.size(), WIDTH as u64 * HEIGHT as u64 * 4);

    let mut encoder = checked_device_op!("create encoder", device, {
        device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("extract:encoder"),
        })
    });

    let stage = checked_device_op!("create stage buffer", device, {
        device.create_buffer(&BufferDescriptor {
            label: Some("extract:stage"),
            size: buf.size(),
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    });
    checked_device_op!("copy buffer", device, {
        encoder.copy_buffer_to_buffer(&buf, 0, &stage, 0, buf.size())
    });

    let index = checked_device_op!("submit extraction", device, {
        queue.submit([encoder.finish()])
    });
    checked_device_op!("wait extraction", device, {
        device.poll(Maintain::WaitForSubmissionIndex(index));
    });
    info!("extract copy done, read stage");

    let data = {
        let slice = stage.slice(..);
        let (tx, rx) = oneshot::channel();
        slice.map_async(MapMode::Read, move |r| {
            debug!("stage mapped result {:?}", r);
            if let Err(err) = tx.send(r) {
                error!("can't dispatch map result {:?}", err);
            }
        });
        device.poll(Maintain::Wait);
        rx.await.context("wait map stage")?.context("map stage")?;
        let data = slice.get_mapped_range().to_vec();
        stage.unmap();
        data
    };

    let data = cast_slice::<_, f32>(&data)
        .iter()
        .copied()
        .map(|g| (255.0 * g) as u8)
        .collect::<Vec<_>>();
    debug!("image top pixel: {}", data[0]);

    debug!("create GaryImage");
    GrayImage::from_vec(WIDTH, HEIGHT, data).context("create image")
}
