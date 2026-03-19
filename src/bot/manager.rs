use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::shared::AppError;

use super::types::{BotDifficulty, BotPlayer};

pub const MAX_BOTS_PER_ROOM: usize = 3;

/// Manages bot players across all rooms
#[derive(Clone)]
pub struct BotManager {
    /// Map of bot UUID to bot player
    bots: Arc<RwLock<HashMap<String, BotPlayer>>>,
}

impl BotManager {
    pub fn new() -> Self {
        Self {
            bots: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new bot for a room
    pub async fn create_bot(
        &self,
        room_id: String,
        difficulty: BotDifficulty,
    ) -> Result<BotPlayer, AppError> {
        // Check bot count limit first
        let bot_count = self.get_bots_in_room(&room_id).await.len();

        if bot_count >= MAX_BOTS_PER_ROOM {
            return Err(AppError::BadRequest(format!(
                "Room {} already has the maximum of {} bots",
                room_id, MAX_BOTS_PER_ROOM
            )));
        }

        // Generate a unique bot name using petnames
        let petname = petname::Petnames::default().generate_one(2, "-");
        let bot_name = format!("{} Bot", petname);

        let bot = BotPlayer::new(room_id, bot_name, difficulty);

        info!(
            bot_uuid = %bot.uuid,
            bot_name = %bot.name,
            room_id = %bot.room_id,
            "Creating new bot"
        );

        // Store the bot
        let mut bots = self.bots.write().await;
        bots.insert(bot.uuid.clone(), bot.clone());

        Ok(bot)
    }

    /// Get a bot by UUID
    pub async fn get_bot(&self, bot_uuid: &str) -> Option<BotPlayer> {
        let bots = self.bots.read().await;
        bots.get(bot_uuid).cloned()
    }

    /// Remove a bot
    pub async fn remove_bot(&self, bot_uuid: &str) -> Result<(), AppError> {
        info!(bot_uuid = %bot_uuid, "Removing bot");

        let mut bots = self.bots.write().await;
        if bots.remove(bot_uuid).is_none() {
            return Err(AppError::NotFound(format!("Bot not found: {}", bot_uuid)));
        }

        Ok(())
    }

    /// Get all bots in a room
    pub async fn get_bots_in_room(&self, room_id: &str) -> Vec<BotPlayer> {
        let bots = self.bots.read().await;
        bots.values()
            .filter(|bot| bot.room_id == room_id)
            .cloned()
            .collect()
    }

    /// Get bot UUIDs in a room (for WebSocket messages)
    pub async fn get_bot_uuids_in_room(&self, room_id: &str) -> Vec<String> {
        let bots = self.bots.read().await;
        bots.values()
            .filter(|bot| bot.room_id == room_id)
            .map(|bot| bot.uuid.clone())
            .collect()
    }

    /// Get bot difficulties in a room keyed by bot UUID for client state rendering.
    pub async fn get_bot_difficulties_in_room(
        &self,
        room_id: &str,
    ) -> HashMap<String, BotDifficulty> {
        let bots = self.bots.read().await;
        bots.values()
            .filter(|bot| bot.room_id == room_id)
            .map(|bot| (bot.uuid.clone(), bot.difficulty))
            .collect()
    }

    /// Remove all bots from a room (called when room is deleted)
    pub async fn remove_all_bots_in_room(&self, room_id: &str) -> Result<(), AppError> {
        info!(room_id = %room_id, "Removing all bots from room");

        let mut bots = self.bots.write().await;
        let bot_uuids: Vec<String> = bots
            .values()
            .filter(|bot| bot.room_id == room_id)
            .map(|bot| bot.uuid.clone())
            .collect();

        for uuid in bot_uuids {
            bots.remove(&uuid);
            debug!(bot_uuid = %uuid, "Removed bot from room");
        }

        Ok(())
    }

    /// Check if a UUID belongs to a bot
    pub async fn is_bot(&self, uuid: &str) -> bool {
        BotPlayer::is_bot_uuid(uuid)
    }

    /// Get the total number of bots
    #[allow(dead_code)] // Public API for monitoring bot counts
    pub async fn bot_count(&self) -> usize {
        let bots = self.bots.read().await;
        bots.len()
    }
}

impl Default for BotManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_bot() {
        let manager = BotManager::new();
        let bot = manager
            .create_bot("room1".to_string(), BotDifficulty::Easy)
            .await
            .unwrap();

        assert!(bot.uuid.starts_with("bot-"));
        assert!(bot.name.ends_with(" Bot"));
        assert!(bot.name.contains('-')); // petname format includes dash
        assert_eq!(bot.room_id, "room1");
        assert_eq!(bot.difficulty, BotDifficulty::Easy);
    }

    #[tokio::test]
    async fn test_get_bot() {
        let manager = BotManager::new();
        let bot = manager
            .create_bot("room1".to_string(), BotDifficulty::Easy)
            .await
            .unwrap();

        let retrieved = manager.get_bot(&bot.uuid).await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().uuid, bot.uuid);
    }

