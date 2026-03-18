use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::{
    event::RoomEventError,
    game::Card,
    room::service::RoomService,
    user::PlayerMappingService,
    websockets::{connection_manager::ConnectionManager, messages::WebSocketMessage},
};

use super::shared::{MessageBroadcaster, PlayerMappingUtils, RoomQueryUtils};

pub struct RoomEventHandlers {
    pub(crate) room_service: Arc<RoomService>,
    pub(crate) connection_manager: Arc<dyn ConnectionManager>,
    player_mapping: Arc<dyn PlayerMappingService>,
    bot_manager: Arc<crate::bot::BotManager>,
}

impl RoomEventHandlers {
    pub fn new(
        room_service: Arc<RoomService>,
        connection_manager: Arc<dyn ConnectionManager>,
        player_mapping: Arc<dyn PlayerMappingService>,
        bot_manager: Arc<crate::bot::BotManager>,
    ) -> Self {
        Self {
            room_service,
            connection_manager,
            player_mapping,
            bot_manager,
        }
    }

    #[allow(dead_code)] // Utility function for future use
    fn cards_to_strings(cards: &[Card]) -> Vec<String> {
        cards.iter().map(|c| c.to_string()).collect()
    }

    pub async fn handle_player_joined(&self, room_id: &str) -> Result<(), RoomEventError> {
        debug!(room_id = %room_id, "Handling player joined event");

        let room = RoomQueryUtils::get_room_or_error(&self.room_service, room_id).await?;

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
        .await?;

        debug!(
            room_id = %room_id,
            players_notified = room.get_player_uuids().len(),
            "Player joined notification sent to all room players"
        );

        Ok(())
    }

    pub async fn handle_player_left(
        &self,
        room_id: &str,
        uuid: &str,
    ) -> Result<(), RoomEventError> {
        debug!(
            room_id = %room_id,
            uuid = %uuid,
            "Handling player left event"
        );

        let room = match RoomQueryUtils::get_room_if_exists(&self.room_service, room_id).await? {
            Some(room) => room,
            None => {
                debug!(room_id = %room_id, "Room was deleted, no notifications needed");
                return Ok(());
            }
        };

        let player_name = PlayerMappingUtils::get_player_name(&self.player_mapping, uuid)
            .await
            .unwrap_or_else(|| uuid.to_string());

        let leave_message = WebSocketMessage::leave(player_name);
        MessageBroadcaster::broadcast_to_players(
            &self.connection_manager,
            room.get_player_uuids(),
            &leave_message,
        )
        .await?;

        let mapping = PlayerMappingUtils::build_uuid_to_name_mapping(
            &self.player_mapping,
            room.get_player_uuids(),
        )
        .await;

        let bot_uuids = self.bot_manager.get_bot_uuids_in_room(room_id).await;
        let bot_difficulties = self.bot_manager.get_bot_difficulties_in_room(room_id).await;

        let players_list_message = WebSocketMessage::players_list(
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
            &players_list_message,
        )
        .await?;

        Ok(())
    }

    pub async fn handle_host_changed(
        &self,
        room_id: &str,
        old_host_uuid: &str,
        new_host_uuid: &str,
    ) -> Result<(), RoomEventError> {
        info!(
            room_id = %room_id,
            old_host_uuid = %old_host_uuid,
            new_host_uuid = %new_host_uuid,
            "Handling host changed event"
        );

        let room = match RoomQueryUtils::get_room_if_exists(&self.room_service, room_id).await? {
            Some(room) => room,
            None => {
                warn!(room_id = %room_id, "Room was deleted, no host change notifications needed");
                return Ok(());
            }
        };

        let new_host_name =
            PlayerMappingUtils::get_player_name(&self.player_mapping, new_host_uuid)
                .await
                .unwrap_or_else(|| new_host_uuid.to_string());

        let host_change_message =
            WebSocketMessage::host_change(new_host_name, new_host_uuid.to_string());
        MessageBroadcaster::broadcast_to_players(
            &self.connection_manager,
            room.get_player_uuids(),
            &host_change_message,
        )
        .await?;

        info!(
            room_id = %room_id,
            old_host_uuid = %old_host_uuid,
            new_host_uuid = %new_host_uuid,
            players_notified = room.get_player_uuids().len(),
            "Host change notification sent to all room players"
        );

        Ok(())
    }

