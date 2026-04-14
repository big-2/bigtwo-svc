use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// Database model for rooms table
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct RoomModel {
    pub id: String,                      // Random pet name generated ID
    pub host_uuid: Option<String>,       // UUID of room host
    pub status: String,                  // "ONLINE" or "OFFLINE"
    pub player_uuids: Vec<String>,       // List of player UUIDs in this room (for internal use)
    pub ready_players: Vec<String>,      // List of player UUIDs who are ready to start
    pub connected_players: Vec<String>,  // Players currently connected via WebSocket
    pub created_at: DateTime<Utc>,       // When the room was created
    pub last_activity_at: DateTime<Utc>, // Last time there was activity in the room
}

impl RoomModel {
    /// Creates a new room model with generated ID
    pub fn new(host_uuid: String) -> Self {
        let room_id = petname::Petnames::default().generate_one(2, "");
        let now = Utc::now();

        Self {
            id: room_id,
            host_uuid: Some(host_uuid.clone()),
            status: "ONLINE".to_string(),
            player_uuids: vec![host_uuid], // Host counts as being in the room immediately
            ready_players: vec![],         // No players ready initially
            connected_players: vec![],
            created_at: now,
            last_activity_at: now,
        }
    }

    /// Get the current number of players
    pub fn get_player_count(&self) -> i32 {
        self.player_uuids.len() as i32
    }

    /// Check if room is at capacity (4 players)
    pub fn is_full(&self) -> bool {
        self.player_uuids.len() >= 4
    }

    /// Check if a player is in this room (by UUID)
    pub fn has_player(&self, player_uuid: &str) -> bool {
        self.player_uuids.contains(&player_uuid.to_string())
    }

    /// Get all player UUIDs in the room
    pub fn get_player_uuids(&self) -> &Vec<String> {
        &self.player_uuids
    }

    /// Add a player to the room (both username and UUID)
    pub fn add_player(&mut self, player_uuid: String) {
        if !self.has_player(&player_uuid) {
            self.player_uuids.push(player_uuid.clone());
        }
        self.mark_connected(&player_uuid);
    }

    /// Remove a player from the room (both username and UUID)
    pub fn remove_player(&mut self, player_uuid: &str) {
        self.player_uuids.retain(|p| p != player_uuid);
        // Also remove from ready list when player leaves
        self.ready_players.retain(|p| p != player_uuid);
        self.connected_players.retain(|p| p != player_uuid);
    }

    /// Mark a player as ready
    pub fn mark_ready(&mut self, player_uuid: &str) {
        if self.has_player(player_uuid) && !self.is_ready(player_uuid) {
            self.ready_players.push(player_uuid.to_string());
        }
    }

    /// Mark a player as not ready
    pub fn mark_unready(&mut self, player_uuid: &str) {
        self.ready_players.retain(|p| p != player_uuid);
    }

    /// Toggle ready state for a player
    pub fn toggle_ready(&mut self, player_uuid: &str) {
        if self.is_ready(player_uuid) {
            self.mark_unready(player_uuid);
        } else {
            self.mark_ready(player_uuid);
        }
    }

    /// Set ready state for a player
    pub fn set_ready(&mut self, player_uuid: &str, is_ready: bool) {
        if is_ready {
            self.mark_ready(player_uuid);
        } else {
            self.mark_unready(player_uuid);
        }
    }

    /// Check if a player is ready
    pub fn is_ready(&self, player_uuid: &str) -> bool {
        self.ready_players.contains(&player_uuid.to_string())
    }

    /// Get all ready player UUIDs
    pub fn get_ready_players(&self) -> &Vec<String> {
        &self.ready_players
    }

    /// Clear all ready states (called when game starts)
    pub fn clear_ready_states(&mut self) {
        self.ready_players.clear();
    }

    /// Mark a player as connected
    pub fn mark_connected(&mut self, player_uuid: &str) {
        if self.has_player(player_uuid)
            && !self.connected_players.contains(&player_uuid.to_string())
        {
            self.connected_players.push(player_uuid.to_string());
        }
    }

    /// Mark a player as disconnected
    pub fn mark_disconnected(&mut self, player_uuid: &str) {
        self.connected_players.retain(|p| p != player_uuid);
    }

    /// Get all currently connected players
    pub fn get_connected_players(&self) -> &Vec<String> {
        &self.connected_players
    }
}
