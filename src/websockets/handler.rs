use async_trait::async_trait;
use axum::{
    extract::{Path, State, WebSocketUpgrade},
    http::{header::SEC_WEBSOCKET_PROTOCOL, HeaderMap, HeaderValue},
    response::Response,
};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::event::EventBus;
use crate::event::RoomEvent;
use crate::game::Card;
// Use Game's public methods to access last non-pass state instead of private types
use crate::shared::{AppError, AppState};
use crate::websockets::messages::{MessageType, WebSocketMessage};

use super::socket::{Connection, MessageHandler};

fn outbound_channel_capacity() -> usize {
    std::env::var("WEBSOCKET_OUTBOUND_CHANNEL_CAPACITY")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(128)
}

/// Message handler for receiving WebSocket messages from the client
pub struct WebsocketReceiveHandler {
    event_bus: EventBus,
}

impl WebsocketReceiveHandler {
    pub fn new(event_bus: EventBus) -> Self {
        Self { event_bus }
    }
}

#[async_trait]
impl MessageHandler for WebsocketReceiveHandler {
    async fn handle_message(&self, username: &str, room_id: &str, message: String) {
        debug!(
            username = %username,
            room_id = %room_id,
            "Received message"
        );

        // Parse message and emit appropriate event
        match serde_json::from_str::<WebSocketMessage>(&message) {
            Ok(ws_message) => match ws_message.message_type {
                MessageType::Chat => {
                    if let Some(content) =
                        ws_message.payload.get("content").and_then(|v| v.as_str())
                    {
                        self.event_bus
                            .emit_to_room(
                                room_id,
                                RoomEvent::ChatMessage {
                                    sender: username.to_string(),
                                    content: content.to_string(),
                                },
                            )
                            .await;
                    }
                }
                MessageType::Leave => {
                    self.event_bus
                        .emit_to_room(
                            room_id,
                            RoomEvent::PlayerLeaveRequested {
                                player: username.to_string(),
                            },
                        )
                        .await;
                }
                MessageType::StartGame => {
                    self.event_bus
                        .emit_to_room(
                            room_id,
                            RoomEvent::TryStartGame {
                                host: username.to_string(),
                            },
                        )
                        .await;
                }
                MessageType::Ready => {
                    if let Some(is_ready) =
                        ws_message.payload.get("is_ready").and_then(|v| v.as_bool())
                    {
                        self.event_bus
                            .emit_to_room(
                                room_id,
                                RoomEvent::PlayerReadyToggled {
                                    player: username.to_string(),
                                    is_ready,
                                },
                            )
                            .await;
                    }
                }
                MessageType::Move => {
                    if let Some(cards_array) =
                        ws_message.payload.get("cards").and_then(|v| v.as_array())
                    {
                        let card_strings: Vec<String> = cards_array
                            .iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect();

                        // Convert card strings to Card objects early
                        let cards: Result<Vec<Card>, _> = card_strings
                            .iter()
                            .map(|card_str| Card::from_string(card_str))
                            .collect();

                        match cards {
                            Ok(cards) => {
                                self.event_bus
                                    .emit_to_room(
                                        room_id,
                                        RoomEvent::TryPlayMove {
                                            player: username.to_string(),
                                            cards,
                                        },
                                    )
                                    .await;
                            }
                            Err(e) => {
                                warn!(
                                    username = %username,
                                    room_id = %room_id,
                                    error = %e,
                                    "Invalid card format in move"
                                );
                                // TODO: Send error message back to client
                            }
                        }
                    }
                }
                MessageType::Heartbeat => {
                    // Client sent heartbeat to check connection health
                    // Emit event to send HEARTBEAT_ACK back to this specific player
                    debug!(
                        username = %username,
                        room_id = %room_id,
                        "Received heartbeat from client"
                    );
                    self.event_bus
                        .emit_to_room(
                            room_id,
                            RoomEvent::HeartbeatReceived {
                                player: username.to_string(),
                            },
                        )
                        .await;
                }
                _ => {
                    debug!(
                        message_type = ?ws_message.message_type,
                        "Unhandled message type"
                    );
                }
            },
            Err(e) => {
                warn!(
                    username = %username,
                    room_id = %room_id,
                    error = %e,
                    "Failed to parse WebSocket message"
                );
            }
        }
    }
}

