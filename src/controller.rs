use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::get,
};
use std::{convert::Infallible, path::Path, sync::Arc};
use tokio::sync::broadcast;
use tokio_stream::{Stream, StreamExt, wrappers::BroadcastStream};
use tower::ServiceExt;
use tower_http::services::ServeDir;

use crate::model::{Asset, Site};

pub struct AppState {
    pub is_dev: bool,
    pub site: Site,
    pub disk: ServeDir,
    pub tx: Option<broadcast::Sender<()>>,
    pub _watcher: Option<notify::RecommendedWatcher>,
}

fn spawn_watcher(tx: broadcast::Sender<()>) -> Option<notify::RecommendedWatcher> {
    use notify::{RecursiveMode, Watcher};

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            if !ev.kind.is_access() {
                let _ = tx.send(());
            }
        }
    })
    .ok()?;

    for dir in ["templates", "static"] {
        if Path::new(dir).exists() {
            let _ = watcher.watch(Path::new(dir), RecursiveMode::Recursive);
        }
    }
    Some(watcher)
}

async fn livereload(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state
        .tx
        .as_ref()
        .expect("livereload without watcher")
        .subscribe();
    let stream = BroadcastStream::new(rx).map(|_| Ok(Event::default().event("reload").data("")));
    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn respond(asset: &Asset, headers: &HeaderMap) -> Response {
    if let Some(inm) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
    {
        if inm
            .split(',')
            .any(|t| t.trim().trim_start_matches("W/") == asset.etag.trim_start_matches("W/"))
        {
            return (
                StatusCode::NOT_MODIFIED,
                [
                    (header::ETAG, asset.etag.as_str()),
                    (header::CACHE_CONTROL, "public, max-age=0, must-revalidate"),
                ],
            )
                .into_response();
        }
    }

    let ae = headers
        .get(header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let (encoding, bytes) = if ae.contains("br") && asset.br.is_some() {
        (Some("br"), asset.br.as_ref().unwrap())
    } else if ae.contains("gzip") && asset.gzip.is_some() {
        (Some("gzip"), asset.gzip.as_ref().unwrap())
    } else {
        (None, &asset.plain)
    };

    let mut res = Response::new(Body::from(bytes.clone()));
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(asset.content_type),
    );
    h.insert(header::ETAG, HeaderValue::from_str(&asset.etag).unwrap());
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=0, must-revalidate"),
    );
    if asset.gzip.is_some() || asset.br.is_some() {
        h.insert(header::VARY, HeaderValue::from_static("accept-encoding"));
    }
    if let Some(enc) = encoding {
        h.insert(header::CONTENT_ENCODING, HeaderValue::from_static(enc));
    }
    res
}

async fn handler(State(state): State<Arc<AppState>>, req: Request) -> Response {
    if !matches!(*req.method(), Method::GET | Method::HEAD) {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, "GET, HEAD")],
        )
            .into_response();
    }

    let is_head = req.method() == Method::HEAD;
    let path = req.uri().path();

    let mut res = if state.is_dev {
        let stem = path
            .trim_matches('/')
            .strip_suffix(".html")
            .unwrap_or(path.trim_matches('/'));
        let stem = if stem.is_empty() { "index" } else { stem };
        let template_file = format!("{stem}.html");

        if Path::new("templates").join(&template_file).is_file() {
            match crate::view::render(&template_file, true) {
                Ok(html) => (
                    StatusCode::OK,
                    [
                        (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                        (header::CACHE_CONTROL, "no-store"),
                    ],
                    html,
                ).into_response(),
                Err(err) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                    format!("<body style=\"font-family:monospace;padding:2rem\"><h2>Template error: {template_file}</h2><pre>{err}</pre>{}</body>", crate::view::SCRIPT),
                ).into_response(),
            }
        } else {
            match state.disk.clone().oneshot(req).await {
                Ok(mut res) => {
                    res.headers_mut()
                        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                    res.into_response()
                }
                Err(_) => StatusCode::NOT_FOUND.into_response(),
            }
        }
    } else {
        match state.site.pages.get(path) {
            Some(asset) => respond(asset, req.headers()),
            None => match state.disk.clone().oneshot(req).await {
                Ok(res) => res.into_response(),
                Err(_) => StatusCode::NOT_FOUND.into_response(),
            },
        }
    };

    if is_head {
        *res.body_mut() = Body::empty();
    }
    res
}

pub fn routes(is_dev: bool) -> Router {
    let (tx, watcher) = if is_dev {
        let (tx, _) = broadcast::channel(16);
        let watcher = spawn_watcher(tx.clone());
        (Some(tx), watcher)
    } else {
        (None, None)
    };

    let state = Arc::new(AppState {
        is_dev,
        site: Site::load(),
        disk: ServeDir::new("static"),
        tx,
        _watcher: watcher,
    });

    let mut router = Router::new();
    if is_dev {
        router = router.route("/__livereload", get(livereload));
    }
    router.fallback(handler).with_state(state)
}
