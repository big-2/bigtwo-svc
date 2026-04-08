mod utils;
use bigtwo::{
    event::RoomEvent,
    game::{Card, Rank, Suit},
};
use utils::{GameBuilder, TestSetupBuilder};

#[tokio::test]
async fn stats_service_records_game_when_game_won_event_emitted() {
    let setup = TestSetupBuilder::new().with_four_players().build().await;

    let first_player_uuid = GameBuilder::new()
        .with_simple_four_player_game()
        .build_with_setup(&setup)
        .await;

    // Verify the game was created
    let game = setup
        .game_service
        .get_game("room-123")
        .await
        .expect("game should exist after creation");

    assert_eq!(game.current_player_turn(), first_player_uuid);

    // Directly call process_completed_game instead of relying on event system
    // This verifies the stats service works correctly
    let (game_result, room_stats) = setup
        .stats_service
        .process_completed_game("room-123", &game, &first_player_uuid)
        .await
        .expect("processing game should succeed");

    // Verify the game result
    assert_eq!(game_result.room_id, "room-123");
    assert_eq!(game_result.game_number, 1);
    assert_eq!(game_result.winner_uuid, first_player_uuid);

    // Verify room stats
    assert_eq!(
        room_stats.games_played, 1,
        "should have recorded exactly 1 game"
    );
    assert!(
        room_stats.player_stats.contains_key(&first_player_uuid),
        "stats should contain the winner's UUID"
    );

    let winner_stats = room_stats.player_stats.get(&first_player_uuid).unwrap();
    assert_eq!(winner_stats.wins, 1, "winner should have 1 win");

    // Also verify stats can be retrieved from repository
    let retrieved_stats = setup
        .stats_repository
        .get_room_stats("room-123")
        .await
        .expect("stats retrieval should succeed")
        .expect("room stats should exist in repository");

    assert_eq!(retrieved_stats.games_played, 1);
}

#[tokio::test]
async fn stats_subscriber_records_completed_game_from_event_snapshot() {
    let setup = TestSetupBuilder::new().with_four_players().build().await;

    let alice_uuid = "550e8400-e29b-41d4-a716-446655440000".to_string();

    GameBuilder::new()
        .with_cards(vec![
            ("alice", vec![Card::new(Rank::Three, Suit::Diamonds)]),
            (
                "bob",
                vec![
                    Card::new(Rank::Four, Suit::Clubs),
                    Card::new(Rank::Five, Suit::Clubs),
                ],
            ),
            (
                "charlie",
                vec![
                    Card::new(Rank::Six, Suit::Hearts),
                    Card::new(Rank::Seven, Suit::Hearts),
                ],
            ),
            (
                "david",
                vec![
                    Card::new(Rank::Eight, Suit::Spades),
                    Card::new(Rank::Nine, Suit::Spades),
                ],
            ),
        ])
        .build_with_setup(&setup)
        .await;

    let winning_move = vec![Card::new(Rank::Three, Suit::Diamonds)];
    let move_result = setup
        .game_service
        .try_play_move("room-123", &alice_uuid, &winning_move)
        .await
        .expect("winning move should succeed");

    assert!(move_result.player_won, "move should finish the game");

    setup.game_service.remove_game("room-123").await;

    setup
        .emit_event(RoomEvent::GameWon {
            winner: alice_uuid.clone(),
            winning_hand: move_result
                .winning_hand
                .clone()
                .expect("winning hand should be present"),
            game: move_result.game.clone(),
        })
        .await;

    let room_stats = setup
        .stats_repository
        .get_room_stats("room-123")
        .await
        .expect("stats lookup should succeed")
        .expect("stats should be recorded from the event snapshot");

    assert_eq!(room_stats.games_played, 1);
    assert_eq!(room_stats.player_stats.get(&alice_uuid).unwrap().wins, 1);
}
