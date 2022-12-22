use tracing::{error, info, subscriber::set_global_default, Level};
use tracing_subscriber::fmt;

mod gpu;

#[tokio::main]
async fn main() {
    set_global_default(fmt().with_max_level(Level::DEBUG).finish()).expect("setup tracing");
    info!("app startup");

    info!("gpu task begin");
    match gpu::run().await {
        Ok(_) => {
            info!("gpu task done");
        }
        Err(err) => {
            error!("gpu task error {:?}", err);
        }
    }
}
