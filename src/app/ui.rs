use std::sync::{Arc, Weak};

use anyhow::{Context, Result};
use parking_lot::RwLock;
use tracing::trace;
use winit::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{EventLoop, EventLoopBuilder, EventLoopProxy},
    window::{Window, WindowBuilder},
};

use crate::app::Gpu;

#[derive(Debug, Clone)]
pub enum UiMessage {
    ProgressUpdate,
}

pub struct Ui {
    event_loop: EventLoop<UiMessage>,
    window: Window,
    gpu: RwLock<Weak<Gpu>>,
}

impl Ui {
    pub fn new() -> Result<Self> {
        let event_loop = EventLoopBuilder::<UiMessage>::with_user_event().build();

        let window = WindowBuilder::new()
            .with_title("wave test")
            .with_inner_size(LogicalSize::new(800, 600))
            .build(&event_loop)
            .context("create window")?;

        Ok(Self {
            event_loop,
            window,
            gpu: Default::default(),
        })
    }

    pub fn create_proxy(&self) -> EventLoopProxy<UiMessage> {
        self.event_loop.create_proxy()
    }

    pub fn get_window(&self) -> &Window {
        &self.window
    }

    pub fn get_gpu(&mut self) -> Result<Arc<Gpu>> {
        if let Some(gpu) = self.gpu.read().upgrade() {
            return Ok(gpu);
        }

        let mut guard = self.gpu.write();
        if let Some(gpu) = guard.upgrade() {
            return Ok(gpu);
        }

        let gpu = Gpu::new(self).context("new gpu")?;
        *guard = Arc::downgrade(&gpu);
        Ok(gpu)
    }

    #[allow(clippy::single_match, clippy::collapsible_match)]
    pub fn run(self) {
        self.event_loop.run(move |event, _target, flow| {
            trace!("event: {:?}", event);

            flow.set_wait();

            match event {
                Event::WindowEvent { event, .. } => match event {
                    WindowEvent::CloseRequested => {
                        flow.set_exit();
                    }
                    _ => {}
                },
                Event::MainEventsCleared => {
                    self.window.request_redraw();
                }
                Event::RedrawRequested(..) => {
                    if let Some(gpu) = self.gpu.read().upgrade() {
                        gpu.draw();
                    }
                }
                Event::UserEvent(message) => match message {
                    UiMessage::ProgressUpdate => {
                        self.window.request_redraw();
                    }
                },
                _ => {}
            }
        });
    }
}
