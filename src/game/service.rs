use crate::{
    game::{cards::Card, core::Game, repository::GameRepository},
    shared::AppError,
    user::PlayerMappingService,
};

/// Result of attempting to play a move in a game
#[derive(Debug, Clone)]
pub struct MoveResult {
    /// The updated game state after the move
    pub game: Game,
    /// Whether the player won the game with this move
    pub player_won: bool,
    /// The winning hand cards if the player won (always Some when player_won is true)
    pub winning_hand: Option<Vec<Card>>,
}

pub struct GameService {
    game_repository: GameRepository,
    player_mapping: std::sync::Arc<dyn PlayerMappingService>,
}

impl GameService {
    pub fn new(player_mapping: std::sync::Arc<dyn PlayerMappingService>) -> Self {
        Self {
            game_repository: GameRepository::new(),
            player_mapping,
        }
    }

    /// Create a new game for the specified room with the given players
    pub async fn create_game(
        &self,
        room_id: &str,
        player_uuids: &[String],
    ) -> Result<Game, AppError> {
        // Input validation
        if room_id.trim().is_empty() {
            return Err(AppError::BadRequest("Room ID cannot be empty".to_string()));
        }

        if player_uuids.len() != 4 {
            return Err(AppError::BadRequest(
                "Big Two requires exactly 4 players".to_string(),
            ));
        }

        for player in player_uuids {
            if player.trim().is_empty() {
                return Err(AppError::BadRequest(
                    "Player UUIDs cannot be empty".to_string(),
                ));
            }
        }

        // Check for duplicate player UUIDs
        let mut unique_players = std::collections::HashSet::new();
        for player in player_uuids {
            if !unique_players.insert(player.trim()) {
                return Err(AppError::BadRequest(
                    "All player UUIDs must be unique".to_string(),
                ));
            }
        }

        // Fetch player names for each UUID
        let mut player_data = Vec::new();
        for uuid in player_uuids {
            let name = self
                .player_mapping
                .get_playername(uuid)
                .await
                .ok_or_else(|| {
                    AppError::BadRequest(format!("Player name not found for UUID: {}", uuid))
                })?;
            player_data.push((name, uuid.clone()));
        }

        self.game_repository
            .create_game(room_id, &player_data)
            .await
            .map_err(|_e| AppError::Internal)?;

        self.game_repository
            .get_game(room_id)
            .await
            .ok_or(AppError::Internal)
    }

    /// Try to play a move for a player in the specified room
    /// Returns the updated game state and whether the player won
    pub async fn try_play_move(
        &self,
        room_id: &str,
        player_uuid: &str,
        cards: &[Card],
    ) -> Result<MoveResult, AppError> {
        // Get current game
        let mut game = self
            .game_repository
            .get_game(room_id)
            .await
            .ok_or_else(|| AppError::NotFound(format!("Game not found for room: {}", room_id)))?;

        // Execute the move and check if player won
        let player_won = game
            .play_cards(player_uuid, cards)
            .map_err(|e| AppError::NotFound(format!("Game error: {}", e)))?;

        let winning_hand = if player_won {
            Some(game.last_played_cards())
        } else {
            None
        };

        if player_won {
            // Remove the completed game immediately so queued moves cannot
            // observe stale state and emit a second terminal event.
            self.game_repository.remove_game(room_id).await;
        } else {
            self.game_repository
                .update_game(room_id, game.clone())
                .await
                .map_err(|_e| AppError::Internal)?;
        }

        Ok(MoveResult {
            game,
            player_won,
            winning_hand,
        })
    }

    /// Get the current game state for a room (read-only access)
    pub async fn get_game(&self, room_id: &str) -> Option<Game> {
        self.game_repository.get_game(room_id).await
    }

    /// Remove a game from the repository (typically after completion)
    pub async fn remove_game(&self, room_id: &str) -> Option<Game> {
        self.game_repository.remove_game(room_id).await
    }

