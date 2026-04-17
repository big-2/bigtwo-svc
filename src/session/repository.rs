use async_trait::async_trait;
use chrono::Utc;
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use std::sync::Mutex;
use tracing::{debug, instrument, warn};

use super::models::SessionModel;
use crate::shared::AppError;

/// Trait for session repository operations
#[async_trait]
pub trait SessionRepository {
    async fn create_session(&self, session: &SessionModel) -> Result<(), AppError>;
    async fn get_session(&self, session_id: &str) -> Result<Option<SessionModel>, AppError>;
    #[allow(dead_code)] // Trait method for session updates
    async fn update_session(&self, session: &SessionModel) -> Result<(), AppError>;
    async fn delete_session(&self, session_id: &str) -> Result<(), AppError>;
    async fn cleanup_expired_session_ids(&self) -> Result<Vec<String>, AppError>;
    #[allow(dead_code)] // Trait method for session cleanup
    async fn cleanup_expired_sessions(&self) -> Result<u64, AppError>;
}

/// In-memory implementation of SessionRepository for development and testing
///
/// This provides a realistic implementation that can be used in development
/// without requiring a real database connection. Data is stored in memory
/// and will be lost when the application restarts.
pub struct InMemorySessionRepository {
    sessions: Mutex<HashMap<String, SessionModel>>,
}

impl Default for InMemorySessionRepository {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemorySessionRepository {
    /// Creates a new empty in-memory repository
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// Creates an in-memory repository with pre-populated sessions
    #[allow(dead_code)] // Test helper for pre-populated sessions
    pub fn with_sessions(sessions: Vec<SessionModel>) -> Self {
        let mut session_map = HashMap::new();
        for session in sessions {
            session_map.insert(session.id.clone(), session);
        }

        Self {
            sessions: Mutex::new(session_map),
        }
    }

    /// Returns the current number of sessions in the repository
    #[allow(dead_code)] // Public API for monitoring session counts
    pub fn session_count(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }

