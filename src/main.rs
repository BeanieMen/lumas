#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod controller;
mod model;
mod view;

fn is_dev() -> bool {
    matches!(std::env::var("APP_ENV").as_deref(), Ok("dev" | "development"))
}

#[tokio::main]
async fn main() {
    use axum::serve::ListenerExt;

    dotenvy::dotenv().ok();

    let dev = is_dev();
    let app = controller::routes(dev);

    let address = "127.0.0.1:3000";
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .expect("failed to bind to address")
        .tap_io(|tcp| {
            let _ = tcp.set_nodelay(true);
        });

    println!(
        "[{}] http://{address}{}",
        if dev { "dev" } else { "prod" },
        if dev { " (live reload on)" } else { "" }
    );
    axum::serve(listener, app).await.expect("server failed");
}
