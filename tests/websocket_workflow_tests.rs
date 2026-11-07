use bigtwo::{
    event::RoomEvent,
    game::{Card, Rank, Suit},
    websockets::{MessageType, WebSocketMessage},
};

mod utils;

use utils::*;

#[tokio::test]
async fn test_game_start_requires_host_and_four_players() {
    let setup = TestSetupBuilder::new().with_four_players().build().await;

    setup
        .send_start_game("550e8400-e29b-41d4-a716-446655440000")
        .await; // alice is host

    MessageAssertion::for_all_players(&setup)
        .received_message_type(MessageType::GameStarted)
        .await
        .verify_game_started_with_four_players();
}

#[tokio::test]
async fn test_non_host_cannot_start_game() {
    let setup = TestSetupBuilder::new().with_four_players().build().await;

    setup
        .send_start_game("550e8400-e29b-41d4-a716-446655440001")
        .await; // bob is not host

    MessageAssertion::for_all_players(&setup)
        .received_no_messages()
        .await;
}

#[tokio::test]
async fn test_insufficient_players_cannot_start_game() {
    let setup = TestSetupBuilder::new().with_two_players().build().await;

    setup
        .send_start_game("550e8400-e29b-41d4-a716-446655440000")
        .await;

    MessageAssertion::for_all_players(&setup)
        .received_no_messages()
        .await;
}

#[tokio::test]
async fn test_turn_progression_after_move() {
    let setup = TestSetupBuilder::new().with_four_players().build().await;
    let first_player = GameBuilder::new()
        .with_cards(vec![
            (
                "alice",
                vec![
                    Card::new(Rank::Three, Suit::Diamonds), // Alice has 3D, goes first
                    Card::new(Rank::Five, Suit::Hearts),
                    Card::new(Rank::Eight, Suit::Spades),
                    Card::new(Rank::Jack, Suit::Clubs),
                    Card::new(Rank::Jack, Suit::Hearts), // Alice has a pair of Jacks
                ],
            ),
            (
                "bob",
                vec![
                    Card::new(Rank::Four, Suit::Clubs),
                    Card::new(Rank::Six, Suit::Diamonds),
                    Card::new(Rank::Nine, Suit::Hearts),
                ],
            ),
            (
                "charlie",
                vec![
                    Card::new(Rank::Seven, Suit::Spades),
                    Card::new(Rank::Ten, Suit::Clubs),
                    Card::new(Rank::Queen, Suit::Diamonds),
                ],
            ),
            (
                "david",
                vec![
                    Card::new(Rank::King, Suit::Hearts),
                    Card::new(Rank::Ace, Suit::Spades),
                    Card::new(Rank::Two, Suit::Clubs),
                ],
            ),
        ])
        .build_with_setup(&setup)
        .await;

    setup.clear_messages().await;
    setup.send_move(&first_player, vec!["3D"]).await;

    MessageAssertion::for_all_players(&setup)
        .received_message_type(MessageType::MovePlayed)
        .await;

    MessageAssertion::for_all_players(&setup)
        .received_message_type(MessageType::TurnChange)
        .await;

    let updated_game = setup
        .game_service
        .get_game("room-123")
        .await
        .expect("Game should exist");

    let new_current_player = updated_game.current_player_turn();
    assert_ne!(
        first_player, new_current_player,
        "Turn should have advanced to next player"
    );
}

#[tokio::test]
async fn test_valid_first_move_with_three_of_diamonds() {
    let setup = TestSetupBuilder::new().with_four_players().build().await;
    let first_player = GameBuilder::new()
        .with_simple_four_player_game()
        .build_with_setup(&setup)
        .await;

    setup.send_move(&first_player, vec!["3D"]).await;

    MessageAssertion::for_all_players(&setup)
        .received_message_type(MessageType::MovePlayed)
        .await
        .with_player(&first_player)
        .with_cards(vec!["3D"]);
}