    #[tokio::test]
    async fn test_remove_bot() {
        let manager = BotManager::new();
        let bot = manager
            .create_bot("room1".to_string(), BotDifficulty::Easy)
            .await
            .unwrap();

        let result = manager.remove_bot(&bot.uuid).await;
        assert!(result.is_ok());

        let retrieved = manager.get_bot(&bot.uuid).await;
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_get_bots_in_room() {
        let manager = BotManager::new();
        let bot1 = manager
            .create_bot("room1".to_string(), BotDifficulty::Easy)
            .await
            .unwrap();
        let bot2 = manager
            .create_bot("room1".to_string(), BotDifficulty::Medium)
            .await
            .unwrap();
        let _bot3 = manager
            .create_bot("room2".to_string(), BotDifficulty::Hard)
            .await
            .unwrap();

        let room1_bots = manager.get_bots_in_room("room1").await;
        assert_eq!(room1_bots.len(), 2);
        assert!(room1_bots.iter().any(|b| b.uuid == bot1.uuid));
        assert!(room1_bots.iter().any(|b| b.uuid == bot2.uuid));
    }

    #[tokio::test]
    async fn test_remove_all_bots_in_room() {
        let manager = BotManager::new();
        manager
            .create_bot("room1".to_string(), BotDifficulty::Easy)
            .await
            .unwrap();
        manager
            .create_bot("room1".to_string(), BotDifficulty::Medium)
            .await
            .unwrap();
        manager
            .create_bot("room2".to_string(), BotDifficulty::Hard)
            .await
            .unwrap();

        let result = manager.remove_all_bots_in_room("room1").await;
        assert!(result.is_ok());

        let room1_bots = manager.get_bots_in_room("room1").await;
        assert_eq!(room1_bots.len(), 0);

        let room2_bots = manager.get_bots_in_room("room2").await;
        assert_eq!(room2_bots.len(), 1);
    }

    #[tokio::test]
    async fn test_is_bot() {
        let manager = BotManager::new();
        assert!(manager.is_bot("bot-123").await);
        assert!(!manager.is_bot("user-123").await);
    }

    #[tokio::test]
    async fn test_bot_naming() {
        let manager = BotManager::new();
        let bot1 = manager
            .create_bot("room1".to_string(), BotDifficulty::Easy)
            .await
            .unwrap();
        let bot2 = manager
            .create_bot("room1".to_string(), BotDifficulty::Easy)
            .await
            .unwrap();

        // Both should have " Bot" suffix and petname format
        assert!(bot1.name.ends_with(" Bot"));
        assert!(bot2.name.ends_with(" Bot"));
        assert!(bot1.name.contains('-'));
        assert!(bot2.name.contains('-'));
        // Names should be unique (very high probability with petnames)
        assert_ne!(bot1.name, bot2.name);
    }

    #[tokio::test]
    async fn test_create_bot_respects_room_limit() {
        let manager = BotManager::new();

        for _ in 0..MAX_BOTS_PER_ROOM {
            manager
                .create_bot("room1".to_string(), BotDifficulty::Easy)
                .await
                .unwrap();
        }

        let result = manager
            .create_bot("room1".to_string(), BotDifficulty::Easy)
            .await;

        assert!(matches!(
            result,
            Err(AppError::BadRequest(message)) if message.contains("maximum")
        ));

        let room_bots = manager.get_bots_in_room("room1").await;
        assert_eq!(room_bots.len(), MAX_BOTS_PER_ROOM);
    }
}
