use std::sync::Arc;

use anyhow::{Context, Error, Result};
use parking_lot::Mutex;
use tokio::runtime::{Builder, Runtime};
use tracing::{instrument, warn};
use wgpu::{
    Adapter, Backends, CompositeAlphaMode, Device, DeviceDescriptor, Instance, PresentMode, Queue,
    RequestAdapterOptions, Surface, SurfaceConfiguration, TextureFormat, TextureUsages,
};
use winit::event_loop::EventLoopProxy;

use crate::app::{ui::UiMessage, Ui};

pub struct Gpu {
    ui: Mutex<EventLoopProxy<UiMessage>>,
    runtime: Runtime,
    instance: Instance,
    surface: Surface,
    surface_config: SurfaceConfiguration,
    adapter: Adapter,
    device: Device,
    queue: Queue,
}

impl Gpu {
    pub fn new(ui: &Ui) -> Result<Arc<Self>> {
        let runtime = Builder::new_multi_thread()
            .enable_all()
            .build()
            .context("build tokio runtime")?;

        let instance = Instance::new(if cfg!(target_family = "wasm") {
            Backends::BROWSER_WEBGPU
        } else if cfg!(windows) {
            Backends::from_iter([Backends::DX12, Backends::DX11])
        } else if cfg!(target_vendor = "apple") {
            Backends::METAL
        } else if cfg!(target_os = "linux") {
            Backends::VULKAN
        } else {
            Backends::all()
        });

        let surface = unsafe { instance.create_surface(ui.get_window()) };
        let adapter = runtime
            .block_on(instance.request_adapter(&RequestAdapterOptions {
                power_preference: Default::default(),
                force_fallback_adapter: false,
                compatible_surface: None,
            }))
            .ok_or_else(|| Error::msg("no adapter found"))?;

        let (device, queue) = runtime
            .block_on(adapter.request_device(
                &DeviceDescriptor {
                    ..Default::default()
                },
                None,
            ))
            .context("request gpu device")?;

        let size = ui.get_window().inner_size();
        let surface_config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format: TextureFormat::Rgba8UnormSrgb,
            width: size.width,
            height: size.height,
            present_mode: PresentMode::Fifo,
            alpha_mode: CompositeAlphaMode::Auto,
        };

        surface.configure(&device, &surface_config);

        Ok(Arc::new(Self {
            ui: Mutex::new(ui.create_proxy()),
            runtime,
            instance,
            surface,
            surface_config,
            adapter,
            device,
            queue,
        }))
    }

    pub fn draw(&self) {
        let surface = match self.surface.get_current_texture() {
            Ok(v) => v,
            Err(err) => {
                warn!("surface get current error {}", err);
                return;
            }
        };

        surface.present();
    }

    pub fn ignite(self: Arc<Self>) {
        self.runtime.spawn(Arc::clone(&self).compute());
    }

    #[instrument(skip_all)]
    async fn compute(self: Arc<Self>) {}

    fn send_message(&self, message: UiMessage) {
        if let Err(err) = self.ui.lock().send_event(message) {
            warn!("send ui message error {}", err);
        }
    }
}
