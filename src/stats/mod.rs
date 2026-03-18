pub mod calculators;
pub mod collectors;
pub mod service;

mod errors;
pub mod models;
pub mod repository;

pub use errors::StatsError;
pub use models::*;
#[allow(unused_imports)] // Public re-exports for handlers and future consumers
pub use repository::{
    GameHistoryRepository, InMemoryGameHistoryRepository, InMemoryStatsRepository,
    PostgresGameHistoryRepository, StatsRepository,
};
#[allow(unused_imports)] // Used by integration tests
pub use service::{StatsRoomSubscriber, StatsService};

use async_trait::async_trait;

use crate::game::Game;

pub type CollectedDataBatch = Vec<CollectedData>;

/// Priority constants for score calculators.
/// Lower values run first. Calculators with higher priority
/// can access and modify scores from lower-priority calculators.
pub mod calculator_priority {
    /// Base score calculation (e.g., card count)
    pub const BASE_SCORE: u32 = 100;
    /// Score multipliers (e.g., 10+ cards doubles score)
    pub const MULTIPLIER: u32 = 200;
}

#[async_trait]
pub trait StatCollector: Send + Sync {
    async fn collect(
        &self,
        game: &Game,
        winner_uuid: &str,
    ) -> Result<CollectedDataBatch, StatsError>;
}

pub trait ScoreCalculator: Send + Sync {
    fn calculate(
        &self,
        player_uuid: &str,
        collected_data: &[CollectedData],
        context: &CalculationContext,
    ) -> i32;

    fn priority(&self) -> u32;
}

pub struct CalculationContext<'a> {
    #[allow(dead_code)] // Context data for score calculations
    pub game_result: &'a GameResult,
    pub current_scores: &'a std::collections::HashMap<String, i32>,
}

impl<'a> CalculationContext<'a> {
    pub fn new(
        game_result: &'a GameResult,
        current_scores: &'a std::collections::HashMap<String, i32>,
    ) -> Self {
        Self {
            game_result,
            current_scores,
        }
    }
}
