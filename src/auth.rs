use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::RwLock;

#[derive(Clone, Debug)]
pub struct HackClubConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub cookie_secure: bool,
}

impl HackClubConfig {
    pub fn from_env() -> Self {
        let app_url = std::env::var("APP_URL").unwrap_or_else(|_| "http://localhost:3000".into());
        let redirect_uri = std::env::var("HACKCLUB_REDIRECT_URI")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| format!("{app_url}/auth/hackclub/callback"));

        let cookie_secure = std::env::var("COOKIE_SECURE")
            .ok()
            .and_then(|v| v.parse::<bool>().ok())
            .unwrap_or_else(|| {
                matches!(
                    std::env::var("APP_ENV").as_deref(),
                    Ok("prod" | "production")
                )
            });

        Self {
            client_id: std::env::var("HACKCLUB_CLIENT_ID").unwrap_or_default(),
            client_secret: std::env::var("HACKCLUB_CLIENT_SECRET").unwrap_or_default(),
            redirect_uri,
            cookie_secure,
        }
    }

    pub fn is_configured(&self) -> bool {
        !self.client_id.trim().is_empty() && !self.client_secret.trim().is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HackClubIdentity {
    pub id: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub primary_email: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HackClubMePayload {
    pub identity: HackClubIdentity,
}

#[derive(Debug, Clone)]
pub struct OAuthState {
    #[allow(dead_code)]
    pub email: Option<String>,
    pub next: Option<String>,
    pub created_at: Instant,
}

pub struct AuthManager {
    pub config: HackClubConfig,
    client: reqwest::Client,
    states: Arc<RwLock<HashMap<String, OAuthState>>>,
}

impl AuthManager {
    pub fn new(config: HackClubConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            states: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn store_state(&self, state: String, email: Option<String>, next: Option<String>) {
        let mut map = self.states.write().await;
        let now = Instant::now();
        map.retain(|_, v| now.duration_since(v.created_at) < Duration::from_secs(600));
        map.insert(
            state,
            OAuthState {
                email,
                next,
                created_at: now,
            },
        );
    }

    pub async fn take_state(&self, state: &str) -> Option<OAuthState> {
        let mut map = self.states.write().await;
        let item = map.remove(state)?;
        if item.created_at.elapsed() < Duration::from_secs(600) {
            Some(item)
        } else {
            None
        }
    }

    pub fn authorize_url(&self, state: &str, email: Option<&str>) -> String {
        let mut url = format!(
            "https://auth.hackclub.com/oauth/authorize?client_id={}&redirect_uri={}&response_type=code&scope=openid+profile+email+name&state={}",
            urlencoding(&self.config.client_id),
            urlencoding(&self.config.redirect_uri),
            urlencoding(state),
        );
        if let Some(e) = email {
            url.push_str("&login_hint=");
            url.push_str(&urlencoding(e));
        }
        url
    }

    pub async fn exchange_code(&self, code: &str) -> Result<HackClubIdentity, String> {
        let params = [
            ("client_id", self.config.client_id.as_str()),
            ("client_secret", self.config.client_secret.as_str()),
            ("redirect_uri", self.config.redirect_uri.as_str()),
            ("code", code),
            ("grant_type", "authorization_code"),
        ];

        let token_resp = self
            .client
            .post("https://auth.hackclub.com/oauth/token")
            .form(&params)
            .send()
            .await
            .map_err(|e| format!("token request error: {e}"))?;

        if !token_resp.status().is_success() {
            let status = token_resp.status();
            let body = token_resp.text().await.unwrap_or_default();
            return Err(format!("token exchange failed ({status}): {body}"));
        }

        let token_json: serde_json::Value = token_resp
            .json()
            .await
            .map_err(|e| format!("failed to parse token JSON: {e}"))?;

        let access_token = token_json
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Hack Club token response missing access_token".to_string())?;

        let me_resp = self
            .client
            .get("https://auth.hackclub.com/api/v1/me")
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| format!("profile request error: {e}"))?;

        if !me_resp.status().is_success() {
            let status = me_resp.status();
            let body = me_resp.text().await.unwrap_or_default();
            return Err(format!("identity request failed ({status}): {body}"));
        }

        let payload: HackClubMePayload = me_resp
            .json()
            .await
            .map_err(|e| format!("failed to parse identity JSON: {e}"))?;

        Ok(payload.identity)
    }

    pub fn generate_session_token() -> (String, String) {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let token = URL_SAFE_NO_PAD.encode(bytes);
        let hash = hash_token(&token);
        (token, hash)
    }

    pub fn session_cookie(&self, token: &str) -> String {
        let secure = if self.config.cookie_secure {
            "; Secure"
        } else {
            ""
        };
        format!(
            "session={token}; Path=/; HttpOnly; SameSite=Lax; Priority=High; Max-Age=1814400{secure}"
        )
    }

    pub fn clear_session_cookie(&self) -> String {
        let secure = if self.config.cookie_secure {
            "; Secure"
        } else {
            ""
        };
        format!("session=; Path=/; HttpOnly; SameSite=Lax; Priority=High; Max-Age=0{secure}")
    }
}

pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_authorize_url_generation() {
        let config = HackClubConfig {
            client_id: "client-123".into(),
            client_secret: "secret-456".into(),
            redirect_uri: "http://localhost:3000/auth/hackclub/callback".into(),
            cookie_secure: false,
        };
        let manager = AuthManager::new(config);

        let url = manager.authorize_url("test_state", Some("user@example.com"));
        assert!(url.starts_with("https://auth.hackclub.com/oauth/authorize"));
        assert!(url.contains("client_id=client-123"));
        assert!(
            url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A3000%2Fauth%2Fhackclub%2Fcallback")
        );
        assert!(url.contains("state=test_state"));
        assert!(url.contains("login_hint=user%40example.com"));
    }

    #[tokio::test]
    async fn test_oauth_state_lifecycle() {
        let config = HackClubConfig {
            client_id: "c".into(),
            client_secret: "s".into(),
            redirect_uri: "r".into(),
            cookie_secure: false,
        };
        let manager = AuthManager::new(config);

        manager
            .store_state(
                "state_1".into(),
                Some("a@b.com".into()),
                Some("/proposal".into()),
            )
            .await;
        let item = manager.take_state("state_1").await;
        assert!(item.is_some());
        let item = item.unwrap();
        assert_eq!(item.email.as_deref(), Some("a@b.com"));
        assert_eq!(item.next.as_deref(), Some("/proposal"));

        // Single-use: taking again returns None
        assert!(manager.take_state("state_1").await.is_none());
    }

    #[test]
    fn test_session_tokens_and_cookies() {
        let config = HackClubConfig {
            client_id: "c".into(),
            client_secret: "s".into(),
            redirect_uri: "r".into(),
            cookie_secure: true,
        };
        let manager = AuthManager::new(config);

        let (token, hash) = AuthManager::generate_session_token();
        assert_eq!(token.len(), 43);
        assert_eq!(hash, hash_token(&token));

        let cookie = manager.session_cookie(&token);
        assert!(cookie.contains(&format!("session={token}")));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("Secure"));

        let clear = manager.clear_session_cookie();
        assert!(clear.contains("session=;"));
        assert!(clear.contains("Max-Age=0"));
    }
}
