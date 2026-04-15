use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::game::{Card, MoveAction};

#[derive(Debug, Clone)]
pub struct GameResult {
    pub game_id: String,
    pub room_id: String,
    #[allow(dead_code)] // Metadata for game tracking
    pub game_number: u32,
    pub winner_uuid: String,
    pub players: Vec<PlayerGameResult>,
    pub moves: Vec<GameMoveResult>,
    pub started_at: DateTime<Utc>,
    #[allow(dead_code)] // Metadata for future analytics
    pub completed_at: DateTime<Utc>,
    #[allow(dead_code)] // Metadata for filtering bot games
    pub had_bots: bool,
}

#[derive(Debug, Clone)]
pub struct PlayerGameResult {
    pub uuid: String,
    pub display_name: String,
    pub won: bool,
    #[allow(dead_code)] // Used in score calculations
    pub cards_remaining: u8,
    #[allow(dead_code)] // Score before multipliers
    pub raw_score: i32,
    pub final_score: i32,
    pub turns_taken: u32,
    pub passes: u32,
    pub plays: u32,
    pub cards_played: u32,
    pub started_first: bool,
}

#[derive(Debug, Clone)]
pub struct GameMoveResult {
    pub sequence: u32,
    pub player_uuid: String,
    pub action: MoveAction,
    pub cards: Vec<Card>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerProfileStatsResponse {
    pub player_uuid: String,
    pub display_name: String,
    pub summary: PlayerStatsSummary,
    pub play_style: PlayerPlayStyle,
    pub splits: PlayerStatsSplits,
    pub recent_form: PlayerRecentForm,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlayerStatsSummary {
    pub games_played: u64,
    pub wins: u64,
    pub win_rate: f64,
    pub current_win_streak: u64,
    pub best_win_streak: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlayerPlayStyle {
    pub total_turns: u64,
    pub total_passes: u64,
    pub pass_rate: f64,
    pub total_plays: u64,
    pub total_cards_played: u64,
    pub average_cards_per_play: f64,
    pub total_single_plays: u64,
    pub total_pair_plays: u64,
    pub total_triple_plays: u64,
    pub total_five_card_plays: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlayerStatsSplits {
    pub human_only: PlayerSplitSummary,
    pub with_bots: PlayerSplitSummary,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlayerSplitSummary {
    pub games_played: u64,
    pub wins: u64,
    pub win_rate: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlayerRecentForm {
    pub last_10: PlayerRecentWindow,
    pub last_25: PlayerRecentWindow,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlayerRecentWindow {
    pub wins: u64,
    pub win_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerRecentGamesResponse {
    pub games: Vec<PlayerRecentGameSummary>,
    pub next_before: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerRecentGameSummary {
    pub game_id: String,
    pub completed_at: DateTime<Utc>,
    pub winner_uuid: String,
    pub cards_remaining: u8,
    pub final_score: i32,
    pub had_bots: bool,
    pub opponents: Vec<GameOpponentSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameOpponentSummary {
    pub player_uuid: String,
    pub display_name: Option<String>,
    pub won: bool,
    pub is_bot: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedGameDetailResponse {
    pub game_id: String,
    pub room_id: String,
    pub game_number: u32,
    pub winner_uuid: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub had_bots: bool,
    pub players: Vec<CompletedGameDetailPlayer>,
    pub moves: Vec<CompletedGameDetailMove>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedGameDetailPlayer {
    pub player_uuid: String,
    pub display_name: Option<String>,
    pub won: bool,
    pub is_bot: bool,
    pub cards_remaining: u8,
    pub raw_score: i32,
    pub final_score: i32,
    pub turns_taken: u32,
    pub passes: u32,
    pub plays: u32,
    pub cards_played: u32,
    pub started_first: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedGameDetailMove {
    pub sequence: u32,
    pub player_uuid: String,
    pub display_name: Option<String>,
    pub is_bot: bool,
    pub action: MoveAction,
    pub cards: Vec<Card>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RoomStats {
    pub room_id: String,
    pub games_played: u32,
    pub player_stats: HashMap<String, PlayerStats>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlayerStats {
    pub uuid: String,
    pub games_played: u32,
    pub wins: u32,
    pub cards_remaining: u32,
    pub total_score: i32,
    pub current_win_streak: u32,
    pub best_win_streak: u32,
}

#[derive(Debug, Clone)]
pub enum CollectedData {
    CardsRemaining {
        player_uuid: String,
        count: u8,
    },
    #[allow(dead_code)] // Enum field used via pattern matching
    WinLoss {
        player_uuid: String,
        won: bool,
    },
}

impl CollectedData {
    #[allow(dead_code)] // Public API for accessing player UUID
    pub fn player_uuid(&self) -> &str {
        match self {
            CollectedData::CardsRemaining { player_uuid, .. } => player_uuid,
            CollectedData::WinLoss { player_uuid, .. } => player_uuid,
        }
    }
}
