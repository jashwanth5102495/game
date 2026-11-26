use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;
use whisper_common::{AppError, House, HousePrivacyLevel, Result};
use whisper_friends::FriendsService;

pub struct HousesService {
    pool: SqlitePool,
    friends: FriendsService,
}

impl HousesService {
    pub fn new(pool: SqlitePool, friends: FriendsService) -> Self {
        Self { pool, friends }
    }

    pub async fn get_by_owner(&self, owner_id: Uuid) -> Result<House> {
        let row = sqlx::query!(
            r#"
            SELECT id, owner_id, privacy_level, customization, created_at, updated_at
            FROM houses
            WHERE owner_id = ?1
            "#,
            owner_id.to_string()
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("House not found".into()))?;

        let privacy_level = match row.privacy_level.as_str() {
            "PUBLIC" => HousePrivacyLevel::Public,
            "PRIVATE" => HousePrivacyLevel::Private,
            _ => HousePrivacyLevel::FriendsOnly,
        };

        let customization = serde_json::from_str(&row.customization)
            .unwrap_or(serde_json::json!({}));

        Ok(House {
            id: Uuid::parse_str(&row.id).map_err(|e| AppError::Internal(e.to_string()))?,
            owner_id,
            privacy_level,
            customization,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    pub async fn can_visit(&self, visitor_id: Uuid, owner_id: Uuid) -> Result<bool> {
        if visitor_id == owner_id {
            return Ok(true);
        }

        // Check if visitor is blocked
        if self.friends.is_blocked(owner_id, visitor_id).await? {
            return Ok(false);
        }

        let house = self.get_by_owner(owner_id).await?;
        match house.privacy_level {
            HousePrivacyLevel::Public => Ok(true),
            HousePrivacyLevel::FriendsOnly => self.friends.are_friends(owner_id, visitor_id).await,
            HousePrivacyLevel::Private => Ok(false),
        }
    }

    pub async fn update_privacy(&self, owner_id: Uuid, privacy: HousePrivacyLevel) -> Result<()> {
        let priv_str = match privacy {
            HousePrivacyLevel::Public => "PUBLIC",
            HousePrivacyLevel::FriendsOnly => "FRIENDS_ONLY",
            HousePrivacyLevel::Private => "PRIVATE",
        };
        let now = Utc::now();

        sqlx::query!(
            "UPDATE houses SET privacy_level = ?1, updated_at = ?2 WHERE owner_id = ?3",
            priv_str,
            now,
            owner_id.to_string()
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }

    pub async fn update_customization(
        &self,
        owner_id: Uuid,
        customization: serde_json::Value,
    ) -> Result<()> {
        let custom_str = customization.to_string();
        let now = Utc::now();

        sqlx::query!(
            "UPDATE houses SET customization = ?1, updated_at = ?2 WHERE owner_id = ?3",
            custom_str,
            now,
            owner_id.to_string()
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }
}
