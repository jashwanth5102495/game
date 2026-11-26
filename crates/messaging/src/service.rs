use chrono::{Duration, Utc};
use dashmap::DashMap;
use sqlx::SqlitePool;
use std::sync::Arc;
use uuid::Uuid;
use whisper_common::{
    crypto::{hash_sha256, verify_signature},
    AppError, EncryptedEnvelope, MessageStatus, Result,
};
use whisper_friends::FriendsService;

pub struct MessagingService {
    pool: SqlitePool,
    friends: FriendsService,
    // In-memory status feed for real-time notification to online senders
    delivery_status_cache: Arc<DashMap<Uuid, MessageStatus>>,
}

impl MessagingService {
    pub fn new(pool: SqlitePool, friends: FriendsService) -> Self {
        Self {
            pool,
            friends,
            delivery_status_cache: Arc::new(DashMap::new()),
        }
    }

    pub async fn submit_envelope(
        &self,
        sender_id: Uuid,
        sender_public_key: &str,
        recipient_id: Uuid,
        ephemeral_public_key: String,
        nonce: String,
        ciphertext: String,
        sender_signature: String,
        burn_token_hash: String,
    ) -> Result<Uuid> {
        if sender_id == recipient_id {
            return Err(AppError::BadRequest("Cannot message yourself".into()));
        }

        // Validate friendship and ensure not blocked
        if !self.friends.are_friends(sender_id, recipient_id).await? {
            return Err(AppError::Forbidden("Only friends can send messages".into()));
        }
        if self.friends.is_blocked(recipient_id, sender_id).await? {
            return Err(AppError::Forbidden("Recipient has blocked communication".into()));
        }

        // Verify sender signature over payload: (sender_id + recipient_id + ciphertext)
        let sig_payload = format!("{}:{}:{}", sender_id, recipient_id, ciphertext);
        let valid_sig = verify_signature(sender_public_key, sig_payload.as_bytes(), &sender_signature)
            .map_err(|e| AppError::Crypto(e.to_string()))?;
        if !valid_sig {
            return Err(AppError::Unauthorized("Invalid envelope signature".into()));
        }

        let message_id = Uuid::new_v4();
        let now = Utc::now();
        let expires_at = now + Duration::hours(48);

        sqlx::query!(
            r#"
            INSERT INTO pending_messages (
                id, sender_id, recipient_id, ephemeral_public_key, nonce,
                ciphertext, sender_signature, burn_token_hash, created_at, expires_at, status
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'QUEUED')
            "#,
            message_id.to_string(),
            sender_id.to_string(),
            recipient_id.to_string(),
            ephemeral_public_key,
            nonce,
            ciphertext,
            sender_signature,
            burn_token_hash,
            now,
            expires_at
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        self.delivery_status_cache.insert(message_id, MessageStatus::Queued);

        tracing::info!(
            "Encrypted envelope queued: msg_id={}, recipient={}",
            message_id,
            recipient_id
        );

        Ok(message_id)
    }

    pub async fn get_pending_messages(&self, recipient_id: Uuid) -> Result<Vec<EncryptedEnvelope>> {
        let now = Utc::now();
        let rows = sqlx::query!(
            r#"
            SELECT id, sender_id, recipient_id, ephemeral_public_key, nonce,
                   ciphertext, sender_signature, burn_token_hash, created_at, expires_at, status
            FROM pending_messages
            WHERE recipient_id = ?1 AND expires_at > ?2
            ORDER BY created_at ASC
            "#,
            recipient_id.to_string(),
            now
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        let mut envelopes = Vec::new();
        for r in rows {
            let mid = Uuid::parse_str(&r.id).unwrap_or_default();
            // Mark as DELIVERED
            let _ = sqlx::query!(
                "UPDATE pending_messages SET status = 'DELIVERED' WHERE id = ?1",
                r.id
            )
            .execute(&self.pool)
            .await;

            self.delivery_status_cache.insert(mid, MessageStatus::Delivered);

            envelopes.push(EncryptedEnvelope {
                message_id: mid,
                sender_id: Uuid::parse_str(&r.sender_id).unwrap_or_default(),
                recipient_id,
                ephemeral_public_key: r.ephemeral_public_key,
                nonce: r.nonce,
                ciphertext: r.ciphertext,
                sender_signature: r.sender_signature,
                burn_token_hash: r.burn_token_hash,
                created_at: r.created_at,
                expires_at: r.expires_at,
                status: MessageStatus::Delivered,
            });
        }

        Ok(envelopes)
    }

    /// Atomically burns the message.
    /// The recipient must present the one-time burn token whose SHA-256 matches burn_token_hash.
    /// The message is permanently deleted from database storage!
    pub async fn burn_message(
        &self,
        recipient_id: Uuid,
        message_id: Uuid,
        burn_token: &str,
    ) -> Result<()> {
        let token_hash = hash_sha256(burn_token.as_bytes());

        // Fetch envelope to check token hash
        let row = sqlx::query!(
            r#"
            SELECT burn_token_hash, sender_id
            FROM pending_messages
            WHERE id = ?1 AND recipient_id = ?2
            "#,
            message_id.to_string(),
            recipient_id.to_string()
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("Message already burned or does not exist".into()))?;

        if row.burn_token_hash != token_hash {
            return Err(AppError::Unauthorized("Invalid burn token".into()));
        }

        // Atomic permanent deletion of ciphertext
        let res = sqlx::query!(
            "DELETE FROM pending_messages WHERE id = ?1",
            message_id.to_string()
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        if res.rows_affected() == 0 {
            return Err(AppError::NotFound("Message already burned".into()));
        }

        // Record minimal tombstone status in memory for sender acknowledgment
        self.delivery_status_cache.insert(message_id, MessageStatus::Burned);

        tracing::info!("Message {} burned and permanently deleted", message_id);

        Ok(())
    }

    pub fn get_status(&self, message_id: Uuid) -> Option<MessageStatus> {
        self.delivery_status_cache.get(&message_id).map(|s| *s)
    }

    pub async fn purge_expired(&self) -> Result<u64> {
        let now = Utc::now();
        let res = sqlx::query!("DELETE FROM pending_messages WHERE expires_at <= ?1", now)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(res.rows_affected())
    }
}
