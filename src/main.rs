#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod controller;
mod model;
mod view;

fn is_dev() -> bool {
    matches!(
        std::env::var("APP_ENV").as_deref(),
        Ok("dev" | "development")
    )
}

fn main() {
    if is_dev() {
        // Tokio is only started in dev; prod never touches it.
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to build tokio runtime")
            .block_on(controller::dev::run());
    } else {
        controller::prod::run();
    }
}
