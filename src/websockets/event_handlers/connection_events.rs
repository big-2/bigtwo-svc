use std::sync::Arc;
use tracing::debug;

use crate::websockets::event_handlers::shared::{MessageBroadcaster, PlayerMappingUtils};
use crate::{
    bot::BotManager,
    event::{EventBus, RoomEvent, RoomEventError},
    game::GameService,
    room::{repository::LeaveRoomResult, service::RoomService},
    stats::service::StatsService,
    user::PlayerMappingService,
    websockets::{connection_manager::ConnectionManager, messages::WebSocketMessage},
};

pub struct ConnectionEventHandlers {
    room_service: Arc<RoomService>,
    #[allow(dead_code)] // Reserved for future connection management features
    connection_manager: Arc<dyn ConnectionManager>,
    player_mapping: Arc<dyn PlayerMappingService>,
    event_bus: EventBus,
    bot_manager: Arc<BotManager>,
    game_service: Arc<GameService>,
    stats_service: Arc<StatsService>,
}

impl ConnectionEventHandlers {
    pub fn new(
        room_service: Arc<RoomService>,
        connection_manager: Arc<dyn ConnectionManager>,
        player_mapping: Arc<dyn PlayerMappingService>,
        event_bus: EventBus,
        bot_manager: Arc<BotManager>,
        game_service: Arc<GameService>,
        stats_service: Arc<StatsService>,
    ) -> Self {
        Self {
            room_service,
            connection_manager,
            player_mapping,
            event_bus,
            bot_manager,
            game_service,
            stats_service,
        }
    }

    pub async fn handle_leave_request(
        &self,
        room_id: &str,
        player_uuid: &str,
    ) -> Result<(), RoomEventError> {
        debug!(
            room_id = %room_id,
            player_uuid = %player_uuid,
            "Processing leave request"
        );

        let room_before = self
            .room_service
            .get_room(room_id)
            .await
            .map_err(|e| RoomEventError::HandlerError(format!("Failed to get room: {}", e)))?;

        let was_host = room_before
            .as_ref()
            .map(|room| room.host_uuid == Some(player_uuid.to_string()))
            .unwrap_or(false);

        match self
            .room_service
            .leave_room(room_id.to_string(), player_uuid.to_string())
            .await
        {
            Ok(LeaveRoomResult::Success(updated_room)) => {
                self.event_bus
                    .emit_to_room(
                        room_id,
                        RoomEvent::PlayerLeft {
                            player: player_uuid.to_string(),
                        },
                    )
                    .await;

                if was_host && updated_room.host_uuid != Some(player_uuid.to_string()) {
                    self.event_bus
                        .emit_to_room(
                            room_id,
                            RoomEvent::HostChanged {
                                old_host: player_uuid.to_string(),
                                new_host: updated_room.host_uuid.clone().unwrap(),
                            },
                        )
                        .await;
                }

                debug!(
                    room_id = %room_id,
                    player_uuid = %player_uuid,
                    "Leave request processed successfully"
                );
            }
            Ok(LeaveRoomResult::RoomDeleted) => {
                debug!(
                    room_id = %room_id,
                    player_uuid = %player_uuid,
                    "Room deleted after player left, cleaning up room state"
                );

                // Get all bots in the room before removing them
                let bots_in_room = self.bot_manager.get_bots_in_room(room_id).await;

                self.game_service.remove_game(room_id).await;

                if let Err(err) = self.stats_service.reset_room_stats(room_id).await {
                    debug!(
                        room_id = %room_id,
                        error = %err,
                        "Failed to reset room stats during room cleanup"
                    );
                }

                // Clean up all bots in the room
                if let Err(e) = self.bot_manager.remove_all_bots_in_room(room_id).await {
                    debug!(
                        room_id = %room_id,
                        error = %e,
                        "Failed to clean up bots, but room is already deleted"
                    );
                }

                // Also remove bot mappings from player mapping service
                for bot in bots_in_room {
                    if !self.player_mapping.remove_player(&bot.uuid).await {
                        // Log but don't fail if bot mapping doesn't exist
                        debug!(
                            room_id = %room_id,
                            bot_uuid = %bot.uuid,
                            "Bot mapping not found or already removed"
                        );
                    }
                }

                self.event_bus.remove_room(room_id).await;
            }
            Ok(_) => {
                debug!(
                    room_id = %room_id,
                    player_uuid = %player_uuid,
                    "Player was not in room or room not found"
                );
            }
            Err(e) => {
                return Err(RoomEventError::HandlerError(format!(
                    "Failed to process leave: {}",
                    e
                )));
            }
        }

        Ok(())
    }

    pub async fn handle_disconnect(
        &self,
        room_id: &str,
        player_uuid: &str,
    ) -> Result<(), RoomEventError> {
        debug!(
            room_id = %room_id,
            player_uuid = %player_uuid,
            "Processing disconnect event"
        );

        if let Some(room) = self
            .room_service
            .get_room(room_id)
            .await
            .map_err(|e| RoomEventError::HandlerError(format!("Failed to fetch room: {}", e)))?
        {
            let mapping = PlayerMappingUtils::build_uuid_to_name_mapping(
                &self.player_mapping,
                room.get_player_uuids(),
            )
            .await;

            let bot_uuids = self.bot_manager.get_bot_uuids_in_room(room_id).await;
            let bot_difficulties = self.bot_manager.get_bot_difficulties_in_room(room_id).await;

            let ws_message = WebSocketMessage::players_list(
                room.get_player_uuids().clone(),
                mapping,
                bot_uuids,
                bot_difficulties,
                room.get_ready_players().clone(),
                room.host_uuid.clone(),
                room.get_connected_players().clone(),
            );

            MessageBroadcaster::broadcast_to_players(
                &self.connection_manager,
                room.get_player_uuids(),
                &ws_message,
            )
            .await
            .map_err(|e| {
                RoomEventError::HandlerError(format!(
                    "Failed to broadcast disconnect update: {}",
                    e
                ))
            })?;
        }

        Ok(())
    }

    pub async fn handle_connect(
        &self,
        room_id: &str,
        player_uuid: &str,
    ) -> Result<(), RoomEventError> {
        debug!(
            room_id = %room_id,
            player_uuid = %player_uuid,
            "Processing connect event"
        );

        // Mark player as connected in the room
        self.room_service
            .mark_player_connected(room_id, player_uuid)
            .await
            .map_err(|e| {
                RoomEventError::HandlerError(format!("Failed to mark player as connected: {}", e))
            })?;

        // Get updated room state
        if let Some(room) = self
            .room_service
            .get_room(room_id)
            .await
            .map_err(|e| RoomEventError::HandlerError(format!("Failed to fetch room: {}", e)))?
        {
            let mapping = PlayerMappingUtils::build_uuid_to_name_mapping(
                &self.player_mapping,
                room.get_player_uuids(),
            )
            .await;

            let bot_uuids = self.bot_manager.get_bot_uuids_in_room(room_id).await;
            let bot_difficulties = self.bot_manager.get_bot_difficulties_in_room(room_id).await;

            let ws_message = WebSocketMessage::players_list(
                room.get_player_uuids().clone(),
                mapping,
                bot_uuids,
                bot_difficulties,
                room.get_ready_players().clone(),
                room.host_uuid.clone(),
                room.get_connected_players().clone(),
            );

            MessageBroadcaster::broadcast_to_players(
                &self.connection_manager,
                room.get_player_uuids(),
                &ws_message,
            )
            .await
            .map_err(|e| {
                RoomEventError::HandlerError(format!("Failed to broadcast connect update: {}", e))
            })?;
        }

        Ok(())
    }
}
