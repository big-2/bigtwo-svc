use std::sync::Arc;

use async_trait::async_trait;
use tracing::info;

use crate::{
    event::{EventBus, RoomEvent, RoomEventError, RoomEventHandler},
    game::{cards::Card, service::GameService},
};

pub struct GameEventRoomSubscriber {
    game_service: Arc<GameService>,
    event_bus: EventBus,
}

#[async_trait]
impl RoomEventHandler for GameEventRoomSubscriber {
    async fn handle_room_event(
        &self,
        room_id: &str,
        event: RoomEvent,
    ) -> Result<(), RoomEventError> {
        info!(
            room_id = %room_id,
            event = ?event,
            "Handling game event for WebSocket connections"
        );

        match event {
            RoomEvent::CreateGame { players } => {
                self.handle_create_game(room_id, &players).await?;
            }
            RoomEvent::TryPlayMove { player, cards } => {
                self.handle_player_played_move(room_id, &player, &cards)
                    .await?;
            }
            RoomEvent::GameWon {
                winner,
                winning_hand,
                game: _,
            } => {
                self.handle_game_won(room_id, &winner, &winning_hand)
                    .await?;
            }
            _ => {}
        }

        Ok(())
    }

    fn handler_name(&self) -> &'static str {
        "GameEventRoomSubscriber"
    }
}

impl GameEventRoomSubscriber {
    pub fn new(game_service: Arc<GameService>, event_bus: EventBus) -> Self {
        Self {
            game_service,
            event_bus,
        }
    }

    async fn handle_create_game(
        &self,
        room_id: &str,
        players: &[String],
    ) -> Result<(), RoomEventError> {
        info!(room_id = %room_id, "Starting Game");

        let game = self
            .game_service
            .create_game(room_id, players)
            .await
            .map_err(|e| RoomEventError::HandlerError(format!("Failed to create game: {}", e)))?;

        let game_message = RoomEvent::StartGame { game: game.clone() };

        self.event_bus.emit_to_room(room_id, game_message).await;

        Ok(())
    }

    async fn handle_player_played_move(
        &self,
        room_id: &str,
        player_uuid: &str,
        cards: &[Card],
    ) -> Result<(), RoomEventError> {
        info!(room_id = %room_id, player_uuid = %player_uuid, cards = ?cards, "Player played move");

        // Execute the move using GameService
        let move_result = self
            .game_service
            .try_play_move(room_id, player_uuid, cards)
            .await
            .map_err(|e| RoomEventError::HandlerError(format!("Failed to play move: {}", e)))?;

        // If player won, emit GameWon event and return
        if move_result.player_won {
            let winning_hand = move_result
                .winning_hand
                .expect("winning_hand should always be Some when player_won is true");
            self.event_bus
                .emit_to_room(
                    room_id,
                    RoomEvent::GameWon {
                        winner: player_uuid.to_string(),
                        winning_hand,
                        game: move_result.game.clone(),
                    },
                )
                .await;
            return Ok(());
        }

        // Emit move played event
        self.event_bus
            .emit_to_room(
                room_id,
                RoomEvent::MovePlayed {
                    player: player_uuid.to_string(),
                    cards: cards.to_vec(),
                    game: move_result.game.clone(),
                },
            )
            .await;

        // Emit turn changed event with the new current player
        self.event_bus
            .emit_to_room(
                room_id,
                RoomEvent::TurnChanged {
                    player: move_result.game.current_player_turn(),
                },
            )
            .await;

        Ok(())
    }

