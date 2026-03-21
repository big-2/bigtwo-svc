use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, instrument, warn};

use super::{
    creator::{SessionCreationConfig, SessionCreator},
    generators::PetNameUsernameGenerator,
    repository::SessionRepository,
    token::TokenConfig,
    types::{SessionClaims, SessionResponse},
};
use crate::{shared::AppError, user::PlayerMappingService};

/// Service for handling session business logic
pub struct SessionService {
    session_creator: SessionCreator,
    token_config: TokenConfig,
    repository: Arc<dyn SessionRepository + Send + Sync>,
    #[allow(dead_code)] // Reserved for future player mapping features
    player_mapping: Arc<dyn PlayerMappingService>,
    #[allow(dead_code)] // Legacy in-memory mapping - now stored in database
    session_to_player_uuid: Arc<RwLock<HashMap<String, String>>>, // session_id -> player_uuid
}

impl SessionService {
    pub fn new(
        repository: Arc<dyn SessionRepository + Send + Sync>,
        player_mapping: Arc<dyn PlayerMappingService>,
    ) -> Self {
        let token_config = TokenConfig::new();
        let session_to_player_uuid = Arc::new(RwLock::new(HashMap::new()));

        // Create session creator with default generators
        let session_creator = SessionCreator::new(
            Arc::new(PetNameUsernameGenerator::new()),
            repository.clone(),
            player_mapping.clone(),
            session_to_player_uuid.clone(),
            token_config.clone(),
            SessionCreationConfig::default(),
        );

        Self {
            session_creator,
            token_config,
            repository,
            player_mapping,
            session_to_player_uuid,
        }
    }

    /// Creates a new session with a generated username and JWT token
    #[instrument(skip(self))]
    pub async fn create_session(&self) -> Result<SessionResponse, AppError> {
        debug!("Starting session creation");

        match self.session_creator.create_session().await {
            Ok(creation_result) => {
                debug!("Session creation completed successfully");
                Ok(creation_result.session_response)
            }
            Err(error) => {
                warn!("Session creation failed: {:?}", error);
                Err(error)
            }
        }
    }

    /// Validates a session token and returns the claims if valid
    #[instrument(skip(self, token))]
    pub async fn validate_session(&self, token: &str) -> Result<SessionClaims, AppError> {
        debug!("Validating session token");

        // First validate JWT token structure and signature
        let claims = self.token_config.validate_token(token)?;
        debug!(
            username = %claims.username,
            session_id = %claims.session_id,
            "JWT token structure validated"
        );

        // Then validate session exists in database and hasn't been revoked
        match self.repository.get_session(&claims.session_id).await? {
            Some(session_model) => {
                if session_model.is_expired() {
                    warn!(
                        session_id = %claims.session_id,
                        "Session found in database but has expired"
                    );
                    return Err(AppError::Unauthorized("Session has expired".to_string()));
                }

                debug!(
                    username = %claims.username,
                    session_id = %claims.session_id,
                    "Session validated successfully against database"
                );

                Ok(claims)
            }
            None => {
                warn!(
                    session_id = %claims.session_id,
                    "Session not found in database - may have been revoked"
                );
                Err(AppError::Unauthorized(
                    "Session not found or has been revoked".to_string(),
                ))
            }
        }
    }

    /// Revokes a session by removing it from the database
    #[instrument(skip(self))]
    pub async fn revoke_session(&self, session_id: &str) -> Result<(), AppError> {
        debug!(session_id = %session_id, "Revoking session");

        // Get player UUID before deletion for cleanup
        let player_uuid = {
            let session_uuid_map = self.session_to_player_uuid.read().await;
            session_uuid_map.get(session_id).cloned()
        };

        // Remove from database
        self.repository.delete_session(session_id).await?;

        // Clean up session → player UUID mapping
        {
            let mut session_uuid_map = self.session_to_player_uuid.write().await;
            session_uuid_map.remove(session_id);
        }

        // Clean up player UUID → playername mapping
        if let Some(uuid) = player_uuid {
            self.player_mapping.remove_player(&uuid).await;
            debug!(
                session_id = %session_id,
                player_uuid = %uuid,
                "Cleaned up player mappings"
            );
        }

        debug!(session_id = %session_id, "Session revoked successfully");
        Ok(())
    }

