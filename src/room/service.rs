use std::sync::Arc;
use tracing::{debug, info, instrument};

use super::{
    models::RoomModel,
    repository::{JoinRoomResult, LeaveRoomResult, RoomRepository},
    types::RoomCreateRequest,
};
use crate::shared::AppError;

/// Service for handling room business logic
pub struct RoomService {
    repository: Arc<dyn RoomRepository + Send + Sync>,
}

impl RoomService {
    pub fn new(repository: Arc<dyn RoomRepository + Send + Sync>) -> Self {
        Self { repository }
    }

    /// Creates a new room with a generated ID
    #[instrument(skip(self))]
    pub async fn create_room(&self, request: RoomCreateRequest) -> Result<RoomModel, AppError> {
        // Create room model with generated ID
        let room_model = RoomModel::new(request.host_uuid);
        debug!(room_id = %room_model.id, "Generated room ID");

        // Store room in repository
        self.repository.create_room(&room_model).await?;

        info!(room_id = %room_model.id, "Room created successfully");

        Ok(room_model)
    }

    /// Gets room details as a response object for API endpoints
    #[instrument(skip(self))]
    pub async fn get_room_details(&self, room_id: String) -> Result<RoomModel, AppError> {
        let room = self
            .repository
            .get_room(&room_id)
            .await?
            .ok_or(AppError::DatabaseError("Room not found".to_string()))?;

        Ok(room)
    }

    /// Gets the full room model for internal use (WebSocket handlers, etc.)
    #[instrument(skip(self))]
    pub async fn get_room(&self, room_id: &str) -> Result<Option<RoomModel>, AppError> {
        debug!(room_id = %room_id, "Getting room model");
        self.repository.get_room(room_id).await
    }

    /// Lists all available rooms
    #[instrument(skip(self))]
    pub async fn list_rooms(&self) -> Result<Vec<RoomModel>, AppError> {
        debug!("Listing all rooms");

        // Get all rooms from repository
        let rooms = self
            .repository
            .list_rooms()
            .await?
            .into_iter()
            .filter(|room| room.get_player_count() > 0)
            .collect::<Vec<_>>();

        info!(room_count = rooms.len(), "Rooms retrieved successfully");

        Ok(rooms)
    }

    /// Joins an existing room by incrementing player count
    #[instrument(skip(self))]
    pub async fn join_room(
        &self,
        room_id: String,
        player_uuid: String,
    ) -> Result<RoomModel, AppError> {
        info!(room_id = %room_id, player_name = %player_uuid, "Attempting to join room");

        // Use the atomic try_join_room method
        let result = self
            .repository
            .try_join_room(&room_id, &player_uuid)
            .await?;

        match result {
            JoinRoomResult::Success(updated_room) => {
                info!(
                    room_id = %room_id,
                    player_name = %player_uuid,
                    new_player_count = updated_room.get_player_count(),
                    "Player joined room successfully"
                );
                Ok(updated_room)
            }
            JoinRoomResult::RoomNotFound => {
                Err(AppError::DatabaseError("Room not found".to_string()))
            }
            JoinRoomResult::RoomFull => Err(AppError::DatabaseError("Room is full".to_string())),
        }
    }

    /// Leaves a room by removing the player from the room
    #[instrument(skip(self))]
    pub async fn leave_room(
        &self,
        room_id: String,
        player_uuid: String,
    ) -> Result<LeaveRoomResult, AppError> {
        debug!(room_id = %room_id, player_uuid = %player_uuid, "Attempting to leave room");

        // Use the atomic leave_room method
        let result = self.repository.leave_room(&room_id, &player_uuid).await?;

        match &result {
            LeaveRoomResult::Success(updated_room) => {
                info!(
                    room_id = %room_id,
                    player_uuid = %player_uuid,
                    new_player_count = updated_room.get_player_count(),
                    "Player left room successfully"
                );
            }
            LeaveRoomResult::RoomDeleted => {
                info!(
                    room_id = %room_id,
                    player_uuid = %player_uuid,
                    "Room deleted after last player left"
                );
            }
            LeaveRoomResult::PlayerNotInRoom => {
                debug!(
                    room_id = %room_id,
                    player_uuid = %player_uuid,
                    "Player was not in room"
                );
            }
            LeaveRoomResult::RoomNotFound => {
                debug!(
                    room_id = %room_id,
                    player_uuid = %player_uuid,
                    "Room not found"
                );
            }
        }

        Ok(result)
    }

