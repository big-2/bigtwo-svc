use std::sync::Arc;
use tracing::info;

use crate::{
    event::{EventBus, RoomEvent, RoomEventError},
    game::{Card, Game, GameService},
    room::service::RoomService,
    websockets::{connection_manager::ConnectionManager, messages::WebSocketMessage},
};

use super::shared::{MessageBroadcaster, RoomQueryUtils};

fn cards_to_strings(cards: &[Card]) -> Vec<String> {
    cards.iter().map(|card| card.to_string()).collect()
}

pub struct GameEventHandlers {
    room_service: Arc<RoomService>,
    connection_manager: Arc<dyn ConnectionManager>,
    game_service: Arc<GameService>,
    event_bus: EventBus,
    bot_manager: Arc<crate::bot::BotManager>,
}

impl GameEventHandlers {
    pub fn new(
        room_service: Arc<RoomService>,
        connection_manager: Arc<dyn ConnectionManager>,
        game_service: Arc<GameService>,
        event_bus: EventBus,
        bot_manager: Arc<crate::bot::BotManager>,
    ) -> Self {
        Self {
            room_service,
            connection_manager,
            game_service,
            event_bus,
            bot_manager,
        }
    }

    pub async fn handle_start_game(&self, room_id: &str, game: Game) -> Result<(), RoomEventError> {
        info!(room_id = %room_id, "Starting game");

        // Clear all ready states when game starts
        self.room_service
            .clear_ready_states(room_id)
            .await
            .map_err(|e| {
                RoomEventError::HandlerError(format!("Failed to clear ready states: {}", e))
            })?;

        let current_player_turn = game.current_player_turn();

        // Build card counts map for all players
        let card_counts: std::collections::HashMap<String, usize> = game
            .players()
            .iter()
            .map(|p| (p.uuid.clone(), p.cards.len()))
            .collect();

        // At game start, no one has played yet, so last_plays_by_player is empty
        let last_plays_by_player = std::collections::HashMap::new();

        for player in game.players() {
            let player_message = WebSocketMessage::game_started(
                current_player_turn.clone(),
                cards_to_strings(&player.cards),
                game.players()
                    .iter()
                    .map(|player| player.uuid.clone())
                    .collect(),
                card_counts.clone(),
                last_plays_by_player.clone(),
            );

            let message_json = serde_json::to_string(&player_message).map_err(|e| {
                RoomEventError::HandlerError(format!(
                    "Failed to serialize GAME_STARTED message: {}",
                    e
                ))
            })?;

            self.connection_manager
                .send_to_player(&player.uuid, &message_json)
                .await;
        }

        // Notify subscribers whose turn it is so bots can act immediately
        self.event_bus
            .emit_to_room(
                room_id,
                RoomEvent::TurnChanged {
                    player: current_player_turn,
                },
            )
            .await;

        Ok(())
    }

    pub async fn handle_try_start_game(
        &self,
        room_id: &str,
        host: &str,
    ) -> Result<(), RoomEventError> {
        info!(
            room_id = %room_id,
            host = %host,
            "Handling start game event"
        );

        let room = RoomQueryUtils::get_room_or_error(&self.room_service, room_id).await?;

        if room.host_uuid != Some(host.to_string()) {
            info!(
                room_id = %room_id,
                host = %host,
                "Host is not the current host, cannot start game. Room host: {:?}",
                room.host_uuid
            );
            return Ok(());
        }

        if room.get_player_uuids().len() != 4 {
            info!(room_id = %room_id, "Room does not have 4 players, cannot start game");
            return Ok(());
        }

        // Check that all human (non-bot) players are ready
        let bot_uuids_set: std::collections::HashSet<_> = self
            .bot_manager
            .get_bot_uuids_in_room(room_id)
            .await
            .into_iter()
            .collect();

        let human_player_uuids: Vec<_> = room
            .get_player_uuids()
            .iter()
            .filter(|uuid| !bot_uuids_set.contains(*uuid))
            .collect();

        let ready_uuids_set: std::collections::HashSet<_> =
            room.get_ready_players().iter().collect();

        let all_humans_ready = human_player_uuids
            .iter()
            .all(|uuid| ready_uuids_set.contains(uuid));

        if !all_humans_ready {
            info!(
                room_id = %room_id,
                "Not all human players are ready, cannot start game"
            );
            // TODO: Send error message back to host
            return Ok(());
        }

        self.event_bus
            .emit_to_room(
                room_id,
                RoomEvent::CreateGame {
                    players: room.get_player_uuids().clone(),
                },
            )
            .await;

        Ok(())
    }

    pub async fn handle_move_played(
        &self,
        room_id: &str,
        player_uuid: &str,
        cards: &[Card],
        game: Game,
    ) -> Result<(), RoomEventError> {
        info!(
            room_id = %room_id,
            player_uuid = %player_uuid,
            cards = ?cards,
            "Handling move played event"
        );

        // Get the player who made the move to find their remaining card count
        let remaining_cards = game
            .players()
            .iter()
            .find(|p| p.uuid == player_uuid)
            .map(|p| p.cards.len())
            .unwrap_or(0);

        let player_message = WebSocketMessage::move_played(
            player_uuid.to_string(),
            cards_to_strings(cards),
            remaining_cards,
        );

        let player_uuids: Vec<String> = game.players().iter().map(|p| p.uuid.clone()).collect();
        MessageBroadcaster::broadcast_to_players(
            &self.connection_manager,
            &player_uuids,
            &player_message,
        )
        .await?;

        Ok(())
    }

    pub async fn handle_turn_changed(
        &self,
        room_id: &str,
        player: &str,
    ) -> Result<(), RoomEventError> {
        info!(
            room_id = %room_id,
            player = %player,
            "Handling turn changed event"
        );

        let game =
            self.game_service
                .get_game(room_id)
                .await
                .ok_or(RoomEventError::HandlerError(format!(
                    "Game not found for room: {}",
                    room_id
                )))?;

        let turn_change_message = WebSocketMessage::turn_change(player.to_string());
        let player_uuids: Vec<String> = game.players().iter().map(|p| p.uuid.clone()).collect();
        MessageBroadcaster::broadcast_to_players(
            &self.connection_manager,
            &player_uuids,
            &turn_change_message,
        )
        .await?;

        info!(
            room_id = %room_id,
            player = %player,
            players_notified = game.players().len(),
            "Turn change notification sent to all players"
        );

        Ok(())
    }

    pub async fn handle_game_won(
        &self,
        room_id: &str,
        winner: &str,
        winning_hand: &[Card],
        game: Game,
    ) -> Result<(), RoomEventError> {
        info!(
            room_id = %room_id,
            winner = %winner,
            "Handling game won event"
        );

        let card_strings = cards_to_strings(winning_hand);
        let game_won_message = WebSocketMessage::game_won(winner.to_string(), card_strings);
        let player_uuids: Vec<String> = game.players().iter().map(|p| p.uuid.clone()).collect();
        MessageBroadcaster::broadcast_to_players(
            &self.connection_manager,
            &player_uuids,
            &game_won_message,
        )
        .await?;

        info!(
            room_id = %room_id,
            winner = %winner,
            players_notified = game.players().len(),
            "Game won notification sent to all players"
        );

        // Remove the completed game from the repository after notifying all players
        // This ensures that on refresh/reconnect, players see the lobby instead of a completed game
        self.game_service.remove_game(room_id).await;
        info!(
            room_id = %room_id,
            "Removed completed game from repository"
        );

        Ok(())
    }
}
