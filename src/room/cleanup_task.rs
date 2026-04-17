use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;
use tracing::{error, info, instrument, warn};

use super::repository::RoomRepository;
use crate::bot::BotManager;
use crate::event::EventBus;
use crate::game::GameService;
use crate::stats::service::StatsService;
use crate::user::PlayerMappingService;

/// Configuration for the cleanup task
#[derive(Debug, Clone)]
pub struct CleanupConfig {
    /// How often to run the cleanup task
    pub cleanup_interval: Duration,
    /// How long a room must be inactive before deletion
    pub inactivity_threshold: Duration,
}

impl Default for CleanupConfig {
    fn default() -> Self {
        Self {
            cleanup_interval: Duration::from_secs(30 * 60), // 30 minutes
            inactivity_threshold: Duration::from_secs(24 * 60 * 60), // 24 hours
        }
    }
}

/// Starts the background cleanup task that periodically removes inactive rooms
#[instrument(skip(
    room_repository,
    game_service,
    event_bus,
    bot_manager,
    player_mapping,
    stats_service
))]
pub async fn start_cleanup_task(
    room_repository: Arc<dyn RoomRepository + Send + Sync>,
    game_service: Arc<GameService>,
    event_bus: Arc<EventBus>,
    bot_manager: Arc<BotManager>,
    player_mapping: Arc<dyn PlayerMappingService>,
    stats_service: Arc<StatsService>,
    config: CleanupConfig,
) {
    info!(
        cleanup_interval_secs = config.cleanup_interval.as_secs(),
        inactivity_threshold_secs = config.inactivity_threshold.as_secs(),
        "Starting room cleanup background task"
    );

    let mut cleanup_interval = interval(config.cleanup_interval);

    loop {
        cleanup_interval.tick().await;

        info!("Running room cleanup task");

        match cleanup_inactive_rooms(
            &room_repository,
            &game_service,
            &event_bus,
            &bot_manager,
            &player_mapping,
            &stats_service,
            config.inactivity_threshold,
        )
        .await
        {
            Ok(deleted_count) => {
                info!(deleted_count = deleted_count, "Room cleanup completed");
            }
            Err(e) => {
                error!(error = %e, "Room cleanup task failed");
            }
        }
    }
}

/// Cleans up rooms that have been inactive for longer than the threshold
#[instrument(skip(
    room_repository,
    game_service,
    event_bus,
    bot_manager,
    player_mapping,
    stats_service
))]
async fn cleanup_inactive_rooms(
    room_repository: &Arc<dyn RoomRepository + Send + Sync>,
    game_service: &Arc<GameService>,
    event_bus: &Arc<EventBus>,
    bot_manager: &Arc<BotManager>,
    player_mapping: &Arc<dyn PlayerMappingService>,
    stats_service: &Arc<StatsService>,
    inactivity_threshold: Duration,
) -> Result<usize, crate::shared::AppError> {
    // Get list of inactive rooms
    let inactive_room_ids = room_repository
        .get_inactive_rooms(inactivity_threshold)
        .await?;

    if inactive_room_ids.is_empty() {
        info!("No inactive rooms to clean up");
        return Ok(0);
    }

    info!(
        count = inactive_room_ids.len(),
        "Found inactive rooms to delete"
    );

    let mut deleted_count = 0;

    for room_id in inactive_room_ids {
        match delete_room(
            room_repository,
            game_service,
            event_bus,
            bot_manager,
            player_mapping,
            stats_service,
            &room_id,
        )
        .await
        {
            Ok(()) => {
                deleted_count += 1;
                info!(room_id = %room_id, "Deleted inactive room");
            }
            Err(e) => {
                warn!(
                    room_id = %room_id,
                    error = %e,
                    "Failed to delete inactive room"
                );
            }
        }
    }

    Ok(deleted_count)
}

