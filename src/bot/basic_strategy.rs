use async_trait::async_trait;
use rand::Rng;
use tracing::debug;

use crate::game::{Card, Game, Hand, Rank, Suit};

use super::types::BotStrategy;

/// Basic bot strategy that plays the lowest valid cards
pub struct BasicBotStrategy;

impl Default for BasicBotStrategy {
    fn default() -> Self {
        Self::new()
    }
}

impl BasicBotStrategy {
    pub fn new() -> Self {
        Self
    }

    /// Find all valid single card moves
    fn find_valid_singles(&self, game: &Game, available_cards: &[Card]) -> Vec<Vec<Card>> {
        Self::find_ordered_moves(game, available_cards, 1)
    }

    /// Find all valid pair moves
    fn find_valid_pairs(&self, game: &Game, available_cards: &[Card]) -> Vec<Vec<Card>> {
        Self::find_ordered_moves(game, available_cards, 2)
    }

    /// Find all valid triple moves
    fn find_valid_triples(&self, game: &Game, available_cards: &[Card]) -> Vec<Vec<Card>> {
        Self::find_ordered_moves(game, available_cards, 3)
    }

    /// Find all valid five-card moves (straights, flushes, full houses, etc.)
    fn find_valid_five_card_hands(&self, game: &Game, available_cards: &[Card]) -> Vec<Vec<Card>> {
        Self::find_ordered_moves(game, available_cards, 5)
    }

    fn find_ordered_moves(game: &Game, available_cards: &[Card], size: usize) -> Vec<Vec<Card>> {
        if available_cards.len() < size || size == 0 {
            return Vec::new();
        }

        Self::generate_suit_sorted_combinations(available_cards, size)
            .into_iter()
            .filter(|combo| Self::is_valid_move(game, combo))
            .collect()
    }

    fn generate_suit_sorted_combinations(cards: &[Card], size: usize) -> Vec<Vec<Card>> {
        if cards.len() < size || size == 0 {
            return Vec::new();
        }

        let mut sorted_cards = cards.to_vec();
        sorted_cards.sort();

        let mut result = Vec::new();
        let mut current = Vec::with_capacity(size);
        Self::build_combinations(&sorted_cards, size, 0, &mut current, &mut result);
        result
    }

    fn build_combinations(
        cards: &[Card],
        size: usize,
        start: usize,
        current: &mut Vec<Card>,
        result: &mut Vec<Vec<Card>>,
    ) {
        if current.len() == size {
            result.push(current.clone());
            return;
        }

        let remaining = size - current.len();
        for i in start..=cards.len() - remaining {
            current.push(cards[i]);
            Self::build_combinations(cards, size, i + 1, current, result);
            current.pop();
        }
    }

    fn is_valid_move(game: &Game, cards: &[Card]) -> bool {
        let hand = match Hand::from_cards(cards) {
            Ok(hand) => hand,
            Err(_) => return false,
        };

        let first_turn = game.played_hands().is_empty();
        let table_clear = game.consecutive_passes() >= 3;

        if first_turn {
            let three_diamonds = Card::new(Rank::Three, Suit::Diamonds);
            if !cards.contains(&three_diamonds) {
                return false;
            }
        }

        if table_clear || first_turn {
            return true;
        }

        if let Some(last_hand) = game.played_hands().iter().rev().find(|h| **h != Hand::Pass) {
            hand.can_beat(last_hand)
        } else {
            true
        }
    }

    /// Choose the best move from available options
    fn choose_best_move(&self, valid_moves: Vec<Vec<Card>>) -> Option<Vec<Card>> {
        let mut rng = rand::rng();
        self.choose_best_move_with_rng(valid_moves, &mut rng)
    }