    async fn handle_game_won(
        &self,
        room_id: &str,
        winner: &str,
        _winning_hand: &[Card],
    ) -> Result<(), RoomEventError> {
        info!(room_id = %room_id, winner = %winner, "Game won");

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{
        cards::{Card, Rank, Suit},
        core::{Game, Player},
        GameService,
    };
    use crate::user::{mapping_service::InMemoryPlayerMappingService, PlayerMappingService};

    #[allow(dead_code)] // Test helper for building game scenarios
    fn create_test_players() -> Vec<Player> {
        vec![
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
                    Card::new(Rank::Seven, Suit::Diamonds),
                    Card::new(Rank::Eight, Suit::Hearts),
                ],
            },
            Player {
                name: "Charlie".to_string(),
                uuid: "charlie-uuid".to_string(),
                cards: vec![
                    Card::new(Rank::Nine, Suit::Spades),
                    Card::new(Rank::Ten, Suit::Clubs),
                    Card::new(Rank::Jack, Suit::Diamonds),
                ],
            },
            Player {
                name: "David".to_string(),
                uuid: "david-uuid".to_string(),
                cards: vec![
                    Card::new(Rank::Queen, Suit::Hearts),
                    Card::new(Rank::King, Suit::Spades),
                    Card::new(Rank::Ace, Suit::Clubs),
                ],
            },
        ]
    }

    #[allow(dead_code)] // Test helper for building game scenarios
    fn create_test_game() -> Game {
        let players = create_test_players();
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
    async fn test_game_room_subscriber_new() {
        let player_mapping = Arc::new(InMemoryPlayerMappingService::new());
        let game_service = Arc::new(GameService::new(player_mapping));
        let event_bus = EventBus::new();

        let subscriber = GameEventRoomSubscriber::new(game_service, event_bus);
        assert_eq!(subscriber.handler_name(), "GameEventRoomSubscriber");
    }

    #[tokio::test]
    async fn test_handle_create_game() {
        let player_mapping = Arc::new(InMemoryPlayerMappingService::new());
        let game_service = Arc::new(GameService::new(player_mapping.clone()));
        let event_bus = EventBus::new();
        let subscriber = GameEventRoomSubscriber::new(game_service.clone(), event_bus);

        let players = vec![
            "550e8400-e29b-41d4-a716-446655440000".to_string(),
            "550e8400-e29b-41d4-a716-446655440001".to_string(),
            "550e8400-e29b-41d4-a716-446655440002".to_string(),
            "550e8400-e29b-41d4-a716-446655440003".to_string(),
        ];

        // Register players in the mapping service
        for (i, uuid) in players.iter().enumerate() {
            player_mapping
                .register_player(uuid.clone(), format!("Player{}", i + 1))
                .await
                .unwrap();
        }

        let result = subscriber.handle_create_game("test_room", &players).await;
        assert!(result.is_ok());

        // Verify game was created
        let game = game_service.get_game("test_room").await;
        assert!(game.is_some());

        let game = game.unwrap();
        assert_eq!(game.players().len(), 4);
        // Note: We can't assert current_player_turn() == "alice-uuid" because new_game()
        // rotates players based on who has 3D, which is random
        assert!(game.players().iter().any(|p| p.name == "Player1"));
    }

    #[tokio::test]
    async fn test_handle_player_played_move_success() {
        let player_mapping = Arc::new(InMemoryPlayerMappingService::new());
        let game_service = Arc::new(GameService::new(player_mapping.clone()));
        let event_bus = EventBus::new();
        let subscriber = GameEventRoomSubscriber::new(game_service.clone(), event_bus);

        // Create a game with 4 players (Big Two requirement)
        let players = vec![
            "550e8400-e29b-41d4-a716-446655440000".to_string(),
            "550e8400-e29b-41d4-a716-446655440001".to_string(),
            "550e8400-e29b-41d4-a716-446655440002".to_string(),
            "550e8400-e29b-41d4-a716-446655440003".to_string(),
        ];

        // Register players in the mapping service
        for (i, uuid) in players.iter().enumerate() {
            player_mapping
                .register_player(uuid.clone(), format!("Player{}", i + 1))
                .await
                .unwrap();
        }

        game_service
            .create_game("test_room", &players)
            .await
            .unwrap();

        // Get the game to find who has 3D
        let game = game_service.get_game("test_room").await.unwrap();
        let player_with_3d = game
            .players()
            .iter()
            .find(|p| p.cards.contains(&Card::new(Rank::Three, Suit::Diamonds)))
            .unwrap();

        // Player with 3D plays it
        let result = subscriber
            .handle_player_played_move(
                "test_room",
                &player_with_3d.uuid,
                &[Card::new(Rank::Three, Suit::Diamonds)],
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_player_played_move_game_not_found() {
        let player_mapping = Arc::new(InMemoryPlayerMappingService::new());
        let game_service = Arc::new(GameService::new(player_mapping.clone()));
        let event_bus = EventBus::new();
        let subscriber = GameEventRoomSubscriber::new(game_service, event_bus);

        let result = subscriber
            .handle_player_played_move(
                "nonexistent_room",
                "alice-uuid",
                &[Card::new(Rank::Three, Suit::Diamonds)],
            )
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Game not found"));
    }

    #[tokio::test]
    async fn test_handle_player_played_move_wrong_turn() {
        let player_mapping = Arc::new(InMemoryPlayerMappingService::new());
        let game_service = Arc::new(GameService::new(player_mapping.clone()));
        let event_bus = EventBus::new();
        let subscriber = GameEventRoomSubscriber::new(game_service.clone(), event_bus);

        // Create a game with 4 players (Big Two requirement)
        let players = vec![
            "550e8400-e29b-41d4-a716-446655440000".to_string(),
            "550e8400-e29b-41d4-a716-446655440001".to_string(),
            "550e8400-e29b-41d4-a716-446655440002".to_string(),
            "550e8400-e29b-41d4-a716-446655440003".to_string(),
        ];

        // Register players in the mapping service
        for (i, uuid) in players.iter().enumerate() {
            player_mapping
                .register_player(uuid.clone(), format!("Player{}", i + 1))
                .await
                .unwrap();
        }

        game_service
            .create_game("test_room", &players)
            .await
            .unwrap();

        // Get the game to see who should play first
        let game = game_service.get_game("test_room").await.unwrap();
        let current_player = game.current_player_turn();

        // Find a different player (not current)
        let wrong_player = game
            .players()
            .iter()
            .find(|p| p.uuid != current_player)
            .unwrap()
            .uuid
            .clone();

        // Try to play with wrong player
        let result = subscriber
            .handle_player_played_move(
                "test_room",
                &wrong_player,
                &[Card::new(Rank::Six, Suit::Clubs)],
            )
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid player"));
    }
}