    /// Toggle ready state for a player in a room
    #[instrument(skip(self))]
    pub async fn toggle_ready(&self, room_id: &str, player_uuid: &str) -> Result<(), AppError> {
        debug!(
            room_id = %room_id,
            player_uuid = %player_uuid,
            "Toggling ready state for player"
        );

        self.repository.toggle_ready(room_id, player_uuid).await?;

        info!(
            room_id = %room_id,
            player_uuid = %player_uuid,
            "Ready state toggled successfully"
        );

        Ok(())
    }

    /// Set ready state for a player in a room
    #[instrument(skip(self))]
    pub async fn set_ready(
        &self,
        room_id: &str,
        player_uuid: &str,
        is_ready: bool,
    ) -> Result<(), AppError> {
        debug!(
            room_id = %room_id,
            player_uuid = %player_uuid,
            is_ready = is_ready,
            "Setting ready state for player"
        );

        self.repository
            .set_ready(room_id, player_uuid, is_ready)
            .await?;

        info!(
            room_id = %room_id,
            player_uuid = %player_uuid,
            is_ready = is_ready,
            "Ready state set successfully"
        );

        Ok(())
    }

    /// Clear all ready states in a room (called when game starts)
    #[instrument(skip(self))]
    pub async fn clear_ready_states(&self, room_id: &str) -> Result<(), AppError> {
        debug!(room_id = %room_id, "Clearing all ready states");

        self.repository.clear_ready_states(room_id).await?;

        info!(room_id = %room_id, "All ready states cleared");

        Ok(())
    }

    /// Mark a player as disconnected within the room
    #[instrument(skip(self))]
    pub async fn mark_player_disconnected(
        &self,
        room_id: &str,
        player_uuid: &str,
    ) -> Result<(), AppError> {
        debug!(
            room_id = %room_id,
            player_uuid = %player_uuid,
            "Marking player as disconnected"
        );
        self.repository
            .mark_player_disconnected(room_id, player_uuid)
            .await
    }

