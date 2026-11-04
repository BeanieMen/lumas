use axum::{
    Router,
    body::Body,
    extract::{Query, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    response::{
        IntoResponse, Redirect, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::get,
};
use serde::Deserialize;
use std::{convert::Infallible, path::Path, sync::Arc};
use tokio::sync::broadcast;
use tokio_stream::{Stream, StreamExt, wrappers::BroadcastStream};
use tower::ServiceExt;
use tower_http::services::ServeDir;

use crate::{
    auth::AuthManager,
    db::{Db, User},
    model::{Asset, Site},
};

pub struct AppState {
    pub is_dev: bool,
    pub site: Site,
    pub disk: ServeDir,
    pub tx: Option<broadcast::Sender<()>>,
    pub _watcher: Option<notify::RecommendedWatcher>,
    pub auth: Arc<AuthManager>,
    pub db: Db,
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

#[derive(Deserialize)]
struct LoginQuery {
    email: Option<String>,
    next: Option<String>,
}

async fn hackclub_login(
    State(state): State<Arc<AppState>>,
    Query(query): Query<LoginQuery>,
) -> Response {
    if !state.auth.config.is_configured() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            "Hack Club OAuth is not configured. Please set HACKCLUB_CLIENT_ID and HACKCLUB_CLIENT_SECRET in .env",
        )
            .into_response();
    }
    let oauth_state = uuid::Uuid::new_v4().to_string();
    state
        .auth
        .store_state(oauth_state.clone(), query.email.clone(), query.next)
        .await;
    let url = state
        .auth
        .authorize_url(&oauth_state, query.email.as_deref());
    Redirect::to(&url).into_response()
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

async fn hackclub_callback(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CallbackQuery>,
) -> Response {
    if let Some(err) = query.error {
        let desc = query.error_description.unwrap_or(err);
        return (
            StatusCode::UNAUTHORIZED,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            format!(
                "<h2>Hack Club sign-in failed</h2><p>{}</p><p><a href=\"/\">Return home</a></p>",
                desc
            ),
        )
            .into_response();
    }

    let Some(state_key) = query.state else {
        return (StatusCode::BAD_REQUEST, "Missing OAuth state").into_response();
    };

    let oauth_state = match state.auth.take_state(&state_key).await {
        Some(s) => s,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                "<h2>Sign-in link expired or invalid</h2><p><a href=\"/auth/hackclub/login\">Try signing in again</a></p>",
            )
                .into_response();
        }
    };

    let Some(code) = query.code else {
        return (StatusCode::BAD_REQUEST, "Missing authorization code").into_response();
    };

    let identity = match state.auth.exchange_code(&code).await {
        Ok(id) => id,
        Err(err) => {
            return (
                StatusCode::UNAUTHORIZED,
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                format!(
                    "<h2>Failed to verify Hack Club identity</h2><p>{}</p><p><a href=\"/auth/hackclub/login\">Try again</a></p>",
                    err
                ),
            )
                .into_response();
        }
    };

    let user = match state.db.upsert_hackclub_user(&identity).await {
        Ok(u) => u,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error saving user: {e}"),
            )
                .into_response();
        }
    };

    let (token, hash) = AuthManager::generate_session_token();
    if let Err(e) = state.db.create_session(&user.id, &hash).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Database error creating session: {e}"),
        )
            .into_response();
    }

    let redirect_target = oauth_state
        .next
        .filter(|n| n.starts_with('/') && !n.starts_with("//"))
        .unwrap_or_else(|| "/".into());

    let cookie_val = state.auth.session_cookie(&token);
    let mut res = Redirect::to(&redirect_target).into_response();
    if let Ok(hv) = HeaderValue::from_str(&cookie_val) {
        res.headers_mut().insert(header::SET_COOKIE, hv);
    }
    res
}

async fn logout(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Some(cookie) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) {
        if let Some(token) = cookie
            .split(';')
            .map(str::trim)
            .find_map(|c| c.strip_prefix("session="))
            .filter(|t| t.len() == 43)
        {
            let hash = crate::auth::hash_token(token);
            let _ = state.db.delete_session(&hash).await;
        }
    }
    let clear_cookie = state.auth.clear_session_cookie();
    let mut res = Redirect::to("/").into_response();
    if let Ok(hv) = HeaderValue::from_str(&clear_cookie) {
        res.headers_mut().insert(header::SET_COOKIE, hv);
    }
    res
}

async fn current_user_me(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    match get_session_user(&state, &headers).await {
        Some(user) => axum::Json(user).into_response(),
        None => (
            StatusCode::UNAUTHORIZED,
            [(header::CONTENT_TYPE, "application/json")],
            r#"{"error":"unauthorized","message":"sign in with Hack Club first"}"#,
        )
            .into_response(),
    }
}

async fn get_session_user(state: &AppState, headers: &HeaderMap) -> Option<User> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    let token = cookie
        .split(';')
        .map(str::trim)
        .find_map(|c| c.strip_prefix("session="))
        .filter(|t| t.len() == 43)?;
    let hash = crate::auth::hash_token(token);
    state.db.get_user_by_token_hash(&hash).await.ok().flatten()
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
    let user = get_session_user(&state, req.headers()).await;

    let mut res = if state.is_dev {
        let stem = path
            .trim_matches('/')
            .strip_suffix(".html")
            .unwrap_or(path.trim_matches('/'));
        let stem = if stem.is_empty() { "index" } else { stem };
        let template_file = format!("{stem}.html");

        if Path::new("templates").join(&template_file).is_file() {
            match crate::view::render(&template_file, true, user.as_ref()) {
                Ok(html) => (
                    StatusCode::OK,
                    [
                        (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                        (header::CACHE_CONTROL, "no-store"),
                    ],
                    html,
                )
                    .into_response(),
                Err(err) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                    format!(
                        "<body style=\"font-family:monospace;padding:2rem\"><h2>Template error: {template_file}</h2><pre>{err}</pre>{}</body>",
                        crate::view::SCRIPT
                    ),
                )
                    .into_response(),
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
        let stem = path
            .trim_matches('/')
            .strip_suffix(".html")
            .unwrap_or(path.trim_matches('/'));
        let stem = if stem.is_empty() { "index" } else { stem };
        let template_file = format!("{stem}.html");

        if user.is_some() && Path::new("templates").join(&template_file).is_file() {
            match crate::view::render(&template_file, false, user.as_ref()) {
                Ok(html) => (
                    StatusCode::OK,
                    [
                        (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                        (header::CACHE_CONTROL, "private, no-cache"),
                    ],
                    html,
                )
                    .into_response(),
                Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            }
        } else {
            match state.site.pages.get(path) {
                Some(asset) => respond(asset, req.headers()),
                None => match state.disk.clone().oneshot(req).await {
                    Ok(res) => res.into_response(),
                    Err(_) => StatusCode::NOT_FOUND.into_response(),
                },
            }
        }
    };

    if is_head {
        *res.body_mut() = Body::empty();
    }
    res
}

pub fn routes(is_dev: bool, auth: Arc<AuthManager>, db: Db) -> Router {
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
        auth,
        db,
    });

    let mut router = Router::new()
        .route("/auth/hackclub/login", get(hackclub_login))
        .route("/auth/hackclub/callback", get(hackclub_callback))
        .route("/auth/logout", get(logout).post(logout))
        .route("/api/v1/me", get(current_user_me))
        .route("/api/me", get(current_user_me));

    if is_dev {
        router = router.route("/__livereload", get(livereload));
    }
    router.fallback(handler).with_state(state)
}
