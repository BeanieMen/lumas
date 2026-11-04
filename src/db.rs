use serde::{Deserialize, Serialize};
use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct User {
    pub id: String,
    pub hca_id: Option<String>,
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone)]
pub struct Db {
    pool: SqlitePool,
}

impl Db {
    pub async fn connect(path: &str) -> Result<Self, sqlx::Error> {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&format!("sqlite://{path}?mode=rwc"))
            .await?;

        sqlx::query("PRAGMA journal_mode = WAL;")
            .execute(&pool)
            .await?;
        sqlx::query("PRAGMA foreign_keys = ON;")
            .execute(&pool)
            .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS users (
                id TEXT PRIMARY KEY,
                hca_id TEXT UNIQUE,
                email TEXT NOT NULL UNIQUE,
                first_name TEXT NOT NULL,
                last_name TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );",
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                token_hash TEXT NOT NULL UNIQUE,
                expires_at TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );",
        )
        .execute(&pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_sessions_token_hash ON sessions(token_hash);")
            .execute(&pool)
            .await?;

        Ok(Self { pool })
    }

    pub async fn get_user(&self, id: &str) -> Result<Option<User>, sqlx::Error> {
        sqlx::query_as::<_, User>(
            "SELECT id, hca_id, email, first_name, last_name, created_at, updated_at
             FROM users WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn upsert_hackclub_user(
        &self,
        identity: &crate::auth::HackClubIdentity,
    ) -> Result<User, sqlx::Error> {
        let email = identity
            .primary_email
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_lowercase();
        let first_name = identity.first_name.as_deref().unwrap_or("Hacker").trim();
        let last_name = identity.last_name.as_deref().unwrap_or("").trim();
        let hca_id = identity.id.trim();

        let existing: Option<User> = sqlx::query_as::<_, User>(
            "SELECT id, hca_id, email, first_name, last_name, created_at, updated_at
             FROM users
             WHERE hca_id = ? OR email = ?
             ORDER BY (hca_id = ?) DESC LIMIT 1",
        )
        .bind(hca_id)
        .bind(&email)
        .bind(hca_id)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(user) = existing {
            sqlx::query(
                "UPDATE users
                 SET hca_id = ?, email = ?, first_name = ?, last_name = ?, updated_at = datetime('now')
                 WHERE id = ?",
            )
            .bind(hca_id)
            .bind(&email)
            .bind(first_name)
            .bind(last_name)
            .bind(&user.id)
            .execute(&self.pool)
            .await?;

            self.get_user(&user.id)
                .await?
                .ok_or(sqlx::Error::RowNotFound)
        } else {
            let id = uuid::Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO users (id, hca_id, email, first_name, last_name, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, datetime('now'), datetime('now'))",
            )
            .bind(&id)
            .bind(hca_id)
            .bind(&email)
            .bind(first_name)
            .bind(last_name)
            .execute(&self.pool)
            .await?;

            self.get_user(&id).await?.ok_or(sqlx::Error::RowNotFound)
        }
    }

    pub async fn create_session(&self, user_id: &str, token_hash: &str) -> Result<(), sqlx::Error> {
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO sessions (id, user_id, token_hash, expires_at, created_at)
             VALUES (?, ?, ?, datetime('now', '+21 days'), datetime('now'))",
        )
        .bind(id)
        .bind(user_id)
        .bind(token_hash)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_user_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<User>, sqlx::Error> {
        sqlx::query_as::<_, User>(
            "SELECT u.id, u.hca_id, u.email, u.first_name, u.last_name, u.created_at, u.updated_at
             FROM sessions s
             JOIN users u ON s.user_id = u.id
             WHERE s.token_hash = ? AND s.expires_at > datetime('now')",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn delete_expired_sessions(&self) -> Result<u64, sqlx::Error> {
        let res = sqlx::query("DELETE FROM sessions WHERE expires_at <= datetime('now')")
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected())
    }

    pub async fn delete_session(&self, token_hash: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::HackClubIdentity;

    #[tokio::test]
    async fn test_db_user_and_session_lifecycle() {
        let db = Db::connect(":memory:").await.expect("connect to memory db");

        let identity = HackClubIdentity {
            id: "hc_12345".into(),
            first_name: Some("Alice".into()),
            last_name: Some("Lovelace".into()),
            primary_email: Some("alice@hackclub.com".into()),
        };

        let user = db
            .upsert_hackclub_user(&identity)
            .await
            .expect("upsert user");
        assert_eq!(user.first_name, "Alice");
        assert_eq!(user.email, "alice@hackclub.com");
        assert_eq!(user.hca_id.as_deref(), Some("hc_12345"));

        // Upsert update
        let updated_identity = HackClubIdentity {
            id: "hc_12345".into(),
            first_name: Some("Alice V2".into()),
            last_name: Some("Lovelace".into()),
            primary_email: Some("alice@hackclub.com".into()),
        };
        let updated_user = db
            .upsert_hackclub_user(&updated_identity)
            .await
            .expect("update user");
        assert_eq!(updated_user.id, user.id);
        assert_eq!(updated_user.first_name, "Alice V2");

        // Session creation and lookup
        let token_hash = "mock_token_hash_value_1234567890";
        db.create_session(&user.id, token_hash)
            .await
            .expect("create session");

        let session_user = db
            .get_user_by_token_hash(token_hash)
            .await
            .expect("lookup session");
        assert!(session_user.is_some());
        assert_eq!(session_user.unwrap().id, user.id);

        // Session deletion
        db.delete_session(token_hash).await.expect("delete session");
        let deleted = db
            .get_user_by_token_hash(token_hash)
            .await
            .expect("lookup deleted");
        assert!(deleted.is_none());
    }
}
