use std::time::Duration;

use tokio::runtime::Builder;
use tracing::info;

fn main() {
    tracing_setup::setup();
    info!("tracing setup");

    let runtime = Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("setup tokio runtime");

    let ui = runtime.block_on(ui_setup::UI::spawn());
    info!("ui setup");

    runtime.block_on(ui.wait());
    info!("ui quit");

    runtime.shutdown_timeout(Duration::from_millis(100));
    info!("tokio shutdown");
}

mod tracing_setup {
    use tracing::{subscriber::set_global_default, Level};
    use tracing_subscriber::fmt;

    pub fn setup() {
        set_global_default(fmt().with_max_level(Level::DEBUG).finish()).expect("setup tracing");
    }
}

mod ui_setup {
    use std::sync::Arc;

    use tokio::{
        sync::{oneshot, Notify},
        task::spawn_blocking,
    };
    use tracing::{debug, info, instrument};
    #[cfg(unix)]
    use winit::platform::unix::EventLoopBuilderExtUnix;
    #[cfg(windows)]
    use winit::platform::windows::EventLoopBuilderExtWindows;
    use winit::{
        dpi::LogicalSize,
        event::{Event, WindowEvent},
        event_loop::{EventLoopBuilder, EventLoopProxy},
        window::WindowBuilder,
    };

    #[derive(Clone, Debug)]
    pub enum Message {}

    #[allow(dead_code)]
    #[derive(Clone)]
    pub struct UI {
        proxy: EventLoopProxy<Message>,
        quit: Arc<Notify>,
    }

    impl UI {
        pub async fn spawn() -> UI {
            let (tx, rx) = oneshot::channel();
            let quit = Arc::new(Notify::new());
            {
                let quit = Arc::clone(&quit);
                spawn_blocking(move || Self::run(tx, quit));
            }
            UI {
                proxy: rx.await.expect("receive event loop proxy"),
                quit,
            }
        }

        pub async fn wait(&self) {
            self.quit.notified().await;
        }

        #[allow(clippy::collapsible_match, clippy::single_match)]
        #[instrument("uiLoop", skip_all)]
        fn run(tx: oneshot::Sender<EventLoopProxy<Message>>, quit: Arc<Notify>) {
            let event_loop = EventLoopBuilder::with_user_event()
                .with_any_thread(true)
                .build();
            info!("event loop create");

            let window = WindowBuilder::new()
                .with_inner_size(LogicalSize::new(800, 600))
                .with_resizable(false)
                .build(&event_loop)
                .expect("create window");
            info!("window create");

            tx.send(event_loop.create_proxy())
                .expect("dispatch event loop proxy");
            info!("event loop proxy dispatched");

            event_loop.run(move |event, _target, flow| {
                debug!("event: {:?}", event);

                flow.set_wait();

                match event {
                    Event::WindowEvent { event, .. } => match event {
                        WindowEvent::CloseRequested => {
                            flow.set_exit();
                        }
                        _ => {}
                    },
                    Event::MainEventsCleared => {
                        window.request_redraw();
                    }
                    Event::LoopDestroyed => {
                        quit.notify_waiters();
                    }
                    _ => {}
                }
            });
        }
    }
}
