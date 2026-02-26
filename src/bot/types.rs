use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::game::{Card, Game};

/// Represents a bot player in the game
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotPlayer {
    pub uuid: String,    // Bot UUID with "bot-" prefix
    pub name: String,    // Display name (e.g., "Bot 1")
    pub room_id: String, // Room the bot belongs to
    pub difficulty: BotDifficulty,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BotDifficulty {
    Easy,
    Medium,
    Hard,
    Ai,
}

impl BotPlayer {
    /// Create a new bot with a unique UUID
    pub fn new(room_id: String, name: String, difficulty: BotDifficulty) -> Self {
        let uuid = format!("bot-{}", Uuid::new_v4());
        Self {
            uuid,
            name,
            room_id,
            difficulty,
        }
    }

    /// Check if a UUID belongs to a bot
    pub fn is_bot_uuid(uuid: &str) -> bool {
        uuid.starts_with("bot-")
    }
}

/// Trait for bot decision-making strategies
#[async_trait]
pub trait BotStrategy: Send + Sync {
    /// Decide which cards to play given the current game state
    /// Returns None if the bot should pass
    async fn decide_move(&self, game: &Game, bot_uuid: &str) -> Option<Vec<Card>>;

    /// Get the name of this strategy
    #[allow(dead_code)] // Trait method for strategy identification
    fn strategy_name(&self) -> &'static str;
}