    pub async fn handle_bot_added(
        &self,
        room_id: &str,
        bot_uuid: &str,
        bot_name: &str,
    ) -> Result<(), RoomEventError> {
        info!(
            room_id = %room_id,
            bot_uuid = %bot_uuid,
            bot_name = %bot_name,
            "Handling bot added event"
        );

        // Auto-mark bot as ready
        self.room_service
            .toggle_ready(room_id, bot_uuid)
            .await
            .map_err(|e| {
                RoomEventError::HandlerError(format!("Failed to mark bot as ready: {}", e))
            })?;

        let room = RoomQueryUtils::get_room_or_error(&self.room_service, room_id).await?;
        let bot_difficulty = self
            .bot_manager
            .get_bot(bot_uuid)
            .await
            .map(|bot| bot.difficulty)
            .ok_or_else(|| {
                RoomEventError::HandlerError(format!(
                    "Bot metadata not found for newly added bot {}",
                    bot_uuid
                ))
            })?;

        // Send BOT_ADDED message to all players
        let bot_added_message =
            WebSocketMessage::bot_added(bot_uuid.to_string(), bot_name.to_string(), bot_difficulty);
        MessageBroadcaster::broadcast_to_players(
            &self.connection_manager,
            room.get_player_uuids(),
            &bot_added_message,
        )
        .await?;

        // Send updated PLAYERS_LIST message
        let mapping = PlayerMappingUtils::build_uuid_to_name_mapping(
            &self.player_mapping,
            room.get_player_uuids(),
        )
        .await;

        let bot_uuids = self.bot_manager.get_bot_uuids_in_room(room_id).await;
        let bot_difficulties = self.bot_manager.get_bot_difficulties_in_room(room_id).await;

        let players_list_message = WebSocketMessage::players_list(
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
            &players_list_message,
        )
        .await?;

        info!(
            room_id = %room_id,
            bot_uuid = %bot_uuid,
            players_notified = room.get_player_uuids().len(),
            "Bot added notification sent to all room players"
        );

        Ok(())
    }

    pub async fn handle_bot_removed(
        &self,
        room_id: &str,
        bot_uuid: &str,
    ) -> Result<(), RoomEventError> {
        info!(
            room_id = %room_id,
            bot_uuid = %bot_uuid,
            "Handling bot removed event"
        );

        let room = match RoomQueryUtils::get_room_if_exists(&self.room_service, room_id).await? {
            Some(room) => room,
            None => {
                debug!(room_id = %room_id, "Room was deleted, no notifications needed");
                return Ok(());
            }
        };

        // Send BOT_REMOVED message to all players
        let bot_removed_message = WebSocketMessage::bot_removed(bot_uuid.to_string());
        MessageBroadcaster::broadcast_to_players(
            &self.connection_manager,
            room.get_player_uuids(),
            &bot_removed_message,
        )
        .await?;

        // Send updated PLAYERS_LIST message
        let mapping = PlayerMappingUtils::build_uuid_to_name_mapping(
            &self.player_mapping,
            room.get_player_uuids(),
        )
        .await;

        let bot_uuids = self.bot_manager.get_bot_uuids_in_room(room_id).await;
        let bot_difficulties = self.bot_manager.get_bot_difficulties_in_room(room_id).await;

        let players_list_message = WebSocketMessage::players_list(
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
            &players_list_message,
        )
        .await?;

        info!(
            room_id = %room_id,
            bot_uuid = %bot_uuid,
            players_notified = room.get_player_uuids().len(),
            "Bot removed notification sent to all room players"
        );

        Ok(())
    }

    pub async fn handle_player_ready_toggled(
        &self,
        room_id: &str,
        player_uuid: &str,
        is_ready: bool,
    ) -> Result<(), RoomEventError> {
        info!(
            room_id = %room_id,
            player_uuid = %player_uuid,
            is_ready = is_ready,
            "Handling player ready toggled event"
        );

        // Set ready state in repository based on the is_ready parameter
        self.room_service
            .set_ready(room_id, player_uuid, is_ready)
            .await
            .map_err(|e| {
                RoomEventError::HandlerError(format!("Failed to set ready state: {}", e))
            })?;

        // Get updated room state
        let room = RoomQueryUtils::get_room_or_error(&self.room_service, room_id).await?;

        // Broadcast updated PLAYERS_LIST with ready state
        let mapping = PlayerMappingUtils::build_uuid_to_name_mapping(
            &self.player_mapping,
            room.get_player_uuids(),
        )
        .await;

        let bot_uuids = self.bot_manager.get_bot_uuids_in_room(room_id).await;
        let bot_difficulties = self.bot_manager.get_bot_difficulties_in_room(room_id).await;

        let players_list_message = WebSocketMessage::players_list(
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
            &players_list_message,
        )
        .await?;

        info!(
            room_id = %room_id,
            player_uuid = %player_uuid,
            is_ready = is_ready,
            players_notified = room.get_player_uuids().len(),
            "Player ready state updated and broadcast to all players"
        );

        Ok(())
    }
}
