// Game history is a list of moves and a list of players (we can derive which player acted based on the history of moves), also has game ID

// The game structure will be passed around to different handlers that can update the state of the game
use crate::game::cards::{Card, Hand, HandError, Rank, Suit};
use chrono::{DateTime, Utc};
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Player {
    pub name: String, // For external compatibility
    pub uuid: String, // For internal identification
    pub cards: Vec<Card>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MoveAction {
    Pass,
    Play,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveRecord {
    pub sequence: u32,
    pub player_uuid: String,
    pub action: MoveAction,
    pub cards: Vec<Card>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum GameError {
    #[error("Invalid player")]
    InvalidPlayerTurn,
    #[error("Game already finished")]
    GameAlreadyFinished,
    #[error("Invalid played cards")]
    InvalidPlayedCards,
    #[error("Cannot pass - must play cards (3 consecutive passes)")]
    CannotPass,
    #[error("Player does not own card: {0}")]
    CardNotOwned(Card),
    #[error("Hand construction error")]
    HandError(HandError),
    #[error("First turn must include the 3 of diamonds")]
    FirstTurnMustIncludeThreeOfDiamonds,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Game {
    id: String,
    created_at: DateTime<Utc>,
    players: Vec<Player>, // The first player in the list is assumed to be the starting player
    current_turn: usize,  // The index of the player who is to act
    consecutive_passes: usize,
    played_hands: Vec<Hand>,
    starting_hands: std::collections::HashMap<String, Vec<Card>>, // Player name -> starting cards
    last_play_by_player: std::collections::HashMap<String, Vec<Card>>, // Player UUID -> last played cards
    move_log: Vec<MoveRecord>,
}

impl Game {
    pub fn new(
        id: String,
        players: Vec<Player>,
        current_turn: usize,
        consecutive_passes: usize,
        played_hands: Vec<Hand>,
        starting_hands: std::collections::HashMap<String, Vec<Card>>,
    ) -> Self {
        Self {
            id,
            created_at: Utc::now(),
            players,
            current_turn,
            consecutive_passes,
            played_hands,
            starting_hands,
            last_play_by_player: std::collections::HashMap::new(),
            move_log: Vec::new(),
        }
    }

    /// Standard constructor: creates a new game with UUIDs and names
    pub fn new_game(id: String, player_data: &[(String, String)]) -> Result<Self, GameError> {
        // Big Two requires exactly 4 players
        if player_data.len() != 4 {
            return Err(GameError::InvalidPlayedCards);
        }

        // Randomly deal the 52 cards to the players
        let mut cards = Card::all_cards();
        cards.shuffle(&mut rand::rng());

        // Create player data with cards
        let player_cards: Vec<(String, String, Vec<Card>)> = player_data
            .iter()
            .map(|(name, uuid)| {
                let mut player_cards: Vec<Card> = cards.drain(0..13).collect();
                player_cards.sort();
                (name.clone(), uuid.clone(), player_cards) // (name, uuid, cards)
            })
            .collect();

        // Delegate to the more general constructor
        Self::new_game_with_cards(id, player_cards)
    }

    /// Testing/specialized constructor: creates a game with predetermined cards (no player count restriction)
    pub fn new_game_with_cards(
        id: String,
        player_data: Vec<(String, String, Vec<Card>)>, // (name, uuid, cards)
    ) -> Result<Self, GameError> {
        // Create players with predetermined cards
        let mut players = Vec::new();
        let mut starting_hands = std::collections::HashMap::new();

        for (name, uuid, cards) in player_data.iter() {
            players.push(Player {
                name: name.clone(),
                uuid: uuid.clone(),
                cards: cards.clone(),
            });
            starting_hands.insert(uuid.clone(), cards.clone()); // Key by UUID for internal use
        }

        // Find who has 3D (they go first)
        let first_player_index = players
            .iter()
            .position(|p| p.cards.contains(&Card::new(Rank::Three, Suit::Diamonds)))
            .ok_or(GameError::InvalidPlayedCards)?;

        // Rotate players so the one with 3D is first
        players.rotate_left(first_player_index);

        Ok(Self::new(id, players, 0, 0, vec![], starting_hands))
    }

    /// Play cards by player UUID
    pub fn play_cards(&mut self, player_uuid: &str, cards: &[Card]) -> Result<bool, GameError> {
        if self.is_finished() {
            return Err(GameError::GameAlreadyFinished);
        }

        self.validate_player_turn(player_uuid)?;

        if cards.is_empty() {
            self.handle_pass()
        } else {
            self.handle_card_play(cards)
        }
    }

    fn validate_player_turn(&self, player_uuid: &str) -> Result<(), GameError> {
        let player = &self.players[self.current_turn];
        if player.uuid != player_uuid {
            return Err(GameError::InvalidPlayerTurn);
        }
        Ok(())
    }

    fn handle_pass(&mut self) -> Result<bool, GameError> {
        if self.consecutive_passes >= 3 {
            return Err(GameError::CannotPass);
        }

        // Track the pass for this player (empty vec indicates pass)
        let player_uuid = self.players[self.current_turn].uuid.clone();
        self.last_play_by_player.insert(player_uuid, vec![]);
        self.record_move(MoveAction::Pass, vec![]);

        self.consecutive_passes += 1;
        self.played_hands.push(Hand::Pass);
        self.advance_turn();
        Ok(false)
    }

    fn handle_card_play(&mut self, cards: &[Card]) -> Result<bool, GameError> {
        let new_hand = Hand::from_cards(cards).map_err(GameError::HandError)?;

        self.validate_card_play(&new_hand, cards)?;
        self.validate_card_ownership(cards)?;
        self.validate_first_turn(cards)?;

        let player_won = self.execute_card_play(cards, new_hand);
        if !player_won {
            self.advance_turn();
        }

        Ok(player_won)
    }

    fn validate_card_play(&self, new_hand: &Hand, _cards: &[Card]) -> Result<(), GameError> {
        // Skip validation if 3 consecutive passes have occurred (table is clear)
        if self.consecutive_passes < 3 && !self.played_hands.is_empty() {
            if let Some(last_non_pass_hand) =
                self.played_hands.iter().rev().find(|h| **h != Hand::Pass)
            {
                if !new_hand.can_beat(last_non_pass_hand) {
                    return Err(GameError::InvalidPlayedCards);
                }
            }
        }
        Ok(())
    }

    fn validate_card_ownership(&self, cards: &[Card]) -> Result<(), GameError> {
        let current_player = &self.players[self.current_turn];
        for card in cards {
            if !current_player.cards.contains(card) {
                return Err(GameError::CardNotOwned(*card));
            }
        }
        Ok(())
    }

    fn validate_first_turn(&self, cards: &[Card]) -> Result<(), GameError> {
        if self.played_hands.is_empty() {
            let three_of_diamonds = Card::new(Rank::Three, Suit::Diamonds);
            if !cards.contains(&three_of_diamonds) {
                return Err(GameError::FirstTurnMustIncludeThreeOfDiamonds);
            }
        }
        Ok(())
    }

    fn execute_card_play(&mut self, cards: &[Card], new_hand: Hand) -> bool {
        // Track the cards played for this player
        let player_uuid = self.players[self.current_turn].uuid.clone();

        // Clear all last plays when table is cleared (3 consecutive passes followed by a play)
        // This happens when consecutive_passes == 3, which means a new round is starting
        if self.consecutive_passes == 3 {
            self.last_play_by_player.clear();
        }

        self.last_play_by_player.insert(player_uuid, cards.to_vec());
        self.record_move(MoveAction::Play, cards.to_vec());

        // Remove played cards from the player's hand
        let current_player = &mut self.players[self.current_turn];
        for card in cards {
            if let Some(pos) = current_player.cards.iter().position(|c| c == card) {
                current_player.cards.remove(pos);
            }
        }

        self.consecutive_passes = 0;
        self.played_hands.push(new_hand);

        // Check if player won (has no cards left)
        current_player.cards.is_empty()
    }

    fn advance_turn(&mut self) {
        self.current_turn = (self.current_turn + 1) % self.players.len();
    }

    fn record_move(&mut self, action: MoveAction, cards: Vec<Card>) {
        let sequence = self.move_log.len() as u32 + 1;
        let player_uuid = self.players[self.current_turn].uuid.clone();
        self.move_log.push(MoveRecord {
            sequence,
            player_uuid,
            action,
            cards,
        });
    }

    pub fn is_finished(&self) -> bool {
        self.players.iter().any(|player| player.cards.is_empty())
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    pub fn players(&self) -> &Vec<Player> {
        &self.players
    }

    /// Get current player uuid
    pub fn current_player_turn(&self) -> String {
        self.players[self.current_turn].uuid.clone()
    }

    /// Get current player (for detailed access)
    #[allow(dead_code)] // Public API for future use
    pub fn current_player(&self) -> &Player {
        &self.players[self.current_turn]
    }

    pub fn consecutive_passes(&self) -> usize {
        self.consecutive_passes
    }

    pub fn last_played_cards(&self) -> Vec<Card> {
        if let Some(last_hand) = self.played_hands.last() {
            last_hand.to_cards()
        } else {
            vec![]
        }
    }

    /// Get the cards from the last non-pass hand played
    pub fn last_non_pass_cards(&self) -> Vec<Card> {
        if let Some(last_non_pass_hand) = self.played_hands.iter().rev().find(|h| **h != Hand::Pass)
        {
            last_non_pass_hand.to_cards()
        } else {
            vec![]
        }
    }

    /// Compute the UUID of the player who played the last non-pass hand
    pub fn last_non_pass_player_uuid(&self) -> Option<String> {
        // If there are no hands or only passes since start, there is no last non-pass player
        if self.played_hands.iter().all(|h| *h == Hand::Pass) {
            return None;
        }

        let num_players = self.players.len();
        if num_players == 0 {
            return None;
        }

        // The last player who played a non-pass is the one before the current player,
        // adjusted by the number of consecutive passes
        let last_index =
            (self.current_turn + num_players - 1 - self.consecutive_passes) % num_players;
        Some(self.players[last_index].uuid.clone())
    }

    #[allow(dead_code)] // Public API for future use
    pub fn get_last_played_hand(&self) -> Option<&Hand> {
        self.played_hands.last()
    }

    pub fn played_hands(&self) -> &[Hand] {
        &self.played_hands
    }

    #[allow(dead_code)] // Public API for accessing initial card distribution
    pub fn starting_hands(&self) -> &std::collections::HashMap<String, Vec<Card>> {
        &self.starting_hands
    }

    /// Get the last cards played by each player (UUID -> cards)
    ///
    /// Returns a reference to a HashMap mapping player UUIDs to their most recent play.
    /// - Empty vec (`[]`) indicates the player passed on their last turn
    /// - Non-empty vec contains the actual cards played
    /// - Players who haven't acted yet won't have entries in the map
    /// - All entries are cleared when the table resets (after 3 consecutive passes followed by a play)
    pub fn last_plays_by_player(&self) -> &std::collections::HashMap<String, Vec<Card>> {
        &self.last_play_by_player
    }

    pub fn move_log(&self) -> &[MoveRecord] {
        &self.move_log
    }
}

#[cfg(test)]
mod tests {
    use super::{Card, Game, GameError, Hand, MoveAction, Player, Rank, Suit};

    #[test]
    fn test_new_game() {
        let game = Game::new_game(
            "1".to_string(),
            &[
                ("Alice".to_string(), "alice-uuid".to_string()),
                ("Bob".to_string(), "bob-uuid".to_string()),
                ("Charlie".to_string(), "charlie-uuid".to_string()),
                ("David".to_string(), "david-uuid".to_string()),
            ],
        )
        .unwrap();
        assert_eq!(game.players().len(), 4);
        assert_eq!(game.consecutive_passes(), 0);

        // Check that the current player is the one who has the 3 of diamonds
        let current_player_uuid = game.current_player_turn();
        let current_player = game
            .players()
            .iter()
            .find(|p| p.uuid == current_player_uuid)
            .unwrap();
        let three_of_diamonds = Card::new(Rank::Three, Suit::Diamonds);
        assert!(
            current_player.cards.contains(&three_of_diamonds),
            "Current player '{}' should have the 3 of diamonds",
            current_player_uuid
        );

        // Check the cards are dealt and are all 52 unique cards
        let mut all_cards = Card::all_cards();
        all_cards.sort();

        // Collect all the cards in the players' hands
        let mut dealt_cards = game
            .players()
            .iter()
            .flat_map(|p| p.cards.clone())
            .collect::<Vec<Card>>();
        dealt_cards.sort();
        // Check that all the cards are unique
        assert_eq!(dealt_cards.len(), 52);
        assert_eq!(dealt_cards, all_cards);

        // Check that each player's cards are sorted
        for player in game.players() {
            let mut sorted_cards = player.cards.clone();
            sorted_cards.sort();
            assert_eq!(
                player.cards, sorted_cards,
                "Player {}'s cards should be sorted",
                player.name
            );
        }
    }

    #[test]
    fn test_card_removal_on_play() {
        let players = vec![
            Player {
                name: "Alice".to_string(),
                uuid: "alice-uuid".to_string(),
                cards: vec![
                    Card::new(Rank::Three, Suit::Diamonds),
                    Card::new(Rank::Four, Suit::Hearts),
                    Card::new(Rank::Five, Suit::Spades),
                ],
            },
            Player {
                name: "Bob".to_string(),
                uuid: "bob-uuid".to_string(),
                cards: vec![Card::new(Rank::Six, Suit::Clubs)],
            },
        ];

        let mut game = Game::new(
            "test".to_string(),
            players.clone(),
            0, // Alice's turn
            0,
            vec![],
            players
                .iter()
                .map(|player| (player.uuid.clone(), player.cards.clone()))
                .collect(),
        );

        let cards_to_play = vec![Card::new(Rank::Three, Suit::Diamonds)];
        let result = game.play_cards("alice-uuid", &cards_to_play);

        assert!(result.is_ok());
        assert!(!result.unwrap()); // Game should continue, Alice didn't win

        // Alice should have 2 cards left
        let alice = &game.players[0];
        assert_eq!(alice.cards.len(), 2);
        assert!(!alice
            .cards
            .contains(&Card::new(Rank::Three, Suit::Diamonds)));
        assert!(alice.cards.contains(&Card::new(Rank::Four, Suit::Hearts)));
        assert!(alice.cards.contains(&Card::new(Rank::Five, Suit::Spades)));
    }

    #[test]
    fn test_win_detection_single_card() {
        let players = vec![
            Player {
                name: "Alice".to_string(),
                uuid: "alice-uuid".to_string(),
                cards: vec![Card::new(Rank::Three, Suit::Diamonds)], // Only one card
            },
            Player {
                name: "Bob".to_string(),
                uuid: "bob-uuid".to_string(),
                cards: vec![Card::new(Rank::Six, Suit::Clubs)],
            },
        ];

        let mut game = Game::new(
            "test".to_string(),
            players.clone(),
            0, // Alice's turn
            0,
            vec![],
            players
                .iter()
                .map(|player| (player.uuid.clone(), player.cards.clone()))
                .collect(),
        );

        let cards_to_play = vec![Card::new(Rank::Three, Suit::Diamonds)];
        let result = game.play_cards("alice-uuid", &cards_to_play);

        assert!(result.is_ok());
        assert!(result.unwrap()); // Alice should win

        // Alice should have 0 cards left
        let alice = &game.players[0];
        assert_eq!(alice.cards.len(), 0);
    }

    #[test]
    fn test_win_detection_multiple_cards() {
        let players = vec![
            Player {
                name: "Alice".to_string(),
                uuid: "alice-uuid".to_string(),
                cards: vec![
                    Card::new(Rank::Three, Suit::Diamonds),
                    Card::new(Rank::Four, Suit::Diamonds),
                ], // Two cards left
            },
            Player {
                name: "Bob".to_string(),
                uuid: "bob-uuid".to_string(),
                cards: vec![Card::new(Rank::Six, Suit::Clubs)],
            },
        ];

        let mut game = Game::new(
            "test".to_string(),
            players.clone(),
            0, // Alice's turn
            0,
            vec![],
            players
                .iter()
                .map(|player| (player.uuid.clone(), player.cards.clone()))
                .collect(),
        );

        // Alice plays first card
        let result1 = game.play_cards("alice-uuid", &[Card::new(Rank::Three, Suit::Diamonds)]);
        assert!(result1.is_ok());
        assert!(!result1.unwrap()); // Game continues

        // Now it's Bob's turn, but Alice needs to play to win
        // Skip Bob's turn by having him pass
        let pass_result = game.play_cards("bob-uuid", &[]);
        assert!(pass_result.is_ok());

        // Alice plays her final card and should win
        let result2 = game.play_cards("alice-uuid", &[Card::new(Rank::Four, Suit::Diamonds)]);
        if result2.is_err() {
            println!("Error on second play: {:?}", result2.as_ref().unwrap_err());
        }
        assert!(result2.is_ok());
        assert!(result2.unwrap()); // Alice should win

        // Alice should have 0 cards left
        let alice = &game.players[0];
        assert_eq!(alice.cards.len(), 0);
    }

    #[test]
    fn test_pass_does_not_remove_cards() {
        let players = vec![
            Player {
                name: "Alice".to_string(),
                uuid: "alice-uuid".to_string(),
                cards: vec![
                    Card::new(Rank::Three, Suit::Diamonds),
                    Card::new(Rank::Four, Suit::Hearts),
                ],
            },
            Player {
                name: "Bob".to_string(),
                uuid: "bob-uuid".to_string(),
                cards: vec![Card::new(Rank::Six, Suit::Clubs)],
            },
        ];

        let mut game = Game::new(
            "test".to_string(),
            players.clone(),
            0, // Alice's turn
            0,
            vec![Hand::from_cards(&[Card::new(Rank::Five, Suit::Spades)]).unwrap()], // Someone played before
            players
                .iter()
                .map(|player| (player.uuid.clone(), player.cards.clone()))
                .collect(),
        );

        // Alice passes (empty cards vector)
        let result = game.play_cards("alice-uuid", &[]);

        assert!(result.is_ok());
        assert!(!result.unwrap()); // Game continues

        // Alice should still have 2 cards
        let alice = &game.players[0];
        assert_eq!(alice.cards.len(), 2);
        assert!(alice
            .cards
            .contains(&Card::new(Rank::Three, Suit::Diamonds)));
        assert!(alice.cards.contains(&Card::new(Rank::Four, Suit::Hearts)));
    }

    #[test]
    fn test_invalid_card_not_removed() {
        let players = vec![
            Player {
                name: "Alice".to_string(),
                uuid: "alice-uuid".to_string(),
                cards: vec![
                    Card::new(Rank::Three, Suit::Diamonds),
                    Card::new(Rank::Four, Suit::Hearts),
                ],
            },
            Player {
                name: "Bob".to_string(),
                uuid: "bob-uuid".to_string(),
                cards: vec![Card::new(Rank::Six, Suit::Clubs)],
            },
        ];

        let mut game = Game::new(
            "test".to_string(),
            players.clone(),
            0, // Alice's turn
            0,
            vec![],
            players
                .iter()
                .map(|player| (player.uuid.clone(), player.cards.clone()))
                .collect(),
        );

        // Try to play a card Alice doesn't have
        let cards_to_play = vec![Card::new(Rank::Ace, Suit::Spades)];
        let result = game.play_cards("alice-uuid", &cards_to_play);

        // Should now return an error for card not owned
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), GameError::CardNotOwned(_)));

        // Alice should still have 2 cards (invalid card wasn't in her hand anyway)
        let alice = &game.players[0];
        assert_eq!(alice.cards.len(), 2);
        assert!(alice
            .cards
            .contains(&Card::new(Rank::Three, Suit::Diamonds)));
        assert!(alice.cards.contains(&Card::new(Rank::Four, Suit::Hearts)));
    }

    #[test]
    fn test_invalid_player_turn() {
        let players = vec![
            Player {
                name: "Alice".to_string(),
                uuid: "alice-uuid".to_string(),
                cards: vec![Card::new(Rank::Three, Suit::Diamonds)],
            },
            Player {
                name: "Bob".to_string(),
                uuid: "bob-uuid".to_string(),
                cards: vec![Card::new(Rank::Six, Suit::Clubs)],
            },
        ];

        let mut game = Game::new(
            "test".to_string(),
            players.clone(),
            0, // Alice's turn
            0,
            vec![],
            players
                .iter()
                .map(|player| (player.uuid.clone(), player.cards.clone()))
                .collect(),
        );

        // Try to play with Bob when it's Alice's turn
        let result = game.play_cards("bob-uuid", &[Card::new(Rank::Six, Suit::Clubs)]);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), GameError::InvalidPlayerTurn));
    }

    #[test]
    fn test_cannot_pass_after_three_consecutive() {
        let players = vec![
            Player {
                name: "Alice".to_string(),
                uuid: "alice-uuid".to_string(),
                cards: vec![Card::new(Rank::Three, Suit::Diamonds)],
            },
            Player {
                name: "Bob".to_string(),
                uuid: "bob-uuid".to_string(),
                cards: vec![Card::new(Rank::Six, Suit::Clubs)],
            },
            Player {
                name: "Charlie".to_string(),
                uuid: "charlie-uuid".to_string(),
                cards: vec![Card::new(Rank::Seven, Suit::Hearts)],
            },
            Player {
                name: "David".to_string(),
                uuid: "david-uuid".to_string(),
                cards: vec![Card::new(Rank::Eight, Suit::Spades)],
            },
        ];

        let mut game = Game::new(
            "test".to_string(),
            players.clone(),
            0, // Alice's turn
            3, // 3 consecutive passes already
            vec![Hand::Pass, Hand::Pass, Hand::Pass],
            players
                .iter()
                .map(|player| (player.uuid.clone(), player.cards.clone()))
                .collect(),
        );

        // Try to pass when 3 consecutive passes already occurred
        let result = game.play_cards("alice-uuid", &[]);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), GameError::CannotPass));
    }

    #[test]
    fn test_invalid_played_cards_cannot_beat_previous() {
        let players = vec![
            Player {
                name: "Alice".to_string(),
                uuid: "alice-uuid".to_string(),
                cards: vec![
                    Card::new(Rank::Three, Suit::Diamonds),
                    Card::new(Rank::Four, Suit::Hearts),
                ],
            },
            Player {
                name: "Bob".to_string(),
                uuid: "bob-uuid".to_string(),
                cards: vec![Card::new(Rank::King, Suit::Spades)],
            },
        ];

        let mut game = Game::new(
            "test".to_string(),
            players.clone(),
            0, // Alice's turn
            0,
            vec![Hand::Single(crate::game::cards::SingleHand::new(
                Card::new(Rank::King, Suit::Hearts),
            ))], // Previous player played King of Hearts
            players
                .iter()
                .map(|player| (player.uuid.clone(), player.cards.clone()))
                .collect(),
        );

        // Try to play a weaker card (3D) when King was played
        let result = game.play_cards("alice-uuid", &[Card::new(Rank::Three, Suit::Diamonds)]);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), GameError::InvalidPlayedCards));
    }

    #[test]
    fn test_play_cards_player_doesnt_have() {
        let players = vec![
            Player {
                name: "Alice".to_string(),
                uuid: "alice-uuid".to_string(),
                cards: vec![
                    Card::new(Rank::Three, Suit::Diamonds),
                    Card::new(Rank::Four, Suit::Hearts),
                ],
            },
            Player {
                name: "Bob".to_string(),
                uuid: "bob-uuid".to_string(),
                cards: vec![Card::new(Rank::Six, Suit::Clubs)],
            },
        ];

        let mut game = Game::new(
            "test".to_string(),
            players.clone(),
            0, // Alice's turn
            0,
            vec![],
            players
                .iter()
                .map(|player| (player.uuid.clone(), player.cards.clone()))
                .collect(),
        );

        // Try to play a card Alice doesn't have (Ace of Spades)
        let result = game.play_cards("alice-uuid", &[Card::new(Rank::Ace, Suit::Spades)]);

        // Should now return an error for card not owned
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), GameError::CardNotOwned(_)));

        // Alice should still have her original 2 cards
        let alice = &game.players[0];
        assert_eq!(alice.cards.len(), 2);
        assert!(alice
            .cards
            .contains(&Card::new(Rank::Three, Suit::Diamonds)));
        assert!(alice.cards.contains(&Card::new(Rank::Four, Suit::Hearts)));
    }

    #[test]
    fn test_invalid_hand_construction() {
        let players = vec![
            Player {
                name: "Alice".to_string(),
                uuid: "alice-uuid".to_string(),
                cards: vec![
                    Card::new(Rank::Three, Suit::Diamonds),
                    Card::new(Rank::Four, Suit::Hearts),
                ],
            },
            Player {
                name: "Bob".to_string(),
                uuid: "bob-uuid".to_string(),
                cards: vec![Card::new(Rank::Six, Suit::Clubs)],
            },
        ];

        let mut game = Game::new(
            "test".to_string(),
            players.clone(),
            0, // Alice's turn
            0,
            vec![],
            players
                .iter()
                .map(|player| (player.uuid.clone(), player.cards.clone()))
                .collect(),
        );

        // Try to play an invalid pair (different ranks)
        let result = game.play_cards(
            "alice-uuid",
            &[
                Card::new(Rank::Three, Suit::Diamonds),
                Card::new(Rank::Four, Suit::Hearts),
            ],
        );

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), GameError::HandError(_)));
    }

    #[test]
    fn test_table_clear_after_three_passes() {
        let players = vec![
            Player {
                name: "Alice".to_string(),
                uuid: "alice-uuid".to_string(),
                cards: vec![Card::new(Rank::Three, Suit::Diamonds)],
            },
            Player {
                name: "Bob".to_string(),
                uuid: "bob-uuid".to_string(),
                cards: vec![Card::new(Rank::Four, Suit::Hearts)],
            },
            Player {
                name: "Charlie".to_string(),
                uuid: "charlie-uuid".to_string(),
                cards: vec![Card::new(Rank::Five, Suit::Spades)],
            },
            Player {
                name: "David".to_string(),
                uuid: "david-uuid".to_string(),
                cards: vec![Card::new(Rank::Six, Suit::Clubs)],
            },
        ];

        let mut game = Game::new(
            "test".to_string(),
            players.clone(),
            0, // Alice's turn
            2, // 2 consecutive passes
            vec![
                Hand::Single(crate::game::cards::SingleHand::new(Card::new(
                    Rank::King,
                    Suit::Hearts,
                ))),
                Hand::Pass,
                Hand::Pass,
            ],
            players
                .iter()
                .map(|player| (player.uuid.clone(), player.cards.clone()))
                .collect(),
        );

        // Alice passes (making it 3 consecutive passes)
        let result1 = game.play_cards("alice-uuid", &[]);
        assert!(result1.is_ok());
        assert_eq!(game.consecutive_passes, 3);

        // Now Bob should be able to play any card (table is clear)
        let result2 = game.play_cards("bob-uuid", &[Card::new(Rank::Four, Suit::Hearts)]);
        assert!(result2.is_ok());
        assert_eq!(game.consecutive_passes, 0); // Reset after card play
    }

    #[test]
    fn test_starting_hands_captured() {
        let game = Game::new_game(
            "1".to_string(),
            &[
                ("Alice".to_string(), "alice-uuid".to_string()),
                ("Bob".to_string(), "bob-uuid".to_string()),
                ("Charlie".to_string(), "charlie-uuid".to_string()),
                ("David".to_string(), "david-uuid".to_string()),
            ],
        )
        .unwrap();

        // Verify starting hands were captured for all players
        let starting_hands = game.starting_hands();
        assert_eq!(starting_hands.len(), 4);
        assert!(starting_hands.contains_key("alice-uuid"));
        assert!(starting_hands.contains_key("bob-uuid"));
        assert!(starting_hands.contains_key("charlie-uuid"));
        assert!(starting_hands.contains_key("david-uuid"));

        // Each player should have 13 starting cards
        for (player_uuid, cards) in starting_hands {
            assert_eq!(
                cards.len(),
                13,
                "Player {} should have 13 starting cards",
                player_uuid
            );
        }

        // Starting hands should match the sorted cards each player received
        for player in game.players() {
            let starting_cards = starting_hands.get(&player.uuid).unwrap();
            // The current cards might be different due to the rotation, but starting hands preserve original distribution
            assert_eq!(starting_cards.len(), 13);
        }
    }

    #[test]
    fn test_first_turn_must_include_three_of_diamonds() {
        let players = vec![
            Player {
                name: "Alice".to_string(),
                uuid: "alice-uuid".to_string(),
                cards: vec![
                    Card::new(Rank::Three, Suit::Diamonds),
                    Card::new(Rank::Four, Suit::Hearts),
                ],
            },
            Player {
                name: "Bob".to_string(),
                uuid: "bob-uuid".to_string(),
                cards: vec![Card::new(Rank::Six, Suit::Clubs)],
            },
        ];

        let mut game = Game::new(
            "test".to_string(),
            players.clone(),
            0, // Alice's turn
            0,
            vec![], // No hands played yet - first turn
            players
                .iter()
                .map(|player| (player.uuid.clone(), player.cards.clone()))
                .collect(),
        );

        // Try to play without 3 of diamonds - should fail
        let result = game.play_cards("alice-uuid", &[Card::new(Rank::Four, Suit::Hearts)]);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            GameError::FirstTurnMustIncludeThreeOfDiamonds
        ));
    }

    #[test]
    fn test_first_turn_with_three_of_diamonds_succeeds() {
        let players = vec![
            Player {
                name: "Alice".to_string(),
                uuid: "alice-uuid".to_string(),
                cards: vec![
                    Card::new(Rank::Three, Suit::Diamonds),
                    Card::new(Rank::Four, Suit::Hearts),
                ],
            },
            Player {
                name: "Bob".to_string(),
                uuid: "bob-uuid".to_string(),
                cards: vec![Card::new(Rank::Six, Suit::Clubs)],
            },
        ];

        let mut game = Game::new(
            "test".to_string(),
            players.clone(),
            0, // Alice's turn
            0,
            vec![], // No hands played yet - first turn
            players
                .iter()
                .map(|player| (player.uuid.clone(), player.cards.clone()))
                .collect(),
        );

        // Play with 3 of diamonds - should succeed
        let result = game.play_cards("alice-uuid", &[Card::new(Rank::Three, Suit::Diamonds)]);
        assert!(result.is_ok());
        assert_eq!(game.current_player_turn(), "bob-uuid"); // Turn should advance
        assert_eq!(game.played_hands().len(), 1); // Hand should be recorded
    }

    #[test]
    fn test_winning_turn_does_not_advance_and_game_rejects_later_moves() {
        let players = vec![
            Player {
                name: "Alice".to_string(),
                uuid: "alice-uuid".to_string(),
                cards: vec![Card::new(Rank::Three, Suit::Diamonds)],
            },
            Player {
                name: "Bob".to_string(),
                uuid: "bob-uuid".to_string(),
                cards: vec![Card::new(Rank::Four, Suit::Clubs)],
            },
        ];

        let mut game = Game::new(
            "test".to_string(),
            players.clone(),
            0,
            0,
            vec![],
            players
                .iter()
                .map(|player| (player.uuid.clone(), player.cards.clone()))
                .collect(),
        );

        let result = game.play_cards("alice-uuid", &[Card::new(Rank::Three, Suit::Diamonds)]);
        assert!(matches!(result, Ok(true)));
        assert!(game.is_finished());
        assert_eq!(game.current_player_turn(), "alice-uuid");

        let stale_move = game.play_cards("bob-uuid", &[Card::new(Rank::Four, Suit::Clubs)]);
        assert!(matches!(stale_move, Err(GameError::GameAlreadyFinished)));
    }

    #[test]
    fn test_first_turn_with_three_of_diamonds_in_combination_succeeds() {
        let players = vec![
            Player {
                name: "Alice".to_string(),
                uuid: "alice-uuid".to_string(),
                cards: vec![
                    Card::new(Rank::Three, Suit::Diamonds),
                    Card::new(Rank::Three, Suit::Hearts),
                    Card::new(Rank::Four, Suit::Hearts),
                ],
            },
            Player {
                name: "Bob".to_string(),
                uuid: "bob-uuid".to_string(),
                cards: vec![Card::new(Rank::Six, Suit::Clubs)],
            },
        ];

        let mut game = Game::new(
            "test".to_string(),
            players.clone(),
            0, // Alice's turn
            0,
            vec![], // No hands played yet - first turn
            players
                .iter()
                .map(|player| (player.uuid.clone(), player.cards.clone()))
                .collect(),
        );

        // Play a pair that includes 3 of diamonds - should succeed
        let result = game.play_cards(
            "alice-uuid",
            &[
                Card::new(Rank::Three, Suit::Diamonds),
                Card::new(Rank::Three, Suit::Hearts),
            ],
        );
        assert!(result.is_ok());
        assert_eq!(game.current_player_turn(), "bob-uuid"); // Turn should advance
        assert_eq!(game.played_hands().len(), 1); // Hand should be recorded
    }

    #[test]
    fn test_second_turn_does_not_require_three_of_diamonds() {
        let players = vec![
            Player {
                name: "Alice".to_string(),
                uuid: "alice-uuid".to_string(),
                cards: vec![
                    Card::new(Rank::Three, Suit::Diamonds),
                    Card::new(Rank::Four, Suit::Hearts),
                ],
            },
            Player {
                name: "Bob".to_string(),
                uuid: "bob-uuid".to_string(),
                cards: vec![Card::new(Rank::Four, Suit::Clubs)],
            },
        ];

        let mut game = Game::new(
            "test".to_string(),
            players.clone(),
            0, // Alice's turn
            0,
            vec![], // No hands played yet - first turn
            players
                .iter()
                .map(|player| (player.uuid.clone(), player.cards.clone()))
                .collect(),
        );

        // Alice plays 3 of diamonds successfully
        let result1 = game.play_cards("alice-uuid", &[Card::new(Rank::Three, Suit::Diamonds)]);
        assert!(result1.is_ok());
        assert_eq!(game.current_player_turn(), "bob-uuid"); // Should be Bob's turn

        // Bob should be able to play any valid card that beats 3♦ or just pass
        let result2 = game.play_cards("bob-uuid", &[Card::new(Rank::Four, Suit::Clubs)]);
        assert!(result2.is_ok());
        // Bob's play empties his hand, so the finished game should stay on Bob's turn.
        assert_eq!(game.current_player_turn(), "bob-uuid");
    }

    #[test]
    fn test_last_plays_tracking() {
        let players = vec![
            Player {
                name: "Alice".to_string(),
                uuid: "alice-uuid".to_string(),
                cards: vec![
                    Card::new(Rank::Three, Suit::Diamonds),
                    Card::new(Rank::Nine, Suit::Hearts),
                ],
            },
            Player {
                name: "Bob".to_string(),
                uuid: "bob-uuid".to_string(),
                cards: vec![
                    Card::new(Rank::Six, Suit::Clubs),
                    Card::new(Rank::Seven, Suit::Hearts),
                ],
            },
            Player {
                name: "Charlie".to_string(),
                uuid: "charlie-uuid".to_string(),
                cards: vec![
                    Card::new(Rank::Eight, Suit::Diamonds),
                    Card::new(Rank::Ten, Suit::Spades),
                ],
            },
        ];

        let mut game = Game::new(
            "test".to_string(),
            players.clone(),
            0, // Alice's turn
            0,
            vec![],
            players
                .iter()
                .map(|player| (player.uuid.clone(), player.cards.clone()))
                .collect(),
        );

        // Initially, no one has played
        assert_eq!(game.last_plays_by_player().len(), 0);

        // Alice plays 3 of diamonds
        let result1 = game.play_cards("alice-uuid", &[Card::new(Rank::Three, Suit::Diamonds)]);
        assert!(result1.is_ok());

        // Check Alice's last play is tracked
        let last_plays = game.last_plays_by_player();
        assert_eq!(last_plays.len(), 1);
        assert_eq!(
            last_plays.get("alice-uuid"),
            Some(&vec![Card::new(Rank::Three, Suit::Diamonds)])
        );

        // Bob plays a higher card
        let result2 = game.play_cards("bob-uuid", &[Card::new(Rank::Six, Suit::Clubs)]);
        assert!(result2.is_ok());

        // Check both plays are tracked
        let last_plays = game.last_plays_by_player();
        assert_eq!(last_plays.len(), 2);
        assert_eq!(
            last_plays.get("alice-uuid"),
            Some(&vec![Card::new(Rank::Three, Suit::Diamonds)])
        );
        assert_eq!(
            last_plays.get("bob-uuid"),
            Some(&vec![Card::new(Rank::Six, Suit::Clubs)])
        );

        // Charlie plays a higher card
        let result3 = game.play_cards("charlie-uuid", &[Card::new(Rank::Eight, Suit::Diamonds)]);
        assert!(result3.is_ok());

        // Check all three plays are tracked
        let last_plays = game.last_plays_by_player();
        assert_eq!(last_plays.len(), 3);
        assert_eq!(
            last_plays.get("alice-uuid"),
            Some(&vec![Card::new(Rank::Three, Suit::Diamonds)])
        );
        assert_eq!(
            last_plays.get("bob-uuid"),
            Some(&vec![Card::new(Rank::Six, Suit::Clubs)])
        );
        assert_eq!(
            last_plays.get("charlie-uuid"),
            Some(&vec![Card::new(Rank::Eight, Suit::Diamonds)])
        );

        // Alice plays again with a higher card (updating her last play)
        let result4 = game.play_cards("alice-uuid", &[Card::new(Rank::Nine, Suit::Hearts)]);
        assert!(result4.is_ok());

        // Check Alice's last play is updated
        let last_plays = game.last_plays_by_player();
        assert_eq!(last_plays.len(), 3);
        assert_eq!(
            last_plays.get("alice-uuid"),
            Some(&vec![Card::new(Rank::Nine, Suit::Hearts)])
        );
    }

    #[test]
    fn test_last_plays_cleared_after_table_reset() {
        let players = vec![
            Player {
                name: "Alice".to_string(),
                uuid: "alice-uuid".to_string(),
                cards: vec![
                    Card::new(Rank::Three, Suit::Diamonds),
                    Card::new(Rank::Four, Suit::Hearts),
                    Card::new(Rank::Five, Suit::Spades),
                ],
            },
            Player {
                name: "Bob".to_string(),
                uuid: "bob-uuid".to_string(),
                cards: vec![
                    Card::new(Rank::Six, Suit::Clubs),
                    Card::new(Rank::Seven, Suit::Hearts),
                ],
            },
            Player {
                name: "Charlie".to_string(),
                uuid: "charlie-uuid".to_string(),
                cards: vec![
                    Card::new(Rank::Eight, Suit::Diamonds),
                    Card::new(Rank::Nine, Suit::Spades),
                ],
            },
            Player {
                name: "David".to_string(),
                uuid: "david-uuid".to_string(),
                cards: vec![
                    Card::new(Rank::Ten, Suit::Clubs),
                    Card::new(Rank::Jack, Suit::Hearts),
                ],
            },
        ];

        let mut game = Game::new(
            "test".to_string(),
            players.clone(),
            0, // Alice's turn
            0,
            vec![],
            players
                .iter()
                .map(|player| (player.uuid.clone(), player.cards.clone()))
                .collect(),
        );

        // Alice plays 3 of diamonds
        let result1 = game.play_cards("alice-uuid", &[Card::new(Rank::Three, Suit::Diamonds)]);
        assert!(result1.is_ok());

        // Verify Alice's play is tracked
        assert_eq!(game.last_plays_by_player().len(), 1);
        assert_eq!(
            game.last_plays_by_player().get("alice-uuid"),
            Some(&vec![Card::new(Rank::Three, Suit::Diamonds)])
        );

        // Bob, Charlie, and David all pass
        let result2 = game.play_cards("bob-uuid", &[]);
        assert!(result2.is_ok());
        let result3 = game.play_cards("charlie-uuid", &[]);
        assert!(result3.is_ok());
        let result4 = game.play_cards("david-uuid", &[]);
        assert!(result4.is_ok());

        // At this point, we have 3 consecutive passes
        assert_eq!(game.consecutive_passes(), 3);

        // All players should have their last plays tracked (Alice played, others passed)
        assert_eq!(game.last_plays_by_player().len(), 4);

        // Alice plays again (table reset happens now because consecutive_passes == 3)
        let result5 = game.play_cards("alice-uuid", &[Card::new(Rank::Four, Suit::Hearts)]);
        assert!(result5.is_ok());

        // After table reset, all previous last plays should be cleared
        // Only Alice's new play should be tracked
        let last_plays = game.last_plays_by_player();
        assert_eq!(last_plays.len(), 1);
        assert_eq!(
            last_plays.get("alice-uuid"),
            Some(&vec![Card::new(Rank::Four, Suit::Hearts)])
        );
        assert_eq!(last_plays.get("bob-uuid"), None);
        assert_eq!(last_plays.get("charlie-uuid"), None);
        assert_eq!(last_plays.get("david-uuid"), None);
    }

    #[test]
    fn test_pass_represented_as_empty_vector() {
        let players = vec![
            Player {
                name: "Alice".to_string(),
                uuid: "alice-uuid".to_string(),
                cards: vec![
                    Card::new(Rank::Three, Suit::Diamonds),
                    Card::new(Rank::Four, Suit::Hearts),
                ],
            },
            Player {
                name: "Bob".to_string(),
                uuid: "bob-uuid".to_string(),
                cards: vec![
                    Card::new(Rank::Six, Suit::Clubs),
                    Card::new(Rank::Seven, Suit::Hearts),
                ],
            },
            Player {
                name: "Charlie".to_string(),
                uuid: "charlie-uuid".to_string(),
                cards: vec![
                    Card::new(Rank::Eight, Suit::Diamonds),
                    Card::new(Rank::Nine, Suit::Spades),
                ],
            },
        ];

        let mut game = Game::new(
            "test".to_string(),
            players.clone(),
            0, // Alice's turn
            0,
            vec![],
            players
                .iter()
                .map(|player| (player.uuid.clone(), player.cards.clone()))
                .collect(),
        );

        // Alice plays 3 of diamonds
        let result1 = game.play_cards("alice-uuid", &[Card::new(Rank::Three, Suit::Diamonds)]);
        assert!(result1.is_ok());

        // Alice's play should be tracked as non-empty vector
        let last_plays = game.last_plays_by_player();
        assert_eq!(last_plays.len(), 1);
        assert_eq!(
            last_plays.get("alice-uuid"),
            Some(&vec![Card::new(Rank::Three, Suit::Diamonds)])
        );

        // Bob passes
        let result2 = game.play_cards("bob-uuid", &[]);
        assert!(result2.is_ok());

        // Bob's pass should be tracked as empty vector
        let last_plays = game.last_plays_by_player();
        assert_eq!(last_plays.len(), 2);
        assert_eq!(
            last_plays.get("alice-uuid"),
            Some(&vec![Card::new(Rank::Three, Suit::Diamonds)])
        );
        assert_eq!(last_plays.get("bob-uuid"), Some(&vec![])); // Empty vec for pass

        // Charlie also passes
        let result3 = game.play_cards("charlie-uuid", &[]);
        assert!(result3.is_ok());

        // Charlie's pass should also be tracked as empty vector
        let last_plays = game.last_plays_by_player();
        assert_eq!(last_plays.len(), 3);
        assert_eq!(
            last_plays.get("alice-uuid"),
            Some(&vec![Card::new(Rank::Three, Suit::Diamonds)])
        );
        assert_eq!(last_plays.get("bob-uuid"), Some(&vec![])); // Empty vec for pass
        assert_eq!(last_plays.get("charlie-uuid"), Some(&vec![])); // Empty vec for pass

        // Alice plays again (after 2 passes, she can play again)
        let result4 = game.play_cards("alice-uuid", &[Card::new(Rank::Four, Suit::Hearts)]);
        assert!(result4.is_ok());

        // Alice's last play should be updated, Bob and Charlie's passes should remain
        let last_plays = game.last_plays_by_player();
        assert_eq!(last_plays.len(), 3);
        assert_eq!(
            last_plays.get("alice-uuid"),
            Some(&vec![Card::new(Rank::Four, Suit::Hearts)])
        );
        assert_eq!(last_plays.get("bob-uuid"), Some(&vec![])); // Still empty for pass
        assert_eq!(last_plays.get("charlie-uuid"), Some(&vec![])); // Still empty for pass
    }

    #[test]
    fn test_move_log_tracks_play_and_pass_order() {
        let players = vec![
            Player {
                name: "Alice".to_string(),
                uuid: "alice-uuid".to_string(),
                cards: vec![
                    Card::new(Rank::Three, Suit::Diamonds),
                    Card::new(Rank::Four, Suit::Hearts),
                ],
            },
            Player {
                name: "Bob".to_string(),
                uuid: "bob-uuid".to_string(),
                cards: vec![
                    Card::new(Rank::Five, Suit::Clubs),
                    Card::new(Rank::Six, Suit::Spades),
                ],
            },
        ];

        let mut game = Game::new(
            "test".to_string(),
            players.clone(),
            0,
            0,
            vec![],
            players
                .iter()
                .map(|player| (player.uuid.clone(), player.cards.clone()))
                .collect(),
        );

        game.play_cards("alice-uuid", &[Card::new(Rank::Three, Suit::Diamonds)])
            .unwrap();
        game.play_cards("bob-uuid", &[]).unwrap();

        let moves = game.move_log();
        assert_eq!(moves.len(), 2);
        assert_eq!(moves[0].sequence, 1);
        assert_eq!(moves[0].player_uuid, "alice-uuid");
        assert_eq!(moves[0].action, MoveAction::Play);
        assert_eq!(moves[1].sequence, 2);
        assert_eq!(moves[1].player_uuid, "bob-uuid");
        assert_eq!(moves[1].action, MoveAction::Pass);
        assert!(moves[1].cards.is_empty());
    }
}
