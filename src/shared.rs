use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::sync::Arc;
use thiserror::Error;

use crate::bot::BotManager;
use crate::event::RoomEventHandler;
use crate::room::repository::RoomRepository;
use crate::room::service::RoomService;
use crate::session::repository::SessionRepository;
use crate::session::service::SessionService;
use crate::stats::{
    service::StatsService, InMemoryGameHistoryRepository, InMemoryStatsRepository, StatsRepository,
};
use crate::websockets::ConnectionManager;
use crate::{event::EventBus, game::GameService, user::PlayerMappingService};

/// Shared application state containing all dependencies
#[derive(Clone)]
pub struct AppState {
    pub session_service: Arc<SessionService>,
    pub room_service: Arc<RoomService>,
    pub event_bus: EventBus,
    pub connection_manager: Arc<dyn ConnectionManager>,
    pub game_service: Arc<GameService>,
    pub player_mapping: Arc<dyn PlayerMappingService>,
    pub bot_manager: Arc<BotManager>,
    pub stats_service: Arc<StatsService>,
    pub activity_subscriber: Arc<dyn RoomEventHandler>,
}

impl AppState {
    /// Direct constructor - prefer using `AppState::builder()` for better ergonomics
    #[allow(clippy::too_many_arguments)]
    #[allow(dead_code)] // Public API - prefer builder pattern
    pub fn new(
        session_service: Arc<SessionService>,
        room_service: Arc<RoomService>,
        event_bus: EventBus,
        connection_manager: Arc<dyn ConnectionManager>,
        game_service: Arc<GameService>,
        player_mapping: Arc<dyn PlayerMappingService>,
        bot_manager: Arc<BotManager>,
        stats_service: Arc<StatsService>,
        activity_subscriber: Arc<dyn RoomEventHandler>,
    ) -> Self {
        Self {
            session_service,
            room_service,
            event_bus,
            connection_manager,
            game_service,
            player_mapping,
            bot_manager,
            stats_service,
            activity_subscriber,
        }
    }

    /// Create a new AppStateBuilder for flexible construction
    pub fn builder() -> AppStateBuilder {
        AppStateBuilder::new()
    }
}

/// Builder for creating AppState with type-safe construction and validation
pub struct AppStateBuilder {
    session_repository: Option<Arc<dyn SessionRepository + Send + Sync>>,
    session_service: Option<Arc<SessionService>>,
    room_service: Option<Arc<RoomService>>,
    connection_manager: Option<Arc<dyn ConnectionManager>>,
    game_service: Option<Arc<GameService>>,
    player_mapping: Option<Arc<dyn PlayerMappingService>>,
    event_bus: Option<EventBus>,
    bot_manager: Option<Arc<BotManager>>,
    stats_repository: Option<Arc<dyn StatsRepository>>,
    stats_service: Option<Arc<StatsService>>,
    activity_subscriber: Option<Arc<dyn RoomEventHandler>>,
}

