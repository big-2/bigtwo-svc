use super::super::{CalculationContext, CollectedData, ScoreCalculator};

pub struct TenPlusMultiplierCalculator;

impl Default for TenPlusMultiplierCalculator {
    fn default() -> Self {
        Self::new()
    }
}

impl TenPlusMultiplierCalculator {
    pub fn new() -> Self {
        Self
    }
}

impl ScoreCalculator for TenPlusMultiplierCalculator {
    fn calculate(
        &self,
        player_uuid: &str,
        collected_data: &[CollectedData],
        context: &CalculationContext,
    ) -> i32 {
        let base_score = context
            .current_scores
            .get(player_uuid)
            .copied()
            .unwrap_or_default();

        let has_ten_plus = collected_data.iter().any(|data| match data {
            CollectedData::CardsRemaining {
                player_uuid: uuid,
                count,
            } => uuid == player_uuid && *count >= 10,
            _ => false,
        });

        if has_ten_plus {
            base_score * 2
        } else {
            base_score
        }
    }

    fn priority(&self) -> u32 {
        crate::stats::calculator_priority::MULTIPLIER
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::GameResult;
    use chrono::Utc;
    use std::collections::HashMap;

    fn build_context<'a>(
        game_result: &'a GameResult,
        scores: &'a HashMap<String, i32>,
    ) -> CalculationContext<'a> {
        CalculationContext {
            game_result,
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
    fn doubles_score_when_ten_or_more_cards() {
        let calculator = TenPlusMultiplierCalculator::new();
        let data = vec![CollectedData::CardsRemaining {
            player_uuid: "player".into(),
            count: 10,
        }];
        let game = sample_game();
        let mut scores = HashMap::new();
        scores.insert("player".into(), 12);

        let score = calculator.calculate("player", &data, &build_context(&game, &scores));
        assert_eq!(score, 24);
    }

    #[test]
    fn leaves_score_unchanged_for_less_than_ten() {
        let calculator = TenPlusMultiplierCalculator::new();
        let data = vec![CollectedData::CardsRemaining {
            player_uuid: "player".into(),
            count: 9,
        }];
        let game = sample_game();
        let mut scores = HashMap::new();
        scores.insert("player".into(), 9);

        let score = calculator.calculate("player", &data, &build_context(&game, &scores));
        assert_eq!(score, 9);
    }

    #[test]
    fn returns_zero_when_no_base_score() {
        let calculator = TenPlusMultiplierCalculator::new();
        let data = vec![CollectedData::CardsRemaining {
            player_uuid: "player".into(),
            count: 11,
        }];
        let game = sample_game();
        let scores = HashMap::new();

        let score = calculator.calculate("player", &data, &build_context(&game, &scores));
        assert_eq!(score, 0);
    }
}
