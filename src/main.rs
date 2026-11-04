#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod auth;
mod controller;
mod db;
mod model;
mod view;

use std::sync::Arc;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let dev = matches!(
        std::env::var("APP_ENV").as_deref(),
        Ok("dev" | "development")
    );

    let db_path = std::env::var("DATABASE_PATH").unwrap_or_else(|_| "ysws.db".into());
    let db = db::Db::connect(&db_path)
        .await
        .expect("failed to initialize sqlite database");

    let auth_config = auth::HackClubConfig::from_env();
    if auth_config.is_configured() {
        println!(
            "[auth] Hack Club OAuth ready (callback: {})",
            auth_config.redirect_uri
        );
    } else {
        println!(
            "[auth] Hack Club OAuth not configured (set HACKCLUB_CLIENT_ID and HACKCLUB_CLIENT_SECRET in .env)"
        );
    }
    let auth = Arc::new(auth::AuthManager::new(auth_config));

    let address = "127.0.0.1:3000";
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .expect("failed to bind address");

    println!(
        "[{}] http://{address}{}",
        if dev { "dev" } else { "prod" },
        if dev { " (live reload on)" } else { "" }
    );

    axum::serve(listener, controller::routes(dev, auth, db))
        .await
        .expect("server failed");
}
