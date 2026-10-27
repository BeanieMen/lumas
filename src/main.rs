#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod controller;
mod model;
mod view;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let dev = matches!(
        std::env::var("APP_ENV").as_deref(),
        Ok("dev" | "development")
    );
    let address = "127.0.0.1:3000";
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .expect("failed to bind address");

    println!(
        "[{}] http://{address}{}",
        if dev { "dev" } else { "prod" },
        if dev { " (live reload on)" } else { "" }
    );

    axum::serve(listener, controller::routes(dev))
        .await
        .expect("server failed");
}
