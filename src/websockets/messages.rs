use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::bot::types::BotDifficulty;

/// Message types for WebSocket communication
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MessageType {
    // Client -> Server
    Chat,
    Move,
    Leave,
    StartGame,
    Ready,
    Heartbeat, // Client heartbeat to check connection health

    // Server -> Client
    PlayersList,
    HostChange,
    MovePlayed,
    TurnChange,
    Error,
    GameStarted,
    GameWon,
    BotAdded,
    BotRemoved,
    StatsUpdated,
    HeartbeatAck, // Server acknowledgment of heartbeat
}

/// Metadata for WebSocket messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketMessageMeta {
    pub timestamp: DateTime<Utc>,
    pub player_uuid: Option<String>,
}

/// Base structure for WebSocket messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketMessage {
    #[serde(rename = "type")]
    pub message_type: MessageType,
    pub payload: serde_json::Value,
    pub meta: Option<WebSocketMessageMeta>,
}

/// Client-to-Server message payloads
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatPayload {
    pub sender_uuid: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeavePayload {
    pub player: String,
}

/// Server-to-Client message payloads
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayersListPayload {
    /// Player UUIDs currently in the room
    pub players: Vec<String>,
    /// Mapping from UUID to display name for UI resolution
    pub mapping: std::collections::HashMap<String, String>,
    /// UUIDs of bot players in the room
    pub bot_uuids: Vec<String>,
    /// Bot difficulty keyed by bot UUID so clients can render bot state without inference
    pub bot_difficulties: std::collections::HashMap<String, BotDifficulty>,
    /// UUIDs of players who are ready
    pub ready_players: Vec<String>,
    /// UUID of the current host
    pub host_uuid: Option<String>,
    /// UUIDs of players currently connected
    pub connected_players: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostChangePayload {
    pub host: String,      // Display name for backwards compatibility
    pub host_uuid: String, // UUID of the new host
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MovePlayedPayload {
    pub player: String,
    pub cards: Vec<String>,
    pub remaining_cards: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameStartedPayload {
    pub current_turn: String,
    pub cards: Vec<String>, // Player's hand
    pub player_list: Vec<String>,
    pub card_counts: std::collections::HashMap<String, usize>, // UUID -> card count
    pub last_plays_by_player: std::collections::HashMap<String, Vec<String>>, // UUID -> last played cards
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnChangePayload {
    pub player: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameWonPayload {
    pub winner: String,
    pub winning_hand: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotAddedPayload {
    pub bot_uuid: String,
    pub bot_name: String,
    pub bot_difficulty: BotDifficulty,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotRemovedPayload {
    pub bot_uuid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsUpdatedPayload {
    pub room_stats: crate::stats::models::RoomStats,
}

/// Helper functions for creating messages
impl WebSocketMessage {
    pub fn new(message_type: MessageType, payload: serde_json::Value) -> Self {
        Self {
            message_type,
            payload,
            meta: Some(WebSocketMessageMeta {
                timestamp: Utc::now(),
                player_uuid: None,
            }),
        }
    }

    /// Create a PLAYERS_LIST message
    pub fn players_list(
        players: Vec<String>,
        mapping: std::collections::HashMap<String, String>,
        bot_uuids: Vec<String>,
        bot_difficulties: std::collections::HashMap<String, BotDifficulty>,
        ready_players: Vec<String>,
        host_uuid: Option<String>,
        connected_players: Vec<String>,
    ) -> Self {
        let payload = PlayersListPayload {
            players,
            mapping,
            bot_uuids,
            bot_difficulties,
            ready_players,
            host_uuid,
            connected_players,
        };
        Self::new(
            MessageType::PlayersList,
            serde_json::to_value(payload).unwrap(),
        )
    }

    /// Create a HOST_CHANGE message
    pub fn host_change(host: String, host_uuid: String) -> Self {
        let payload = HostChangePayload { host, host_uuid };
        Self::new(
            MessageType::HostChange,
            serde_json::to_value(payload).unwrap(),
        )
    }

    /// Create an ERROR message
    #[allow(dead_code)] // Message constructor for error responses
    pub fn error(message: String) -> Self {
        let payload = ErrorPayload {
            message,
            error_type: None,
        };
        Self::new(MessageType::Error, serde_json::to_value(payload).unwrap())
    }

    pub fn connected_elsewhere() -> Self {
        let payload = ErrorPayload {
            message: "Room open in another tab".to_string(),
            error_type: Some("connected_elsewhere".to_string()),
        };
        Self::new(MessageType::Error, serde_json::to_value(payload).unwrap())
    }

    /// Create a GAME_STARTED message
    pub fn game_started(
        current_turn: String,
        cards: Vec<String>,
        player_list: Vec<String>,
        card_counts: std::collections::HashMap<String, usize>,
        last_plays_by_player: std::collections::HashMap<String, Vec<String>>,
    ) -> Self {
        let payload = GameStartedPayload {
            current_turn,
            cards,
            player_list,
            card_counts,
            last_plays_by_player,
        };
        Self::new(
            MessageType::GameStarted,
            serde_json::to_value(payload).unwrap(),
        )
    }

    /// Create a MOVE_PLAYED message
    pub fn move_played(player: String, cards: Vec<String>, remaining_cards: usize) -> Self {
        let payload = MovePlayedPayload {
            player,
            cards,
            remaining_cards,
        };
        Self::new(
            MessageType::MovePlayed,
            serde_json::to_value(payload).unwrap(),
        )
    }

    /// Create a CHAT message
    pub fn chat(sender_uuid: String, content: String) -> Self {
        let payload = ChatPayload {
            sender_uuid,
            content,
        };
        Self::new(MessageType::Chat, serde_json::to_value(payload).unwrap())
    }

    /// Create a LEAVE message
    pub fn leave(player: String) -> Self {
        let payload = LeavePayload { player };
        Self::new(MessageType::Leave, serde_json::to_value(payload).unwrap())
    }

    /// Create a TURN_CHANGE message
    pub fn turn_change(player: String) -> Self {
        let payload = TurnChangePayload { player };
        Self::new(
            MessageType::TurnChange,
            serde_json::to_value(payload).unwrap(),
        )
    }

    /// Create a GAME_WON message
    pub fn game_won(winner: String, winning_hand: Vec<String>) -> Self {
        let payload = GameWonPayload {
            winner,
            winning_hand,
        };
        Self::new(MessageType::GameWon, serde_json::to_value(payload).unwrap())
    }

    /// Create a BOT_ADDED message
    pub fn bot_added(bot_uuid: String, bot_name: String, bot_difficulty: BotDifficulty) -> Self {
        let payload = BotAddedPayload {
            bot_uuid,
            bot_name,
            bot_difficulty,
        };
        Self::new(
            MessageType::BotAdded,
            serde_json::to_value(payload).unwrap(),
        )
    }

    /// Create a BOT_REMOVED message
    pub fn bot_removed(bot_uuid: String) -> Self {
        let payload = BotRemovedPayload { bot_uuid };
        Self::new(
            MessageType::BotRemoved,
            serde_json::to_value(payload).unwrap(),
        )
    }

    /// Create a STATS_UPDATED message
    pub fn stats_updated(room_stats: crate::stats::models::RoomStats) -> Self {
        let payload = StatsUpdatedPayload { room_stats };
        Self::new(
            MessageType::StatsUpdated,
            serde_json::to_value(payload).unwrap(),
        )
    }

    /// Create a HEARTBEAT_ACK message
    pub fn heartbeat_ack() -> Self {
        Self::new(MessageType::HeartbeatAck, serde_json::json!({}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_constructors_and_serialization() {
        // players_list
        let mut map = std::collections::HashMap::new();
        map.insert("u1".to_string(), "Alice".to_string());
        let m = WebSocketMessage::players_list(
            vec!["u1".to_string()],
            map.clone(),
            vec![],
            std::collections::HashMap::new(),
            vec![],
            Some("host-uuid".to_string()),
            vec!["u1".to_string()],
        );
        assert!(matches!(m.message_type, MessageType::PlayersList));
        let s = serde_json::to_string(&m).unwrap();
        let back: WebSocketMessage = serde_json::from_str(&s).unwrap();
        assert!(matches!(back.message_type, MessageType::PlayersList));

        // error
        let e = WebSocketMessage::error("oops".to_string());
        assert!(matches!(e.message_type, MessageType::Error));

        // host_change
        let h = WebSocketMessage::host_change("u1".to_string(), "host-uuid".to_string());
        assert!(matches!(h.message_type, MessageType::HostChange));

        // game_started
        let mut card_counts = std::collections::HashMap::new();
        card_counts.insert("u1".to_string(), 13);
        let last_plays = std::collections::HashMap::new();
        let gs = WebSocketMessage::game_started(
            "u1".to_string(),
            vec!["3D".to_string()],
            vec!["u1".to_string()],
            card_counts,
            last_plays,
        );
        assert!(matches!(gs.message_type, MessageType::GameStarted));

        // move_played
        let mp = WebSocketMessage::move_played("u1".to_string(), vec!["3D".to_string()], 12);
        assert!(matches!(mp.message_type, MessageType::MovePlayed));

        // chat
        let c = WebSocketMessage::chat("u1".to_string(), "hi".to_string());
        assert!(matches!(c.message_type, MessageType::Chat));

        // leave
        let l = WebSocketMessage::leave("u1".to_string());
        assert!(matches!(l.message_type, MessageType::Leave));

        // turn_change
        let t = WebSocketMessage::turn_change("u2".to_string());
        assert!(matches!(t.message_type, MessageType::TurnChange));

        // game_won
        let gw = WebSocketMessage::game_won("u3".to_string(), vec!["Card1".to_string()]);
        assert!(matches!(gw.message_type, MessageType::GameWon));

        // bot_added
        let ba = WebSocketMessage::bot_added(
            "bot-123".to_string(),
            "Bot 1".to_string(),
            BotDifficulty::Ai,
        );
        assert!(matches!(ba.message_type, MessageType::BotAdded));

        // bot_removed
        let br = WebSocketMessage::bot_removed("bot-123".to_string());
        assert!(matches!(br.message_type, MessageType::BotRemoved));

        // stats_updated
        let room_stats = crate::stats::models::RoomStats::default();
        let su = WebSocketMessage::stats_updated(room_stats);
        assert!(matches!(su.message_type, MessageType::StatsUpdated));
    }
}
