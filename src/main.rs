use axum::{
    Router,
    response::Html,
    routing::get,
};
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

            if path.extension().and_then(|s| s.to_str()) != Some("html") {
                continue;
            }

            let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };

            if file_name == "base.html" || file_name.starts_with('_') {
                continue;
            }

            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };

            let template_name = file_name.to_string();
            let route = if stem == "index" {
                "/".to_string()
            } else {
                format!("/{stem}")
            };

            let handler = move || {
                let template_name = template_name.clone();
                async move {
                    render_template(&template_name, minijinja::Value::UNDEFINED)
                }
            };

            app = app.route(&route, get(handler));

            if stem != "index" {
                let html_route = format!("{route}.html");
                let template_name = file_name.to_string();

                app = app.route(
                    &html_route,
                    get(move || {
                        let template_name = template_name.clone();
                        async move {
                            render_template(&template_name, minijinja::Value::UNDEFINED)
                        }
                    }),
                );
            }
        }
    }

    app.fallback_service(ServeDir::new("templates"))
}

#[tokio::main]
async fn main() {
    let app = build_routes(Router::new());

    let address = "127.0.0.1:3000";
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .expect("failed to bind to address");

    println!("Server running on http://{address}");

    axum::serve(listener, app)
        .await
        .expect("server failed to start");
}
