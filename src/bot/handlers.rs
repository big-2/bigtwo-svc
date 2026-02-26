use axum::{
    extract::{Path, State},
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::{
    event::RoomEvent,
    session::SessionClaims,
    shared::{AppError, AppState},
};

use super::{manager::MAX_BOTS_PER_ROOM, types::BotDifficulty};

#[derive(Debug, Serialize, Deserialize)]
pub struct AddBotRequest {
    #[serde(default = "default_difficulty")]
    pub difficulty: BotDifficulty,
}

fn default_difficulty() -> BotDifficulty {
    BotDifficulty::Easy
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BotResponse {
    pub uuid: String,
    pub name: String,
    pub difficulty: BotDifficulty,
}

/// Add a bot to a room
/// POST /room/{room_id}/bot
/// Requires authentication and host privileges
pub async fn add_bot_to_room(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Extension(claims): Extension<SessionClaims>,
    Json(request): Json<AddBotRequest>,
) -> Result<Json<BotResponse>, AppError> {
    info!(
        room_id = %room_id,
        username = %claims.username,
        difficulty = ?request.difficulty,
        "Request to add bot to room"
    );

    // Get the room
    let room = state
        .room_service
        .get_room(&room_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Room not found: {}", room_id)))?;

    // Get player UUID from session
    let player_uuid = state
        .session_service
        .get_player_uuid_by_session(&claims.session_id)
        .await?
        .ok_or_else(|| AppError::Unauthorized("No player UUID for session".to_string()))?;

    // Verify the requester is the host
    let host_uuid = room
        .host_uuid
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("Room has no host".to_string()))?;

    if &player_uuid != host_uuid {
        warn!(
            room_id = %room_id,
            player_uuid = %player_uuid,
            host_uuid = %host_uuid,
            "Non-host attempted to add bot"
        );
        return Err(AppError::Forbidden(
            "Only the host can add bots".to_string(),
        ));
    }

    // Check if room is full by humans/total capacity
    if room.is_full() {
        return Err(AppError::BadRequest("Room is full".to_string()));
    }

    // Check current bot count to enforce per-room limit
    let existing_bot_count = state.bot_manager.get_bots_in_room(&room_id).await.len();
    if existing_bot_count >= MAX_BOTS_PER_ROOM {
        return Err(AppError::BadRequest(format!(
            "Room {} already has the maximum of {} bots",
            room_id, MAX_BOTS_PER_ROOM
        )));
    }

    // Create the bot
    let bot = state
        .bot_manager
        .create_bot(room_id.clone(), request.difficulty)
        .await?;

    // Register the bot in player mapping
    if let Err(e) = state
        .player_mapping
        .register_player(bot.uuid.clone(), bot.name.clone())
        .await
    {
        warn!(
            room_id = %room_id,
            bot_uuid = %bot.uuid,
            error = %e,
            "Failed to register bot in player mapping, cleaning up bot"
        );
        // Clean up the bot that was created
        let _ = state.bot_manager.remove_bot(&bot.uuid).await;
        return Err(AppError::Internal);
    }

    // Add the bot to the room
    if let Err(e) = state
        .room_service
        .join_room(room_id.clone(), bot.uuid.clone())
        .await
    {
        warn!(
            room_id = %room_id,
            bot_uuid = %bot.uuid,
            error = %e,
            "Failed to add bot to room, cleaning up"
        );
        // Clean up the bot and mapping
        let _ = state.bot_manager.remove_bot(&bot.uuid).await;
        return Err(e);
    }

    // Emit PlayerJoined event
    state
        .event_bus
        .emit_to_room(
            &room_id,
            RoomEvent::PlayerJoined {
                player: bot.uuid.clone(),
            },
        )
        .await;

    // Emit BotAdded event for WebSocket notification
    state
        .event_bus
        .emit_to_room(
            &room_id,
            RoomEvent::BotAdded {
                bot_uuid: bot.uuid.clone(),
                bot_name: bot.name.clone(),
            },
        )
        .await;

    info!(
        room_id = %room_id,
        bot_uuid = %bot.uuid,
        bot_name = %bot.name,
        "Bot added to room successfully"
    );

    Ok(Json(BotResponse {
        uuid: bot.uuid,
        name: bot.name,
        difficulty: bot.difficulty,
    }))
}

/// Remove a bot from a room
/// DELETE /room/{room_id}/bot/{bot_uuid}
/// Requires authentication and host privileges
pub async fn remove_bot_from_room(
    State(state): State<AppState>,
    Path((room_id, bot_uuid)): Path<(String, String)>,
    Extension(claims): Extension<SessionClaims>,
) -> Result<Json<serde_json::Value>, AppError> {
    info!(
        room_id = %room_id,
        bot_uuid = %bot_uuid,
        username = %claims.username,
        "Request to remove bot from room"
    );

    // Verify the UUID is actually a bot
    if !state.bot_manager.is_bot(&bot_uuid).await {
        return Err(AppError::BadRequest(format!(
            "UUID is not a bot: {}",
            bot_uuid
        )));
    }

    // Get the room
    let room = state
        .room_service
        .get_room(&room_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Room not found: {}", room_id)))?;

    // Get player UUID from session
    let player_uuid = state
        .session_service
        .get_player_uuid_by_session(&claims.session_id)
        .await?
        .ok_or_else(|| AppError::Unauthorized("No player UUID for session".to_string()))?;

    // Verify the requester is the host
    let host_uuid = room
        .host_uuid
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("Room has no host".to_string()))?;

    if &player_uuid != host_uuid {
        warn!(
            room_id = %room_id,
            player_uuid = %player_uuid,
            host_uuid = %host_uuid,
            "Non-host attempted to remove bot"
        );
        return Err(AppError::Forbidden(
            "Only the host can remove bots".to_string(),
        ));
    }

    if !state.bot_manager.is_bot(&bot_uuid).await {
        return Err(AppError::BadRequest(format!(
            "UUID is not a bot: {}",
            bot_uuid
        )));
    }

    let bot = state
        .bot_manager
        .get_bot(&bot_uuid)
        .await
        .ok_or_else(|| AppError::NotFound(format!("Bot not found: {}", bot_uuid)))?;

    if bot.room_id != room_id {
        return Err(AppError::BadRequest(
            "Bot not in specified room".to_string(),
        ));
    }

    // Remove the bot from the room first to maintain room invariants
    state
        .room_service
        .leave_room(room_id.clone(), bot_uuid.clone())
        .await?;

    // Remove the bot from bot manager
    state.bot_manager.remove_bot(&bot_uuid).await?;

    // Also remove the bot's player mapping to keep mappings clean
    state.player_mapping.remove_player(&bot_uuid).await;

    // Emit PlayerLeft event
    state
        .event_bus
        .emit_to_room(
            &room_id,
            RoomEvent::PlayerLeft {
                player: bot_uuid.clone(),
            },
        )
        .await;

    // Emit BotRemoved event for WebSocket notification
    state
        .event_bus
        .emit_to_room(
            &room_id,
            RoomEvent::BotRemoved {
                bot_uuid: bot_uuid.clone(),
            },
        )
        .await;

    info!(
        room_id = %room_id,
        bot_uuid = %bot_uuid,
        "Bot removed from room successfully"
    );

    Ok(Json(serde_json::json!({
        "message": "Bot removed successfully"
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_add_bot_request_deserialization() {
        let json = r#"{"difficulty": "easy"}"#;
        let request: AddBotRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.difficulty, BotDifficulty::Easy);
    }

    #[tokio::test]
    async fn test_add_bot_request_deserialization_ai() {
        let json = r#"{"difficulty": "ai"}"#;
        let request: AddBotRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.difficulty, BotDifficulty::Ai);
    }

    #[tokio::test]
    async fn test_add_bot_request_default_difficulty() {
        let json = r#"{}"#;
        let request: AddBotRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.difficulty, BotDifficulty::Easy);
    }

    #[tokio::test]
    async fn test_bot_response_serialization() {
        let response = BotResponse {
            uuid: "bot-123".to_string(),
            name: "happy-turtle Bot".to_string(),
            difficulty: BotDifficulty::Medium,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("bot-123"));
        assert!(json.contains("happy-turtle Bot"));
        assert!(json.contains("medium"));
    }

    // Additional integration-style tests would go here
}
