mod model;

use axum::{
    http::header,
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use model::Model;
use std::time::SystemTime;

fn render_template(template_name: &str, context: minijinja::Value) -> Html<String> {
    let mut env = minijinja::Environment::new();
    env.set_loader(minijinja::path_loader("templates"));

    let template = env
        .get_template(template_name)
        .expect("template does not exist");

    let rendered = template
        .render(context)
        .expect("failed to render template");

    Html(rendered)
}

async fn home() -> Html<String> {
    let model = Model::get();

    render_template(
        "index.html",
        minijinja::context! {
            title => model.title,
            heading => model.heading,
            message => model.message,
        },
    )
}

async fn styles() -> impl IntoResponse {
    let css_content = std::fs::read_to_string("templates/style.css")
        .unwrap_or_default();

    ([(header::CONTENT_TYPE, "text/css")], css_content)
}

async fn dev_version() -> String {
    let mut latest = SystemTime::UNIX_EPOCH;

    if let Ok(entries) = std::fs::read_dir("templates") {
        for entry in entries.flatten() {
            if let Ok(metadata) = entry.metadata() {
                if let Ok(modified) = metadata.modified() {
                    latest = latest.max(modified);
                }
            }
        }
    }

    format!("{latest:?}")
}

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/", get(home))
        .route("/style.css", get(styles))
        .route("/dev/version", get(dev_version));

    let address = "127.0.0.1:3000";
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .expect("failed to bind to address");

    println!("Server running on http://{address}");

    axum::serve(listener, app)
        .await
        .expect("server failed to start");
}
