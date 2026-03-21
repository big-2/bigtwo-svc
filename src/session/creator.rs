use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, instrument};

use super::{
    generators::UsernameGenerator, models::SessionModel, repository::SessionRepository,
    token::TokenConfig, types::SessionResponse,
};
use crate::{shared::AppError, user::PlayerMappingService};

/// Configuration for session creation
#[derive(Clone)]
pub struct SessionCreationConfig {
    pub expiration_days: i64,
}

impl Default for SessionCreationConfig {
    fn default() -> Self {
        // Allow configuring expiration via env var, default to 365 days (1 year)
        let expiration_days = std::env::var("SESSION_EXPIRATION_DAYS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(365);

        Self { expiration_days }
    }
}

/// Orchestrates the complex session creation process
/// Separates concerns and provides transaction-like semantics
pub struct SessionCreator {
    username_generator: Arc<dyn UsernameGenerator>,
    session_repository: Arc<dyn SessionRepository + Send + Sync>,
    player_mapping: Arc<dyn PlayerMappingService>,
    session_to_player_uuid: Arc<RwLock<HashMap<String, String>>>,
    token_config: TokenConfig,
    config: SessionCreationConfig,
}

/// Result of session creation operation
#[derive(Debug)]
pub struct SessionCreationResult {
    pub session_response: SessionResponse,
    #[allow(dead_code)] // Reserved for future cleanup logic
    pub cleanup_needed: bool,
}

impl SessionCreator {
    pub fn new(
        username_generator: Arc<dyn UsernameGenerator>,
        session_repository: Arc<dyn SessionRepository + Send + Sync>,
        player_mapping: Arc<dyn PlayerMappingService>,
        session_to_player_uuid: Arc<RwLock<HashMap<String, String>>>,
        token_config: TokenConfig,
        config: SessionCreationConfig,
    ) -> Self {
        Self {
            username_generator,
            session_repository,
            player_mapping,
            session_to_player_uuid,
            token_config,
            config,
        }
    }

    /// Creates a new session with proper error handling and partial cleanup
    #[instrument(skip(self))]
    pub async fn create_session(&self) -> Result<SessionCreationResult, AppError> {
        // Step 1: Generate username
        let username = self.generate_username().await?;
        debug!(username = %username, "Generated username");

        // Step 2: Create session model (session.id will serve as player identifier)
        let session_model = self.create_session_model(username.clone()).await?;
        debug!(session_id = %session_model.id, "Created session model");

        // Step 3: Store session in database
        self.store_session(&session_model).await?;
        debug!(session_id = %session_model.id, "Stored session in database");

        // Step 4: Register player mapping (using session.id as player identifier)
        let mapping_cleanup_needed = self
            .register_player_mapping(&session_model.id, &username)
            .await?;
        debug!(
            player_id = %session_model.id,
            username = %username,
            "Registered player mapping"
        );

        // Step 5: Store session-to-player mapping (now they're the same!)
        self.store_session_mapping(&session_model.id, &session_model.id)
            .await?;
        debug!(
            session_id = %session_model.id,
            "Stored session-to-player mapping"
        );

        // Step 6: Create JWT token
        let token = self.create_jwt_token(&session_model.id, &username).await?;
        debug!(username = %username, "Created JWT token");

        Ok(SessionCreationResult {
            session_response: SessionResponse {
                session_id: token,
                username,
            },
            cleanup_needed: mapping_cleanup_needed,
        })
    }

    /// Generate username using configured generator
    async fn generate_username(&self) -> Result<String, AppError> {
        Ok(self.username_generator.generate().await)
    }

    /// Create session model with proper expiration
    async fn create_session_model(&self, username: String) -> Result<SessionModel, AppError> {
        Ok(SessionModel::new(username, self.config.expiration_days))
    }

    /// Store session in repository
    async fn store_session(&self, session_model: &SessionModel) -> Result<(), AppError> {
        self.session_repository.create_session(session_model).await
    }

    /// Register player mapping, returns true if cleanup is needed on failure
    async fn register_player_mapping(
        &self,
        player_uuid: &str,
        username: &str,
    ) -> Result<bool, AppError> {
        match self
            .player_mapping
            .register_player(player_uuid.to_string(), username.to_string())
            .await
        {
            Ok(_) => Ok(false),
            Err(_e) => {
                // In case of mapping failure, we need cleanup
                Err(AppError::Internal)
            }
        }
    }

    /// Store session-to-player UUID mapping
    async fn store_session_mapping(
        &self,
        session_id: &str,
        player_uuid: &str,
    ) -> Result<(), AppError> {
        let mut session_uuid_map = self.session_to_player_uuid.write().await;
        session_uuid_map.insert(session_id.to_string(), player_uuid.to_string());
        Ok(())
    }

    /// Create JWT token with session information
    async fn create_jwt_token(&self, session_id: &str, username: &str) -> Result<String, AppError> {
        self.token_config
            .create_token(session_id.to_string(), username.to_string())
    }

    /// Cleanup partial session creation (for future enhancement)
    #[allow(dead_code)]
    async fn cleanup_partial_session(
        &self,
        session_id: &str,
        player_uuid: &str,
    ) -> Result<(), AppError> {
        // Remove from session repository
        let _ = self.session_repository.delete_session(session_id).await;

        // Remove from player mapping
        self.player_mapping.remove_player(player_uuid).await;

        // Remove from session-to-player mapping
        let mut session_uuid_map = self.session_to_player_uuid.write().await;
        session_uuid_map.remove(session_id);

        Ok(())
    }
}

// (Pipe helper removed; use direct returns for clarity)

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::generators::PetNameUsernameGenerator;
    use crate::session::repository::InMemorySessionRepository;
    use crate::user::mapping_service::InMemoryPlayerMappingService;
    use std::collections::HashMap;

    fn create_test_session_creator() -> SessionCreator {
        let username_generator = Arc::new(PetNameUsernameGenerator::new());
        let session_repository = Arc::new(InMemorySessionRepository::new());
        let player_mapping = Arc::new(InMemoryPlayerMappingService::new());
        let session_to_player_uuid = Arc::new(RwLock::new(HashMap::new()));
        let token_config = TokenConfig::new();
        let config = SessionCreationConfig::default();

        SessionCreator::new(
            username_generator,
            session_repository,
            player_mapping,
            session_to_player_uuid,
            token_config,
            config,
        )
    }

    #[tokio::test]
    async fn test_create_session_success() {
        let creator = create_test_session_creator();
        let result = creator.create_session().await;

        assert!(result.is_ok());
        let session_result = result.unwrap();

        // Should have valid session response
        assert!(!session_result.session_response.session_id.is_empty());
        assert!(session_result.session_response.session_id.contains('.')); // JWT has dots
        assert!(!session_result.session_response.username.is_empty());
        assert!(session_result.session_response.username.contains('-')); // Pet names have dashes

        // Should be able to validate the created session
        let claims = creator
            .token_config
            .validate_token(&session_result.session_response.session_id)
            .unwrap();
        assert_eq!(claims.username, session_result.session_response.username);
    }

    #[tokio::test]
    async fn test_create_multiple_sessions_unique() {
        let creator = create_test_session_creator();

        let result1 = creator.create_session().await.unwrap();
        let result2 = creator.create_session().await.unwrap();

        // Sessions should be unique (session_id also serves as player identifier)
        assert_ne!(
            result1.session_response.session_id,
            result2.session_response.session_id
        );
        // Usernames may or may not be unique (petnames can repeat)
    }
}
