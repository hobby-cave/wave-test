use tokio::runtime::Builder;
use tracing::{error, info};

use crate::app::Ui;

mod app;

fn main() {
    println!("app startup");

    app::log_setup();
    info!("tracing init");

    let mut ui = match Ui::new() {
        Ok(v) => v,
        Err(err) => {
            error!("create ui error {:?}", err);
            return;
        }
    };
    info!("ui init");

    let runtime = match Builder::new_multi_thread().enable_all().build() {
        Ok(v) => v,
        Err(err) => {
            error!("create tokio runtime error {:?}", err);
            return;
        }
    };

    let gpu = match runtime.block_on(ui.get_gpu()) {
        Ok(v) => v,
        Err(err) => {
            error!("create gpu error {:?}", err);
            return;
        }
    };
    info!("gpu init");

    runtime.spawn(gpu.ignite());
    info!("gpu task begin");

    info!("enter ui event loop");
    ui.run();
    info!("exit ui event loop");
    info!("quit");
}