/// WebSocket endpoint that handles authentication via Sec-WebSocket-Protocol header
/// GET /ws/{room_id} with JWT token in Sec-WebSocket-Protocol header
pub async fn websocket_handler(
    ws: WebSocketUpgrade,
    Path(room_id): Path<String>,
    headers: HeaderMap,
    State(app_state): State<AppState>,
) -> Result<Response, AppError> {
    debug!(
        room_id = %room_id,
        "WebSocket connection requested"
    );

    // Extract JWT from Sec-WebSocket-Protocol header
    let jwt_token = headers
        .get(SEC_WEBSOCKET_PROTOCOL)
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| {
            warn!("Missing or invalid Sec-WebSocket-Protocol header");
            AppError::Unauthorized("Missing authentication token".to_string())
        })?
        .to_string();

    // Validate JWT token and get username from claims
    let claims = app_state
        .session_service
        .validate_session(&jwt_token)
        .await?;
    let username = claims.username.clone();

    debug!(
        room_id = %room_id,
        username = %username,
        "WebSocket authentication successful"
    );

    // Verify room exists using room service
    let room_option = app_state.room_service.get_room(&room_id).await?;
    if room_option.is_none() {
        warn!(
            room_id = %room_id,
            "Room not found, rejecting WebSocket connection"
        );
        return Err(AppError::NotFound("Room not found".to_string()));
    }

    debug!(
        room_id = %room_id,
        username = %username,
        "Room verified, establishing WebSocket connection"
    );
    let protocol_header_value = HeaderValue::from_str(&jwt_token).map_err(|e| {
        warn!(error = %e, "Invalid Sec-WebSocket-Protocol header value");
        AppError::Unauthorized("Invalid authentication token".to_string())
    })?;

    let mut response = ws.on_upgrade(move |socket| {
        handle_websocket_connection(socket, room_id, username, claims.session_id, app_state)
    });

    response
        .headers_mut()
        .insert(SEC_WEBSOCKET_PROTOCOL, protocol_header_value);

    Ok(response)
}

