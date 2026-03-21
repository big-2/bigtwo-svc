//! Test setup utilities - builders and infrastructure for test environments
#![allow(dead_code)] // Test utilities may not all be used in every test

use std::sync::Arc;
use tokio::task::JoinHandle;

use bigtwo::{
    bot::BotManager,
    event::{EventBus, RoomSubscription},
    game::{GameEventRoomSubscriber, GameService},
    room::{
        models::RoomModel,
        repository::{InMemoryRoomRepository, RoomRepository},
        service::RoomService,
    },
    stats::{service::StatsRoomSubscriber, InMemoryStatsRepository, StatsRepository, StatsService},
    user::{mapping_service::InMemoryPlayerMappingService, PlayerMappingService},
    websockets::{WebSocketRoomSubscriber, WebsocketReceiveHandler},
};

use super::mocks::MockConnectionManager;

// ============================================================================
// Test Setup Infrastructure
// ============================================================================

pub struct TestSetup {
    pub event_bus: EventBus,
    pub mock_conn_manager: Arc<MockConnectionManager>,
    pub input_handler: WebsocketReceiveHandler,
    pub game_service: Arc<GameService>,
    pub room_service: Arc<RoomService>,
    pub stats_service: Arc<StatsService>,
    pub stats_repository: Arc<dyn StatsRepository>,
    pub players: Vec<(String, String)>,
    pub _subscription_handle: JoinHandle<()>,
    pub bot_manager: Arc<BotManager>,
}

pub struct TestSetupBuilder {
    /// (uuid, playername) pairs
    players: Vec<(String, String)>,
    room_id: String,
    bot_manager: Arc<BotManager>,
}

impl Default for TestSetupBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TestSetupBuilder {
    pub fn new() -> Self {
        Self {
            players: vec![],
            room_id: "room-123".to_string(),
            bot_manager: Arc::new(BotManager::new()),
        }
    }

    pub fn with_players(mut self, players: Vec<(String, String)>) -> Self {
        self.players = players;
        self
    }

    pub fn with_two_players(self) -> Self {
        self.with_players(vec![
            (
                "550e8400-e29b-41d4-a716-446655440000".to_string(),
                "alice".to_string(),
            ),
            (
                "550e8400-e29b-41d4-a716-446655440001".to_string(),
                "bob".to_string(),
            ),
        ])
    }

    pub fn with_four_players(self) -> Self {
        self.with_players(vec![
            (
                "550e8400-e29b-41d4-a716-446655440000".to_string(),
                "alice".to_string(),
            ),
            (
                "550e8400-e29b-41d4-a716-446655440001".to_string(),
                "bob".to_string(),
            ),
            (
                "550e8400-e29b-41d4-a716-446655440002".to_string(),
                "charlie".to_string(),
            ),
            (
                "550e8400-e29b-41d4-a716-446655440003".to_string(),
                "david".to_string(),
            ),
        ])
    }

    pub async fn build(self) -> TestSetup {
        let event_bus = EventBus::new();
        let repo = Arc::new(InMemoryRoomRepository::new());
        let mock_conn_manager = Arc::new(MockConnectionManager::new());
        let player_mapping = Arc::new(InMemoryPlayerMappingService::new());
        let game_service = Arc::new(GameService::new(player_mapping.clone()));
        let bot_manager = Arc::clone(&self.bot_manager);
        let stats_repository = Arc::new(InMemoryStatsRepository::new()) as Arc<dyn StatsRepository>;
        let stats_service = Arc::new(
            StatsService::builder(stats_repository.clone())
                .with_bot_manager(bot_manager.clone())
                .build(),
        );

        // Create room
        let now = chrono::Utc::now();
        let room = RoomModel {
            id: self.room_id.clone(),
            host_uuid: Some(self.players.first().unwrap().0.clone()),
            status: "ONLINE".to_string(),
            player_uuids: self.players.iter().map(|p| p.0.clone()).collect(),
            // Mark all players as ready by default for testing
            ready_players: self.players.iter().map(|p| p.0.clone()).collect(),
            connected_players: self.players.iter().map(|p| p.0.clone()).collect(),
            created_at: now,
            last_activity_at: now,
        };
        repo.create_room(&room).await.unwrap();

        // Connect players
        for (uuid, _) in &self.players {
            mock_conn_manager.add_connected_player(uuid).await;
        }

        // Register name -> uuid mapping in the mock connection manager so that
        // messages addressed by name get recorded under the UUID key
        for (uuid, name) in &self.players {
            mock_conn_manager.register_player_mapping(name, uuid).await;
        }

        // Register UUID -> playername mapping in the actual player mapping service
        // so GameService can resolve names during game creation
        for (uuid, name) in &self.players {
            player_mapping
                .register_player(uuid.clone(), name.clone())
                .await
                .expect("failed to register player mapping");
        }

        let input_handler = WebsocketReceiveHandler::new(event_bus.clone());

        // Create room service
        let room_service = Arc::new(RoomService::new(repo.clone()));

        // Create game event subscriber to handle game logic
        let game_subscriber = GameEventRoomSubscriber::new(game_service.clone(), event_bus.clone());

        let game_subscription = RoomSubscription::new(
            self.room_id.clone(),
            Arc::new(game_subscriber),
            event_bus.clone(),
        );
        let _game_subscription_handle = game_subscription.start().await;

        // Create websocket subscriber to handle message broadcasting
        let output_subscriber = WebSocketRoomSubscriber::new(
            Arc::clone(&room_service),
            mock_conn_manager.clone(),
            game_service.clone(),
            player_mapping.clone(),
            event_bus.clone(),
            Arc::clone(&bot_manager),
        );

        let subscription = RoomSubscription::new(
            self.room_id.clone(),
            Arc::new(output_subscriber),
            event_bus.clone(),
        );
        let subscription_handle = subscription.start().await;

        // Stats subscriber to capture game results
        let stats_subscriber = StatsRoomSubscriber::new(
            Arc::clone(&stats_service),
            Arc::clone(&room_service),
            event_bus.clone(),
        );
        let stats_subscription = RoomSubscription::new(
            self.room_id.clone(),
            Arc::new(stats_subscriber),
            event_bus.clone(),
        );
        let _stats_handle = stats_subscription.start().await;

        // Give subscribers a brief moment to initialize to avoid race conditions
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;

        TestSetup {
            event_bus,
            mock_conn_manager,
            input_handler,
            game_service,
            room_service,
            stats_service,
            stats_repository,
            players: self.players,
            _subscription_handle: subscription_handle,
            bot_manager,
        }
    }
}
