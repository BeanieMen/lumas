use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    io::Write,
    sync::Arc,
};

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
    serve::ListenerExt,
};
use bytes::Bytes;
use tower_http::services::ServeDir;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

struct Page {
    plain: Bytes,
    gzip: Bytes,
    br: Bytes,
    etag: HeaderValue,
}

impl Page {
    fn new(html: String) -> Self {
        let plain = Bytes::from(html);

        let mut hasher = DefaultHasher::new();
        plain.hash(&mut hasher);
        let etag = HeaderValue::from_str(&format!("W/\"{:x}\"", hasher.finish())).unwrap();

        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        gz.write_all(&plain).unwrap();
        let gzip = Bytes::from(gz.finish().unwrap());

        let mut br_buf = Vec::new();
        {
            let mut w = brotli::CompressorWriter::new(&mut br_buf, 4096, 11, 22);
            w.write_all(&plain).unwrap();
        } // dropped here -> stream finalized
        let br = Bytes::from(br_buf);

        Self { plain, gzip, br, etag }
    }
}

// zero rendering zero allocation pick precomputed bytes and send.
async fn serve_page(State(page): State<Arc<Page>>, headers: HeaderMap) -> Response {
    if headers.get(header::IF_NONE_MATCH) == Some(&page.etag) {
        return (
            StatusCode::NOT_MODIFIED,
            [(header::ETAG, page.etag.clone())],
        )
            .into_response();
    }

    let accept = headers
        .get(header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let (body, encoding) = if accept.contains("br") {
        (page.br.clone(), Some("br"))
    } else if accept.contains("gzip") {
        (page.gzip.clone(), Some("gzip"))
    } else {
        (page.plain.clone(), None)
    };

    let mut res = (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8")),
            (header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=0, must-revalidate")),
            (header::VARY, HeaderValue::from_static("accept-encoding")),
            (header::ETAG, page.etag.clone()),
        ],
        body,
    )
        .into_response();

    if let Some(enc) = encoding {
        res.headers_mut()
            .insert(header::CONTENT_ENCODING, HeaderValue::from_static(enc));
    }
    res
}

fn build_routes() -> Router {
    // One environment, built once.
    let mut env = minijinja::Environment::new();
    env.set_loader(minijinja::path_loader("templates"));

    let mut app = Router::new();

    let Ok(entries) = std::fs::read_dir("templates") else {
        return app.fallback_service(ServeDir::new("static"));
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("html") {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else { continue };
        if file_name == "base.html" || file_name.starts_with('_') {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };

        // Render + compress ONCE, at startup. Template errors fail fast here.
        let html = env
            .get_template(file_name)
            .unwrap_or_else(|e| panic!("template {file_name}: {e}"))
            .render(minijinja::context! {})
            .unwrap_or_else(|e| panic!("render {file_name}: {e}"));
        let page = Arc::new(Page::new(html));

        let route = if stem == "index" { "/".to_string() } else { format!("/{stem}") };
        app = app.route(&route, get(serve_page).with_state(page.clone()));

        if stem != "index" {
            app = app.route(&format!("{route}.html"), get(serve_page).with_state(page));
        }
    }

    // Static assets: precompressed variants (.br / .gz next to the file) are served as-is.
    app.fallback_service(
        ServeDir::new("static")
            .precompressed_br()
            .precompressed_gzip(),
    )
}

#[tokio::main]
async fn main() {
    let app = build_routes();

    let address = "127.0.0.1:3000";
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .expect("failed to bind to address")
        .tap_io(|tcp| {
            let _ = tcp.set_nodelay(true);
        });

    println!("Server running on http://{address}");

    axum::serve(listener, app).await.expect("server failed");
}