/// Handle the upgraded WebSocket connection
async fn handle_websocket_connection(
    socket: axum::extract::ws::WebSocket,
    room_id: String,
    username: String,
    session_id: String,
    app_state: AppState,
) {
    debug!(
        room_id = %room_id,
        username = %username,
        "WebSocket connection established"
    );

    // Create the outbound channel (app -> client)
    let (outbound_sender, outbound_receiver) = mpsc::channel::<String>(outbound_channel_capacity());

    // Register connection with the connection manager
    // Resolve stable player UUID from session id for connection identity
    let player_uuid = match app_state
        .session_service
        .get_player_uuid_by_session(&session_id)
        .await
    {
        Ok(opt) => match opt {
            Some(uuid) => uuid,
            None => {
                warn!(session_id = %session_id, "No player UUID for session");
                return;
            }
        },
        Err(_e) => {
            warn!(session_id = %session_id, "Failed to get player UUID for session");
            return;
        }
    };

    app_state
        .connection_manager
        .add_connection(player_uuid.clone(), outbound_sender.clone())
        .await;

    // Mark player as connected in the room (synchronously before sending initial state)
    // This ensures the initial PLAYERS_LIST we send below has correct connected status
    if let Err(e) = app_state
        .room_service
        .mark_player_connected(&room_id, &player_uuid)
        .await
    {
        warn!(
            room_id = %room_id,
            player_uuid = %player_uuid,
            error = %e,
            "Failed to mark player as connected"
        );
    }

    // Send initial room state to the newly connected player
    // This happens immediately so the connecting player gets state without delay
    if let Ok(Some(room)) = app_state.room_service.get_room(&room_id).await {
        let mut mapping: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for uuid in room.get_player_uuids() {
            // Use session_service which has database fallback for persistent usernames
            if let Some(name) = app_state.session_service.get_playername_by_uuid(uuid).await {
                mapping.insert(uuid.clone(), name);
            } else {
                warn!(
                    room_id = %room_id,
                    uuid = %uuid,
                    "Player not found in mapping or database"
                );
            }
        }

        let bot_uuids = app_state.bot_manager.get_bot_uuids_in_room(&room_id).await;
        let bot_difficulties = app_state
            .bot_manager
            .get_bot_difficulties_in_room(&room_id)
            .await;

        let initial_message = crate::websockets::messages::WebSocketMessage::players_list(
            room.get_player_uuids().clone(),
            mapping,
            bot_uuids,
            bot_difficulties,
            room.get_ready_players().clone(),
            room.host_uuid.clone(),
            room.get_connected_players().clone(),
        );
        if let Ok(message_json) = serde_json::to_string(&initial_message) {
            let _ = outbound_sender.try_send(message_json);
            debug!(
                room_id = %room_id,
                username = %username,
                "Sent initial PLAYERS_LIST to newly connected player"
            );
        }
    }

    // Emit PlayerConnected event to broadcast updated state to all players (including this one)
    // This ensures all players in the room see the updated connected status
    app_state
        .event_bus
        .emit_to_room(
            &room_id,
            crate::event::RoomEvent::PlayerConnected {
                player: player_uuid.clone(),
            },
        )
        .await;

    // If a game is active, send game hydration data to the reconnecting player.
    // Finished games are intentionally skipped so reconnects do not re-enter
    // a match that has already ended.
    if let Some(game) = app_state.game_service.get_game(&room_id).await {
        let hydration_messages = build_reconnect_hydration_messages(&game, &player_uuid);
        if hydration_messages.is_empty() && !game.is_finished() {
            warn!(
                room_id = %room_id,
                username = %username,
                player_uuid = %player_uuid,
                "Reconnecting player UUID not present in active game"
            );
        } else {
            for hydration_message in hydration_messages {
                if let Ok(message_json) = serde_json::to_string(&hydration_message) {
                    let _ = outbound_sender.send(message_json).await;
                    debug!(
                        room_id = %room_id,
                        username = %username,
                        message_type = ?hydration_message.message_type,
                        "Sent reconnect hydration message"
                    );
                }
            }
        }
    }

    // Wrap the axum WebSocket in our simple interface
    let socket_wrapper = Box::new(socket);

    // Create message handler (using the new GameRoomMessageHandler)
    let message_handler = Arc::new(WebsocketReceiveHandler::new(app_state.event_bus.clone()));

    // Create and run the connection
    let connection = Connection::new(
        player_uuid.clone(),
        room_id.clone(),
        socket_wrapper,
        outbound_receiver,
        message_handler,
    );

    // Run the connection until disconnect
    match connection.run().await {
        Ok(()) => {
            debug!(
                room_id = %room_id,
                username = %username,
                "WebSocket connection closed cleanly"
            );
        }
        Err(e) => {
            warn!(
                room_id = %room_id,
                username = %username,
                error = ?e,
                "WebSocket connection error"
            );
        }
    }

    // Cleanup: remove from connection manager and emit disconnect event
    app_state
        .connection_manager
        .remove_connection(&player_uuid)
        .await;

    // Emit disconnect event - let the event system handle the rest
    app_state
        .event_bus
        .emit_to_room(
            &room_id,
            crate::event::RoomEvent::PlayerDisconnected {
                player: player_uuid.clone(),
            },
        )
        .await;

    if let Err(e) = app_state
        .room_service
        .mark_player_disconnected(&room_id, &player_uuid)
        .await
    {
        warn!(
            room_id = %room_id,
            player_uuid = %player_uuid,
            error = %e,
            "Failed to mark player as disconnected"
        );
    }

    debug!(
        room_id = %room_id,
        username = %username,
        "WebSocket disconnect event emitted"
    );
}

