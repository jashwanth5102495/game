use chrono::{Duration, Utc};
use rand::distributions::Alphanumeric;
use rand::Rng;
use sqlx::SqlitePool;
use uuid::Uuid;
use whisper_common::{
    crypto::verify_signature, AnonymousAccount, AppError, FriendInvitation,
    FriendRelationship, FriendshipStatus, Result,
};

pub struct FriendsService {
    pool: SqlitePool,
}

impl FriendsService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Generate a 6-character random invitation code or secure token valid for 24 hours.
    pub async fn create_invitation(
        &self,
        creator_id: Uuid,
        creator_public_key: &str,
        signature: &str,
    ) -> Result<FriendInvitation> {
        let code: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(8)
            .map(char::from)
            .collect::<String>()
            .to_uppercase();

        let expires_at = Utc::now() + Duration::hours(24);

        // Verify that creator signed the invitation code
        let valid = verify_signature(creator_public_key, code.as_bytes(), signature)
            .map_err(|e| AppError::Crypto(e.to_string()))?;
        if !valid {
            return Err(AppError::Unauthorized("Invalid invitation signature".into()));
        }

        sqlx::query!(
            r#"
            INSERT INTO friend_invitations (code, creator_id, expires_at, signature)
            VALUES (?1, ?2, ?3, ?4)
            "#,
            code,
            creator_id.to_string(),
            expires_at,
            signature
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(FriendInvitation {
            code,
            creator_id,
            expires_at,
            signature: signature.to_string(),
        })
    }

    pub async fn redeem_invitation(&self, user_id: Uuid, code: &str) -> Result<AnonymousAccount> {
        let now = Utc::now();
        let inv = sqlx::query!(
            r#"
            SELECT code, creator_id, expires_at, signature
            FROM friend_invitations
            WHERE code = ?1 AND expires_at > ?2
            "#,
            code,
            now
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("Invitation code expired or invalid".into()))?;

        let creator_id = Uuid::parse_str(&inv.creator_id)
            .map_err(|e| AppError::Internal(e.to_string()))?;

        if creator_id == user_id {
            return Err(AppError::BadRequest("Cannot add yourself as friend".into()));
        }

        // Check if blocked
        if self.is_blocked(creator_id, user_id).await? || self.is_blocked(user_id, creator_id).await? {
            return Err(AppError::Forbidden("Communication blocked".into()));
        }

        // Establish bidirectional friendship relationship
        self.upsert_relationship(user_id, creator_id, FriendshipStatus::Accepted).await?;
        self.upsert_relationship(creator_id, user_id, FriendshipStatus::Accepted).await?;

        // Invalidate single-use invitation
        let _ = sqlx::query!("DELETE FROM friend_invitations WHERE code = ?1", code)
            .execute(&self.pool)
            .await;

        // Fetch creator account
        let creator_acc = sqlx::query!(
            "SELECT id, public_key, dh_public_key, display_name, avatar_config, recovery_hash, created_at, last_seen_at FROM accounts WHERE id = ?1",
            inv.creator_id
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(AnonymousAccount {
            id: creator_id,
            public_key: creator_acc.public_key,
            dh_public_key: creator_acc.dh_public_key,
            display_name: creator_acc.display_name,
            avatar_config: serde_json::from_str(&creator_acc.avatar_config).unwrap_or_default(),
            recovery_hash: creator_acc.recovery_hash,
            created_at: creator_acc.created_at,
            last_seen_at: creator_acc.last_seen_at,
        })
    }

    pub async fn list_friends(&self, user_id: Uuid) -> Result<Vec<AnonymousAccount>> {
        let rows = sqlx::query!(
            r#"
            SELECT a.id, a.public_key, a.dh_public_key, a.display_name, a.avatar_config, a.recovery_hash, a.created_at, a.last_seen_at
            FROM friend_relationships r
            JOIN accounts a ON r.friend_id = a.id
            WHERE r.user_id = ?1 AND r.status = 'ACCEPTED'
            "#,
            user_id.to_string()
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        let accounts = rows
            .into_iter()
            .map(|r| AnonymousAccount {
                id: Uuid::parse_str(&r.id).unwrap_or_default(),
                public_key: r.public_key,
                dh_public_key: r.dh_public_key,
                display_name: r.display_name,
                avatar_config: serde_json::from_str(&r.avatar_config).unwrap_or_default(),
                recovery_hash: r.recovery_hash,
                created_at: r.created_at,
                last_seen_at: r.last_seen_at,
            })
            .collect();

        Ok(accounts)
    }

    pub async fn remove_friend(&self, user_id: Uuid, friend_id: Uuid) -> Result<()> {
        let uid = user_id.to_string();
        let fid = friend_id.to_string();
        sqlx::query!(
            "DELETE FROM friend_relationships WHERE (user_id = ?1 AND friend_id = ?2) OR (user_id = ?2 AND friend_id = ?1)",
            uid,
            fid
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }

    pub async fn block_user(&self, user_id: Uuid, target_id: Uuid) -> Result<()> {
        let uid = user_id.to_string();
        let tid = target_id.to_string();
        let now = Utc::now();

        // First remove existing friendship if any
        let _ = sqlx::query!(
            "DELETE FROM friend_relationships WHERE (user_id = ?1 AND friend_id = ?2) OR (user_id = ?2 AND friend_id = ?1)",
            uid,
            tid
        )
        .execute(&self.pool)
        .await;

        // Insert block
        sqlx::query!(
            r#"
            INSERT INTO block_relationships (user_id, blocked_id, created_at)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(user_id, blocked_id) DO NOTHING
            "#,
            uid,
            tid,
            now
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }

    pub async fn is_blocked(&self, user_id: Uuid, target_id: Uuid) -> Result<bool> {
        let count = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM block_relationships WHERE user_id = ?1 AND blocked_id = ?2",
            user_id.to_string(),
            target_id.to_string()
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(count > 0)
    }

    pub async fn are_friends(&self, user_a: Uuid, user_b: Uuid) -> Result<bool> {
        let count = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM friend_relationships WHERE user_id = ?1 AND friend_id = ?2 AND status = 'ACCEPTED'",
            user_a.to_string(),
            user_b.to_string()
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(count > 0)
    }

    async fn upsert_relationship(
        &self,
        user_id: Uuid,
        friend_id: Uuid,
        status: FriendshipStatus,
    ) -> Result<()> {
        let status_str = match status {
            FriendshipStatus::Accepted => "ACCEPTED",
            FriendshipStatus::Pending => "PENDING",
            FriendshipStatus::Rejected => "REJECTED",
            FriendshipStatus::Blocked => "BLOCKED",
        };
        let now = Utc::now();

        sqlx::query!(
            r#"
            INSERT INTO friend_relationships (user_id, friend_id, status, created_at)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(user_id, friend_id) DO UPDATE SET status = ?3
            "#,
            user_id.to_string(),
            friend_id.to_string(),
            status_str,
            now
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }
}
