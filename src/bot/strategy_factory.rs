use std::sync::{Arc, OnceLock};

use super::{
    ai_strategy::AiBotStrategy,
    basic_strategy::BasicBotStrategy,
    types::{BotDifficulty, BotStrategy},
};

/// Factory for creating bot strategies based on difficulty level
pub struct BotStrategyFactory;

impl BotStrategyFactory {
    /// Create a strategy instance for the given difficulty level
    pub fn create_strategy(difficulty: BotDifficulty) -> Arc<dyn BotStrategy> {
        static EASY_STRATEGY: OnceLock<Arc<dyn BotStrategy>> = OnceLock::new();
        static MEDIUM_STRATEGY: OnceLock<Arc<dyn BotStrategy>> = OnceLock::new();
        static HARD_STRATEGY: OnceLock<Arc<dyn BotStrategy>> = OnceLock::new();
        static AI_STRATEGY: OnceLock<Arc<dyn BotStrategy>> = OnceLock::new();
        static EXPERT_STRATEGY: OnceLock<Arc<dyn BotStrategy>> = OnceLock::new();

        match difficulty {
            BotDifficulty::Easy => EASY_STRATEGY
                .get_or_init(|| Arc::new(BasicBotStrategy::new()))
                .clone(),
            BotDifficulty::Medium => MEDIUM_STRATEGY
                .get_or_init(|| Arc::new(BasicBotStrategy::new()))
                .clone(),
            BotDifficulty::Hard => HARD_STRATEGY
                .get_or_init(|| Arc::new(BasicBotStrategy::new()))
                .clone(),
            BotDifficulty::Ai => AI_STRATEGY
                .get_or_init(|| Arc::new(AiBotStrategy::new()))
                .clone(),
            BotDifficulty::Expert => EXPERT_STRATEGY
                .get_or_init(|| Arc::new(AiBotStrategy::with_strategy(Some("pimc"))))
                .clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_easy_strategy() {
        let strategy = BotStrategyFactory::create_strategy(BotDifficulty::Easy);
        assert_eq!(strategy.strategy_name(), "BasicBotStrategy");
    }

    #[test]
    fn test_create_medium_strategy() {
        let strategy = BotStrategyFactory::create_strategy(BotDifficulty::Medium);
        // Currently uses BasicBotStrategy, will be updated when MediumBotStrategy is implemented
        assert_eq!(strategy.strategy_name(), "BasicBotStrategy");
    }

    #[test]
    fn test_create_hard_strategy() {
        let strategy = BotStrategyFactory::create_strategy(BotDifficulty::Hard);
        // Currently uses BasicBotStrategy, will be updated when HardBotStrategy is implemented
        assert_eq!(strategy.strategy_name(), "BasicBotStrategy");
    }

    #[test]
    fn test_create_ai_strategy() {
        let strategy = BotStrategyFactory::create_strategy(BotDifficulty::Ai);
        assert_eq!(strategy.strategy_name(), "AiBotStrategy");
    }

    #[test]
    fn test_create_expert_strategy() {
        let strategy = BotStrategyFactory::create_strategy(BotDifficulty::Expert);
        assert_eq!(strategy.strategy_name(), "AiBotStrategy");
    }
}
