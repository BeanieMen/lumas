use axum::{
    Router,
    http::header,
    response::{Html, IntoResponse},
    routing::get,
};

fn render_template(template_name: &str, context: minijinja::Value) -> Html<String> {
    let mut env = minijinja::Environment::new();
    env.set_loader(minijinja::path_loader("templates"));

    let template = env
        .get_template(template_name)
        .expect("template does not exist");

    let rendered = template.render(context).expect("failed to render template");

    Html(rendered)
}

async fn home() -> Html<String> {
    render_template("index.html", minijinja::Value::UNDEFINED)
}

async fn proposal() -> Html<String> {

    render_template(
        "proposal.html",
        minijinja::Value::UNDEFINED
    )
}


async fn styles() -> impl IntoResponse {
    let css_content = std::fs::read_to_string("templates/style.css").unwrap_or_default();

    ([(header::CONTENT_TYPE, "text/css")], css_content)
}

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/", get(home))
        .route("/style.css", get(styles))
        .route("/proposal", get(proposal))
        .route("/style.css", get(styles));
    
    let address = "127.0.0.1:3000";
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .expect("failed to bind to address");

    println!("Server running on http://{address}");

    axum::serve(listener, app)
        .await
        .expect("server failed to start");
}
