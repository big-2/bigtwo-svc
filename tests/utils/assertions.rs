//! Test assertion helpers - fluent API for verifying test expectations
#![allow(dead_code)] // Test utilities may not all be used in every test

use serde_json;

use bigtwo::websockets::{MessageType, WebSocketMessage};

use super::setup::TestSetup;

// ============================================================================
// Assertion Helpers
// ============================================================================

pub struct MessageAssertion<'a> {
    setup: &'a TestSetup,
    players: Vec<&'a str>, // uuid
}

impl<'a> MessageAssertion<'a> {
    /// Create an assertion for all players in the setup
    pub fn for_all_players(setup: &'a TestSetup) -> Self {
        let players = setup.players.iter().map(|s| s.0.as_str()).collect();
        Self { setup, players }
    }

    /// Create an assertion for specific players
    pub fn for_players(setup: &'a TestSetup, players: Vec<&'a str>) -> Self {
        Self { setup, players }
    }

    /// Assert that players received a specific message type (consumes the message from queue)
    pub async fn received_message_type(self, expected_type: MessageType) -> MessageContent {
        let mut messages = vec![];

        for player in &self.players {
            let message = self
                .setup
                .mock_conn_manager
                .consume_message_for(player)
                .await;
            assert!(
                message.is_some(),
                "{} should have received a message",
                player
            );

            let msg: WebSocketMessage = serde_json::from_str(&message.unwrap()).unwrap();
            assert_eq!(
                msg.message_type, expected_type,
                "{} received wrong message type",
                player
            );
            messages.push(msg);
        }

        // For GameStarted messages, players get different cards, so we only check the first player
        // For other message types, verify all messages have the same payload
        if messages.len() > 1 && expected_type != MessageType::GameStarted {
            let first_payload = &messages[0].payload;
            for (i, msg) in messages.iter().enumerate().skip(1) {
                assert_eq!(
                    &msg.payload, first_payload,
                    "Player {} payload differs from player {}",
                    self.players[i], self.players[0]
                );
            }
        }

        MessageContent {
            payload: messages[0].payload.clone(),
        }
    }

    /// Assert that players received no messages
    pub async fn received_no_messages(self) {
        for player in &self.players {
            let messages = self.setup.mock_conn_manager.get_messages_for(player).await;
            assert!(
                messages.is_empty(),
                "{} should not have received any messages",
                player
            );
        }
    }

    /// Assert that only the rejected player was told why, and everyone else heard nothing
    pub async fn only_player_told_move_rejected(self, rejected_player: &str, reason: &str) {
        for player in &self.players {
            let messages = self.setup.mock_conn_manager.get_messages_for(player).await;
            if *player != rejected_player {
                assert!(
                    messages.is_empty(),
                    "{} should not hear about another player's rejected move",
                    player
                );
                continue;
            }

            assert_eq!(messages.len(), 1, "{} should get one error", player);
            let message: WebSocketMessage = serde_json::from_str(&messages[0]).unwrap();
            assert_eq!(message.message_type, MessageType::Error);
            assert_eq!(message.payload["message"], reason);
            assert_eq!(message.payload["error_type"], "move_rejected");
        }
    }

    /// Count how many messages of a specific type a player received (non-consuming)
    pub async fn count_message_type(&self, player: &str, msg_type: MessageType) -> usize {
        let messages = self.setup.mock_conn_manager.get_messages_for(player).await;
        messages
            .iter()
            .filter_map(|msg_str| serde_json::from_str::<WebSocketMessage>(msg_str).ok())
            .filter(|msg| msg.message_type == msg_type)
            .count()
    }

    /// Assert that players received a sequence of message types in order
    pub async fn received_message_sequence(
        self,
        expected_types: Vec<MessageType>,
    ) -> Vec<MessageContent> {
        let mut result_messages = vec![];

        for player in &self.players {
            let player_messages = self.setup.mock_conn_manager.get_messages_for(player).await;
            assert!(
                player_messages.len() >= expected_types.len(),
                "{} should have received {} messages, but only got {}",
                player,
                expected_types.len(),
                player_messages.len()
            );

            // Check each expected message type in order
            for (i, expected_type) in expected_types.iter().enumerate() {
                let msg: WebSocketMessage = serde_json::from_str(&player_messages[i])
                    .unwrap_or_else(|e| {
                        panic!("Failed to parse message {} for {}: {}", i, player, e)
                    });

                assert_eq!(
                    msg.message_type, *expected_type,
                    "{} message {} has wrong type: expected {:?}, got {:?}",
                    player, i, expected_type, msg.message_type
                );

                // Only collect messages from the first player to avoid duplicates
                if player == &self.players[0] {
                    result_messages.push(MessageContent {
                        payload: msg.payload,
                    });
                }
            }
        }

        result_messages
    }
}

// ============================================================================
// Message Content Assertions
// ============================================================================

pub struct MessageContent {
    payload: serde_json::Value,
}

impl MessageContent {
    /// Assert the message has a specific sender
    pub fn with_sender(self, expected_sender: &str) -> Self {
        assert_eq!(self.payload["sender"], expected_sender);
        self
    }

    /// Assert the message has specific content
    pub fn with_content(self, expected_content: &str) -> Self {
        assert_eq!(self.payload["content"], expected_content);
        self
    }

    /// Assert the message has a specific player
    pub fn with_player(self, expected_player: &str) -> Self {
        assert_eq!(self.payload["player"], expected_player);
        self
    }

    /// Assert the message has specific cards
    pub fn with_cards(self, expected_cards: Vec<&str>) -> Self {
        let actual_cards: Vec<String> =
            serde_json::from_value(self.payload["cards"].clone()).unwrap();
        let expected_cards: Vec<String> =
            expected_cards.into_iter().map(|s| s.to_string()).collect();
        assert_eq!(actual_cards, expected_cards);
        self
    }

    /// Assert the message has a specific current turn
    pub fn with_current_turn(self, expected_player: &str) -> Self {
        assert_eq!(self.payload["current_turn"], expected_player);
        self
    }

    /// Verify a game started message with four players
    pub fn verify_game_started_with_four_players(self) -> Self {
        // Verify player count and card distribution
        let player_list: Vec<String> =
            serde_json::from_value(self.payload["player_list"].clone()).unwrap();
        assert_eq!(player_list.len(), 4);

        let cards: Vec<String> = serde_json::from_value(self.payload["cards"].clone()).unwrap();
        assert_eq!(cards.len(), 13);

        // Verify current turn (uuid) is one of the players in the list
        let current_turn = self.payload["current_turn"].as_str().unwrap();
        assert!(player_list.contains(&current_turn.to_string()));

        self
    }
}
