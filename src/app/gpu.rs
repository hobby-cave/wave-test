use std::{borrow::Cow, future::Future, sync::Arc};

use anyhow::{Context, Error, Result};
use parking_lot::Mutex;
use tracing::{instrument, warn};
use wgpu::{
    Adapter, Backends, CompositeAlphaMode, Device, DeviceDescriptor, ErrorFilter, Instance,
    PresentMode, Queue, RequestAdapterOptions, ShaderModuleDescriptor, ShaderSource, Surface,
    SurfaceConfiguration, TextureFormat, TextureUsages,
};
use winit::event_loop::EventLoopProxy;

use crate::app::{ui::UiMessage, Ui};

pub struct Gpu {
    ui: Mutex<EventLoopProxy<UiMessage>>,
    instance: Instance,
    surface: Surface,
    surface_config: SurfaceConfiguration,
    adapter: Adapter,
    device: Device,
    queue: Queue,
}

impl Gpu {
    pub async fn new(ui: &Ui) -> Result<Arc<Self>> {
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
        let adapter = instance
            .request_adapter(&RequestAdapterOptions {
                power_preference: Default::default(),
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await
            .ok_or_else(|| Error::msg("no adapter found"))?;

        let (device, queue) = adapter
            .request_device(
                &DeviceDescriptor {
                    ..Default::default()
                },
                None,
            )
            .await
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

    pub fn ignite(self: Arc<Self>) -> impl 'static + Future<Output = ()> + Send {
        async move {
            let result = self.compute().await;
            self.send_message(UiMessage::ComputeComplete(Arc::new(result)));
        }
    }

    #[instrument(skip_all)]
    async fn compute(self: &Arc<Self>) -> Result<()> {
        let shader = self
            .checked_device_op(async {
                self.device.create_shader_module(ShaderModuleDescriptor {
                    label: Some("compute.wgsl"),
                    source: ShaderSource::Wgsl(Cow::Borrowed(include_str!(
                        "../shaders/compute.wgsl"
                    ))),
                })
            })
            .await
            .context("create shader")?;
        
        Ok(())
    }

    fn send_message(&self, message: UiMessage) {
        if let Err(err) = self.ui.lock().send_event(message) {
            warn!("send ui message error {}", err);
        }
    }

    async fn checked_device_op<F, R>(&self, fut: F) -> Result<R>
    where
        F: Future<Output = R>,
    {
        self.device.push_error_scope(ErrorFilter::Validation);
        let r = fut.await;
        if let Some(err) = self.device.pop_error_scope().await {
            return Err(Error::msg(format!("device error {}", err)));
        }
        Ok(r)
    }
}