    /// Build legal non-pass actions for the bot's current turn.
    pub(crate) fn find_legal_moves_for_bot_turn(game: &Game, bot_uuid: &str) -> Vec<Vec<Card>> {
        // Verify it's the bot's turn
        if game.current_player_turn() != bot_uuid {
            return Vec::new();
        }

        // Get the bot's available cards
        let bot_player = match game.players().iter().find(|p| p.uuid == bot_uuid) {
            Some(player) => player,
            None => return Vec::new(),
        };
        let available_cards = &bot_player.cards;
        let strategy = Self;

        // Find all valid moves
        let mut all_valid_moves = Vec::new();
        all_valid_moves.extend(strategy.find_valid_singles(game, available_cards));

        if available_cards.len() >= 2 {
            all_valid_moves.extend(strategy.find_valid_pairs(game, available_cards));
        }
        if available_cards.len() >= 3 {
            all_valid_moves.extend(strategy.find_valid_triples(game, available_cards));
        }
        if available_cards.len() >= 5 {
            all_valid_moves.extend(strategy.find_valid_five_card_hands(game, available_cards));
        }

        all_valid_moves
    }

    /// Choose the best move using lowest average rank, with randomized tie-breaking across categories.
    /// Categories are based on hand size: 1 (single), 2 (pair), 3 (triple), 5 (five-card combo).
    /// When multiple categories share the same best average, randomly pick one of those categories,
    /// then select that category's best move (by average) deterministically.
    pub(crate) fn choose_best_move_with_rng<R: Rng + ?Sized>(
        &self,
        valid_moves: Vec<Vec<Card>>,
        rng: &mut R,
    ) -> Option<Vec<Card>> {
        if valid_moves.is_empty() {
            return None;
        }

        // Helper to compute (sum, len) once
        fn sum_and_len(cards: &[Card]) -> (u32, usize) {
            let sum = cards.iter().map(|c| c.rank as u32).sum::<u32>();
            (sum, cards.len())
        }

        // Compare averages without floating point: compare sum_a/len_a vs sum_b/len_b
        fn cmp_average(a: (u32, usize), b: (u32, usize)) -> std::cmp::Ordering {
            let (sum_a, len_a) = (a.0 as u64, a.1 as u64);
            let (sum_b, len_b) = (b.0 as u64, b.1 as u64);
            (sum_a * len_b).cmp(&(sum_b * len_a))
        }

        // Partition moves by category size and find the best (lowest average) within each category.
        use std::collections::HashMap;
        let mut best_by_category: HashMap<usize, (Vec<Card>, (u32, usize))> = HashMap::new();

        for mv in valid_moves.into_iter() {
            let key = mv.len();
            let avg_key = sum_and_len(&mv);
            best_by_category
                .entry(key)
                .and_modify(|(best_mv, best_key)| {
                    let ord = cmp_average(avg_key, *best_key);
                    if ord == std::cmp::Ordering::Less
                        || (ord == std::cmp::Ordering::Equal && mv < *best_mv)
                    {
                        *best_mv = mv.clone();
                        *best_key = avg_key;
                    }
                })
                .or_insert((mv, avg_key));
        }

        if best_by_category.is_empty() {
            return None;
        }

        // Identify the global best average across categories
        let mut best_avg_overall: Option<(u32, usize)> = None;
        for (_, (_, avg_key)) in best_by_category.iter() {
            best_avg_overall = match best_avg_overall {
                None => Some(*avg_key),
                Some(curr) => {
                    if cmp_average(*avg_key, curr) == std::cmp::Ordering::Less {
                        Some(*avg_key)
                    } else {
                        Some(curr)
                    }
                }
            };
        }
        let best_avg_overall = best_avg_overall.unwrap();

        // Collect categories tied for the best average
        let mut tied_categories: Vec<usize> = best_by_category
            .iter()
            .filter_map(|(cat, (_mv, avg_key))| {
                if cmp_average(*avg_key, best_avg_overall) == std::cmp::Ordering::Equal {
                    Some(*cat)
                } else {
                    None
                }
            })
            .collect();

        // Ensure stable order before random selection (for deterministic seeded tests)
        tied_categories.sort_unstable();

        // Randomly choose one category among the best ties
        let chosen_idx = rng.random_range(0..tied_categories.len());
        let chosen_category = tied_categories[chosen_idx];

        // Return that category's best move
        best_by_category.remove(&chosen_category).map(|(mv, _)| mv)
    }
}