    /// Extends a session's expiration time
    #[instrument(skip(self))]
    pub async fn extend_session(&self, session_id: &str) -> Result<(), AppError> {
        debug!(session_id = %session_id, "Extending session expiration");

        let mut session = self
            .repository
            .get_session(session_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Session not found".to_string()))?;

        session.extend_expiration(self.token_config.expiration_days);
        self.repository.update_session(&session).await?;

        debug!(session_id = %session_id, "Session expiration extended successfully");
        Ok(())
    }

    /// Cleans up expired sessions from the database
    #[instrument(skip(self))]
    pub async fn cleanup_expired_sessions(&self) -> Result<u64, AppError> {
        debug!("Starting cleanup of expired sessions");

        let removed_count = self.repository.cleanup_expired_sessions().await?;

        debug!(
            removed_sessions = removed_count,
            "Expired sessions cleanup completed"
        );
        Ok(removed_count)
    }

    /// Gets player UUID by session ID
    /// Since session.id IS the player identifier, this just validates the session exists
    #[instrument(skip(self))]
    pub async fn get_player_uuid_by_session(
        &self,
        session_id: &str,
    ) -> Result<Option<String>, AppError> {
        // Check if session exists in database
        let session = self.repository.get_session(session_id).await?;

        match session {
            Some(_) => {
                // Session exists, return the session_id as the player identifier
                Ok(Some(session_id.to_string()))
            }
            None => Ok(None),
        }
    }

    /// Gets playername by player identifier (session ID)
    /// First checks in-memory cache, then falls back to database
    #[instrument(skip(self))]
    pub async fn get_playername_by_uuid(&self, player_id: &str) -> Option<String> {
        // First check in-memory mapping (fast path for active sessions)
        if let Some(username) = self.player_mapping.get_playername(player_id).await {
            return Some(username);
        }

        // Fall back to database lookup (for sessions after restart)
        // Since player_id IS the session_id, we can look it up directly
        match self.repository.get_session(player_id).await {
            Ok(Some(session)) => {
                // Re-populate in-memory mapping for future lookups
                let _ = self
                    .player_mapping
                    .register_player(player_id.to_string(), session.username.clone())
                    .await;
                Some(session.username)
            }
            Ok(None) => None,
            Err(e) => {
                warn!(error = ?e, player_id = %player_id, "Failed to get playername from database");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::repository::InMemorySessionRepository;
    use crate::user::mapping_service::InMemoryPlayerMappingService;

    #[tokio::test]
    async fn test_create_session() {
        let repo = Arc::new(InMemorySessionRepository::new());
        let mapping = Arc::new(InMemoryPlayerMappingService::new());
        let service = SessionService::new(repo, mapping.clone());
        let result = service.create_session().await;

        assert!(result.is_ok());
        let session = result.unwrap();

        // Should have a token (JWT)
        assert!(!session.session_id.is_empty());
        assert!(session.session_id.contains('.')); // JWT has dots

        // Should have a username
        assert!(!session.username.is_empty());
        assert!(session.username.contains('-')); // Pet names have dashes

        // Should have created a player mapping - verify via session service
        let claims = service.validate_session(&session.session_id).await.unwrap();
        let uuid = service
            .get_player_uuid_by_session(&claims.session_id)
            .await
            .unwrap();
        assert!(uuid.is_some());

        let player_uuid = uuid.unwrap();
        let playername = mapping.get_playername(&player_uuid).await;
        assert_eq!(playername, Some(session.username));
    }

    #[tokio::test]
    async fn test_validate_session_success() {
        let repo = Arc::new(InMemorySessionRepository::new());
        let mapping = Arc::new(InMemoryPlayerMappingService::new());
        let service = SessionService::new(repo.clone(), mapping);

        // Create a session
        let session_response = service.create_session().await.unwrap();

        // Validate the session
        let claims = service
            .validate_session(&session_response.session_id)
            .await
            .unwrap();
        assert_eq!(claims.username, session_response.username);
    }

    #[tokio::test]
    async fn test_validate_session_not_found() {
        let repo = Arc::new(InMemorySessionRepository::new());
        let mapping = Arc::new(InMemoryPlayerMappingService::new());
        let service = SessionService::new(repo, mapping);

        // Create a token manually (not in database)
        let token_config = TokenConfig::new();
        let token = token_config
            .create_token("non-existent-session".to_string(), "test-user".to_string())
            .unwrap();

        // Should fail validation because session is not in database
        let result = service.validate_session(&token).await;
        assert!(matches!(result, Err(AppError::Unauthorized(_))));
    }

    #[tokio::test]
    async fn test_revoke_session() {
        let repo = Arc::new(InMemorySessionRepository::new());
        let mapping = Arc::new(InMemoryPlayerMappingService::new());
        let service = SessionService::new(repo.clone(), mapping);

        // Create a session
        let session_response = service.create_session().await.unwrap();
        let claims = service
            .validate_session(&session_response.session_id)
            .await
            .unwrap();

        // Revoke the session
        service.revoke_session(&claims.session_id).await.unwrap();

        // Should fail validation after revocation
        let result = service.validate_session(&session_response.session_id).await;
        assert!(matches!(result, Err(AppError::Unauthorized(_))));
    }

    #[tokio::test]
    async fn test_player_uuid_mapping() {
        let repo = Arc::new(InMemorySessionRepository::new());
        let mapping = Arc::new(InMemoryPlayerMappingService::new());
        let service = SessionService::new(repo.clone(), mapping.clone());

        // Create a session
        let session_response = service.create_session().await.unwrap();
        let claims = service
            .validate_session(&session_response.session_id)
            .await
            .unwrap();

        // Test UUID lookup by session ID
        let uuid = service
            .get_player_uuid_by_session(&claims.session_id)
            .await
            .unwrap();
        assert!(uuid.is_some());

        let player_uuid = uuid.unwrap();

        // Test playername lookup by UUID
        let playername = service.get_playername_by_uuid(&player_uuid).await;
        assert_eq!(playername, Some(session_response.username.clone()));

        // Verify the mapping is working correctly by checking the original username
        assert_eq!(playername.unwrap(), session_response.username);
    }
}