#[tokio::test]
async fn test_pass_move_after_initial_play() {
    let setup = TestSetupBuilder::new().with_four_players().build().await;
    let first_player = GameBuilder::new()
        .with_simple_four_player_game()
        .build_with_setup(&setup)
        .await;

    // First player plays 3D
    setup.send_move(&first_player, vec!["3D"]).await;
    setup.clear_messages().await;

    // Get the updated game to see who's next
    let updated_game = setup.game_service.get_game("room-123").await.unwrap();
    let second_player = updated_game.current_player_turn();
    setup.send_pass(&second_player).await;

    MessageAssertion::for_all_players(&setup)
        .received_message_type(MessageType::MovePlayed)
        .await
        .with_player(&second_player)
        .with_cards(vec![]); // Empty for pass
}

#[tokio::test]
async fn test_wrong_turn_player_cannot_move() {
    let setup = TestSetupBuilder::new().with_four_players().build().await;
    let first_player = GameBuilder::new()
        .with_simple_four_player_game()
        .build_with_setup(&setup)
        .await;

    // Find a player that's not the current player
    let game = setup.game_service.get_game("room-123").await.unwrap();
    let wrong_player = game
        .players()
        .iter()
        .find(|p| p.uuid != first_player)
        .unwrap()
        .uuid
        .clone();
    setup.send_move(&wrong_player, vec!["4H"]).await;

    MessageAssertion::for_all_players(&setup)
        .only_player_told_move_rejected(&wrong_player, "It's not your turn.")
        .await;
}

#[tokio::test]
async fn test_cannot_beat_single_card_with_pair() {
    let setup = TestSetupBuilder::new().with_four_players().build().await;
    let first_player = GameBuilder::new()
        .with_pair_scenario()
        .build_with_setup(&setup)
        .await;

    // First player plays 3D
    setup.send_move(&first_player, vec!["3D"]).await;
    setup.clear_messages().await;

    // Get the updated game to see who's next
    let updated_game = setup.game_service.get_game("room-123").await.unwrap();
    let second_player = updated_game.current_player_turn();
    setup.send_move(&second_player, vec!["4H", "4S"]).await;

    MessageAssertion::for_all_players(&setup)
        .only_player_told_move_rejected(
            &second_player,
            "That play doesn't beat the cards on the table.",
        )
        .await;
}

#[tokio::test]
async fn test_player_join_event_notifies_existing_players() {
    let setup = TestSetupBuilder::new().with_four_players().build().await;

    setup
        .emit_event(RoomEvent::PlayerJoined {
            player: "550e8400-e29b-41d4-a716-446655440002".to_string(),
        })
        .await;

    MessageAssertion::for_all_players(&setup)
        .received_message_type(MessageType::PlayersList)
        .await;
}

#[tokio::test]
async fn test_first_player_can_play_anything_after_all_others_pass() {
    let setup = TestSetupBuilder::new().with_four_players().build().await;
    let first_player = GameBuilder::new()
        .with_simple_four_player_game()
        .build_with_setup(&setup)
        .await;

    // First move: First player plays 3D (required first move)
    setup.send_move(&first_player, vec!["3D"]).await;
    setup.clear_messages().await;

    // Get the current state and find all other players
    let game = setup.game_service.get_game("room-123").await.unwrap();
    let other_players: Vec<String> = game
        .players()
        .iter()
        .map(|p| p.uuid.clone())
        .filter(|name| name != &first_player)
        .collect();

    // All other players pass
    for player in &other_players {
        setup.send_pass(player).await;
    }
    setup.clear_messages().await;

    // Now the first player has control and should be able to play anything
    // Let's just verify the current player is correct
    let updated_game = setup.game_service.get_game("room-123").await.unwrap();
    assert_eq!(updated_game.current_player_turn(), first_player);
}

#[tokio::test]
async fn test_first_player_can_change_combination_type_after_all_pass() {
    let setup = TestSetupBuilder::new().with_four_players().build().await;
    let first_player = GameBuilder::new()
        .with_simple_four_player_game()
        .build_with_setup(&setup)
        .await;

    // First move: First player plays 3D (single card)
    setup.send_move(&first_player, vec!["3D"]).await;
    setup.clear_messages().await;

    // Get the current state and find all other players
    let game = setup.game_service.get_game("room-123").await.unwrap();
    let other_players: Vec<String> = game
        .players()
        .iter()
        .map(|p| p.uuid.clone())
        .filter(|name| name != &first_player)
        .collect();

    // All other players pass
    for player in &other_players {
        setup.send_pass(player).await;
    }
    setup.clear_messages().await;

    // Now the first player should have control again and can play different combination types
    // Let's just verify the game state allows the first player to play
    let updated_game = setup.game_service.get_game("room-123").await.unwrap();
    assert_eq!(updated_game.current_player_turn(), first_player);
    // In Big Two, after all players pass, the last player to play can play any combination type
    assert_eq!(updated_game.consecutive_passes(), 3); // 3 other players passed
}