#[derive(Error, Debug)]
pub enum AppStateBuilderError {
    #[error("Missing required dependency: {0}")]
    MissingDependency(&'static str),
}

impl AppStateBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            session_repository: None,
            session_service: None,
            room_service: None,
            connection_manager: None,
            game_service: None,
            player_mapping: None,
            event_bus: None,
            bot_manager: None,
            stats_repository: None,
            stats_service: None,
            activity_subscriber: None,
        }
    }

    pub fn with_session_repository(
        mut self,
        repo: Arc<dyn SessionRepository + Send + Sync>,
    ) -> Self {
        self.session_repository = Some(repo);
        self
    }

    pub fn with_session_service(mut self, service: Arc<SessionService>) -> Self {
        self.session_service = Some(service);
        self
    }

    pub fn with_room_service(mut self, service: Arc<RoomService>) -> Self {
        self.room_service = Some(service);
        self
    }

    /// Convenience method for providing a room repository
    /// This creates a RoomService with the given repository and player mapping
    #[allow(dead_code)] // Builder method for test configuration
    pub fn with_room_repository(mut self, repo: Arc<dyn RoomRepository + Send + Sync>) -> Self {
        self.room_service = Some(Arc::new(RoomService::new(repo)));
        self
    }

    pub fn with_connection_manager(mut self, manager: Arc<dyn ConnectionManager>) -> Self {
        self.connection_manager = Some(manager);
        self
    }

    pub fn with_game_service(mut self, service: Arc<GameService>) -> Self {
        self.game_service = Some(service);
        self
    }

    pub fn with_player_mapping(mut self, mapping: Arc<dyn PlayerMappingService>) -> Self {
        self.player_mapping = Some(mapping);
        self
    }

    pub fn with_event_bus(mut self, event_bus: EventBus) -> Self {
        self.event_bus = Some(event_bus);
        self
    }

    pub fn with_bot_manager(mut self, manager: Arc<BotManager>) -> Self {
        self.bot_manager = Some(manager);
        self
    }

    pub fn with_stats_repository(mut self, repository: Arc<dyn StatsRepository>) -> Self {
        self.stats_repository = Some(repository);
        self
    }

    pub fn with_stats_service(mut self, stats_service: Arc<StatsService>) -> Self {
        self.stats_service = Some(stats_service);
        self
    }

    pub fn with_activity_subscriber(mut self, subscriber: Arc<dyn RoomEventHandler>) -> Self {
        self.activity_subscriber = Some(subscriber);
        self
    }

    /// Build AppState with validation
    pub fn build(self) -> Result<AppState, AppStateBuilderError> {
        let player_mapping = self
            .player_mapping
            .ok_or(AppStateBuilderError::MissingDependency("player_mapping"))?;
        let session_service = self
            .session_service
            .ok_or(AppStateBuilderError::MissingDependency("session_service"))?;
        let event_bus = self.event_bus.unwrap_or_default();
        let connection_manager =
            self.connection_manager
                .ok_or(AppStateBuilderError::MissingDependency(
                    "connection_manager",
                ))?;
        let game_service = self
            .game_service
            .ok_or(AppStateBuilderError::MissingDependency("game_service"))?;

        // If room_service is provided, use it; otherwise create one with subscription dependencies
        let room_service = if let Some(room_service) = self.room_service {
            room_service
        } else {
            return Err(AppStateBuilderError::MissingDependency("room_service"));
        };

        let bot_manager = self
            .bot_manager
            .unwrap_or_else(|| Arc::new(BotManager::new()));

        let stats_repository = self.stats_repository.unwrap_or_else(|| {
            Arc::new(InMemoryStatsRepository::new()) as Arc<dyn StatsRepository>
        });

        let stats_service = self.stats_service.unwrap_or_else(|| {
            Arc::new(
                StatsService::builder(stats_repository.clone())
                    .with_game_history_repository(Arc::new(InMemoryGameHistoryRepository::new()))
                    .with_bot_manager(bot_manager.clone())
                    .build(),
            )
        });

        let activity_subscriber =
            self.activity_subscriber
                .ok_or(AppStateBuilderError::MissingDependency(
                    "activity_subscriber",
                ))?;

        Ok(AppState {
            session_service,
            room_service,
            event_bus,
            connection_manager,
            game_service,
            player_mapping,
            bot_manager,
            stats_service,
            activity_subscriber,
        })
    }

    /// Build AppState without validation (for backward compatibility)
    /// This method provides defaults for missing dependencies
    #[cfg(test)]
    #[allow(dead_code)] // Test helper for simplified test setup
    pub fn build_with_defaults(self) -> AppState {
        use crate::user::mapping_service::InMemoryPlayerMappingService;

        let player_mapping = self
            .player_mapping
            .unwrap_or_else(|| Arc::new(InMemoryPlayerMappingService::new()));

        let session_repository = self.session_repository.unwrap_or_else(|| {
            Arc::new(crate::session::repository::InMemorySessionRepository::new())
        });

        let session_service = self.session_service.unwrap_or_else(|| {
            Arc::new(SessionService::new(
                session_repository.clone(),
                player_mapping.clone(),
            ))
        });

        let room_service = self.room_service.unwrap_or_else(|| {
            Arc::new(RoomService::new(Arc::new(
                crate::room::repository::InMemoryRoomRepository::new(),
            )))
        });

        let game_service = self
            .game_service
            .unwrap_or_else(|| Arc::new(GameService::new(player_mapping.clone())));

        let connection_manager = self
            .connection_manager
            .unwrap_or_else(|| Arc::new(crate::websockets::InMemoryConnectionManager::new()));

        let bot_manager = self
            .bot_manager
            .unwrap_or_else(|| Arc::new(BotManager::new()));

        let stats_repository = self.stats_repository.unwrap_or_else(|| {
            Arc::new(InMemoryStatsRepository::new()) as Arc<dyn StatsRepository>
        });

        let stats_service = self.stats_service.unwrap_or_else(|| {
            Arc::new(
                StatsService::builder(stats_repository.clone())
                    .with_game_history_repository(Arc::new(InMemoryGameHistoryRepository::new()))
                    .with_bot_manager(bot_manager.clone())
                    .build(),
            )
        });

        let activity_tracker = Arc::new(crate::room::activity_tracker::ActivityTracker::new(
            Arc::new(crate::room::repository::InMemoryRoomRepository::new()),
        ));
        let activity_subscriber = self.activity_subscriber.unwrap_or_else(|| {
            Arc::new(
                crate::room::activity_room_subscriber::ActivityRoomSubscriber::new(
                    activity_tracker,
                ),
            ) as Arc<dyn RoomEventHandler>
        });

        AppState {
            session_service,
            room_service,
            event_bus: self.event_bus.unwrap_or_default(),
            connection_manager,
            game_service,
            player_mapping,
            bot_manager,
            stats_service,
            activity_subscriber,
        }
    }
}