/// Deletes a room and cleans up associated resources
async fn delete_room(
    room_repository: &Arc<dyn RoomRepository + Send + Sync>,
    game_service: &Arc<GameService>,
    event_bus: &Arc<EventBus>,
    bot_manager: &Arc<BotManager>,
    player_mapping: &Arc<dyn PlayerMappingService>,
    stats_service: &Arc<StatsService>,
    room_id: &str,
) -> Result<(), crate::shared::AppError> {
    let bots_in_room = bot_manager.get_bots_in_room(room_id).await;

    // Remove game state if exists
    game_service.remove_game(room_id).await;

    if let Err(err) = stats_service.reset_room_stats(room_id).await {
        warn!(room_id = %room_id, error = %err, "Failed to reset room stats during cleanup");
    }

    if let Err(err) = bot_manager.remove_all_bots_in_room(room_id).await {
        warn!(room_id = %room_id, error = %err, "Failed to remove room bots during cleanup");
    }

    for bot in bots_in_room {
        let _ = player_mapping.remove_player(&bot.uuid).await;
    }

    // Delete the room from repository
    room_repository.delete_room(room_id).await?;

    event_bus.remove_room(room_id).await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::BotManager;
    use crate::room::{models::RoomModel, repository::InMemoryRoomRepository};
    use crate::stats::{InMemoryStatsRepository, StatsService};
    use crate::user::{mapping_service::InMemoryPlayerMappingService, PlayerMappingService};

    fn test_stats_service() -> Arc<StatsService> {
        Arc::new(StatsService::builder(Arc::new(InMemoryStatsRepository::new())).build())
    }

    #[tokio::test]
    async fn test_cleanup_removes_inactive_rooms() {
        let concrete_repo = Arc::new(InMemoryRoomRepository::new());
        let repo: Arc<dyn RoomRepository + Send + Sync> = concrete_repo.clone();
        let player_mapping = Arc::new(InMemoryPlayerMappingService::new());
        let game_service = Arc::new(GameService::new(player_mapping.clone()));
        let event_bus = Arc::new(EventBus::new());
        let bot_manager = Arc::new(BotManager::new());
        let stats_service = test_stats_service();

        // Create a room
        let room = RoomModel::new("test-host".to_string());
        let room_id = room.id.clone();
        concrete_repo.create_room(&room).await.unwrap();

        // Verify room exists
        assert!(concrete_repo.get_room(&room_id).await.unwrap().is_some());

        // Wait a bit so the room becomes inactive
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Run cleanup with a very short threshold (should delete the room)
        let deleted = cleanup_inactive_rooms(
            &repo,
            &game_service,
            &event_bus,
            &bot_manager,
            &(player_mapping.clone() as Arc<dyn PlayerMappingService>),
            &stats_service,
            Duration::from_millis(1),
        )
        .await
        .unwrap();

        assert_eq!(deleted, 1);

        // Verify room was deleted
        assert!(concrete_repo.get_room(&room_id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_cleanup_preserves_active_rooms() {
        let concrete_repo = Arc::new(InMemoryRoomRepository::new());
        let repo: Arc<dyn RoomRepository + Send + Sync> = concrete_repo.clone();
        let player_mapping = Arc::new(InMemoryPlayerMappingService::new());
        let game_service = Arc::new(GameService::new(player_mapping.clone()));
        let event_bus = Arc::new(EventBus::new());
        let bot_manager = Arc::new(BotManager::new());
        let stats_service = test_stats_service();

        // Create a room
        let room = RoomModel::new("test-host".to_string());
        let room_id = room.id.clone();
        concrete_repo.create_room(&room).await.unwrap();

        // Run cleanup with a very long threshold (should not delete the room)
        let deleted = cleanup_inactive_rooms(
            &repo,
            &game_service,
            &event_bus,
            &bot_manager,
            &(player_mapping.clone() as Arc<dyn PlayerMappingService>),
            &stats_service,
            Duration::from_secs(24 * 60 * 60), // 24 hours
        )
        .await
        .unwrap();

        assert_eq!(deleted, 0);

        // Verify room still exists
        assert!(concrete_repo.get_room(&room_id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_cleanup_handles_multiple_rooms() {
        let concrete_repo = Arc::new(InMemoryRoomRepository::new());
        let repo: Arc<dyn RoomRepository + Send + Sync> = concrete_repo.clone();
        let player_mapping = Arc::new(InMemoryPlayerMappingService::new());
        let game_service = Arc::new(GameService::new(player_mapping.clone()));
        let event_bus = Arc::new(EventBus::new());
        let bot_manager = Arc::new(BotManager::new());
        let stats_service = test_stats_service();

        // Create multiple rooms
        let room1 = RoomModel::new("host1".to_string());
        let room2 = RoomModel::new("host2".to_string());
        let room3 = RoomModel::new("host3".to_string());

        concrete_repo.create_room(&room1).await.unwrap();
        concrete_repo.create_room(&room2).await.unwrap();
        concrete_repo.create_room(&room3).await.unwrap();

        // Wait a bit so the rooms become inactive
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // All rooms should be inactive with a short threshold
        let deleted = cleanup_inactive_rooms(
            &repo,
            &game_service,
            &event_bus,
            &bot_manager,
            &(player_mapping.clone() as Arc<dyn PlayerMappingService>),
            &stats_service,
            Duration::from_millis(1),
        )
        .await
        .unwrap();

        assert_eq!(deleted, 3);
    }

    // Note: Cleanup with game state is tested in the integration test suite
    // This test is skipped due to complexity with player mapping setup

    #[tokio::test]
    async fn test_cleanup_with_no_rooms() {
        let concrete_repo = Arc::new(InMemoryRoomRepository::new());
        let repo: Arc<dyn RoomRepository + Send + Sync> = concrete_repo.clone();
        let player_mapping = Arc::new(InMemoryPlayerMappingService::new());
        let game_service = Arc::new(GameService::new(player_mapping.clone()));
        let event_bus = Arc::new(EventBus::new());
        let bot_manager = Arc::new(BotManager::new());
        let stats_service = test_stats_service();

        // No rooms created

        let deleted = cleanup_inactive_rooms(
            &repo,
            &game_service,
            &event_bus,
            &bot_manager,
            &(player_mapping.clone() as Arc<dyn PlayerMappingService>),
            &stats_service,
            Duration::from_millis(1),
        )
        .await
        .unwrap();

        assert_eq!(deleted, 0);
    }

    #[tokio::test]
    async fn test_delete_room_handles_nonexistent_room() {
        let repo: Arc<dyn RoomRepository + Send + Sync> = Arc::new(InMemoryRoomRepository::new());
        let player_mapping = Arc::new(InMemoryPlayerMappingService::new());
        let game_service = Arc::new(GameService::new(player_mapping.clone()));
        let event_bus = Arc::new(EventBus::new());
        let bot_manager = Arc::new(BotManager::new());
        let stats_service = test_stats_service();

        // Try to delete a room that doesn't exist
        let result = delete_room(
            &repo,
            &game_service,
            &event_bus,
            &bot_manager,
            &(player_mapping as Arc<dyn PlayerMappingService>),
            &stats_service,
            "nonexistent-room",
        )
        .await;

        assert!(result.is_err());
    }
}
