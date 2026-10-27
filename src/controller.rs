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
use std::{convert::Infallible, path::Path, time::Duration};
use tokio::sync::broadcast;
use tokio_stream::{Stream, StreamExt, wrappers::BroadcastStream};
use tower::ServiceExt;

use crate::model::{Asset, Entry, Site};

type Tx = broadcast::Sender<()>;

pub struct AppState {
    pub is_dev: bool,
    pub site: Site,
    pub tx: Option<Tx>,
}

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
            while nrx.recv_timeout(Duration::from_millis(50)).is_ok() {}
            let _ = tx2.send(());
        }
    });

    tx
}

async fn livereload(State(state): State<&'static AppState>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let tx = state.tx.as_ref().expect("livereload called without watcher");
    let stream = BroadcastStream::new(tx.subscribe())
        .map(|_| Ok(Event::default().event("reload").data("")));
    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn accepts(header: &str, coding: &str) -> bool {
    header.split(',').any(|part| {
        let mut it = part.split(';');
        if !it.next().unwrap_or("").trim().eq_ignore_ascii_case(coding) {
            return false;
        }
        it.all(|p| match p.trim().strip_prefix("q=") {
            Some(q) => q.trim().parse::<f32>().map_or(true, |q| q > 0.0),
            None => true,
        })
    })
}

fn etag_matches(inm: &[u8], etag: &[u8]) -> bool {
    let bare = |t: &[u8]| -> usize { if t.starts_with(b"W/") { 2 } else { 0 } };
    let e = &etag[bare(etag)..];
    inm.split(|&b| b == b',').any(|t| {
        let t = t.trim_ascii();
        t == b"*" || &t[bare(t)..] == e
    })
}

fn respond(a: &Asset, h: &HeaderMap) -> Response {
    if let Some(inm) = h.get(header::IF_NONE_MATCH) {
        if etag_matches(inm.as_bytes(), a.etag.as_bytes()) {
            let mut res = Response::new(Body::empty());
            *res.status_mut() = StatusCode::NOT_MODIFIED;
            *res.headers_mut() = a.not_modified.clone();
            return res;
        }
    }

    let ae = h
        .get(header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let v = a
        .br
        .as_ref()
        .filter(|_| accepts(ae, "br"))
        .or_else(|| a.gzip.as_ref().filter(|_| accepts(ae, "gzip")))
        .unwrap_or(&a.plain);

    let mut res = Response::new(Body::from(v.body.clone()));
    *res.headers_mut() = v.headers.clone();
    res
}

async fn handler(State(state): State<&'static AppState>, req: Request) -> Response {
    if !matches!(*req.method(), Method::GET | Method::HEAD) {
        return (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "GET, HEAD")]).into_response();
    }

    if state.is_dev {
        let path = req.uri().path().trim_matches('/');
        let stem = path.strip_suffix(".html").unwrap_or(path);
        let stem = if stem.is_empty() { "index" } else { stem };

        let valid = stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            && stem != "base"
            && !stem.starts_with('_');

        if valid {
            let file = format!("{stem}.html");
            if Path::new("templates").join(&file).is_file() {
                let mut res = crate::view::render_dev(&file);
                res.headers_mut()
                    .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                return res;
            }
        }

        match state.site.disk.clone().oneshot(req).await {
            Ok(mut res) => {
                res.headers_mut()
                    .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                res.into_response()
            }
            Err(e) => match e {},
        }
    } else {
        match state.site.lookup(req.uri().path()) {
            Some(Entry::Mem(a)) => respond(a, req.headers()),
            Some(Entry::Disk) => match state.site.disk.clone().oneshot(req).await {
                Ok(res) => res.into_response(),
                Err(e) => match e {},
            },
            None => StatusCode::NOT_FOUND.into_response(),
        }
    }
}

pub fn routes(is_dev: bool) -> Router {
    let tx = if is_dev { Some(spawn_watcher()) } else { None };
    let site = Site::load();
    let state: &'static AppState = Box::leak(Box::new(AppState { is_dev, site, tx }));

    let mut router = Router::new();
    if is_dev {
        router = router.route("/__livereload", get(livereload));
    }
    router.fallback(handler).with_state(state)
}