#[async_trait]
impl BotStrategy for BasicBotStrategy {
    async fn decide_move(&self, game: &Game, bot_uuid: &str) -> Option<Vec<Card>> {
        let all_valid_moves = Self::find_legal_moves_for_bot_turn(game, bot_uuid);
        let chosen_move = self.choose_best_move(all_valid_moves);

        debug!(
            bot_uuid = %bot_uuid,
            chosen_move = ?chosen_move,
            "Bot decided on move"
        );

        chosen_move
    }

    fn strategy_name(&self) -> &'static str {
        "BasicBotStrategy"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::SingleHand;
    use crate::game::{Game, Player};
    use rand::{rngs::StdRng, Rng, SeedableRng};

    #[tokio::test]
    async fn test_bot_plays_three_diamonds_first_turn() {
        let strategy = BasicBotStrategy::new();

        let players = vec![
            Player {
                name: "Bot".to_string(),
                uuid: "bot-123".to_string(),
                cards: vec![
                    Card::new(Rank::Three, Suit::Diamonds),
                    Card::new(Rank::Four, Suit::Hearts),
                ],
            },
            Player {
                name: "Human".to_string(),
                uuid: "human-456".to_string(),
                cards: vec![Card::new(Rank::Five, Suit::Spades)],
            },
        ];

        let game = Game::new(
            "test".to_string(),
            players.clone(),
            0,
            0,
            vec![],
            players
                .iter()
                .map(|p| (p.uuid.clone(), p.cards.clone()))
                .collect(),
        );

        let move_decision = strategy.decide_move(&game, "bot-123").await;
        assert!(move_decision.is_some());
        assert_eq!(
            move_decision.unwrap(),
            vec![Card::new(Rank::Three, Suit::Diamonds)]
        );
    }

    #[tokio::test]
    async fn test_bot_passes_when_no_valid_moves() {
        let strategy = BasicBotStrategy::new();

        let players = vec![
            Player {
                name: "Bot".to_string(),
                uuid: "bot-123".to_string(),
                cards: vec![Card::new(Rank::Three, Suit::Diamonds)],
            },
            Player {
                name: "Human".to_string(),
                uuid: "human-456".to_string(),
                cards: vec![Card::new(Rank::Five, Suit::Spades)],
            },
        ];

        let game = Game::new(
            "test".to_string(),
            players.clone(),
            0,
            0,
            vec![Hand::Single(SingleHand::new(Card::new(
                Rank::King,
                Suit::Spades,
            )))],
            players
                .iter()
                .map(|p| (p.uuid.clone(), p.cards.clone()))
                .collect(),
        );

        let move_decision = strategy.decide_move(&game, "bot-123").await;
        assert!(move_decision.is_none()); // Bot should pass
    }

    #[tokio::test]
    async fn test_bot_plays_lowest_card() {
        let strategy = BasicBotStrategy::new();

        let players = vec![
            Player {
                name: "Bot".to_string(),
                uuid: "bot-123".to_string(),
                cards: vec![
                    Card::new(Rank::Three, Suit::Diamonds),
                    Card::new(Rank::King, Suit::Spades),
                    Card::new(Rank::Ace, Suit::Hearts),
                ],
            },
            Player {
                name: "Human".to_string(),
                uuid: "human-456".to_string(),
                cards: vec![Card::new(Rank::Five, Suit::Spades)],
            },
        ];

        let game = Game::new(
            "test".to_string(),
            players.clone(),
            0,
            0,
            vec![],
            players
                .iter()
                .map(|p| (p.uuid.clone(), p.cards.clone()))
                .collect(),
        );

        let move_decision = strategy.decide_move(&game, "bot-123").await;
        assert!(move_decision.is_some());
        // Should play 3D (first turn requirement)
        assert_eq!(
            move_decision.unwrap(),
            vec![Card::new(Rank::Three, Suit::Diamonds)]
        );
    }

