use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;
use tracing::{debug, info, instrument, warn};

use super::models::RoomModel;
use crate::shared::AppError;

/// Result of attempting to join a room
#[derive(Debug, Clone)]
pub enum JoinRoomResult {
    /// Successfully joined the room, returns updated room data
    Success(RoomModel),
    /// Room is at capacity (4 players)
    RoomFull,
    /// Room does not exist
    RoomNotFound,
}

/// Result of attempting to leave a room
#[derive(Debug, Clone)]
pub enum LeaveRoomResult {
    /// Successfully left the room, returns updated room data
    Success(RoomModel),
    /// Player was not in the room
    PlayerNotInRoom,
    /// Room does not exist
    RoomNotFound,
    /// Room was deleted because no players left
    RoomDeleted,
}

/// Trait for room repository operations
#[async_trait]
pub trait RoomRepository {
    async fn create_room(&self, room: &RoomModel) -> Result<(), AppError>;
    async fn get_room(&self, room_id: &str) -> Result<Option<RoomModel>, AppError>;
    async fn list_rooms(&self) -> Result<Vec<RoomModel>, AppError>;

    /// Atomically attempts to join a room by checking capacity and incrementing player count
    /// This prevents race conditions when multiple players try to join simultaneously
    async fn try_join_room(
        &self,
        room_id: &str,
        player_uuid: &str,
    ) -> Result<JoinRoomResult, AppError>;

    /// Atomically attempts to remove a player from a room
    async fn leave_room(
        &self,
        room_id: &str,
        player_uuid: &str,
    ) -> Result<LeaveRoomResult, AppError>;

    /// Mark a player as disconnected within a room
    async fn mark_player_disconnected(
        &self,
        room_id: &str,
        player_uuid: &str,
    ) -> Result<(), AppError>;

    /// Mark a player as connected within a room
    async fn mark_player_connected(&self, room_id: &str, player_uuid: &str)
        -> Result<(), AppError>;

    /// Toggle ready state for a player in a room
    async fn toggle_ready(&self, room_id: &str, player_uuid: &str) -> Result<(), AppError>;

    /// Set ready state for a player in a room
    async fn set_ready(
        &self,
        room_id: &str,
        player_uuid: &str,
        is_ready: bool,
    ) -> Result<(), AppError>;

    /// Clear all ready states in a room (called when game starts)
    async fn clear_ready_states(&self, room_id: &str) -> Result<(), AppError>;

    /// Update the last activity timestamp for a room
    async fn update_last_activity(&self, room_id: &str) -> Result<(), AppError>;

    /// Get rooms that have been inactive for longer than the specified duration
    async fn get_inactive_rooms(
        &self,
        inactive_duration: std::time::Duration,
    ) -> Result<Vec<String>, AppError>;

    /// Delete a room by ID
    async fn delete_room(&self, room_id: &str) -> Result<(), AppError>;
}

/// In-memory implementation of RoomRepository for development and testing
pub struct InMemoryRoomRepository {
    rooms: Mutex<HashMap<String, RoomModel>>,
}

