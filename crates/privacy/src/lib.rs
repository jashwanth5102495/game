use chrono::{DateTime, Utc};
use dashmap::DashMap;
use sqlx::SqlitePool;
use std::sync::Arc;
use uuid::Uuid;
use whisper_common::{AppError, Result};

pub struct PrivacyService {
    pool: SqlitePool,
    // Rate limit window: PlayerId -> (Count, WindowStart)
    rate_limits: Arc<DashMap<Uuid, (u32, DateTime<Utc>)>>,
}

impl PrivacyService {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            rate_limits: Arc::new(DashMap::new()),
        }
    }

    pub fn check_rate_limit(&self, player_id: Uuid, max_actions_per_min: u32) -> Result<()> {
        let now = Utc::now();
        let mut entry = self.rate_limits.entry(player_id).or_insert((0, now));
        if (now - entry.1).num_seconds() > 60 {
            entry.0 = 1;
            entry.1 = now;
            Ok(())
        } else if entry.0 >= max_actions_per_min {
            Err(AppError::RateLimited("Action rate limit exceeded".into()))
        } else {
            entry.0 += 1;
            Ok(())
        }
    }

    pub async fn submit_report(
        &self,
        reporter_id: Uuid,
        reported_id: Uuid,
        reason: &str,
        optional_evidence_ciphertext: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now();
        let report_id = Uuid::new_v4();

        sqlx::query!(
            r#"
            INSERT INTO abuse_reports (id, reporter_id, reported_id, reason, evidence_ciphertext, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            report_id.to_string(),
            reporter_id.to_string(),
            reported_id.to_string(),
            reason,
            optional_evidence_ciphertext,
            now
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        tracing::info!("Abuse report recorded: id={}", report_id);
        Ok(())
    }
}