    /// Mark a player as connected within the room
    #[instrument(skip(self))]
    pub async fn mark_player_connected(
        &self,
        room_id: &str,
        player_uuid: &str,
    ) -> Result<(), AppError> {
        debug!(
            room_id = %room_id,
            player_uuid = %player_uuid,
            "Marking player as connected"
        );
        self.repository
            .mark_player_connected(room_id, player_uuid)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::room::repository::InMemoryRoomRepository;

    #[tokio::test]
    async fn test_create_room_success() {
        let repo = Arc::new(InMemoryRoomRepository::new());
        let service = RoomService::new(repo.clone());

        let request = RoomCreateRequest {
            host_uuid: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        };

        let result = service.create_room(request).await;
        assert!(result.is_ok());

        let created = result.unwrap();
        assert_eq!(created.status, "ONLINE");
        assert_eq!(created.get_player_count(), 1);
        assert!(!created.id.is_empty());

        // Verify room was actually stored in repository by trying to get it
        let stored_room = repo.get_room(&created.id).await.unwrap();
        assert!(stored_room.is_some());
        assert_eq!(
            stored_room.unwrap().host_uuid,
            Some("550e8400-e29b-41d4-a716-446655440000".to_string()) // Should store the UUID, not the name
        );
    }

    #[tokio::test]
    async fn test_create_room_generates_unique_ids() {
        let repo = Arc::new(InMemoryRoomRepository::new());
        let service = RoomService::new(repo.clone());

        let request1 = RoomCreateRequest {
            host_uuid: "550e8400-e29b-41d4-a716-446655440001".to_string(),
        };
        let request2 = RoomCreateRequest {
            host_uuid: "550e8400-e29b-41d4-a716-446655440002".to_string(),
        };

        let response1 = service.create_room(request1).await.unwrap();
        let response2 = service.create_room(request2).await.unwrap();

        // IDs should be different
        assert_ne!(response1.id, response2.id);

        // Both should be stored and retrievable
        let stored_room1 = repo.get_room(&response1.id).await.unwrap();
        assert!(stored_room1.is_some());
        assert_eq!(
            stored_room1.unwrap().host_uuid,
            Some("550e8400-e29b-41d4-a716-446655440001".to_string())
        );

        let stored_room2 = repo.get_room(&response2.id).await.unwrap();
        assert!(stored_room2.is_some());
        assert_eq!(
            stored_room2.unwrap().host_uuid,
            Some("550e8400-e29b-41d4-a716-446655440002".to_string())
        );
    }

    #[tokio::test]
    async fn test_create_room_with_empty_host_name() {
        let repo = Arc::new(InMemoryRoomRepository::new());
        let service = RoomService::new(repo);

        let request = RoomCreateRequest {
            host_uuid: "".to_string(),
        };

        // Should still work - validation could be added later if needed
        let result = service.create_room(request).await;
        assert!(result.is_ok());

        let created = result.unwrap();
        assert_eq!(created.host_uuid, Some("".to_string()));
    }

    #[tokio::test]
    async fn test_create_room_preserves_host_name() {
        let repo = Arc::new(InMemoryRoomRepository::new());
        let service = RoomService::new(repo);

        let test_cases = vec![
            ("550e8400-e29b-41d4-a716-446655440000", "simple-name"),
            ("550e8400-e29b-41d4-a716-446655440001", "name with spaces"),
            (
                "550e8400-e29b-41d4-a716-446655440002",
                "name-with-special-chars!@#",
            ),
            (
                "550e8400-e29b-41d4-a716-446655440003",
                "very-long-name-that-goes-on-and-on-and-on",
            ),
        ];

        for (uuid, _name) in test_cases {
            let request = RoomCreateRequest {
                host_uuid: uuid.to_string(),
            };

            let created = service.create_room(request).await.unwrap();
            assert_eq!(created.host_uuid, Some(uuid.to_string()));
        }
    }

    #[tokio::test]
    async fn test_list_rooms() {
        let repository = Arc::new(InMemoryRoomRepository::new());
        let service = RoomService::new(repository.clone());

        // Create some test rooms
        let room1 = RoomModel::new("test-host-1".to_string());
        let room2 = RoomModel::new("test-host-2".to_string());

        repository.create_room(&room1).await.unwrap();
        repository.create_room(&room2).await.unwrap();

        // List all rooms
        let rooms = service.list_rooms().await.unwrap();

        assert_eq!(rooms.len(), 2);
        assert!(rooms
            .iter()
            .any(|r| r.host_uuid == Some("test-host-1".to_string())));
        assert!(rooms
            .iter()
            .any(|r| r.host_uuid == Some("test-host-2".to_string())));
    }

    #[tokio::test]
    async fn test_list_rooms_excludes_zero_player_rooms() {
        let repository = Arc::new(InMemoryRoomRepository::new());
        let service = RoomService::new(repository.clone());

        let visible_room = RoomModel::new("host-with-player".to_string());
        let mut empty_room = RoomModel::new("ghost-host".to_string());
        empty_room.player_uuids.clear();

        repository.create_room(&visible_room).await.unwrap();
        repository.create_room(&empty_room).await.unwrap();

        let rooms = service.list_rooms().await.unwrap();

        assert_eq!(rooms.len(), 1);
        assert_eq!(rooms[0].host_uuid, Some("host-with-player".to_string()));
    }

    #[tokio::test]
    async fn test_join_room_success() {
        let repository = Arc::new(InMemoryRoomRepository::new());
        let service = RoomService::new(repository.clone());

        // Create a room first
        let create_request = RoomCreateRequest {
            host_uuid: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        };
        let created_room = service.create_room(create_request).await.unwrap();

        // Join the room
        let joined_room = service
            .join_room(created_room.id.clone(), "test-player".to_string())
            .await
            .unwrap();

        assert_eq!(joined_room.id, created_room.id);
        assert_eq!(joined_room.get_player_count(), 2);
    }

    #[tokio::test]
    async fn test_join_nonexistent_room() {
        let repository = Arc::new(InMemoryRoomRepository::new());
        let service = RoomService::new(repository);

        let result = service
            .join_room("nonexistent-room".to_string(), "test-player".to_string())
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Room not found"));
    }

    #[tokio::test]
    async fn test_room_capacity_limit() {
        let repository = Arc::new(InMemoryRoomRepository::new());
        let service = RoomService::new(repository.clone());

        // Create a room
        let create_request = RoomCreateRequest {
            host_uuid: "test-host-uuid".to_string(),
        };
        let created_room = service.create_room(create_request).await.unwrap();

        // Add 3 players to reach capacity (host + 3 others = 4 total)
        service
            .join_room(created_room.id.clone(), "player1".to_string())
            .await
            .unwrap();
        service
            .join_room(created_room.id.clone(), "player2".to_string())
            .await
            .unwrap();
        service
            .join_room(created_room.id.clone(), "player3".to_string())
            .await
            .unwrap();
        // Verify room is at capacity
        let room = repository
            .get_room(&created_room.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(room.get_player_count(), 4);

        // Try to join again - should fail
        let result = service
            .join_room(created_room.id.clone(), "player5".to_string())
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Room is full"));
    }

    #[tokio::test]
    async fn test_multiple_room_joins_with_capacity() {
        let repository = Arc::new(InMemoryRoomRepository::new());
        let service = RoomService::new(repository.clone());

        // Create a room
        let create_request = RoomCreateRequest {
            host_uuid: "550e8400-e29b-41d4-a716-446655440003".to_string(),
        };
        let created_room = service.create_room(create_request).await.unwrap();
        assert_eq!(created_room.get_player_count(), 1);

        // Add second player
        let second_join = service
            .join_room(created_room.id.clone(), "player2".to_string())
            .await
            .unwrap();
        assert_eq!(second_join.get_player_count(), 2);

        // Add third player
        let third_join = service
            .join_room(created_room.id.clone(), "player3".to_string())
            .await
            .unwrap();
        assert_eq!(third_join.get_player_count(), 3);

        // Add fourth player
        let fourth_join = service
            .join_room(created_room.id.clone(), "player4".to_string())
            .await
            .unwrap();
        assert_eq!(fourth_join.get_player_count(), 4);

        // Verify final state
        let final_room = repository
            .get_room(&created_room.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(final_room.get_player_count(), 4);
        assert_eq!(
            final_room.host_uuid,
            Some("550e8400-e29b-41d4-a716-446655440003".to_string())
        ); // Should store UUID
        assert_eq!(final_room.status, "ONLINE");
    }

    #[tokio::test]
    async fn test_concurrent_room_joins() {
        let repository = Arc::new(InMemoryRoomRepository::new());
        let service = RoomService::new(repository.clone());

        // Create a room
        let create_request = RoomCreateRequest {
            host_uuid: "550e8400-e29b-41d4-a716-446655440003".to_string(),
        };
        let created_room = service.create_room(create_request).await.unwrap();

        // Manually add 2 players to get to 3 total
        service
            .join_room(created_room.id.clone(), "player2".to_string())
            .await
            .unwrap();
        service
            .join_room(created_room.id.clone(), "player3".to_string())
            .await
            .unwrap();

        // Verify we're at 3 players including the host
        let room = repository
            .get_room(&created_room.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(room.get_player_count(), 3);

        // Simulate multiple concurrent join attempts
        let room_id = created_room.id.clone();
        let service = Arc::new(service);

        let handles = (0..5)
            .map(|i| {
                let service = Arc::clone(&service);
                let room_id = room_id.clone();
                tokio::spawn(async move {
                    service
                        .join_room(room_id, format!("concurrent-player-{}", i))
                        .await
                })
            })
            .collect::<Vec<_>>();

        let results = futures::future::join_all(handles).await;

        // Only 1 should succeed (reaching capacity of 4), others should fail
        let successes = results.into_iter().filter_map(|r| r.unwrap().ok()).count();
        assert_eq!(successes, 1);

        // Verify final room state
        let final_room = repository
            .get_room(&created_room.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            final_room.get_player_count(),
            4,
            "Final room should have exactly 4 players"
        );
    }

    #[tokio::test]
    async fn test_set_ready_marks_player_as_ready() {
        let repository = Arc::new(InMemoryRoomRepository::new());
        let service = RoomService::new(repository.clone());

        // Create a room and add a player
        let create_request = RoomCreateRequest {
            host_uuid: "host-uuid".to_string(),
        };
        let created_room = service.create_room(create_request).await.unwrap();
        service
            .join_room(created_room.id.clone(), "player1".to_string())
            .await
            .unwrap();

        // Mark player as ready
        service
            .set_ready(&created_room.id, "player1", true)
            .await
            .unwrap();

        // Verify player is ready
        let room = repository
            .get_room(&created_room.id)
            .await
            .unwrap()
            .unwrap();
        assert!(room.is_ready("player1"));
        assert_eq!(room.get_ready_players().len(), 1);
    }

    #[tokio::test]
    async fn test_set_ready_unmarks_player_as_ready() {
        let repository = Arc::new(InMemoryRoomRepository::new());
        let service = RoomService::new(repository.clone());

        // Create a room and add a player
        let create_request = RoomCreateRequest {
            host_uuid: "host-uuid".to_string(),
        };
        let created_room = service.create_room(create_request).await.unwrap();
        service
            .join_room(created_room.id.clone(), "player1".to_string())
            .await
            .unwrap();

        // Mark player as ready, then unready
        service
            .set_ready(&created_room.id, "player1", true)
            .await
            .unwrap();
        service
            .set_ready(&created_room.id, "player1", false)
            .await
            .unwrap();

        // Verify player is not ready
        let room = repository
            .get_room(&created_room.id)
            .await
            .unwrap()
            .unwrap();
        assert!(!room.is_ready("player1"));
        assert_eq!(room.get_ready_players().len(), 0);
    }

    #[tokio::test]
    async fn test_set_ready_multiple_players() {
        let repository = Arc::new(InMemoryRoomRepository::new());
        let service = RoomService::new(repository.clone());

        // Create a room and add multiple players
        let create_request = RoomCreateRequest {
            host_uuid: "host-uuid".to_string(),
        };
        let created_room = service.create_room(create_request).await.unwrap();
        service
            .join_room(created_room.id.clone(), "player1".to_string())
            .await
            .unwrap();
        service
            .join_room(created_room.id.clone(), "player2".to_string())
            .await
            .unwrap();
        service
            .join_room(created_room.id.clone(), "player3".to_string())
            .await
            .unwrap();

        // Mark players 1 and 3 as ready
        service
            .set_ready(&created_room.id, "player1", true)
            .await
            .unwrap();
        service
            .set_ready(&created_room.id, "player3", true)
            .await
            .unwrap();

        // Verify ready states
        let room = repository
            .get_room(&created_room.id)
            .await
            .unwrap()
            .unwrap();
        assert!(room.is_ready("player1"));
        assert!(!room.is_ready("player2"));
        assert!(room.is_ready("player3"));
        assert_eq!(room.get_ready_players().len(), 2);
    }

    #[tokio::test]
    async fn test_clear_ready_states() {
        let repository = Arc::new(InMemoryRoomRepository::new());
        let service = RoomService::new(repository.clone());

        // Create a room and add players
        let create_request = RoomCreateRequest {
            host_uuid: "host-uuid".to_string(),
        };
        let created_room = service.create_room(create_request).await.unwrap();
        service
            .join_room(created_room.id.clone(), "player1".to_string())
            .await
            .unwrap();
        service
            .join_room(created_room.id.clone(), "player2".to_string())
            .await
            .unwrap();

        // Mark both players as ready
        service
            .set_ready(&created_room.id, "player1", true)
            .await
            .unwrap();
        service
            .set_ready(&created_room.id, "player2", true)
            .await
            .unwrap();

        // Clear all ready states
        service.clear_ready_states(&created_room.id).await.unwrap();

        // Verify no players are ready
        let room = repository
            .get_room(&created_room.id)
            .await
            .unwrap()
            .unwrap();
        assert!(!room.is_ready("player1"));
        assert!(!room.is_ready("player2"));
        assert_eq!(room.get_ready_players().len(), 0);
    }

    #[tokio::test]
    async fn test_ready_state_cleared_when_player_leaves() {
        let repository = Arc::new(InMemoryRoomRepository::new());
        let service = RoomService::new(repository.clone());

        // Create a room and add players
        let create_request = RoomCreateRequest {
            host_uuid: "host-uuid".to_string(),
        };
        let created_room = service.create_room(create_request).await.unwrap();
        service
            .join_room(created_room.id.clone(), "player1".to_string())
            .await
            .unwrap();
        service
            .join_room(created_room.id.clone(), "player2".to_string())
            .await
            .unwrap();

        // Mark player1 as ready
        service
            .set_ready(&created_room.id, "player1", true)
            .await
            .unwrap();

        // Player1 leaves
        service
            .leave_room(created_room.id.clone(), "player1".to_string())
            .await
            .unwrap();

        // Verify player1 is no longer in ready list
        let room = repository
            .get_room(&created_room.id)
            .await
            .unwrap()
            .unwrap();
        assert!(!room.is_ready("player1"));
        assert_eq!(room.get_ready_players().len(), 0);
        assert_eq!(room.get_player_count(), 2); // Host and player2 remain
    }

    #[tokio::test]
    async fn test_set_ready_nonexistent_room() {
        let repository = Arc::new(InMemoryRoomRepository::new());
        let service = RoomService::new(repository);

        // Try to set ready state in nonexistent room
        let result = service.set_ready("nonexistent-room", "player1", true).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_set_ready_idempotent() {
        let repository = Arc::new(InMemoryRoomRepository::new());
        let service = RoomService::new(repository.clone());

        // Create a room and add a player
        let create_request = RoomCreateRequest {
            host_uuid: "host-uuid".to_string(),
        };
        let created_room = service.create_room(create_request).await.unwrap();
        service
            .join_room(created_room.id.clone(), "player1".to_string())
            .await
            .unwrap();

        // Mark player as ready multiple times
        service
            .set_ready(&created_room.id, "player1", true)
            .await
            .unwrap();
        service
            .set_ready(&created_room.id, "player1", true)
            .await
            .unwrap();
        service
            .set_ready(&created_room.id, "player1", true)
            .await
            .unwrap();

        // Verify player is ready (only once in the list)
        let room = repository
            .get_room(&created_room.id)
            .await
            .unwrap()
            .unwrap();
        assert!(room.is_ready("player1"));
        assert_eq!(room.get_ready_players().len(), 1);
    }
}
