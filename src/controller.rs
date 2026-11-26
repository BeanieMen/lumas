use axum::{
    Router,
    body::Body,
    extract::{Form, Query, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    response::{
        IntoResponse, Redirect, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
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

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[derive(Deserialize)]
struct LoginQuery {
    email: Option<String>,
    next: Option<String>,
}

async fn hackclub_login(
    State(state): State<Arc<AppState>>,
    Query(query): Query<LoginQuery>,
    headers: HeaderMap,
) -> Response {
    if !state.auth.config.is_configured() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            "Hack Club OAuth is not configured. Please set HACKCLUB_CLIENT_ID and HACKCLUB_CLIENT_SECRET in .env",
        )
            .into_response();
    }
    if !browser_has_login_cookie(&headers) {
        return Redirect::to("/?collect-details=1").into_response();
    }

    let oauth_state = uuid::Uuid::new_v4().to_string();
    state
        .auth
        .store_state(
            oauth_state.clone(),
            query.email.clone(),
            query.next,
            None,
            None,
        )
        .await;
    let url = state
        .auth
        .authorize_url(&oauth_state, query.email.as_deref());
    Redirect::to(&url).into_response()
}

#[derive(Deserialize)]
struct SignupForm {
    email: Option<String>,
    name: Option<String>,
    age: Option<i64>,
}

async fn hackclub_signup(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(form): Form<SignupForm>,
) -> Response {
    let email = form
        .email
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let name = form
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let signup_details = match (name, form.age) {
        (Some(name), Some(age)) if name.chars().count() <= 100 && (1..=18).contains(&age) => {
            Some((name.to_owned(), age))
        }
        (None, None) => None,
        _ => return (StatusCode::BAD_REQUEST, "Invalid signup details").into_response(),
    };
    if signup_details.is_none() && !browser_has_login_cookie(&headers) {
        return (
            StatusCode::BAD_REQUEST,
            "Name and age are required for first-time sign-in",
        )
            .into_response();
    }
    if email.is_some_and(|email| {
        email.len() > 254 || !email.contains('@') || email.chars().any(char::is_whitespace)
    }) {
        return (StatusCode::BAD_REQUEST, "Invalid signup details").into_response();
    }

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
        .store_state(
            oauth_state.clone(),
            email.map(str::to_owned),
            None,
            signup_details.as_ref().map(|(name, _)| name.clone()),
            signup_details.map(|(_, age)| age),
        )
        .await;
    let url = state.auth.authorize_url(&oauth_state, email);
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
                html_escape(&desc)
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
                    html_escape(&err)
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

    if let (Some(signup_name), Some(age)) = (oauth_state.signup_name.as_deref(), oauth_state.age) {
        if let Err(e) = state
            .db
            .set_signup_details(&user.id, signup_name, age)
            .await
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error saving signup details: {e}"),
            )
                .into_response();
        }
    }

    let (token, hash) = AuthManager::generate_session_token();
    if let Err(e) = state.db.create_session(&user.id, &hash).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Database error creating session: {e}"),
        )
            .into_response();
    }

    let first_signup = oauth_state.signup_name.is_some() && oauth_state.age.is_some();
    let show_welcome = if first_signup {
        match state.db.mark_first_welcome_shown(&user.id).await {
            Ok(shown) => shown,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Database error tracking first welcome: {e}"),
                )
                    .into_response();
            }
        }
    } else {
        false
    };
    let mut redirect_target = if show_welcome {
        "/welcome".into()
    } else {
        oauth_state
            .next
            .filter(|n| n.starts_with('/') && !n.starts_with("//"))
            .unwrap_or_else(|| "/".into())
    };
    redirect_target.push(if redirect_target.contains('?') {
        '&'
    } else {
        '?'
    });
    redirect_target.push_str("auth=success");

    let cookie_val = state.auth.session_cookie(&token);
    let mut res = Redirect::to(&redirect_target).into_response();
    if let Ok(hv) = HeaderValue::from_str(&cookie_val) {
        res.headers_mut().insert(header::SET_COOKIE, hv);
    }
    let browser_cookie = format!(
        "lumas_browser_login=1; Path=/; Max-Age=31536000; HttpOnly; SameSite=Lax{}",
        if state.auth.config.cookie_secure {
            "; Secure"
        } else {
            ""
        }
    );
    if let Ok(hv) = HeaderValue::from_str(&browser_cookie) {
        res.headers_mut().append(header::SET_COOKIE, hv);
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

fn browser_has_login_cookie(headers: &HeaderMap) -> bool {
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|cookies| {
            cookies
                .split(';')
                .map(str::trim)
                .any(|cookie| cookie == "lumas_browser_login=1")
        })
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
            match crate::view::render(&template_file, true, user.as_ref(), state.auth.config.is_configured()) {
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
            match crate::view::render(
                &template_file,
                false,
                user.as_ref(),
                state.auth.config.is_configured(),
            ) {
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

async fn security_headers(req: Request, next: axum::middleware::Next) -> Response {
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(
        header::HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    h.insert(
        header::HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY"),
    );
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
        site: Site::load(auth.config.is_configured()),
        disk: ServeDir::new("static"),
        tx,
        _watcher: watcher,
        auth,
        db,
    });

    let mut router = Router::new()
        .route("/auth/hackclub/login", get(hackclub_login))
        .route("/auth/hackclub/signup", post(hackclub_signup))
        .route("/auth/hackclub/callback", get(hackclub_callback))
        .route("/auth/logout", axum::routing::post(logout))
        .route("/api/v1/me", get(current_user_me))
        .route("/api/me", get(current_user_me));

    if is_dev {
        router = router.route("/__livereload", get(livereload));
    }

    // Periodically purge expired sessions so the table does not grow unbounded.
    let sweep_db = state.db.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
        tick.tick().await; // first tick fires immediately: sweep at startup too
        loop {
            sweep_db.delete_expired_sessions().await.ok();
            tick.tick().await;
        }
    });

    router
        .fallback(handler)
        .with_state(state)
        .layer(axum::middleware::from_fn(security_headers))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::HackClubConfig;

    #[tokio::test]
    async fn signup_redirect_keeps_details_in_server_side_oauth_state() {
        let auth = Arc::new(AuthManager::new(HackClubConfig {
            client_id: "client-id".into(),
            client_secret: "client-secret".into(),
            redirect_uri: "http://localhost/auth/hackclub/callback".into(),
            cookie_secure: false,
        }));
        let db = Db::connect(":memory:")
            .await
            .expect("connect test database");
        let app_state = Arc::new(AppState {
            is_dev: false,
            site: Site::load(true),
            disk: ServeDir::new("static"),
            tx: None,
            _watcher: None,
            auth: auth.clone(),
            db,
        });
        let app = Router::new()
            .route("/auth/hackclub/login", get(hackclub_login))
            .route("/auth/hackclub/signup", post(hackclub_signup))
            .with_state(app_state);

        let first_login_request = Request::builder()
            .uri("/auth/hackclub/login")
            .body(Body::empty())
            .expect("build first login request");
        let first_login_response = app
            .clone()
            .oneshot(first_login_request)
            .await
            .expect("send first login request");
        assert_eq!(first_login_response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            first_login_response
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok()),
            Some("/?collect-details=1")
        );

        let returning_login_request = Request::builder()
            .uri("/auth/hackclub/login")
            .header(header::COOKIE, "lumas_browser_login=1")
            .body(Body::empty())
            .expect("build returning login request");
        let returning_login_response = app
            .clone()
            .oneshot(returning_login_request)
            .await
            .expect("send returning login request");
        assert_eq!(returning_login_response.status(), StatusCode::SEE_OTHER);
        assert!(
            returning_login_response
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .is_some_and(
                    |location| location.starts_with("https://auth.hackclub.com/oauth/authorize")
                )
        );

        let invalid_request = Request::builder()
            .method(Method::POST)
            .uri("/auth/hackclub/signup")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from("email=ada%40example.com&name=Ada&age=19"))
            .expect("build invalid signup request");
        let invalid_response = app
            .clone()
            .oneshot(invalid_request)
            .await
            .expect("send invalid signup request");
        assert_eq!(invalid_response.status(), StatusCode::BAD_REQUEST);

        let returning_browser_request = Request::builder()
            .method(Method::POST)
            .uri("/auth/hackclub/signup")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::empty())
            .expect("build missing-cache signup request");
        let returning_browser_response = app
            .clone()
            .oneshot(returning_browser_request)
            .await
            .expect("send missing-cache signup request");
        assert_eq!(returning_browser_response.status(), StatusCode::BAD_REQUEST);

        let returning_browser_request = Request::builder()
            .method(Method::POST)
            .uri("/auth/hackclub/signup")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(header::COOKIE, "lumas_browser_login=1")
            .body(Body::empty())
            .expect("build returning browser signup request");
        let returning_browser_response = app
            .clone()
            .oneshot(returning_browser_request)
            .await
            .expect("send returning browser signup request");
        assert!(returning_browser_response.status().is_redirection());
        let returning_browser_url = returning_browser_response
            .headers()
            .get(header::LOCATION)
            .expect("OAuth redirect location")
            .to_str()
            .expect("valid redirect location");
        let returning_browser_state = returning_browser_url
            .split("state=")
            .nth(1)
            .expect("OAuth state")
            .split('&')
            .next()
            .expect("state value");
        let returning_browser_oauth_state = auth
            .take_state(returning_browser_state)
            .await
            .expect("server-side OAuth state");
        assert_eq!(returning_browser_oauth_state.signup_name, None);
        assert_eq!(returning_browser_oauth_state.age, None);

        let first_browser_request = Request::builder()
            .method(Method::POST)
            .uri("/auth/hackclub/signup")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from("name=Browser%20Hacker&age=16"))
            .expect("build first browser signup request");
        let first_browser_response = app
            .clone()
            .oneshot(first_browser_request)
            .await
            .expect("send first browser signup request");
        assert!(first_browser_response.status().is_redirection());
        let first_browser_url = first_browser_response
            .headers()
            .get(header::LOCATION)
            .expect("OAuth redirect location")
            .to_str()
            .expect("valid redirect location");
        assert!(!first_browser_url.contains("email="));
        let first_browser_state = first_browser_url
            .split("state=")
            .nth(1)
            .expect("OAuth state")
            .split('&')
            .next()
            .expect("state value");
        let first_browser_oauth_state = auth
            .take_state(first_browser_state)
            .await
            .expect("server-side OAuth state");
        assert_eq!(
            first_browser_oauth_state.signup_name.as_deref(),
            Some("Browser Hacker")
        );
        assert_eq!(first_browser_oauth_state.age, Some(16));

        let request = Request::builder()
            .method(Method::POST)
            .uri("/auth/hackclub/signup")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(
                "email=ada%40example.com&name=Ada%20Lovelace&age=16",
            ))
            .expect("build signup request");
        let response = app.oneshot(request).await.expect("send signup request");
        assert!(response.status().is_redirection());
        let authorize_url = response
            .headers()
            .get(header::LOCATION)
            .expect("OAuth redirect location")
            .to_str()
            .expect("valid redirect location");
        assert!(authorize_url.contains("email=ada%40example.com"));
        assert!(!authorize_url.contains("Lovelace"));
        assert!(!authorize_url.contains("age=16"));

        let state_key = authorize_url
            .split("state=")
            .nth(1)
            .expect("OAuth state")
            .split('&')
            .next()
            .expect("state value");
        let oauth_state = auth
            .take_state(state_key)
            .await
            .expect("server-side OAuth state");
        assert_eq!(oauth_state.signup_name.as_deref(), Some("Ada Lovelace"));
        assert_eq!(oauth_state.age, Some(16));
    }
}
