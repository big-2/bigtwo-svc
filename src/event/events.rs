use serde::{Deserialize, Serialize};

use crate::game::{Card, Game};

/// Room-specific events (delivered only to room subscribers)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RoomEvent {
    /// A player joined this room
    PlayerJoined { player: String },
    /// A player left this room
    PlayerLeft { player: String },
    /// The host of this room changed
    HostChanged { old_host: String, new_host: String },
    /// A chat message was sent in this room
    ChatMessage { sender: String, content: String },
    /// A player explicitly requested to leave (different from disconnect)
    PlayerLeaveRequested { player: String },
    /// WebSocket connection was established for a player
    PlayerConnected { player: String },
    /// WebSocket connection was lost for a player  
    PlayerDisconnected { player: String },
    /// Host attempt to start game
    TryStartGame { host: String },
    /// Create game (emitted when TryStartGame is successful)
    CreateGame { players: Vec<String> },
    /// Start game (emitted when CreateGame is successful)
    StartGame { game: Game },
    /// Player played move
    TryPlayMove { player: String, cards: Vec<Card> },
    /// Player played move
    MovePlayed {
        player: String,
        cards: Vec<Card>,
        game: Game,
    },
    /// Turn changed to next player
    TurnChanged { player: String },
    /// Game won by a player
    GameWon {
        /// UUID of the player who won the game
        winner: String,
        /// The cards that were played in the winning move
        winning_hand: Vec<Card>,
        /// Snapshot of the completed game state for downstream subscribers
        game: Game,
    },
    /// A bot was added to the room
    BotAdded { bot_uuid: String, bot_name: String },
    /// A bot was removed from the room
    BotRemoved { bot_uuid: String },
    /// A player toggled their ready state
    PlayerReadyToggled { player: String, is_ready: bool },
    /// Room stats were updated (typically after a game completion)
    StatsUpdated {
        room_stats: crate::stats::models::RoomStats,
    },
    /// Heartbeat received from a player (for connection health check)
    HeartbeatReceived { player: String },
}