    /// Checks if a session exists by ID (useful for debugging)
    #[allow(dead_code)] // Public API for checking session existence
    pub fn has_session(&self, session_id: &str) -> bool {
        self.sessions.lock().unwrap().contains_key(session_id)
    }
}

#[async_trait]
impl SessionRepository for InMemorySessionRepository {
    #[instrument(skip(self, session))]
    async fn create_session(&self, session: &SessionModel) -> Result<(), AppError> {
        debug!(session_id = %session.id, username = %session.username, "Creating session in memory");

        let mut sessions = self.sessions.lock().unwrap();
        if sessions.contains_key(&session.id) {
            warn!(session_id = %session.id, "Session already exists in memory");
            return Err(AppError::DatabaseError(
                "Session already exists".to_string(),
            ));
        }
        sessions.insert(session.id.clone(), session.clone());

        debug!(session_id = %session.id, "Session created successfully in memory");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn get_session(&self, session_id: &str) -> Result<Option<SessionModel>, AppError> {
        debug!(session_id = %session_id, "Fetching session from memory");

        let sessions = self.sessions.lock().unwrap();
        let session = sessions.get(session_id).cloned();

        match &session {
            Some(s) => {
                debug!(session_id = %session_id, username = %s.username, "Session found in memory")
            }
            None => debug!(session_id = %session_id, "Session not found in memory"),
        }

        Ok(session)
    }

    #[instrument(skip(self, session))]
    async fn update_session(&self, session: &SessionModel) -> Result<(), AppError> {
        debug!(session_id = %session.id, "Updating session in memory");

        let mut sessions = self.sessions.lock().unwrap();
        if !sessions.contains_key(&session.id) {
            warn!(session_id = %session.id, "Session not found for update in memory");
            return Err(AppError::NotFound("Session not found".to_string()));
        }
        sessions.insert(session.id.clone(), session.clone());

        debug!(session_id = %session.id, "Session updated successfully in memory");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn delete_session(&self, session_id: &str) -> Result<(), AppError> {
        debug!(session_id = %session_id, "Deleting session from memory");

        let mut sessions = self.sessions.lock().unwrap();
        if sessions.remove(session_id).is_none() {
            warn!(session_id = %session_id, "Session not found for deletion in memory");
            return Err(AppError::NotFound("Session not found".to_string()));
        }

        debug!(session_id = %session_id, "Session deleted successfully from memory");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn cleanup_expired_sessions(&self) -> Result<u64, AppError> {
        debug!("Cleaning up expired sessions from memory");

        let removed_count = self.cleanup_expired_session_ids().await?.len();
        debug!(
            expired_sessions_removed = removed_count,
            "Expired sessions cleaned up from memory"
        );
        Ok(removed_count as u64)
    }

    #[instrument(skip(self))]
    async fn cleanup_expired_session_ids(&self) -> Result<Vec<String>, AppError> {
        let mut sessions = self.sessions.lock().unwrap();
        let now = Utc::now();
        let expired_session_ids: Vec<String> = sessions
            .iter()
            .filter(|(_, session)| session.expires_at <= now)
            .map(|(session_id, _)| session_id.clone())
            .collect();

        for session_id in &expired_session_ids {
            sessions.remove(session_id);
        }

        Ok(expired_session_ids)
    }
}

/// PostgreSQL implementation of session repository
pub struct PostgresSessionRepository {
    pool: PgPool,
}

impl PostgresSessionRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SessionRepository for PostgresSessionRepository {
    #[instrument(skip(self, session))]
    async fn create_session(&self, session: &SessionModel) -> Result<(), AppError> {
        debug!(session_id = %session.id, username = %session.username, "Creating session in database");

        sqlx::query(
            "INSERT INTO user_sessions (id, username, created_at, expires_at, last_accessed) VALUES ($1, $2, $3, $4, $5)"
        )
        .bind(&session.id)
        .bind(&session.username)
        .bind(session.created_at)
        .bind(session.expires_at)
        .bind(session.last_accessed)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            warn!(error = %e, "Failed to create session in database");
            AppError::DatabaseError(e.to_string())
        })?;

        debug!(session_id = %session.id, "Session created successfully in database");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn get_session(&self, session_id: &str) -> Result<Option<SessionModel>, AppError> {
        debug!(session_id = %session_id, "Fetching session from database");

        let row = sqlx::query(
            "SELECT id, username, created_at, expires_at, last_accessed FROM user_sessions WHERE id = $1"
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            warn!(error = %e, session_id = %session_id, "Failed to fetch session from database");
            AppError::DatabaseError(e.to_string())
        })?;

        let session = match row {
            Some(row) => {
                let session = SessionModel {
                    id: row.get("id"),
                    username: row.get("username"),
                    created_at: row.get("created_at"),
                    expires_at: row.get("expires_at"),
                    last_accessed: row.get("last_accessed"),
                };
                debug!(session_id = %session_id, username = %session.username, "Session found in database");
                Some(session)
            }
            None => {
                debug!(session_id = %session_id, "Session not found in database");
                None
            }
        };

        Ok(session)
    }

    #[instrument(skip(self, session))]
    async fn update_session(&self, session: &SessionModel) -> Result<(), AppError> {
        debug!(session_id = %session.id, "Updating session in database");

        let result = sqlx::query(
            "UPDATE user_sessions SET username = $2, expires_at = $3, last_accessed = $4 WHERE id = $1"
        )
        .bind(&session.id)
        .bind(&session.username)
        .bind(session.expires_at)
        .bind(session.last_accessed)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            warn!(error = %e, session_id = %session.id, "Failed to update session in database");
            AppError::DatabaseError(e.to_string())
        })?;

        if result.rows_affected() == 0 {
            warn!(session_id = %session.id, "Session not found for update");
            return Err(AppError::NotFound("Session not found".to_string()));
        }

        debug!(session_id = %session.id, "Session updated successfully in database");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn delete_session(&self, session_id: &str) -> Result<(), AppError> {
        debug!(session_id = %session_id, "Deleting session from database");

        let result = sqlx::query("DELETE FROM user_sessions WHERE id = $1")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                warn!(error = %e, session_id = %session_id, "Failed to delete session from database");
                AppError::DatabaseError(e.to_string())
            })?;

        if result.rows_affected() == 0 {
            warn!(session_id = %session_id, "Session not found for deletion");
            return Err(AppError::NotFound("Session not found".to_string()));
        }

        debug!(session_id = %session_id, "Session deleted successfully from database");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn cleanup_expired_sessions(&self) -> Result<u64, AppError> {
        debug!("Cleaning up expired sessions from database");

        let rows_affected = self.cleanup_expired_session_ids().await?.len() as u64;
        debug!(
            expired_sessions_removed = rows_affected,
            "Expired sessions cleaned up"
        );
        Ok(rows_affected)
    }

    #[instrument(skip(self))]
    async fn cleanup_expired_session_ids(&self) -> Result<Vec<String>, AppError> {
        let now = Utc::now();
        let rows = sqlx::query("DELETE FROM user_sessions WHERE expires_at < $1 RETURNING id")
            .bind(now)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| {
                warn!(error = %e, "Failed to cleanup expired sessions");
                AppError::DatabaseError(e.to_string())
            })?;