impl Default for AppStateBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Error, Debug)]
pub enum AppError {
    #[error("JWT error: {0}")]
    JwtError(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Internal server error")]
    Internal,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            AppError::JwtError(msg) => (StatusCode::BAD_REQUEST, msg),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg),
            AppError::DatabaseError(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error: {}", msg),
            ),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg),
            AppError::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            ),
        };

        let body = Json(json!({
            "error": error_message
        }));

        (status, body).into_response()
    }
}

#[cfg(test)]
pub mod test_utils {
    use super::*;
    use crate::room::models::RoomModel;
    use crate::room::repository::RoomRepository;
    use crate::session::models::SessionModel;
    use async_trait::async_trait;

    /// Dummy session repository that does nothing - for tests that don't care about sessions
    pub struct DummySessionRepository;

    #[async_trait]
    impl SessionRepository for DummySessionRepository {
        async fn create_session(&self, _session: &SessionModel) -> Result<(), AppError> {
            Ok(())
        }
        async fn get_session(&self, _session_id: &str) -> Result<Option<SessionModel>, AppError> {
            Ok(None)
        }
        async fn update_session(&self, _session: &SessionModel) -> Result<(), AppError> {
            Ok(())
        }
        async fn delete_session(&self, _session_id: &str) -> Result<(), AppError> {
            Ok(())
        }
        async fn cleanup_expired_session_ids(&self) -> Result<Vec<String>, AppError> {
            Ok(vec![])
        }
        async fn cleanup_expired_sessions(&self) -> Result<u64, AppError> {
            Ok(0)
        }
    }

    /// Dummy room repository that does nothing - for tests that don't care about rooms
    pub struct DummyRoomRepository;

    #[async_trait]
    impl RoomRepository for DummyRoomRepository {
        async fn create_room(&self, _room: &RoomModel) -> Result<(), AppError> {
            Ok(())
        }
        async fn get_room(&self, _room_id: &str) -> Result<Option<RoomModel>, AppError> {
            Ok(None)
        }
        async fn list_rooms(&self) -> Result<Vec<RoomModel>, AppError> {
            Ok(Vec::new())
        }
        async fn try_join_room(
            &self,
            _room_id: &str,
            _player_name: &str,
        ) -> Result<crate::room::repository::JoinRoomResult, AppError> {
            let now = chrono::Utc::now();
            Ok(crate::room::repository::JoinRoomResult::Success(
                RoomModel {
                    id: "dummy-room".to_string(),
                    host_uuid: Some("dummy-host".to_string()),
                    status: "ONLINE".to_string(),
                    player_uuids: vec!["dummy-host-uuid".to_string()],
                    ready_players: vec![],
                    connected_players: vec!["dummy-host-uuid".to_string()],
                    created_at: now,
                    last_activity_at: now,
                },
            ))
        }

        async fn leave_room(
            &self,
            _room_id: &str,
            _player_uuid: &str,
        ) -> Result<crate::room::repository::LeaveRoomResult, AppError> {
            // Always return success for dummy implementation
            Ok(crate::room::repository::LeaveRoomResult::PlayerNotInRoom)
        }

