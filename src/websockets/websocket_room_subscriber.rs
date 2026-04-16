use async_trait::async_trait;
use std::sync::Arc;
use tracing::debug;

use crate::{
    event::{RoomEvent, RoomEventError, RoomEventHandler},
    game::GameService,
    room::service::RoomService,
    stats::service::StatsService,
    user::PlayerMappingService,
    websockets::connection_manager::ConnectionManager,
};

use super::event_handlers::{
    ChatEventHandlers, ConnectionEventHandlers, GameEventHandlers, RoomEventHandlers,
};

/// WebSocket-specific room event handler
///
/// Handles room events by delegating to specialized event handlers:
/// - RoomEventHandlers: PlayerJoined, PlayerLeft, HostChanged
/// - ChatEventHandlers: ChatMessage
/// - GameEventHandlers: StartGame, MovePlayed, TurnChanged, GameWon, GameReset
/// - ConnectionEventHandlers: PlayerDisconnected, leave requests
pub struct WebSocketRoomSubscriber {
    room_handlers: RoomEventHandlers,
    chat_handlers: ChatEventHandlers,
    game_handlers: GameEventHandlers,
    connection_handlers: ConnectionEventHandlers,
}

#[async_trait]
impl RoomEventHandler for WebSocketRoomSubscriber {
    async fn handle_room_event(
        &self,
        room_id: &str,
        event: RoomEvent,
    ) -> Result<(), RoomEventError> {
        debug!(
            room_id = %room_id,
            event = ?event,
            "Handling room event for WebSocket connections"
        );

        match event {
            RoomEvent::PlayerJoined { player: _ } => {
                self.room_handlers.handle_player_joined(room_id).await
            }
            RoomEvent::PlayerLeft { player } => {
                self.room_handlers
                    .handle_player_left(room_id, &player)
                    .await
            }
            RoomEvent::HostChanged { old_host, new_host } => {
                self.room_handlers
                    .handle_host_changed(room_id, &old_host, &new_host)
                    .await
            }
            RoomEvent::ChatMessage { sender, content } => {
                self.chat_handlers
                    .handle_chat_message(room_id, &sender, &content)
                    .await
            }
            RoomEvent::PlayerLeaveRequested { player } => {
                self.connection_handlers
                    .handle_leave_request(room_id, &player)
                    .await
            }
            RoomEvent::PlayerConnected { player } => {
                self.connection_handlers
                    .handle_connect(room_id, &player)
                    .await
            }
            RoomEvent::PlayerDisconnected { player } => {
                self.connection_handlers
                    .handle_disconnect(room_id, &player)
                    .await
            }
            RoomEvent::StartGame { game } => {
                self.game_handlers.handle_start_game(room_id, game).await
            }
            RoomEvent::TryStartGame { host } => {
                self.game_handlers
                    .handle_try_start_game(room_id, &host)
                    .await
            }
            RoomEvent::MovePlayed {
                player,
                cards,
                game,
            } => {
                self.game_handlers
                    .handle_move_played(room_id, &player, &cards, game)
                    .await
            }
            RoomEvent::TurnChanged { player } => {
                self.game_handlers
                    .handle_turn_changed(room_id, &player)
                    .await
            }
            RoomEvent::GameWon {
                winner,
                winning_hand,
                game,
            } => {
                self.game_handlers
                    .handle_game_won(room_id, &winner, &winning_hand, game)
                    .await
            }
            RoomEvent::BotAdded { bot_uuid, bot_name } => {
                self.room_handlers
                    .handle_bot_added(room_id, &bot_uuid, &bot_name)
                    .await
            }
            RoomEvent::BotRemoved { bot_uuid } => {
                self.room_handlers
                    .handle_bot_removed(room_id, &bot_uuid)
                    .await
            }
            RoomEvent::PlayerReadyToggled { player, is_ready } => {
                self.room_handlers
                    .handle_player_ready_toggled(room_id, &player, is_ready)
                    .await
            }
            RoomEvent::StatsUpdated { room_stats } => {
                // Broadcast stats update to all connected players
                let room = match self.room_handlers.room_service.get_room(room_id).await {
                    Ok(Some(room)) => room,
                    Ok(None) => {
                        debug!(room_id = %room_id, "Room not found for stats broadcast");
                        return Ok(());
                    }
                    Err(e) => {
                        return Err(RoomEventError::HandlerError(format!(
                            "Failed to fetch room: {}",
                            e
                        )));
                    }
                };

                let stats_message = super::messages::WebSocketMessage::stats_updated(room_stats);
                super::event_handlers::shared::MessageBroadcaster::broadcast_to_players(
                    &self.room_handlers.connection_manager,
                    room.get_player_uuids(),
                    &stats_message,
                )
                .await?;

                debug!(
                    room_id = %room_id,
                    players_notified = room.get_player_uuids().len(),
                    "Stats update notification sent to all players"
                );

                Ok(())
            }
            RoomEvent::HeartbeatReceived { player } => {
                // Client sent heartbeat, respond with HEARTBEAT_ACK to confirm connection is alive
                let heartbeat_ack = super::messages::WebSocketMessage::heartbeat_ack();

                if let Ok(message_json) = serde_json::to_string(&heartbeat_ack) {
                    self.room_handlers
                        .connection_manager
                        .send_to_player(&player, &message_json)
                        .await;

                    debug!(
                        room_id = %room_id,
                        player = %player,
                        "Sent HEARTBEAT_ACK to player"
                    );
                }

                Ok(())
            }
            _ => {
                debug!(
                    room_id = %room_id,
                    event = ?event,
                    "Unhandled event type in WebSocketRoomSubscriber"
                );
                Ok(())
            }
        }
    }

    fn handler_name(&self) -> &'static str {
        "WebSocketRoomSubscriber"
    }
}

impl WebSocketRoomSubscriber {
    pub fn new(
        room_service: Arc<RoomService>,
        connection_manager: Arc<dyn ConnectionManager>,
        game_service: Arc<GameService>,
        player_mapping: Arc<dyn PlayerMappingService>,
        event_bus: crate::event::EventBus,
        bot_manager: Arc<crate::bot::BotManager>,
        stats_service: Arc<StatsService>,
    ) -> Self {
        let room_handlers = RoomEventHandlers::new(
            Arc::clone(&room_service),
            Arc::clone(&connection_manager),
            Arc::clone(&player_mapping),
            Arc::clone(&bot_manager),
        );

        let chat_handlers =
            ChatEventHandlers::new(Arc::clone(&room_service), Arc::clone(&connection_manager));

        let game_handlers = GameEventHandlers::new(
            Arc::clone(&room_service),
            Arc::clone(&connection_manager),
            Arc::clone(&game_service),
            event_bus.clone(),
            Arc::clone(&bot_manager),
        );

        let connection_handlers = ConnectionEventHandlers::new(
            Arc::clone(&room_service),
            Arc::clone(&connection_manager),
            Arc::clone(&player_mapping),
            event_bus.clone(),
            bot_manager,
            Arc::clone(&game_service),
            stats_service,
        );

        Self {
            room_handlers,
            chat_handlers,
            game_handlers,
            connection_handlers,
        }
    }
}
