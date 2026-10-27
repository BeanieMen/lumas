mod model;

use axum::{response::Html, routing::get, Router};
use model::Model;
use tower_http::services::ServeDir;

fn render_template(path: &str, ctx: minijinja::Value) -> Html<String> {
    let mut env = minijinja::Environment::new();
    env.set_loader(minijinja::path_loader("web"));
    let tmpl = env.get_template(path).unwrap();
    Html(tmpl.render(ctx).unwrap())
}

async fn index() -> Html<String> {
    let data = Model::get();
    render_template(
        "index.html",
        minijinja::context! {
            title => data.title,
            heading => data.heading,
            message => data.message,
        },
    )
}

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/", get(index))
        .fallback_service(ServeDir::new("web"));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await.unwrap();
    println!("Server running on http://127.0.0.1:3000");
    axum::serve(listener, app).await.unwrap();
}