fn build_reconnect_hydration_messages(
    game: &crate::game::Game,
    player_uuid: &str,
) -> Vec<WebSocketMessage> {
    if game.is_finished() {
        return Vec::new();
    }

    let Some(player) = game.players().iter().find(|p| p.uuid == player_uuid) else {
        return Vec::new();
    };

    let card_counts: std::collections::HashMap<String, usize> = game
        .players()
        .iter()
        .map(|p| (p.uuid.clone(), p.cards.len()))
        .collect();
    let last_plays_by_player: std::collections::HashMap<String, Vec<String>> = game
        .last_plays_by_player()
        .iter()
        .map(|(uuid, cards)| (uuid.clone(), cards.iter().map(|c| c.to_string()).collect()))
        .collect();

    let mut messages = vec![WebSocketMessage::game_started(
        game.current_player_turn().clone(),
        player.cards.iter().map(|card| card.to_string()).collect(),
        game.players().iter().map(|p| p.uuid.clone()).collect(),
        card_counts,
        last_plays_by_player,
    )];

    let last_cards = game
        .last_non_pass_cards()
        .iter()
        .map(|c| c.to_string())
        .collect::<Vec<_>>();
    if !last_cards.is_empty() {
        if let Some(last_player_uuid) = game.last_non_pass_player_uuid() {
            let remaining_cards = game
                .players()
                .iter()
                .find(|p| p.uuid == last_player_uuid)
                .map(|p| p.cards.len())
                .unwrap_or(0);

            messages.push(WebSocketMessage::move_played(
                last_player_uuid,
                last_cards,
                remaining_cards,
            ));
        }
    }

    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        event::RoomEvent,
        game::{Game, Player, Rank, Suit},
    };

    #[tokio::test]
    async fn test_receive_handler_emits_events_for_chat_leave_start_move() {
        let bus = EventBus::new();
        let handler = WebsocketReceiveHandler::new(bus.clone());

        // Subscribe to room
        let mut rx = bus.subscribe_to_room("r1").await;

        // Chat
        let chat = serde_json::json!({
            "type": "CHAT",
            "payload": {"content": "hi"},
            "meta": null
        });
        handler
            .handle_message("alice", "r1", chat.to_string())
            .await;

        // Leave
        let leave = serde_json::json!({"type":"LEAVE","payload":{},"meta":null});
        handler
            .handle_message("alice", "r1", leave.to_string())
            .await;

        // Start
        let start = serde_json::json!({"type":"START_GAME","payload":{},"meta":null});
        handler
            .handle_message("host", "r1", start.to_string())
            .await;

        // Move with valid cards
        let mv = serde_json::json!({
            "type":"MOVE",
            "payload": {"cards":["3D","4D"]},
            "meta": null
        });
        handler.handle_message("alice", "r1", mv.to_string()).await;

        // Drain 4 events in any order
        let mut seen = vec![];
        for _ in 0..4 {
            if let Ok(ev) = rx.try_recv() {
                seen.push(ev);
            } else {
                // fallback to await if not yet ready
                seen.push(rx.recv().await.unwrap());
            }
        }

        // Check types observed
        assert!(seen
            .iter()
            .any(|e| matches!(e, RoomEvent::ChatMessage { .. })));
        assert!(seen
            .iter()
            .any(|e| matches!(e, RoomEvent::PlayerLeaveRequested { .. })));
        assert!(seen
            .iter()
            .any(|e| matches!(e, RoomEvent::TryStartGame { .. })));
        assert!(seen
            .iter()
            .any(|e| matches!(e, RoomEvent::TryPlayMove { .. })));
    }

    #[test]
    fn reconnect_hydration_skips_finished_games() {
        let game = Game::new(
            "room-1".to_string(),
            vec![
                Player {
                    name: "Alice".to_string(),
                    uuid: "alice".to_string(),
                    cards: vec![],
                },
                Player {
                    name: "Bob".to_string(),
                    uuid: "bob".to_string(),
                    cards: vec![Card::new(Rank::Four, Suit::Clubs)],
                },
            ],
            0,
            0,
            vec![],
            std::collections::HashMap::new(),
        );

        assert!(game.is_finished());
        assert!(build_reconnect_hydration_messages(&game, "alice").is_empty());
    }

    #[test]
    fn reconnect_hydration_includes_active_game_snapshot() {
        let game = Game::new(
            "room-1".to_string(),
            vec![
                Player {
                    name: "Alice".to_string(),
                    uuid: "alice".to_string(),
                    cards: vec![Card::new(Rank::Three, Suit::Diamonds)],
                },
                Player {
                    name: "Bob".to_string(),
                    uuid: "bob".to_string(),
                    cards: vec![Card::new(Rank::Four, Suit::Clubs)],
                },
            ],
            0,
            0,
            vec![],
            std::collections::HashMap::new(),
        );

        let messages = build_reconnect_hydration_messages(&game, "alice");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].message_type, MessageType::GameStarted);
    }
}