    #[tokio::test]
    async fn test_bot_plays_pair_to_beat_pair() {
        let strategy = BasicBotStrategy::new();

        let bot_cards = vec![
            Card::new(Rank::Four, Suit::Hearts),
            Card::new(Rank::Four, Suit::Spades),
            Card::new(Rank::Six, Suit::Clubs),
        ];

        let mut last_pair_cards = vec![
            Card::new(Rank::Three, Suit::Clubs),
            Card::new(Rank::Three, Suit::Spades),
        ];
        last_pair_cards.sort();
        let last_pair = Hand::from_cards(&last_pair_cards).unwrap();

        let players = vec![
            Player {
                name: "Bot".to_string(),
                uuid: "bot-123".to_string(),
                cards: bot_cards.clone(),
            },
            Player {
                name: "Human".to_string(),
                uuid: "human-456".to_string(),
                cards: vec![Card::new(Rank::Three, Suit::Diamonds)],
            },
        ];

        let starting_hands = players
            .iter()
            .map(|p| (p.uuid.clone(), p.cards.clone()))
            .collect();

        let game = Game::new(
            "test".to_string(),
            players.clone(),
            0,
            0,
            vec![last_pair],
            starting_hands,
        );

        let move_decision = strategy.decide_move(&game, "bot-123").await;
        assert!(move_decision.is_some());

        let mut expected_pair = vec![
            Card::new(Rank::Four, Suit::Hearts),
            Card::new(Rank::Four, Suit::Spades),
        ];
        expected_pair.sort();
        assert_eq!(move_decision.unwrap(), expected_pair);
    }

    #[tokio::test]
    async fn test_bot_plays_triple_to_beat_triple() {
        let strategy = BasicBotStrategy::new();

        let bot_cards = vec![
            Card::new(Rank::Six, Suit::Diamonds),
            Card::new(Rank::Six, Suit::Hearts),
            Card::new(Rank::Six, Suit::Spades),
            Card::new(Rank::Nine, Suit::Clubs),
        ];

        let mut last_triple_cards = vec![
            Card::new(Rank::Five, Suit::Clubs),
            Card::new(Rank::Five, Suit::Diamonds),
            Card::new(Rank::Five, Suit::Spades),
        ];
        last_triple_cards.sort();
        let last_triple = Hand::from_cards(&last_triple_cards).unwrap();

        let players = vec![
            Player {
                name: "Bot".to_string(),
                uuid: "bot-123".to_string(),
                cards: bot_cards.clone(),
            },
            Player {
                name: "Human".to_string(),
                uuid: "human-456".to_string(),
                cards: vec![Card::new(Rank::Three, Suit::Diamonds)],
            },
        ];

        let starting_hands = players
            .iter()
            .map(|p| (p.uuid.clone(), p.cards.clone()))
            .collect();

        let game = Game::new(
            "test".to_string(),
            players.clone(),
            0,
            0,
            vec![last_triple],
            starting_hands,
        );

        let move_decision = strategy.decide_move(&game, "bot-123").await;
        assert!(move_decision.is_some());

        let mut expected_triple = vec![
            Card::new(Rank::Six, Suit::Diamonds),
            Card::new(Rank::Six, Suit::Hearts),
            Card::new(Rank::Six, Suit::Spades),
        ];
        expected_triple.sort();
        assert_eq!(move_decision.unwrap(), expected_triple);
    }

