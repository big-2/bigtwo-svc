use async_trait::async_trait;
use chrono::Utc;
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

use super::errors::FeedbackError;
use super::models::FeedbackRecord;

/// Window used when attributing feedback to a recently completed game.
/// Long enough to cover a player reading the game-over screen, short enough
/// that stale rooms do not get matched.
const GAME_RESOLUTION_WINDOW_MINUTES: i64 = 30;

#[async_trait]
pub trait FeedbackRepository: Send + Sync {
    /// Inserts a feedback row. Returns `FeedbackError::AlreadySubmitted` if the
    /// player already has one.
    async fn insert_feedback(&self, record: FeedbackRecord) -> Result<(), FeedbackError>;

    /// Whether this player has ever submitted feedback.
    async fn has_submitted(&self, player_uuid: &str) -> Result<bool, FeedbackError>;

    /// Best-effort resolution of the game a player just finished in `room_id`.
    /// Returns `None` when no recent completed game is known, which is always
    /// acceptable - `game_id` is context, not a requirement.
    async fn resolve_recent_game_id(&self, room_id: &str) -> Result<Option<String>, FeedbackError>;
}

pub struct PostgresFeedbackRepository {
    pool: PgPool,
}

impl PostgresFeedbackRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl FeedbackRepository for PostgresFeedbackRepository {
    async fn insert_feedback(&self, record: FeedbackRecord) -> Result<(), FeedbackError> {
        let chips: Vec<String> = record.chips.clone();

        // ON CONFLICT DO NOTHING against uq_game_feedback_player turns the
        // duplicate case into "zero rows affected" rather than a driver error.
        let result = sqlx::query(
            r#"
            INSERT INTO game_feedback (
                player_uuid, session_id, game_id, room_id, rating, chips,
                comment, won, bot_difficulty, had_bots, client, app_version, created_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            ON CONFLICT (player_uuid) DO NOTHING
            "#,
        )
        .bind(&record.player_uuid)
        .bind(&record.session_id)
        .bind(&record.game_id)
        .bind(&record.room_id)
        .bind(record.rating.as_str())
        .bind(&chips)
        .bind(&record.comment)
        .bind(record.won)
        .bind(&record.bot_difficulty)
        .bind(record.had_bots)
        .bind(&record.client)
        .bind(&record.app_version)
        .bind(record.created_at)
        .execute(&self.pool)
        .await
        .map_err(|e| FeedbackError::Repository(e.to_string()))?;

        if result.rows_affected() == 0 {
            debug!(
                player_uuid = %record.player_uuid,
                "Feedback already submitted for player"
            );
            return Err(FeedbackError::AlreadySubmitted);
        }

        Ok(())
    }

    async fn has_submitted(&self, player_uuid: &str) -> Result<bool, FeedbackError> {
        let row = sqlx::query(
            r#"SELECT EXISTS (SELECT 1 FROM game_feedback WHERE player_uuid = $1) AS submitted"#,
        )
        .bind(player_uuid)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| FeedbackError::Repository(e.to_string()))?;

        row.try_get::<bool, _>("submitted")
            .map_err(|e| FeedbackError::Repository(e.to_string()))
    }

    async fn resolve_recent_game_id(&self, room_id: &str) -> Result<Option<String>, FeedbackError> {
        let cutoff = Utc::now() - chrono::Duration::minutes(GAME_RESOLUTION_WINDOW_MINUTES);

        let row = sqlx::query(
            r#"
            SELECT game_id
            FROM completed_games
            WHERE room_id = $1 AND completed_at >= $2
            ORDER BY completed_at DESC
            LIMIT 1
            "#,
        )
        .bind(room_id)
        .bind(cutoff)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| FeedbackError::Repository(e.to_string()))?;

        match row {
            Some(row) => row
                .try_get::<String, _>("game_id")
                .map(Some)
                .map_err(|e| FeedbackError::Repository(e.to_string())),
            None => Ok(None),
        }
    }
}

/// In-memory implementation. Required because `./scripts/dev.sh --memory` runs
/// the whole service without Postgres, and used by the unit and integration
/// tests.
#[derive(Debug, Default)]
pub struct InMemoryFeedbackRepository {
    by_player: Arc<RwLock<HashMap<String, FeedbackRecord>>>,
    /// room_id -> game_id, populated by tests via `seed_recent_game`.
    recent_games: Arc<RwLock<HashMap<String, String>>>,
}

impl InMemoryFeedbackRepository {
    pub fn new() -> Self {
        Self {
            by_player: Arc::new(RwLock::new(HashMap::new())),
            recent_games: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Test/dev helper: associate a room with a completed game so that
    /// `resolve_recent_game_id` has something to return.
    #[allow(dead_code)] // Test helper, used from integration tests
    pub async fn seed_recent_game(&self, room_id: &str, game_id: &str) {
        self.recent_games
            .write()
            .await
            .insert(room_id.to_string(), game_id.to_string());
    }

    /// Test helper: read back a stored record.
    #[allow(dead_code)] // Test helper, used from integration tests
    pub async fn get_for_player(&self, player_uuid: &str) -> Option<FeedbackRecord> {
        self.by_player.read().await.get(player_uuid).cloned()
    }

    /// Test helper: total number of stored rows.
    #[allow(dead_code)] // Test helper, used from integration tests
    pub async fn count(&self) -> usize {
        self.by_player.read().await.len()
    }
}

#[async_trait]
impl FeedbackRepository for InMemoryFeedbackRepository {
    async fn insert_feedback(&self, record: FeedbackRecord) -> Result<(), FeedbackError> {
        let mut store = self.by_player.write().await;
        if store.contains_key(&record.player_uuid) {
            return Err(FeedbackError::AlreadySubmitted);
        }
        store.insert(record.player_uuid.clone(), record);
        Ok(())
    }

    async fn has_submitted(&self, player_uuid: &str) -> Result<bool, FeedbackError> {
        Ok(self.by_player.read().await.contains_key(player_uuid))
    }

    async fn resolve_recent_game_id(&self, room_id: &str) -> Result<Option<String>, FeedbackError> {
        Ok(self.recent_games.read().await.get(room_id).cloned())
    }
}
