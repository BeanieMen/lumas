use axum::{
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
};

pub const LIVERELOAD_SCRIPT: &str = r#"<script>(()=>{let lost=false;const c=()=>{const es=new EventSource("/__livereload");es.addEventListener("reload",()=>location.reload());es.onopen=()=>{if(lost)location.reload()};es.onerror=()=>{lost=true;es.close();setTimeout(c,500)}};c()})()</script>"#;

pub fn inject_livereload(mut html: String) -> String {
    match html.rfind("</body>") {
        Some(i) => html.insert_str(i, LIVERELOAD_SCRIPT),
        None => html.push_str(LIVERELOAD_SCRIPT),
    }
    html
}

pub fn create_env() -> minijinja::Environment<'static> {
    let mut env = minijinja::Environment::new();
    env.set_loader(minijinja::path_loader("templates"));
    env
}

pub fn render_dev(file_name: &str) -> Response {
    let env = create_env();

    let result = env
        .get_template(file_name)
        .and_then(|t| t.render(minijinja::context! {}));

    match result {
        Ok(html) => Html(inject_livereload(html)).into_response(),
        Err(e) => {
            let body = format!(
                "<body style=\"font-family:monospace;padding:2rem\"><h2>Template error: {file_name}</h2><pre>{e:#}</pre></body>"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                inject_livereload(body),
            )
                .into_response()
        }
    }
}

pub fn render_template(env: &minijinja::Environment, file_name: &str) -> String {
    env.get_template(file_name)
        .unwrap_or_else(|e| panic!("template {file_name}: {e}"))
        .render(minijinja::context! {})
        .unwrap_or_else(|e| panic!("render {file_name}: {e}"))
}
