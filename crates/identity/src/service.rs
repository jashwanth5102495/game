use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;
use whisper_common::{AnonymousAccount, AppError, AvatarConfig, Result};

pub struct IdentityService {
    pool: SqlitePool,
}

impl IdentityService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn register(
        &self,
        public_key: String,
        dh_public_key: String,
        display_name: String,
        avatar_config: AvatarConfig,
        recovery_hash: Option<String>,
    ) -> Result<AnonymousAccount> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let avatar_json = serde_json::to_string(&avatar_config)
            .map_err(|e| AppError::Internal(e.to_string()))?;

        sqlx::query!(
            r#"
            INSERT INTO accounts (id, public_key, dh_public_key, display_name, avatar_config, recovery_hash, created_at, last_seen_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            id.to_string(),
            public_key,
            dh_public_key,
            display_name,
            avatar_json,
            recovery_hash,
            now,
            now
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        // Automatically create a default house for the new anonymous player
        let house_id = Uuid::new_v4();
        let default_customization = serde_json::json!({
            "wallpaper": "blue_stripes",
            "flooring": "oak_wood",
            "bed_color": "teal",
            "rug": "round_cream",
            "lamp_on": true
        });
        let custom_str = default_customization.to_string();

        sqlx::query!(
            r#"
            INSERT INTO houses (id, owner_id, privacy_level, customization, created_at, updated_at)
            VALUES (?1, ?2, 'FRIENDS_ONLY', ?3, ?4, ?5)
            "#,
            house_id.to_string(),
            id.to_string(),
            custom_str,
            now,
            now
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(AnonymousAccount {
            id,
            public_key,
            dh_public_key,
            display_name,
            avatar_config,
            recovery_hash,
            created_at: now,
            last_seen_at: now,
        })
    }

    pub async fn get_account(&self, id: Uuid) -> Result<AnonymousAccount> {
        let row = sqlx::query!(
            r#"
            SELECT id, public_key, dh_public_key, display_name, avatar_config, recovery_hash, created_at, last_seen_at
            FROM accounts
            WHERE id = ?1
            "#,
            id.to_string()
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?
        .ok_or_else(|| AppError::NotFound(format!("Account {} not found", id)))?;

        let avatar: AvatarConfig = serde_json::from_str(&row.avatar_config)
            .unwrap_or_default();

        Ok(AnonymousAccount {
            id: Uuid::parse_str(&row.id).map_err(|e| AppError::Internal(e.to_string()))?,
            public_key: row.public_key,
            dh_public_key: row.dh_public_key,
            display_name: row.display_name,
            avatar_config: avatar,
            recovery_hash: row.recovery_hash,
            created_at: row.created_at,
            last_seen_at: row.last_seen_at,
        })
    }

    pub async fn update_avatar(&self, id: Uuid, avatar: AvatarConfig) -> Result<()> {
        let avatar_json = serde_json::to_string(&avatar)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let now = Utc::now();

        let res = sqlx::query!(
            r#"
            UPDATE accounts
            SET avatar_config = ?1, last_seen_at = ?2
            WHERE id = ?3
            "#,
            avatar_json,
            now,
            id.to_string()
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        if res.rows_affected() == 0 {
            return Err(AppError::NotFound("Account not found".into()));
        }

        Ok(())
    }

    pub async fn update_display_name(&self, id: Uuid, name: &str) -> Result<()> {
        let now = Utc::now();
        let res = sqlx::query!(
            r#"
            UPDATE accounts
            SET display_name = ?1, last_seen_at = ?2
            WHERE id = ?3
            "#,
            name,
            now,
            id.to_string()
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        if res.rows_affected() == 0 {
            return Err(AppError::NotFound("Account not found".into()));
        }

        Ok(())
    }

    pub async fn touch_last_seen(&self, id: Uuid) -> Result<()> {
        let now = Utc::now();
        let _ = sqlx::query!(
            "UPDATE accounts SET last_seen_at = ?1 WHERE id = ?2",
            now,
            id.to_string()
        )
        .execute(&self.pool)
        .await;

        Ok(())
    }
}