    #[tokio::test]
    async fn test_bot_plays_five_card_hand_to_beat_straight() {
        let strategy = BasicBotStrategy::new();

        let bot_cards = vec![
            Card::new(Rank::Six, Suit::Clubs),
            Card::new(Rank::Seven, Suit::Diamonds),
            Card::new(Rank::Eight, Suit::Hearts),
            Card::new(Rank::Nine, Suit::Spades),
            Card::new(Rank::Ten, Suit::Clubs),
            Card::new(Rank::Queen, Suit::Hearts),
        ];

        let players = vec![
            Player {
                name: "Bot".to_string(),
                uuid: "bot-123".to_string(),
                cards: bot_cards.clone(),
            },
            Player {
                name: "Human".to_string(),
                uuid: "human-456".to_string(),
                cards: vec![Card::new(Rank::Three, Suit::Diamonds)],
            },
        ];

        let mut last_straight_cards = vec![
            Card::new(Rank::Five, Suit::Clubs),
            Card::new(Rank::Six, Suit::Diamonds),
            Card::new(Rank::Seven, Suit::Hearts),
            Card::new(Rank::Eight, Suit::Spades),
            Card::new(Rank::Nine, Suit::Clubs),
        ];
        last_straight_cards.sort();
        let last_straight = Hand::from_cards(&last_straight_cards).unwrap();

        let starting_hands = players
            .iter()
            .map(|p| (p.uuid.clone(), p.cards.clone()))
            .collect();

        let game = Game::new(
            "test".to_string(),
            players.clone(),
            0,
            0,
            vec![last_straight],
            starting_hands,
        );

        let move_decision = strategy.decide_move(&game, "bot-123").await;
        assert!(move_decision.is_some());

        let mut expected_straight = vec![
            Card::new(Rank::Six, Suit::Clubs),
            Card::new(Rank::Seven, Suit::Diamonds),
            Card::new(Rank::Eight, Suit::Hearts),
            Card::new(Rank::Nine, Suit::Spades),
            Card::new(Rank::Ten, Suit::Clubs),
        ];
        expected_straight.sort();
        assert_eq!(move_decision.unwrap(), expected_straight);
    }

    #[test]
    fn test_choose_best_move_uses_average_within_category() {
        let strategy = BasicBotStrategy::new();

        // Only five-card straights: 3-7 vs 4-8. Expect 3-7 (lower average)
        let mut straight_3_to_7 = vec![
            Card::new(Rank::Three, Suit::Clubs),
            Card::new(Rank::Four, Suit::Diamonds),
            Card::new(Rank::Five, Suit::Hearts),
            Card::new(Rank::Six, Suit::Spades),
            Card::new(Rank::Seven, Suit::Clubs),
        ];
        let mut straight_4_to_8 = vec![
            Card::new(Rank::Four, Suit::Clubs),
            Card::new(Rank::Five, Suit::Diamonds),
            Card::new(Rank::Six, Suit::Hearts),
            Card::new(Rank::Seven, Suit::Spades),
            Card::new(Rank::Eight, Suit::Clubs),
        ];
        straight_3_to_7.sort();
        straight_4_to_8.sort();

        let valid_moves = vec![straight_4_to_8.clone(), straight_3_to_7.clone()];

        let mut rng = StdRng::seed_from_u64(12345);
        let chosen = strategy
            .choose_best_move_with_rng(valid_moves, &mut rng)
            .unwrap();
        assert_eq!(chosen, straight_3_to_7);
    }

    #[test]
    fn test_random_category_selection_with_seeded_rng() {
        let strategy = BasicBotStrategy::new();

        // Provide one best candidate per category with identical averages (all Fives)
        let single_five = vec![Card::new(Rank::Five, Suit::Hearts)]; // size 1
        let mut pair_fives = vec![
            Card::new(Rank::Five, Suit::Diamonds),
            Card::new(Rank::Five, Suit::Clubs),
        ]; // size 2
        pair_fives.sort();
        let mut triple_fives = vec![
            Card::new(Rank::Five, Suit::Spades),
            Card::new(Rank::Five, Suit::Hearts),
            Card::new(Rank::Five, Suit::Diamonds),
        ]; // size 3
        triple_fives.sort();

        // Note: We don't include a five-card hand to keep categories [1,2,3]
        let valid_moves = vec![
            single_five.clone(),
            pair_fives.clone(),
            triple_fives.clone(),
        ];

        // Use a seeded RNG and clone it to compute the expected chosen index deterministically
        let seed = 42u64;
        let mut rng_for_choice = StdRng::seed_from_u64(seed);
        let mut rng_for_expect = rng_for_choice.clone();

        let chosen = strategy
            .choose_best_move_with_rng(valid_moves, &mut rng_for_choice)
            .unwrap();

        // Categories are sorted as [1,2,3]; derive expected index from the seed
        let expected_index = rng_for_expect.random_range(0..3);
        let expected_len = [1usize, 2usize, 3usize][expected_index];
        assert_eq!(chosen.len(), expected_len);
    }
}