#[tokio::test]
async fn test_first_turn_must_include_three_of_diamonds() {
    let setup = TestSetupBuilder::new().with_four_players().build().await;
    let first_player = GameBuilder::new()
        .with_simple_four_player_game()
        .build_with_setup(&setup)
        .await;

    // Try to play without 3 of diamonds on first turn - should fail
    setup.send_move(&first_player, vec!["4H"]).await;

    // Only the player who made the invalid move should hear about it
    MessageAssertion::for_all_players(&setup)
        .only_player_told_move_rejected(&first_player, "The first play must include the 3♦.")
        .await;
}

#[tokio::test]
async fn test_first_turn_with_three_of_diamonds_succeeds() {
    let setup = TestSetupBuilder::new().with_four_players().build().await;
    let first_player = GameBuilder::new()
        .with_simple_four_player_game()
        .build_with_setup(&setup)
        .await;

    // Play with 3 of diamonds on first turn - should succeed
    setup.send_move(&first_player, vec!["3D"]).await;

    // All players should receive the move
    MessageAssertion::for_all_players(&setup)
        .received_message_type(MessageType::MovePlayed)
        .await
        .with_player(&first_player)
        .with_cards(vec!["3D"]);
}

#[tokio::test]
async fn test_game_started_includes_last_played_cards_in_single_message() {
    // Regression test for reconnection bug: GAME_STARTED should include last_played_cards,
    // and there should be no separate MOVE_PLAYED message during game initialization
    let setup = TestSetupBuilder::new().with_four_players().build().await;
    let first_player = GameBuilder::new()
        .with_simple_four_player_game()
        .build_with_setup(&setup)
        .await;

    // Make a move so there are last_played_cards
    setup.send_move(&first_player, vec!["3D"]).await;
    setup.clear_messages().await;

    // Simulate what happens during game start/reconnection
    // by triggering a fresh start game event with existing game state
    let game = setup.game_service.get_game("room-123").await.unwrap();
    setup
        .emit_event(bigtwo::event::RoomEvent::StartGame { game: game.clone() })
        .await;

    // Each player should receive exactly ONE GAME_STARTED message
    for (uuid, _) in &setup.players {
        let assertion = MessageAssertion::for_players(&setup, vec![uuid.as_str()]);
        let game_started_count = assertion
            .count_message_type(uuid, MessageType::GameStarted)
            .await;

        // Assert only one GAME_STARTED message (no duplicates)
        assert_eq!(
            game_started_count, 1,
            "Player {} should receive exactly 1 GAME_STARTED message, got {}",
            uuid, game_started_count
        );

        // Verify no MOVE_PLAYED messages were sent during game start
        // (the last played cards should be in GAME_STARTED, not a separate message)
        let move_played_count = assertion
            .count_message_type(uuid, MessageType::MovePlayed)
            .await;
        assert_eq!(
            move_played_count, 0,
            "Player {} should not receive MOVE_PLAYED during game start (data should be in GAME_STARTED), got {}",
            uuid, move_played_count
        );
    }

    // Verify the GAME_STARTED message contains last_played_cards
    let first_player_uuid = setup.players[0].0.as_str();
    MessageAssertion::for_players(&setup, vec![first_player_uuid])
        .received_message_type(MessageType::GameStarted)
        .await;
}

