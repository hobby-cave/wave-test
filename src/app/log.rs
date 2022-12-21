use tracing::{subscriber::set_global_default, Level};
use tracing_subscriber::fmt;

pub fn setup() {
    set_global_default(fmt().with_max_level(Level::DEBUG).finish()).expect("setup tracing");
}