    /// Create a new game with predetermined card distributions
    #[allow(dead_code)] // Public API for testing/debugging scenarios
    pub async fn create_game_with_cards(
        &self,
        room_id: &str,
        player_data: Vec<(String, String, Vec<Card>)>, // (name, uuid, cards)
    ) -> Result<Game, AppError> {
        let game = Game::new_game_with_cards(room_id.to_string(), player_data)
            .map_err(|e| AppError::BadRequest(format!("Invalid game setup: {}", e)))?;

        self.game_repository
            .update_game(room_id, game.clone())
            .await
            .map_err(|_e| AppError::Internal)?;

        Ok(game)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{
        cards::{Card, Rank, Suit},
        core::{Game, Player},
    };

    fn create_test_players() -> Vec<String> {
        vec![
            "550e8400-e29b-41d4-a716-446655440000".to_string(),
            "550e8400-e29b-41d4-a716-446655440001".to_string(),
            "550e8400-e29b-41d4-a716-446655440002".to_string(),
            "550e8400-e29b-41d4-a716-446655440003".to_string(),
        ]
    }

    #[allow(dead_code)] // Test helper for building game scenarios
    fn create_test_game() -> Game {
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
                    Card::new(Rank::Seven, Suit::Diamonds),
                ],
            },
        ];
        let starting_hands = players
            .iter()
            .map(|player| (player.name.clone(), player.cards.clone()))
            .collect();

        Game::new(
            "test_room".to_string(),
            players,
            0,      // Alice's turn
            0,      // No consecutive passes
            vec![], // No last played cards
            starting_hands,
        )
    }

    #[tokio::test]
    async fn test_create_game_success() {
        use crate::user::mapping_service::InMemoryPlayerMappingService;
        let player_mapping = std::sync::Arc::new(InMemoryPlayerMappingService::new());

        // Register test players in the mapping service
        let players = create_test_players();
        for player in &players {
            player_mapping
                .register_player(player.clone(), format!("Player{}", player))
                .await
                .unwrap();
        }

        let service = GameService::new(player_mapping);
        let result = service.create_game("test_room", &players).await;

        assert!(result.is_ok());
        let game = result.unwrap();
        assert_eq!(game.players().len(), 4);
    }

    #[tokio::test]
    async fn test_try_play_move_success() {
        use crate::user::mapping_service::InMemoryPlayerMappingService;
        let player_mapping = std::sync::Arc::new(InMemoryPlayerMappingService::new());

        // Create a game with 4 players (Big Two requirement)
        let players = vec![
            "550e8400-e29b-41d4-a716-446655440000".to_string(),
            "550e8400-e29b-41d4-a716-446655440001".to_string(),
            "550e8400-e29b-41d4-a716-446655440002".to_string(),
            "550e8400-e29b-41d4-a716-446655440003".to_string(),
        ];

        // Register test players in the mapping service
        for player in &players {
            player_mapping
                .register_player(player.clone(), format!("Player{}", player))
                .await
                .unwrap();
        }

        let service = GameService::new(player_mapping);
        service.create_game("test_room", &players).await.unwrap();

        // Get the game to see who has 3D and goes first
        let game = service.get_game("test_room").await.unwrap();
        let first_player = game.current_player_turn();

        // First player plays 3D (valid first move)
        let result = service
            .try_play_move(
                "test_room",
                &first_player,
                &[Card::new(Rank::Three, Suit::Diamonds)],
            )
            .await;

        assert!(result.is_ok());
        let move_result = result.unwrap();
        assert!(!move_result.player_won); // Player should not win yet
    }

    #[tokio::test]
    async fn test_try_play_move_game_not_found() {
        use crate::user::mapping_service::InMemoryPlayerMappingService;
        let player_mapping = std::sync::Arc::new(InMemoryPlayerMappingService::new());
        let service = GameService::new(player_mapping);

        let result = service
            .try_play_move(
                "nonexistent_room",
                "Alice",
                &[Card::new(Rank::Three, Suit::Diamonds)],
            )
            .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::NotFound(msg) => assert!(msg.contains("Game not found")),
            _ => panic!("Expected NotFound error"),
        }
    }

    #[tokio::test]
    async fn test_try_play_move_player_wins() {
        use crate::user::mapping_service::InMemoryPlayerMappingService;
        let player_mapping = std::sync::Arc::new(InMemoryPlayerMappingService::new());

        // Create a game with 4 players (Big Two requirement)
        let players = vec![
            "550e8400-e29b-41d4-a716-446655440000".to_string(),
            "550e8400-e29b-41d4-a716-446655440001".to_string(),
            "550e8400-e29b-41d4-a716-446655440002".to_string(),
            "550e8400-e29b-41d4-a716-446655440003".to_string(),
        ];

        // Register test players in the mapping service
        for player in &players {
            player_mapping
                .register_player(player.clone(), format!("Player{}", player))
                .await
                .unwrap();
        }

        let service = GameService::new(player_mapping);
        service.create_game("test_room", &players).await.unwrap();

        // Get the game and check who goes first
        let game = service.get_game("test_room").await.unwrap();

        // Find the player with 3D (since they must play it first)
        let player_with_3d = game
            .players()
            .iter()
            .find(|p| p.cards.contains(&Card::new(Rank::Three, Suit::Diamonds)))
            .unwrap();

        // For this test, we'll just verify the try_play_move functionality
        // In a real game, players won't win immediately after the first move
        let result = service
            .try_play_move(
                "test_room",
                &player_with_3d.uuid,
                &[Card::new(Rank::Three, Suit::Diamonds)],
            )
            .await;

        assert!(result.is_ok());
        let move_result = result.unwrap();
        // In a normal game with full hands, this won't be a win
        assert!(!move_result.player_won);
    }

    #[tokio::test]
    async fn test_winning_hand_capture() {
        use crate::user::mapping_service::InMemoryPlayerMappingService;
        let player_mapping = std::sync::Arc::new(InMemoryPlayerMappingService::new());

        let service = GameService::new(player_mapping.clone());

        // Create a game with predetermined cards where a player can win immediately
        let winning_cards = vec![Card::new(Rank::Three, Suit::Diamonds)];
        let player_data = vec![
            (
                "Alice".to_string(),
                "alice-uuid".to_string(),
                winning_cards.clone(),
            ),
            (
                "Bob".to_string(),
                "bob-uuid".to_string(),
                vec![Card::new(Rank::Four, Suit::Hearts)],
            ),
            (
                "Charlie".to_string(),
                "charlie-uuid".to_string(),
                vec![Card::new(Rank::Five, Suit::Spades)],
            ),
            (
                "David".to_string(),
                "david-uuid".to_string(),
                vec![Card::new(Rank::Six, Suit::Clubs)],
            ),
        ];

        service
            .create_game_with_cards("test_room", player_data)
            .await
            .unwrap();

        // Alice plays her only card and wins
        let result = service
            .try_play_move("test_room", "alice-uuid", &winning_cards)
            .await;

        assert!(result.is_ok());
        let move_result = result.unwrap();

        // Verify player won
        assert!(move_result.player_won);

        // Verify winning_hand is Some and contains the correct cards
        assert!(move_result.winning_hand.is_some());
        let captured_winning_hand = move_result.winning_hand.unwrap();
        assert_eq!(captured_winning_hand, winning_cards);

        // Winning moves should immediately remove the live game so
        // queued post-win moves cannot generate duplicate terminal events.
        assert!(service.get_game("test_room").await.is_none());
    }
}