        async fn toggle_ready(&self, _room_id: &str, _player_uuid: &str) -> Result<(), AppError> {
            Ok(())
        }

        async fn set_ready(
            &self,
            _room_id: &str,
            _player_uuid: &str,
            _is_ready: bool,
        ) -> Result<(), AppError> {
            Ok(())
        }

        async fn clear_ready_states(&self, _room_id: &str) -> Result<(), AppError> {
            Ok(())
        }

        async fn mark_player_disconnected(
            &self,
            _room_id: &str,
            _player_uuid: &str,
        ) -> Result<(), AppError> {
            Ok(())
        }

        async fn mark_player_connected(
            &self,
            _room_id: &str,
            _player_uuid: &str,
        ) -> Result<(), AppError> {
            Ok(())
        }

        async fn update_last_activity(&self, _room_id: &str) -> Result<(), AppError> {
            Ok(())
        }

        async fn get_inactive_rooms(
            &self,
            _inactive_duration: std::time::Duration,
        ) -> Result<Vec<String>, AppError> {
            Ok(Vec::new())
        }

        async fn delete_room(&self, _room_id: &str) -> Result<(), AppError> {
            Ok(())
        }
    }

    /// Dummy connection manager that does nothing - for tests that don't care about websockets
    pub struct DummyConnectionManager;

    #[async_trait]
    impl ConnectionManager for DummyConnectionManager {
        async fn add_connection(
            &self,
            _username: String,
            _sender: tokio::sync::mpsc::Sender<String>,
        ) {
        }

        async fn remove_connection(&self, _username: &str) {}

        async fn send_to_player(&self, _username: &str, _message: &str) {}

        async fn send_to_players(&self, _usernames: &[String], _message: &str) {}

        async fn count_online_players(&self) -> usize {
            0
        }
    }

    /// Test-specific builder that extends the main AppStateBuilder with test defaults
    pub type AppStateBuilder = super::AppStateBuilder;

    impl AppStateBuilder {
        /// Build AppState with test defaults for missing dependencies
        pub fn build_with_test_defaults(self) -> AppState {
            let session_repository = self
                .session_repository
                .unwrap_or_else(|| Arc::new(DummySessionRepository));
            let player_mapping = self.player_mapping.unwrap_or_else(|| {
                Arc::new(crate::user::mapping_service::InMemoryPlayerMappingService::new())
            });
            let session_service = self.session_service.unwrap_or_else(|| {
                Arc::new(SessionService::new(
                    session_repository.clone(),
                    player_mapping.clone(),
                ))
            });
            let room_service = self
                .room_service
                .unwrap_or_else(|| Arc::new(RoomService::new(Arc::new(DummyRoomRepository))));

            let game_service = self
                .game_service
                .unwrap_or_else(|| Arc::new(GameService::new(player_mapping.clone())));

            let bot_manager = self
                .bot_manager
                .unwrap_or_else(|| Arc::new(crate::bot::BotManager::new()));

            let stats_repository = self.stats_repository.unwrap_or_else(|| {
                Arc::new(InMemoryStatsRepository::new()) as Arc<dyn StatsRepository>
            });

            let stats_service = self.stats_service.unwrap_or_else(|| {
                Arc::new(
                    StatsService::builder(stats_repository.clone())
                        .with_bot_manager(bot_manager.clone())
                        .build(),
                )
            });

            let activity_tracker = Arc::new(crate::room::activity_tracker::ActivityTracker::new(
                Arc::new(DummyRoomRepository),
            ));
            let activity_subscriber = self.activity_subscriber.unwrap_or_else(|| {
                Arc::new(
                    crate::room::activity_room_subscriber::ActivityRoomSubscriber::new(
                        activity_tracker,
                    ),
                ) as Arc<dyn RoomEventHandler>
            });

            AppState {
                session_service,
                room_service,
                event_bus: self.event_bus.unwrap_or_default(),
                connection_manager: self
                    .connection_manager
                    .unwrap_or_else(|| Arc::new(DummyConnectionManager)),
                game_service,
                player_mapping,
                bot_manager,
                stats_service,
                activity_subscriber,
            }
        }
    }
}
