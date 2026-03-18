use super::super::{CalculationContext, CollectedData, ScoreCalculator};

pub struct CardCountScoreCalculator;

impl Default for CardCountScoreCalculator {
    fn default() -> Self {
        Self::new()
    }
}

impl CardCountScoreCalculator {
    pub fn new() -> Self {
        Self
    }
}

impl ScoreCalculator for CardCountScoreCalculator {
    fn calculate(
        &self,
        player_uuid: &str,
        collected_data: &[CollectedData],
        _context: &CalculationContext,
    ) -> i32 {
        collected_data
            .iter()
            .find_map(|data| match data {
                CollectedData::CardsRemaining {
                    player_uuid: uuid,
                    count,
                } if uuid == player_uuid => Some(*count as i32),
                _ => None,
            })
            .unwrap_or_default()
    }

    fn priority(&self) -> u32 {
        crate::stats::calculator_priority::BASE_SCORE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::GameResult;
    use chrono::Utc;
    use std::collections::HashMap;

    fn build_context<'a>(
        game: &'a GameResult,
        scores: &'a HashMap<String, i32>,
    ) -> CalculationContext<'a> {
        CalculationContext {
            game_result: game,
            current_scores: scores,
        }
    }

    fn sample_game() -> GameResult {
        GameResult {
            game_id: "room:1".into(),
            room_id: "room".into(),
            game_number: 1,
            winner_uuid: "winner".into(),
            players: vec![],
            moves: vec![],
            started_at: Utc::now(),
            completed_at: Utc::now(),
            had_bots: false,
        }
    }

    #[test]
    fn returns_card_count_for_player() {
        let calculator = CardCountScoreCalculator::new();
        let data = vec![CollectedData::CardsRemaining {
            player_uuid: "player".into(),
            count: 7,
        }];
        let game = sample_game();
        let scores = HashMap::new();

        let score = calculator.calculate("player", &data, &build_context(&game, &scores));
        assert_eq!(score, 7);
    }

    #[test]
    fn returns_zero_when_data_missing() {
        let calculator = CardCountScoreCalculator::new();
        let data = vec![CollectedData::CardsRemaining {
            player_uuid: "other".into(),
            count: 5,
        }];
        let game = sample_game();
        let scores = HashMap::new();

        let score = calculator.calculate("player", &data, &build_context(&game, &scores));
        assert_eq!(score, 0);
    }
}