        Ok(rows.into_iter().map(|row| row.get("id")).collect())
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use chrono::Duration;

    /// Test helper functions for creating test data
    mod helpers {
        use super::*;

        /// Creates a valid session for testing
        pub fn create_test_session(username: &str, expiration_days: i64) -> SessionModel {
            SessionModel::new(username.to_string(), expiration_days)
        }

        /// Creates an expired session for testing
        pub fn create_expired_session(username: &str) -> SessionModel {
            let mut session = SessionModel::new(username.to_string(), 7);
            session.expires_at = Utc::now() - Duration::hours(1);
            session
        }

        /// Creates multiple test sessions with different usernames
        pub fn create_test_sessions(count: usize) -> Vec<SessionModel> {
            (0..count)
                .map(|i| create_test_session(&format!("user-{}", i), 7))
                .collect()
        }
    }

    use helpers::*;

    #[tokio::test]
    async fn test_create_and_get_session() {
        let repo = InMemorySessionRepository::new();
        let session = create_test_session("test-user", 7);

        // Create session
        repo.create_session(&session).await.unwrap();

        // Get session
        let retrieved = repo.get_session(&session.id).await.unwrap();
        assert!(retrieved.is_some());
        let retrieved_session = retrieved.unwrap();
        assert_eq!(retrieved_session.id, session.id);
        assert_eq!(retrieved_session.username, session.username);
    }

    #[tokio::test]
    async fn test_get_nonexistent_session() {
        let repo = InMemorySessionRepository::new();

        let result = repo.get_session("nonexistent-id").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_create_duplicate_session() {
        let repo = InMemorySessionRepository::new();
        let session = create_test_session("test-user", 7);

        // Create session
        repo.create_session(&session).await.unwrap();

        // Try to create the same session again
        let result = repo.create_session(&session).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::DatabaseError(_)));
    }

    #[tokio::test]
    async fn test_update_session() {
        let repo = InMemorySessionRepository::new();
        let mut session = create_test_session("test-user", 7);

        // Create session
        repo.create_session(&session).await.unwrap();

        // Update session
        session.username = "updated-user".to_string();
        session.last_accessed = Some(Utc::now());
        repo.update_session(&session).await.unwrap();

        // Verify update
        let retrieved = repo.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(retrieved.username, "updated-user");
    }

    #[tokio::test]
    async fn test_update_nonexistent_session() {
        let repo = InMemorySessionRepository::new();
        let session = create_test_session("test-user", 7);

        // Try to update non-existent session
        let result = repo.update_session(&session).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_delete_session() {
        let repo = InMemorySessionRepository::new();
        let session = create_test_session("test-user", 7);

        // Create session
        repo.create_session(&session).await.unwrap();

        // Delete session
        repo.delete_session(&session.id).await.unwrap();

        // Verify deletion
        let result = repo.get_session(&session.id).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_delete_nonexistent_session() {
        let repo = InMemorySessionRepository::new();

        // Try to delete non-existent session
        let result = repo.delete_session("nonexistent-id").await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_cleanup_expired_sessions() {
        let repo = InMemorySessionRepository::new();

        // Create expired session
        let expired_session = create_expired_session("expired-user");
        repo.create_session(&expired_session).await.unwrap();

        // Create valid session
        let valid_session = create_test_session("valid-user", 7);
        repo.create_session(&valid_session).await.unwrap();

        // Cleanup expired sessions
        let removed_count = repo.cleanup_expired_sessions().await.unwrap();
        assert_eq!(removed_count, 1);

        // Verify only valid session remains
        let expired_result = repo.get_session(&expired_session.id).await.unwrap();
        assert!(expired_result.is_none());

        let valid_result = repo.get_session(&valid_session.id).await.unwrap();
        assert!(valid_result.is_some());
    }

    #[tokio::test]
    async fn test_cleanup_no_expired_sessions() {
        let repo = InMemorySessionRepository::new();

        // Create valid session
        let valid_session = create_test_session("valid-user", 7);
        repo.create_session(&valid_session).await.unwrap();

        // Cleanup expired sessions
        let removed_count = repo.cleanup_expired_sessions().await.unwrap();
        assert_eq!(removed_count, 0);

        // Verify session still exists
        let result = repo.get_session(&valid_session.id).await.unwrap();
        assert!(result.is_some());
    }

    #[tokio::test]
    async fn test_in_memory_repository_with_preloaded_sessions() {
        let sessions = create_test_sessions(3);
        let repo = InMemorySessionRepository::with_sessions(sessions.clone());

        assert_eq!(repo.session_count(), 3);

        for session in &sessions {
            assert!(repo.has_session(&session.id));
        }
    }
}
