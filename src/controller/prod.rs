use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use tower::ServiceExt;

use crate::model::{Asset, Entry, Site};

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

async fn handler(State(site): State<&'static Site>, req: Request) -> Response {
    if !matches!(*req.method(), Method::GET | Method::HEAD) {
        return (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "GET, HEAD")]).into_response();
    }

    match site.lookup(req.uri().path()) {
        Some(Entry::Mem(a)) => respond(a, req.headers()),
        Some(Entry::Disk) => match site.disk.clone().oneshot(req).await {
            Ok(res) => res.into_response(),
            Err(e) => match e {},
        },
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

pub fn routes() -> Router {
    let site: &'static Site = Box::leak(Box::new(Site::load()));
    Router::new().fallback(handler).with_state(site)
}
