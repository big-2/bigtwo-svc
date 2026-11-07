//! Game scenario builders - pre-configured game states for testing
#![allow(dead_code)] // Test utilities may not all be used in every test

use bigtwo::game::{Card, Rank, Suit};

use super::setup::TestSetup;

// ============================================================================
// Card Creation Macro
// ============================================================================

#[macro_export]
macro_rules! cards {
    ($($rank:ident $suit:ident),* $(,)?) => {
        vec![$($crate::Card::new($crate::Rank::$rank, $crate::Suit::$suit)),*]
    };
}

// ============================================================================
// Game Setup Utilities
// ============================================================================

pub struct GameBuilder {
    player_data: Vec<(String, String, Vec<Card>)>, // (name, uuid, cards)
}

impl Default for GameBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl GameBuilder {
    pub fn new() -> Self {
        Self {
            player_data: vec![],
        }
    }

    /// Create a simple four-player game scenario
    pub fn with_simple_four_player_game(self) -> Self {
        self.with_cards(vec![
            (
                "alice",
                vec![
                    Card::new(Rank::Three, Suit::Diamonds),
                    Card::new(Rank::Four, Suit::Hearts),
                    Card::new(Rank::Five, Suit::Spades),
                ],
            ), // Alice has 3D, goes first
            (
                "bob",
                vec![
                    Card::new(Rank::Six, Suit::Clubs),
                    Card::new(Rank::Seven, Suit::Diamonds),
                    Card::new(Rank::Eight, Suit::Hearts),
                ],
            ),
            (
                "charlie",
                vec![
                    Card::new(Rank::Nine, Suit::Spades),
                    Card::new(Rank::Ten, Suit::Clubs),
                    Card::new(Rank::Jack, Suit::Diamonds),
                ],
            ),
            (
                "david",
                vec![
                    Card::new(Rank::Queen, Suit::Hearts),
                    Card::new(Rank::King, Suit::Spades),
                    Card::new(Rank::Ace, Suit::Clubs),
                ],
            ),
        ])
    }

    /// Create a scenario with pairs for testing pair gameplay
    pub fn with_pair_scenario(self) -> Self {
        self.with_cards(vec![
            (
                "alice",
                vec![
                    Card::new(Rank::Four, Suit::Hearts),
                    Card::new(Rank::Four, Suit::Spades),
                    Card::new(Rank::King, Suit::Clubs),
                ],
            ), // Alice has pair of 4s
            (
                "bob",
                vec![
                    Card::new(Rank::Three, Suit::Diamonds),
                    Card::new(Rank::Five, Suit::Hearts),
                    Card::new(Rank::Five, Suit::Clubs),
                ],
            ), // Bob has 3D (goes first) and pair of 5s
            (
                "charlie",
                vec![
                    Card::new(Rank::Seven, Suit::Spades),
                    Card::new(Rank::Eight, Suit::Clubs),
                    Card::new(Rank::Nine, Suit::Diamonds),
                ],
            ),
            (
                "david",
                vec![
                    Card::new(Rank::Ten, Suit::Hearts),
                    Card::new(Rank::Jack, Suit::Spades),
                    Card::new(Rank::Queen, Suit::Clubs),
                ],
            ),
        ])
    }

    /// Create a custom game with specific card distributions
    pub fn with_cards(mut self, cards: Vec<(&str, Vec<Card>)>) -> Self {
        self.player_data = cards
            .into_iter()
            .map(|(name, cards)| {
                // Map provided player name to the UUIDs used in TestSetupBuilder
                let uuid = match name {
                    "alice" => "550e8400-e29b-41d4-a716-446655440000".to_string(),
                    "bob" => "550e8400-e29b-41d4-a716-446655440001".to_string(),
                    "charlie" => "550e8400-e29b-41d4-a716-446655440002".to_string(),
                    "david" => "550e8400-e29b-41d4-a716-446655440003".to_string(),
                    _ => format!("{}-uuid", name),
                };
                (name.to_string(), uuid, cards)
            })
            .collect();
        self
    }

    /// Build the game and return the first player's name
    pub async fn build_with_setup(self, setup: &TestSetup) -> String {
        let first_player_name = if self.player_data.is_empty() {
            // No custom cards specified - use standard random dealing
            let player_names = setup.players.clone();
            setup
                .game_service
                .create_game(
                    "room-123",
                    &player_names
                        .iter()
                        .map(|p| p.0.clone())
                        .collect::<Vec<String>>(),
                )
                .await
                .unwrap();

            // Get the game to find who has 3D (they go first)
            let game = setup.game_service.get_game("room-123").await.unwrap();
            game.current_player_turn()
        } else {
            // Custom cards specified - create game with predetermined cards
            self.create_custom_game(setup).await
        };

        first_player_name
    }

    async fn create_custom_game(&self, setup: &TestSetup) -> String {
        // Use the new public API to create game with predetermined cards
        let game = setup
            .game_service
            .create_game_with_cards("room-123", self.player_data.clone())
            .await
            .unwrap();

        // Return the name of the first player (who has 3D)
        game.current_player_turn()
    }
}