impl Default for InMemoryRoomRepository {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryRoomRepository {
    /// Creates a new empty in-memory repository
    pub fn new() -> Self {
        Self {
            rooms: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl RoomRepository for InMemoryRoomRepository {
    #[instrument(skip(self, room))]
    async fn create_room(&self, room: &RoomModel) -> Result<(), AppError> {
        let mut rooms = self.rooms.lock().unwrap();
        if rooms.contains_key(&room.id) {
            warn!(room_id = %room.id, "Room already exists in memory");
            return Err(AppError::DatabaseError("Room already exists".to_string()));
        }
        rooms.insert(room.id.clone(), room.clone());

        debug!(room_id = %room.id, "Room created successfully in memory");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn get_room(&self, room_id: &str) -> Result<Option<RoomModel>, AppError> {
        debug!(room_id = %room_id, "Fetching room from memory");

        let rooms = self.rooms.lock().unwrap();

        Ok(rooms.get(room_id).cloned())
    }

    #[instrument(skip(self))]
    async fn list_rooms(&self) -> Result<Vec<RoomModel>, AppError> {
        debug!("Listing all rooms in memory");

        let rooms = self.rooms.lock().unwrap();
        let room_list = rooms.values().cloned().collect();

        debug!("Rooms listed successfully in memory");
        Ok(room_list)
    }

    #[instrument(skip(self))]
    async fn try_join_room(
        &self,
        room_id: &str,
        player_uuid: &str,
    ) -> Result<JoinRoomResult, AppError> {
        debug!(
            room_id = %room_id,
            player_uuid = %player_uuid,
            "Attempting to join room atomically with UUID"
        );

        let mut rooms = self.rooms.lock().unwrap();

        // Get the room or return RoomNotFound
        let room = match rooms.get_mut(room_id) {
            Some(room) => room,
            None => {
                debug!(room_id = %room_id, "Room not found");
                return Ok(JoinRoomResult::RoomNotFound);
            }
        };

        // Idempotency: if player is already in room, treat as success even if room is full
        if room.has_player(player_uuid) {
            debug!(
                room_id = %room_id,
                player_uuid = %player_uuid,
                "Player already in room"
            );
            return Ok(JoinRoomResult::Success(room.clone()));
        }

        // Check if room is at capacity (after idempotency check)
        if room.is_full() {
            debug!(room_id = %room_id, current_count = room.get_player_count(), "Room is full");
            return Ok(JoinRoomResult::RoomFull);
        }

        // Add player to the room (both username and UUID)
        room.add_player(player_uuid.to_string());

        // Clone the updated room data to return
        let updated_room = room.clone();

        info!(
            room_id = %room_id,
            player_uuid = %player_uuid,
            new_player_count = updated_room.get_player_count(),
            "Player joined room successfully with UUID (atomic)"
        );

        Ok(JoinRoomResult::Success(updated_room))
    }

    #[instrument(skip(self))]
    async fn leave_room(
        &self,
        room_id: &str,
        player_uuid: &str,
    ) -> Result<LeaveRoomResult, AppError> {
        debug!(
            room_id = %room_id,
            player_uuid = %player_uuid,
            "Attempting to leave room atomically with UUID"
        );

        let mut rooms = self.rooms.lock().unwrap();

        // Get the room or return RoomNotFound
        let room = match rooms.get_mut(room_id) {
            Some(room) => room,
            None => {
                debug!(room_id = %room_id, "Room not found");
                return Ok(LeaveRoomResult::RoomNotFound);
            }
        };

        // Check if player is in the room (by either username or UUID)
        if !room.has_player(player_uuid) {
            debug!(
                room_id = %room_id,
                player_uuid = %player_uuid,
                "Player not in room"
            );
            return Ok(LeaveRoomResult::PlayerNotInRoom);
        }

        // Remove player from the room (both username and UUID)
        room.remove_player(player_uuid);

        // If room is now empty, delete it
        if room.player_uuids.is_empty() {
            info!(room_id = %room_id, "Room is now empty, deleting");
            rooms.remove(room_id);
            return Ok(LeaveRoomResult::RoomDeleted);
        }

        // If only bots remain, delete the room
        let only_bots_remain = room
            .player_uuids
            .iter()
            .all(|uuid| crate::bot::types::BotPlayer::is_bot_uuid(uuid));

        if only_bots_remain {
            info!(room_id = %room_id, "Only bots remain in room, deleting");
            rooms.remove(room_id);
            return Ok(LeaveRoomResult::RoomDeleted);
        }

        // If the leaving player was the host, assign new host to first remaining human player
        if room.host_uuid.is_some() && room.host_uuid.as_ref().unwrap() == player_uuid {
            // Find first non-bot player
            let new_host = room
                .player_uuids
                .iter()
                .find(|uuid| !crate::bot::types::BotPlayer::is_bot_uuid(uuid))
                .cloned();

            if let Some(new_host) = new_host {
                info!(
                    room_id = %room_id,
                    old_host = %player_uuid,
                    new_host = %new_host,
                    "Host left, assigning new human host"
                );
                room.host_uuid = Some(new_host);
            } else {
                // No human players left, only bots - this shouldn't happen due to earlier check
                warn!(
                    room_id = %room_id,
                    "No human players available to become host"
                );
                room.host_uuid = None;
            }
        }

        // Clone the updated room data to return
        let updated_room = room.clone();

        info!(
            room_id = %room_id,
            new_player_count = updated_room.get_player_count(),
            "Player left room successfully with UUID (atomic)"
        );
        Ok(LeaveRoomResult::Success(updated_room))
    }

    #[instrument(skip(self))]
    async fn toggle_ready(&self, room_id: &str, player_uuid: &str) -> Result<(), AppError> {
        debug!(
            room_id = %room_id,
            player_uuid = %player_uuid,
            "Toggling ready state"
        );

        let mut rooms = self.rooms.lock().unwrap();

        let room = rooms.get_mut(room_id).ok_or_else(|| {
            warn!(room_id = %room_id, "Room not found");
            AppError::NotFound("Room not found".to_string())
        })?;

        room.toggle_ready(player_uuid);

        debug!(
            room_id = %room_id,
            player_uuid = %player_uuid,
            is_ready = room.is_ready(player_uuid),
            "Player ready state toggled"
        );

        Ok(())
    }

    #[instrument(skip(self))]
    async fn set_ready(
        &self,
        room_id: &str,
        player_uuid: &str,
        is_ready: bool,
    ) -> Result<(), AppError> {
        debug!(
            room_id = %room_id,
            player_uuid = %player_uuid,
            is_ready = is_ready,
            "Setting ready state"
        );

        let mut rooms = self.rooms.lock().unwrap();

        let room = rooms.get_mut(room_id).ok_or_else(|| {
            warn!(room_id = %room_id, "Room not found");
            AppError::NotFound("Room not found".to_string())
        })?;

        room.set_ready(player_uuid, is_ready);

        debug!(
            room_id = %room_id,
            player_uuid = %player_uuid,
            is_ready = is_ready,
            "Player ready state set"
        );

        Ok(())
    }

    #[instrument(skip(self))]
    async fn clear_ready_states(&self, room_id: &str) -> Result<(), AppError> {
        debug!(room_id = %room_id, "Clearing all ready states");

        let mut rooms = self.rooms.lock().unwrap();

        let room = rooms.get_mut(room_id).ok_or_else(|| {
            warn!(room_id = %room_id, "Room not found");
            AppError::NotFound("Room not found".to_string())
        })?;

        room.clear_ready_states();

        debug!(room_id = %room_id, "All ready states cleared");

        Ok(())
    }

    #[instrument(skip(self))]
    async fn mark_player_disconnected(
        &self,
        room_id: &str,
        player_uuid: &str,
    ) -> Result<(), AppError> {
        let mut rooms = self.rooms.lock().unwrap();

        let room = rooms.get_mut(room_id).ok_or_else(|| {
            warn!(room_id = %room_id, "Room not found when marking disconnected");
            AppError::NotFound("Room not found".to_string())
        })?;

        room.mark_disconnected(player_uuid);

        Ok(())
    }

    #[instrument(skip(self))]
    async fn mark_player_connected(
        &self,
        room_id: &str,
        player_uuid: &str,
    ) -> Result<(), AppError> {
        let mut rooms = self.rooms.lock().unwrap();

        let room = rooms.get_mut(room_id).ok_or_else(|| {
            warn!(room_id = %room_id, "Room not found when marking connected");
            AppError::NotFound("Room not found".to_string())
        })?;

        room.mark_connected(player_uuid);

        Ok(())
    }

    #[instrument(skip(self))]
    async fn update_last_activity(&self, room_id: &str) -> Result<(), AppError> {
        debug!(room_id = %room_id, "Updating last activity timestamp");

        let mut rooms = self.rooms.lock().unwrap();

        let room = rooms.get_mut(room_id).ok_or_else(|| {
            warn!(room_id = %room_id, "Room not found when updating activity");
            AppError::NotFound("Room not found".to_string())
        })?;

        room.last_activity_at = chrono::Utc::now();

        debug!(
            room_id = %room_id,
            last_activity = %room.last_activity_at,
            "Last activity timestamp updated"
        );

        Ok(())
    }

    #[instrument(skip(self))]
    async fn get_inactive_rooms(
        &self,
        inactive_duration: std::time::Duration,
    ) -> Result<Vec<String>, AppError> {
        debug!(
            inactive_duration_secs = inactive_duration.as_secs(),
            "Finding inactive rooms"
        );

        let rooms = self.rooms.lock().unwrap();
        let now = chrono::Utc::now();
        let threshold = now
            - chrono::Duration::from_std(inactive_duration)
                .map_err(|e| AppError::DatabaseError(format!("Invalid duration: {}", e)))?;

        let inactive_room_ids: Vec<String> = rooms
            .values()
            .filter(|room| room.last_activity_at < threshold)
            .map(|room| room.id.clone())
            .collect();

        debug!(
            count = inactive_room_ids.len(),
            threshold = %threshold,
            "Found inactive rooms"
        );

        Ok(inactive_room_ids)
    }

    #[instrument(skip(self))]
    async fn delete_room(&self, room_id: &str) -> Result<(), AppError> {
        debug!(room_id = %room_id, "Deleting room");

        let mut rooms = self.rooms.lock().unwrap();

        if rooms.remove(room_id).is_some() {
            debug!(room_id = %room_id, "Room deleted successfully");
            Ok(())
        } else {
            warn!(room_id = %room_id, "Room not found when attempting to delete");
            Err(AppError::NotFound("Room not found".to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Creates a test room with a specific ID and host
    pub fn create_test_room_with_host(room_id: &str, host_uuid: &str) -> RoomModel {
        let now = chrono::Utc::now();
        RoomModel {
            id: room_id.to_string(),
            host_uuid: Some(host_uuid.to_string()),
            status: "ONLINE".to_string(),
            player_uuids: vec![host_uuid.to_string()], // Test UUID for host
            ready_players: vec![],
            connected_players: vec![host_uuid.to_string()],
            created_at: now,
            last_activity_at: now,
        }
    }

    #[tokio::test]
    async fn test_create_and_get_room() {
        let repo = InMemoryRoomRepository::new();
        let room = create_test_room_with_host("test-room", "test-host");

        // Create room
        repo.create_room(&room).await.unwrap();

        // Get room
        let retrieved = repo.get_room(&room.id).await.unwrap();
        assert!(retrieved.is_some());
        let retrieved_room = retrieved.unwrap();
        assert_eq!(retrieved_room.id, room.id);
        assert_eq!(retrieved_room.host_uuid, room.host_uuid);
        assert_eq!(retrieved_room.status, "ONLINE");
        assert_eq!(retrieved_room.get_player_count(), 1);
    }

    #[tokio::test]
    async fn test_get_nonexistent_room() {
        let repo = InMemoryRoomRepository::new();

        let result = repo.get_room("nonexistent-room").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_create_duplicate_room() {
        let repo = InMemoryRoomRepository::new();
        let room = create_test_room_with_host("test-room", "test-host");

        // Create room
        repo.create_room(&room).await.unwrap();

        // Try to create the same room again
        let result = repo.create_room(&room).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::DatabaseError(_)));
    }

    #[tokio::test]
    async fn test_multiple_rooms() {
        let repo = InMemoryRoomRepository::new();
        let room1 = create_test_room_with_host("room-1", "host-1");
        let room2 = create_test_room_with_host("room-2", "host-2");
        let room3 = create_test_room_with_host("room-3", "host-3");

        // Create all rooms
        repo.create_room(&room1).await.unwrap();
        repo.create_room(&room2).await.unwrap();
        repo.create_room(&room3).await.unwrap();

        // Verify all rooms exist
        let retrieved1 = repo.get_room(&room1.id).await.unwrap();
        assert!(retrieved1.is_some());
        assert_eq!(retrieved1.unwrap().host_uuid, Some("host-1".to_string()));

        let retrieved2 = repo.get_room(&room2.id).await.unwrap();
        assert!(retrieved2.is_some());
        assert_eq!(retrieved2.unwrap().host_uuid, Some("host-2".to_string()));

        let retrieved3 = repo.get_room(&room3.id).await.unwrap();
        assert!(retrieved3.is_some());
        assert_eq!(retrieved3.unwrap().host_uuid, Some("host-3".to_string()));
    }

    #[tokio::test]
    async fn test_list_rooms_empty() {
        let repo = InMemoryRoomRepository::new();

        let rooms = repo.list_rooms().await.unwrap();
        assert!(rooms.is_empty());
    }

    #[tokio::test]
    async fn test_list_rooms_single() {
        let repo = InMemoryRoomRepository::new();
        let room = create_test_room_with_host("test-room", "test-host");

        repo.create_room(&room).await.unwrap();

        let rooms = repo.list_rooms().await.unwrap();
        assert_eq!(rooms.len(), 1);
        assert_eq!(rooms[0].id, "test-room");
        assert_eq!(rooms[0].host_uuid, Some("test-host".to_string()));
    }

    #[tokio::test]
    async fn test_list_rooms_multiple() {
        let repo = InMemoryRoomRepository::new();
        let room1 = create_test_room_with_host("room-1", "host-1");
        let room2 = create_test_room_with_host("room-2", "host-2");
        let room3 = create_test_room_with_host("room-3", "host-3");

        // Create all rooms
        repo.create_room(&room1).await.unwrap();
        repo.create_room(&room2).await.unwrap();
        repo.create_room(&room3).await.unwrap();

        // List all rooms
        let rooms = repo.list_rooms().await.unwrap();
        assert_eq!(rooms.len(), 3);

        // Verify all rooms are present (order may vary due to HashMap)
        let room_ids: std::collections::HashSet<String> =
            rooms.iter().map(|r| r.id.clone()).collect();
        assert!(room_ids.contains("room-1"));
        assert!(room_ids.contains("room-2"));
        assert!(room_ids.contains("room-3"));

        // Verify host names are correct
        let room_hosts: std::collections::HashMap<String, Option<String>> = rooms
            .iter()
            .map(|r| (r.id.clone(), r.host_uuid.clone()))
            .collect();
        assert_eq!(room_hosts.get("room-1"), Some(&Some("host-1".to_string())));
        assert_eq!(room_hosts.get("room-2"), Some(&Some("host-2".to_string())));
        assert_eq!(room_hosts.get("room-3"), Some(&Some("host-3".to_string())));
    }

    // UUID-specific tests
    #[tokio::test]
    async fn test_try_join_room_with_uuid_success() {
        let repo = InMemoryRoomRepository::new();
        let room = create_test_room_with_host("test-room", "host");
        repo.create_room(&room).await.unwrap();

        // Join room with UUID
        let result = repo.try_join_room("test-room", "player1").await.unwrap();

        match result {
            JoinRoomResult::Success(updated_room) => {
                assert_eq!(updated_room.get_player_count(), 2);
                assert!(updated_room.has_player("host"));
                assert!(updated_room.has_player("player1"));
                assert_eq!(updated_room.player_uuids.len(), 2);
            }
            _ => panic!("Expected success"),
        }
    }

    #[tokio::test]
    async fn test_try_join_room_with_uuid_duplicate() {
        let repo = InMemoryRoomRepository::new();
        let mut room = create_test_room_with_host("test-room", "host");
        room.add_player("player1".to_string());
        repo.create_room(&room).await.unwrap();

        // Try to join with same UUID
        let result = repo.try_join_room("test-room", "player1").await.unwrap();

        match result {
            JoinRoomResult::Success(updated_room) => {
                assert_eq!(updated_room.get_player_count(), 2); // Should not add duplicate
            }
            _ => panic!("Expected success for duplicate join"),
        }
    }

    #[tokio::test]
    async fn test_try_join_room_with_uuid_full_room() {
        let repo = InMemoryRoomRepository::new();
        let mut room = create_test_room_with_host("test-room", "host");

        // Fill room to capacity
        room.add_player("player1".to_string());
        room.add_player("player2".to_string());
        room.add_player("player3".to_string());
        repo.create_room(&room).await.unwrap();

        // Try to join full room
        let result = repo.try_join_room("test-room", "player4").await.unwrap();

        assert!(matches!(result, JoinRoomResult::RoomFull));
    }

    #[tokio::test]
    async fn test_try_join_room_idempotent_when_full() {
        let repo = InMemoryRoomRepository::new();
        let mut room = create_test_room_with_host("test-room", "host");

        // Fill room to capacity including the target player
        room.add_player("player1".to_string());
        room.add_player("player2".to_string());
        room.add_player("player3".to_string());
        repo.create_room(&room).await.unwrap();

        // Now room is full (host + player1 + player2 + player3 in tests count terms)
        // Re-join with an existing player should succeed (idempotent)
        let result = repo.try_join_room("test-room", "player1").await.unwrap();

        match result {
            JoinRoomResult::Success(updated_room) => {
                assert!(updated_room.has_player("player1"));
                assert_eq!(updated_room.get_player_count(), 4);
            }
            _ => panic!("Expected idempotent success for existing player in full room"),
        }
    }

    #[tokio::test]
    async fn test_leave_room_with_uuid_success() {
        let repo = InMemoryRoomRepository::new();
        let mut room = create_test_room_with_host("test-room", "host");
        room.add_player("player1".to_string());
        repo.create_room(&room).await.unwrap();

        // Leave room with UUID
        let result = repo.leave_room("test-room", "player1").await.unwrap();

        match result {
            LeaveRoomResult::Success(updated_room) => {
                assert_eq!(updated_room.get_player_count(), 1);
                assert!(!updated_room.has_player("player1"));
                assert!(!updated_room.has_player("player1-uuid"));
                assert_eq!(updated_room.player_uuids.len(), 1);
            }
            _ => panic!("Expected success"),
        }
    }

    #[tokio::test]
    async fn test_leave_room_with_uuid_host_change() {
        let repo = InMemoryRoomRepository::new();
        let mut room = create_test_room_with_host("test-room", "host");
        room.add_player("player1".to_string());
        repo.create_room(&room).await.unwrap();

        // Host leaves room
        let result = repo.leave_room("test-room", "host").await.unwrap();

        match result {
            LeaveRoomResult::Success(updated_room) => {
                assert_eq!(updated_room.get_player_count(), 1);
                assert_eq!(updated_room.host_uuid, Some("player1".to_string())); // player1 becomes new host
                assert!(!updated_room.has_player("host"));
                assert!(!updated_room.has_player("host-uuid"));
            }
            _ => panic!("Expected success"),
        }
    }

    #[tokio::test]
    async fn test_leave_room_with_uuid_empty_room_deleted() {
        let repo = InMemoryRoomRepository::new();
        let room = create_test_room_with_host("test-room", "host");
        repo.create_room(&room).await.unwrap();

        // Last player leaves room
        let result = repo.leave_room("test-room", "host").await.unwrap();

        assert!(matches!(result, LeaveRoomResult::RoomDeleted));

        // Verify room is deleted
        let room_check = repo.get_room("test-room").await.unwrap();
        assert!(room_check.is_none());
    }

    #[tokio::test]
    async fn test_room_model_uuid_methods() {
        let mut room = RoomModel::new("test-host".to_string());

        // Test adding players with UUIDs
        room.add_player("player1".to_string());
        room.add_player("player2".to_string());

        assert_eq!(room.get_player_count(), 3);
        assert!(room.has_player("player1"));
        assert!(room.has_player("player2"));
        assert!(room.has_player("test-host"));

        // Test removing player by UUID
        room.remove_player("player1");
        assert_eq!(room.get_player_count(), 2);
        assert!(!room.has_player("player1"));
        assert!(room.has_player("player2"));

        // Test removing player by both username and UUID
        room.remove_player("player2");
        assert_eq!(room.get_player_count(), 1);
        assert!(!room.has_player("player2"));
        assert!(room.has_player("test-host"));
    }

    #[tokio::test]
    async fn test_leave_room_with_only_bots_remaining_deletes_room() {
        let repo = InMemoryRoomRepository::new();
        let mut room = create_test_room_with_host("test-room", "human-player");
        room.add_player("bot-12345".to_string());
        room.add_player("bot-67890".to_string());
        repo.create_room(&room).await.unwrap();

        // Human player leaves, only bots remain
        let result = repo.leave_room("test-room", "human-player").await.unwrap();

        // Room should be deleted because only bots remain
        assert!(matches!(result, LeaveRoomResult::RoomDeleted));

        // Verify room is deleted
        let room_check = repo.get_room("test-room").await.unwrap();
        assert!(room_check.is_none());
    }

    #[tokio::test]
    async fn test_leave_room_with_bots_and_humans_remaining_does_not_delete() {
        let repo = InMemoryRoomRepository::new();
        let mut room = create_test_room_with_host("test-room", "human1");
        room.add_player("human2".to_string());
        room.add_player("bot-12345".to_string());
        repo.create_room(&room).await.unwrap();

        // One human leaves, another human and bot remain
        let result = repo.leave_room("test-room", "human1").await.unwrap();

        // Room should not be deleted because a human remains
        match result {
            LeaveRoomResult::Success(updated_room) => {
                assert_eq!(updated_room.get_player_count(), 2);
                assert!(updated_room.has_player("human2"));
                assert!(updated_room.has_player("bot-12345"));
            }
            _ => panic!("Expected success, got {:?}", result),
        }

        // Verify room still exists
        let room_check = repo.get_room("test-room").await.unwrap();
        assert!(room_check.is_some());
    }

    #[tokio::test]
    async fn test_leave_room_bot_leaves_humans_remain() {
        let repo = InMemoryRoomRepository::new();
        let mut room = create_test_room_with_host("test-room", "human1");
        room.add_player("human2".to_string());
        room.add_player("bot-12345".to_string());
        repo.create_room(&room).await.unwrap();

        // Bot leaves, humans remain
        let result = repo.leave_room("test-room", "bot-12345").await.unwrap();

        // Room should not be deleted because humans remain
        match result {
            LeaveRoomResult::Success(updated_room) => {
                assert_eq!(updated_room.get_player_count(), 2);
                assert!(updated_room.has_player("human1"));
                assert!(updated_room.has_player("human2"));
                assert!(!updated_room.has_player("bot-12345"));
            }
            _ => panic!("Expected success, got {:?}", result),
        }
    }

    #[tokio::test]
    async fn test_toggle_ready() {
        let repo = InMemoryRoomRepository::new();
        let mut room = create_test_room_with_host("test-room", "player1");
        room.add_player("player2".to_string());
        repo.create_room(&room).await.unwrap();

        // Initially no one is ready
        let room_check = repo.get_room("test-room").await.unwrap().unwrap();
        assert!(!room_check.is_ready("player1"));
        assert!(!room_check.is_ready("player2"));

        // Toggle player1 to ready
        repo.toggle_ready("test-room", "player1").await.unwrap();
        let room_check = repo.get_room("test-room").await.unwrap().unwrap();
        assert!(room_check.is_ready("player1"));
        assert!(!room_check.is_ready("player2"));

        // Toggle player1 to unready
        repo.toggle_ready("test-room", "player1").await.unwrap();
        let room_check = repo.get_room("test-room").await.unwrap().unwrap();
        assert!(!room_check.is_ready("player1"));

        // Toggle both players ready
        repo.toggle_ready("test-room", "player1").await.unwrap();
        repo.toggle_ready("test-room", "player2").await.unwrap();
        let room_check = repo.get_room("test-room").await.unwrap().unwrap();
        assert!(room_check.is_ready("player1"));
        assert!(room_check.is_ready("player2"));
    }

    #[tokio::test]
    async fn test_ready_state_cleared_on_player_leave() {
        let repo = InMemoryRoomRepository::new();
        let mut room = create_test_room_with_host("test-room", "player1");
        room.add_player("player2".to_string());
        repo.create_room(&room).await.unwrap();

        // Mark player2 as ready
        repo.toggle_ready("test-room", "player2").await.unwrap();
        let room_check = repo.get_room("test-room").await.unwrap().unwrap();
        assert!(room_check.is_ready("player2"));

        // Player2 leaves
        repo.leave_room("test-room", "player2").await.unwrap();

        // If player2 rejoins, they should not be ready
        let result = repo.try_join_room("test-room", "player2").await.unwrap();
        match result {
            JoinRoomResult::Success(updated_room) => {
                assert!(!updated_room.is_ready("player2"));
            }
            _ => panic!("Expected success"),
        }
    }

    #[tokio::test]
    async fn test_room_model_ready_methods() {
        let mut room = RoomModel::new("host".to_string());
        room.add_player("player1".to_string());
        room.add_player("player2".to_string());

        // Test mark_ready
        room.mark_ready("player1");
        assert!(room.is_ready("player1"));
        assert!(!room.is_ready("player2"));

        // Test duplicate mark_ready (should be idempotent)
        room.mark_ready("player1");
        assert_eq!(room.get_ready_players().len(), 1);

        // Test mark_unready
        room.mark_unready("player1");
        assert!(!room.is_ready("player1"));

        // Test toggle
        room.toggle_ready("player1");
        assert!(room.is_ready("player1"));
        room.toggle_ready("player1");
        assert!(!room.is_ready("player1"));

        // Test clear_ready_states
        room.mark_ready("player1");
        room.mark_ready("player2");
        assert_eq!(room.get_ready_players().len(), 2);
        room.clear_ready_states();
        assert_eq!(room.get_ready_players().len(), 0);
    }

    #[tokio::test]
    async fn test_leave_room_host_leaves_with_bots_human_becomes_host() {
        let repo = InMemoryRoomRepository::new();
        let mut room = create_test_room_with_host("test-room", "human-host");
        room.add_player("bot-12345".to_string()); // Bot should never become host
        room.add_player("human-player".to_string()); // This human should become host
        room.add_player("bot-67890".to_string()); // Another bot
        repo.create_room(&room).await.unwrap();

        // Human host leaves
        let result = repo.leave_room("test-room", "human-host").await.unwrap();

        // Verify that the first human (not bot) becomes the new host
        match result {
            LeaveRoomResult::Success(updated_room) => {
                assert_eq!(updated_room.get_player_count(), 3);
                assert_eq!(
                    updated_room.host_uuid,
                    Some("human-player".to_string()),
                    "First human player should become host, not bot"
                );
                assert!(updated_room.has_player("bot-12345"));
                assert!(updated_room.has_player("human-player"));
                assert!(updated_room.has_player("bot-67890"));
                assert!(!updated_room.has_player("human-host"));
            }
            _ => panic!("Expected success, got {:?}", result),
        }
    }
}
