use axum::{
    Router,
    extract::{Request, State},
    http::{HeaderValue, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::get,
    serve::ListenerExt,
};
use std::{convert::Infallible, path::Path, time::Duration};
use tokio::sync::broadcast;
use tokio_stream::{Stream, StreamExt, wrappers::BroadcastStream};
use tower::ServiceExt;
use tower_http::services::ServeDir;

type Tx = broadcast::Sender<()>;

fn spawn_watcher() -> Tx {
    use notify::{RecursiveMode, Watcher};

    let (tx, _) = broadcast::channel(16);
    let (ntx, nrx) = std::sync::mpsc::channel::<()>();

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            if !ev.kind.is_access() {
                let _ = ntx.send(());
            }
        }
    })
    .expect("failed to create file watcher");

    for dir in ["templates", "static"] {
        if Path::new(dir).exists() {
            watcher
                .watch(Path::new(dir), RecursiveMode::Recursive)
                .unwrap_or_else(|e| panic!("watch {dir}: {e}"));
        }
    }

    let tx2 = tx.clone();
    std::thread::spawn(move || {
        let _watcher = watcher;
        while nrx.recv().is_ok() {
            // debounce bursts of events (editors write several times per save)
            while nrx.recv_timeout(Duration::from_millis(50)).is_ok() {}
            let _ = tx2.send(());
        }
    });

    tx
}

async fn livereload(State(tx): State<Tx>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = BroadcastStream::new(tx.subscribe())
        .map(|_| Ok(Event::default().event("reload").data("")));
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn serve(req: Request) -> Response {
    let path = req.uri().path().trim_matches('/');
    let stem = path.strip_suffix(".html").unwrap_or(path);
    let stem = if stem.is_empty() { "index" } else { stem };

    let valid = stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && stem != "base"
        && !stem.starts_with('_');

    if valid {
        let file = format!("{stem}.html");
        if Path::new("templates").join(&file).is_file() {
            return crate::view::render_dev(&file);
        }
    }

    match ServeDir::new("static").oneshot(req).await {
        Ok(res) => res.into_response(),
        Err(e) => match e {},
    }
}

// no-store: otherwise the browser may keep serving stale CSS/JS after a live reload.
async fn handler(req: Request) -> Response {
    let mut res = serve(req).await;
    res.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

fn routes() -> Router {
    Router::new()
        .route("/__livereload", get(livereload))
        .fallback(handler)
        .with_state(spawn_watcher())
}

pub async fn run() {
    let app = routes();
    let address = "127.0.0.1:3000";
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .expect("failed to bind to address")
        .tap_io(|tcp| {
            let _ = tcp.set_nodelay(true);
        });

    println!("[dev] http://{address} (live reload on)");
    axum::serve(listener, app).await.expect("server failed");
}