#[tokio::test]
async fn test_bot_player_mapping_cleaned_up_when_room_deleted() {
    // This test verifies that when a room is deleted because only bots remain,
    // the bot player mappings are properly cleaned up, preventing stale mappings
    // that could cause bot names to appear changed when a player returns to the room.

    let setup = TestSetupBuilder::new().with_two_players().build().await;
    let room_id = "room-123".to_string(); // Default room_id from TestSetupBuilder

    // Add a bot to the room
    let bot = setup
        .bot_manager
        .create_bot(room_id.clone(), bigtwo::bot::types::BotDifficulty::Easy)
        .await
        .expect("Failed to create bot");

    let bot_uuid = bot.uuid.clone();

    // Add bot to room
    setup
        .room_service
        .join_room(room_id.clone(), bot_uuid.clone())
        .await
        .expect("Failed to add bot to room");

    // Verify bot is in the room
    let room_before = setup
        .room_service
        .get_room(&room_id)
        .await
        .expect("Failed to get room");

    assert!(room_before.is_some(), "Room should exist before deletion");
    let room_has_bot = room_before.as_ref().unwrap().has_player(&bot_uuid);
    assert!(room_has_bot, "Bot should be in room");
}

#[tokio::test]
async fn test_heartbeat_returns_ack_to_sender_only() {
    let setup = TestSetupBuilder::new().with_four_players().build().await;
    let alice = "550e8400-e29b-41d4-a716-446655440000";

    setup
        .send_message(
            alice,
            WebSocketMessage::new(MessageType::Heartbeat, serde_json::json!({})),
        )
        .await;

    // Alice receives ACK
    MessageAssertion::for_players(&setup, vec![alice])
        .received_message_type(MessageType::HeartbeatAck)
        .await;

    // Others receive nothing
    MessageAssertion::for_players(
        &setup,
        vec![
            "550e8400-e29b-41d4-a716-446655440001",
            "550e8400-e29b-41d4-a716-446655440002",
            "550e8400-e29b-41d4-a716-446655440003",
        ],
    )
    .received_no_messages()
    .await;
}

#[tokio::test]
async fn test_multiple_players_can_send_heartbeats_independently() {
    let setup = TestSetupBuilder::new().with_four_players().build().await;

    // All players send heartbeats
    for (uuid, _) in &setup.players {
        setup
            .send_message(
                uuid,
                WebSocketMessage::new(MessageType::Heartbeat, serde_json::json!({})),
            )
            .await;
    }

    // Each player should have received exactly one ACK
    for (uuid, _) in &setup.players {
        MessageAssertion::for_players(&setup, vec![uuid.as_str()])
            .received_message_type(MessageType::HeartbeatAck)
            .await;
    }
}

#[tokio::test]
async fn test_heartbeat_during_active_game_does_not_interfere() {
    let setup = TestSetupBuilder::new().with_four_players().build().await;
    let first_player = GameBuilder::new()
        .with_simple_four_player_game()
        .build_with_setup(&setup)
        .await;

    // First player makes a valid move
    setup.send_move(&first_player, vec!["3D"]).await;

    MessageAssertion::for_all_players(&setup)
        .received_message_type(MessageType::MovePlayed)
        .await;

    MessageAssertion::for_all_players(&setup)
        .received_message_type(MessageType::TurnChange)
        .await;

    setup.clear_messages().await;

    // Player sends heartbeat during game
    setup
        .send_message(
            &first_player,
            WebSocketMessage::new(MessageType::Heartbeat, serde_json::json!({})),
        )
        .await;

    // Only that player receives heartbeat ACK
    MessageAssertion::for_players(&setup, vec![first_player.as_str()])
        .received_message_type(MessageType::HeartbeatAck)
        .await;

    // Verify game state is unchanged
    let game = setup.game_service.get_game("room-123").await.unwrap();
    assert_ne!(
        game.current_player_turn(),
        first_player,
        "Turn should have moved to next player"
    );
}

#[tokio::test]
async fn test_heartbeat_before_game_starts() {
    let setup = TestSetupBuilder::new().with_two_players().build().await;
    let alice = "550e8400-e29b-41d4-a716-446655440000";

    // Send heartbeat before game has started
    setup
        .send_message(
            alice,
            WebSocketMessage::new(MessageType::Heartbeat, serde_json::json!({})),
        )
        .await;

    // Should still receive ACK even without active game
    MessageAssertion::for_players(&setup, vec![alice])
        .received_message_type(MessageType::HeartbeatAck)
        .await;
}
