use axum::{response::Html, routing::get, Router};
use tower_http::services::ServeDir;

fn render_template(template_name: &str, context: minijinja::Value) -> Html<String> {
    let mut env = minijinja::Environment::new();
    env.set_loader(minijinja::path_loader("templates"));

    let template = env
        .get_template(template_name)
        .expect("template does not exist");

    let rendered = template.render(context).expect("failed to render template");

    Html(rendered)
}

fn build_routes(mut app: Router) -> Router {
    if let Ok(entries) = std::fs::read_dir("templates") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("html") {
                if let Some(file_name) = path.file_name().and_then(|s| s.to_str()) {
                    if file_name == "base.html" || file_name.starts_with('_') {
                        continue;
                    }

                    let stem = path.file_stem().unwrap().to_str().unwrap().to_string();
                    let route = if stem == "index" {
                        "/".to_string()
                    } else {
                        format!("/{stem}")
                    };

                    let template_name: &'static str =
                        Box::leak(file_name.to_string().into_boxed_str());
                    let route_path: &'static str = Box::leak(route.into_boxed_str());

                    app = app.route(
                        route_path,
                        get(move || async move {
                            render_template(template_name, minijinja::Value::UNDEFINED)
                        }),
                    );
                }
            }
        }
    }

    app
}

#[tokio::main]
async fn main() {
    let app = build_routes(Router::new()).fallback_service(ServeDir::new("web"));

    let address = "127.0.0.1:3000";
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .expect("failed to bind to address");

    println!("Server running on http://{address}");

    axum::serve(listener, app)
        .await
        .expect("server failed to start");
}
