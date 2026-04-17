use axum::{
    extract::{Path, State},
    Extension, Json,
};
use std::sync::Arc;
use tracing::{debug, info, instrument};

use super::types::{CreateRoomApiRequest, JoinRoomRequest, RoomCreateRequest, RoomResponse};
use crate::{
    bot::BotRoomSubscriber,
    event::{RoomEvent, RoomSubscription},
    game::GameEventRoomSubscriber,
    session::SessionClaims,
    shared::{AppError, AppState},
    stats::service::StatsRoomSubscriber,
    websockets::WebSocketRoomSubscriber,
};

/// HTTP handler for creating a new room
///
/// POST /room
/// Returns room information with generated ID
/// Requires valid session (X-Session-ID header)
#[instrument(name = "create_room", skip(state))]
pub async fn create_room(
    State(state): State<AppState>,
    Extension(claims): Extension<SessionClaims>,
    Json(_request): Json<CreateRoomApiRequest>,
) -> Result<Json<RoomResponse>, AppError> {
    // Get host UUID from authenticated session instead of trusting client
    let host_uuid = state
        .session_service
        .get_player_uuid_by_session(&claims.session_id)
        .await?
        .ok_or_else(|| AppError::Unauthorized("No player UUID for session".to_string()))?;

    info!(host_uuid = %host_uuid, "Creating new room");

    // Create request using authenticated session's UUID
    let request = RoomCreateRequest { host_uuid };

    // Create room using business-logic-only service
    let service = Arc::clone(&state.room_service);
    let room_model = service.create_room(request).await?;

    // Set up WebSocket subscription for this room at the composition edge
    let subscriber_room_service = Arc::clone(&state.room_service);
    let room_subscriber = Arc::new(WebSocketRoomSubscriber::new(
        subscriber_room_service,
        Arc::clone(&state.connection_manager),
        Arc::clone(&state.game_service),
        Arc::clone(&state.player_mapping),
        state.event_bus.clone(),
        Arc::clone(&state.bot_manager),
        Arc::clone(&state.stats_service),
    ));

    let room_subscription = RoomSubscription::new(
        room_model.id.clone(),
        room_subscriber,
        state.event_bus.clone(),
    );
    // Start background task - we don't need to track the JoinHandle
    drop(room_subscription.start().await);

    // Set up game event subscription for this room
    let game_subscriber = Arc::new(GameEventRoomSubscriber::new(
        Arc::clone(&state.game_service),
        state.event_bus.clone(),
    ));

    let game_subscription = RoomSubscription::new(
        room_model.id.clone(),
        game_subscriber,
        state.event_bus.clone(),
    );
    // Start background task - we don't need to track the JoinHandle
    drop(game_subscription.start().await);

    // Set up bot subscription for this room
    let bot_subscriber = Arc::new(BotRoomSubscriber::new(
        Arc::clone(&state.bot_manager),
        Arc::clone(&state.game_service),
        state.event_bus.clone(),
    ));

    let bot_subscription = RoomSubscription::new(
        room_model.id.clone(),
        bot_subscriber,
        state.event_bus.clone(),
    );
    // Start background task - we don't need to track the JoinHandle
    drop(bot_subscription.start().await);

    // Set up stats subscription for this room
    let stats_subscriber = Arc::new(StatsRoomSubscriber::new(
        Arc::clone(&state.stats_service),
        Arc::clone(&state.room_service),
        state.event_bus.clone(),
    ));

    let stats_subscription = RoomSubscription::new(
        room_model.id.clone(),
        stats_subscriber,
        state.event_bus.clone(),
    );
    // Start background task - we don't need to track the JoinHandle
    drop(stats_subscription.start().await);

    // Set up activity tracking subscription for this room
    let activity_subscriber = Arc::clone(&state.activity_subscriber);
    let activity_subscription = RoomSubscription::new(
        room_model.id.clone(),
        activity_subscriber,
        state.event_bus.clone(),
    );
    // Start background task - we don't need to track the JoinHandle
    drop(activity_subscription.start().await);

    // Map host UUID to display name for response
    let host_uuid = room_model.host_uuid.clone().unwrap_or_default();
    let host_name = state
        .player_mapping
        .get_playername(&host_uuid)
        .await
        .unwrap_or(host_uuid);
    let room = RoomResponse {
        id: room_model.id.clone(),
        host_name,
        status: room_model.status.clone(),
        player_count: room_model.get_player_count(),
    };

    info!(
        room_id = %room.id,
        host_name = %room.host_name,
        "Room created successfully with WebSocket subscription active"
    );

    Ok(Json(room))
}

/// HTTP handler for listing all rooms
///
/// GET /rooms
/// Returns array of all available rooms
#[instrument(name = "list_rooms", skip(state))]
pub async fn list_rooms(
    State(state): State<AppState>,
) -> Result<Json<Vec<RoomResponse>>, AppError> {
    info!("Listing all rooms");

    // Use injected service from app state
    let service = Arc::clone(&state.room_service);
    let models = service.list_rooms().await?;
    let mut rooms = Vec::with_capacity(models.len());
    for m in models {
        let host_uuid = m.host_uuid.clone().unwrap_or_default();
        let host_name = state
            .player_mapping
            .get_playername(&host_uuid)
            .await
            .unwrap_or(host_uuid);
        let player_count = m.get_player_count();
        rooms.push(RoomResponse {
            id: m.id,
            host_name,
            status: m.status,
            player_count,
        });
    }

    info!(room_count = rooms.len(), "Rooms listed successfully");

    Ok(Json(rooms))
}

/// HTTP handler for joining a room
///
/// POST /room/{room_id}/join
///
/// Joins a player to a room and returns room information.
/// Requires valid session (X-Session-ID header)
#[instrument(name = "join_room", skip(state))]
pub async fn join_room(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Extension(claims): Extension<SessionClaims>,
    Json(_request): Json<JoinRoomRequest>,
) -> Result<Json<RoomResponse>, AppError> {
    info!(
        room_id = %room_id,
        username = %claims.username,
        session_id = %claims.session_id,
        "Player joining room"
    );

    let service = Arc::clone(&state.room_service);

    // Resolve stable player UUID from session
    let player_uuid = state
        .session_service
        .get_player_uuid_by_session(&claims.session_id)
        .await?
        .ok_or_else(|| AppError::Unauthorized("No player UUID for session".to_string()))?;

    // Ensure player UUID is mapped to username (in case of reconnection)
    if let Err(e) = state
        .player_mapping
        .register_player(player_uuid.clone(), claims.username.clone())
        .await
    {
        // Log but don't fail - player may already be registered
        debug!(
            player_uuid = %player_uuid,
            username = %claims.username,
            error = %e,
            "Player mapping registration failed (may already exist)"
        );
    }

    // Join the room (business logic) using UUID
    let room_model = service
        .join_room(room_id.clone(), player_uuid.clone())
        .await?;
    let host_uuid = room_model.host_uuid.clone().unwrap_or_default();
    let host_name = state
        .player_mapping
        .get_playername(&host_uuid)
        .await
        .unwrap_or(host_uuid);
    let room = RoomResponse {
        id: room_model.id.clone(),
        host_name,
        status: room_model.status.clone(),
        player_count: room_model.get_player_count(),
    };

    // Emit room-specific event directly to room subscribers
    state
        .event_bus
        .emit_to_room(
            &room_id,
            RoomEvent::PlayerJoined {
                player: player_uuid,
            },
        )
        .await;

    info!(
        room_id = %room_id,
        username = %claims.username,
        player_count = room.player_count,
        "Player joined room successfully"
    );

    // Always return JSON response with room information
    Ok(Json(room))
}

pub async fn get_room_details(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> Result<Json<RoomResponse>, AppError> {
    // Get room without requiring auth - just returns room info
    let service = Arc::clone(&state.room_service);
    let room_model = service.get_room_details(room_id.clone()).await?;
    let host_uuid = room_model.host_uuid.clone().unwrap_or_default();
    let host_name = state
        .player_mapping
        .get_playername(&host_uuid)
        .await
        .unwrap_or(host_uuid);
    let room = RoomResponse {
        id: room_model.id.clone(),
        host_name,
        status: room_model.status.clone(),
        player_count: room_model.get_player_count(),
    };

    Ok(Json(room))
}

/// HTTP handler for getting room stats
///
/// GET /room/{room_id}/stats
/// Returns current stats for the room
#[instrument(name = "get_room_stats", skip(state))]
pub async fn get_room_stats(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
) -> Result<Json<crate::stats::models::RoomStats>, AppError> {
    let stats = state
        .stats_service
        .get_room_stats(&room_id)
        .await
        .map_err(|e| AppError::DatabaseError(format!("Failed to get room stats: {}", e)))?
        .ok_or_else(|| AppError::NotFound(format!("No stats found for room: {}", room_id)))?;

    Ok(Json(stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::room::repository::InMemoryRoomRepository;
    use crate::shared::test_utils::AppStateBuilder;
    use crate::user::mapping_service::PlayerMappingService;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        Router,
    };
    use tower::ServiceExt; // for `oneshot`

    #[tokio::test]
    async fn test_create_room_handler() {
        // This test now requires authentication, so we'll test the service directly
        // instead of testing the HTTP handler which requires session middleware
        let room_repository = Arc::new(InMemoryRoomRepository::new());
        let player_mapping =
            Arc::new(crate::user::mapping_service::InMemoryPlayerMappingService::new());
        // Add mapping for test UUID
        player_mapping
            .register_player(
                "550e8400-e29b-41d4-a716-446655440000".to_string(),
                "test-player".to_string(),
            )
            .await
            .unwrap();

        let service = super::super::service::RoomService::new(room_repository);

        // Create room using service directly with UUID
        let request = RoomCreateRequest {
            host_uuid: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        };

        let model = service.create_room(request).await.unwrap();
        let id = model.id.clone();
        let status = model.status.clone();
        let player_count = model.get_player_count();
        let host_uuid = model.host_uuid.clone().unwrap_or_default();
        let host_name = player_mapping
            .get_playername(&host_uuid)
            .await
            .unwrap_or(host_uuid);
        let room_response = RoomResponse {
            id,
            host_name,
            status,
            player_count,
        };

        // Verify room response
        assert!(!room_response.id.is_empty());
        assert_eq!(room_response.host_name, "test-player");
        assert_eq!(room_response.status, "ONLINE");
        assert_eq!(room_response.player_count, 1);
    }

    #[tokio::test]
    async fn test_create_room_handler_with_special_characters() {
        let room_repository = Arc::new(InMemoryRoomRepository::new());
        let player_mapping =
            Arc::new(crate::user::mapping_service::InMemoryPlayerMappingService::new());
        // Add mapping for test UUID
        player_mapping
            .register_player(
                "550e8400-e29b-41d4-a716-446655440001".to_string(),
                "player-with-special-chars!@#".to_string(),
            )
            .await
            .unwrap();

        let service = super::super::service::RoomService::new(room_repository);

        let request = RoomCreateRequest {
            host_uuid: "550e8400-e29b-41d4-a716-446655440001".to_string(),
        };

        let model = service.create_room(request).await.unwrap();
        let host_uuid = model.host_uuid.clone().unwrap_or_default();
        let host_name = player_mapping
            .get_playername(&host_uuid)
            .await
            .unwrap_or(host_uuid);
        assert_eq!(host_name, "player-with-special-chars!@#");
    }

    #[tokio::test]
    async fn test_create_room_handler_empty_host_name() {
        let room_repository = Arc::new(InMemoryRoomRepository::new());
        let player_mapping =
            Arc::new(crate::user::mapping_service::InMemoryPlayerMappingService::new());

        let service = super::super::service::RoomService::new(room_repository);

        let request = RoomCreateRequest {
            host_uuid: "".to_string(),
        };

        let model = service.create_room(request).await.unwrap();
        let host_uuid = model.host_uuid.clone().unwrap_or_default();
        let host_name = player_mapping
            .get_playername(&host_uuid)
            .await
            .unwrap_or(host_uuid);
        assert_eq!(host_name, ""); // Empty UUID should map to empty string
    }

    #[tokio::test]
    async fn test_list_rooms_handler_empty() {
        let room_repository = Arc::new(InMemoryRoomRepository::new());
        let app_state = AppStateBuilder::new()
            .with_room_repository(room_repository)
            .build_with_test_defaults();

        let app = Router::new()
            .route("/rooms", axum::routing::get(list_rooms))
            .with_state(app_state);

        let request = Request::builder()
            .method("GET")
            .uri("/rooms")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let rooms: Vec<RoomResponse> = serde_json::from_slice(&body).unwrap();

        assert!(rooms.is_empty());
    }

    #[tokio::test]
    async fn test_list_rooms_handler_with_rooms() {
        let room_repository = Arc::new(InMemoryRoomRepository::new());
        let app_state = AppStateBuilder::new()
            .with_room_repository(room_repository.clone())
            .build_with_test_defaults();

        // Create some rooms first using the service directly
        let service = Arc::clone(&app_state.room_service);
        let request1 = RoomCreateRequest {
            host_uuid: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        };
        let request2 = RoomCreateRequest {
            host_uuid: "550e8400-e29b-41d4-a716-446655440001".to_string(),
        };
        let created_room1 = service.create_room(request1).await.unwrap();
        let created_room2 = service.create_room(request2).await.unwrap();

        let app = Router::new()
            .route("/rooms", axum::routing::get(list_rooms))
            .with_state(app_state);

        let request = Request::builder()
            .method("GET")
            .uri("/rooms")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let rooms: Vec<RoomResponse> = serde_json::from_slice(&body).unwrap();

        assert_eq!(rooms.len(), 2);

        // Verify both rooms are present (order may vary)
        let room_ids: std::collections::HashSet<String> =
            rooms.iter().map(|r| r.id.clone()).collect();
        assert!(room_ids.contains(&created_room1.id));
        assert!(room_ids.contains(&created_room2.id));

        // Verify host names are correct (should be UUIDs since no mapping was set up)
        let room_hosts: std::collections::HashMap<String, String> = rooms
            .iter()
            .map(|r| (r.id.clone(), r.host_name.clone()))
            .collect();
        assert_eq!(
            room_hosts.get(&created_room1.id),
            Some(&"550e8400-e29b-41d4-a716-446655440000".to_string())
        );
        assert_eq!(
            room_hosts.get(&created_room2.id),
            Some(&"550e8400-e29b-41d4-a716-446655440001".to_string())
        );

        // Verify all rooms have expected structure
        for room in &rooms {
            assert!(!room.id.is_empty());
            assert_eq!(room.status, "ONLINE");
            assert_eq!(room.player_count, 1);
        }
    }

    #[tokio::test]
    async fn test_list_rooms_handler_single_room() {
        let room_repository = Arc::new(InMemoryRoomRepository::new());
        let app_state = AppStateBuilder::new()
            .with_room_repository(room_repository.clone())
            .build_with_test_defaults();

        // Create one room using the service directly
        let service = Arc::clone(&app_state.room_service);
        let request = RoomCreateRequest {
            host_uuid: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        };
        let created_room = service.create_room(request).await.unwrap();

        let app = Router::new()
            .route("/rooms", axum::routing::get(list_rooms))
            .with_state(app_state);

        let request = Request::builder()
            .method("GET")
            .uri("/rooms")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let rooms: Vec<RoomResponse> = serde_json::from_slice(&body).unwrap();

        assert_eq!(rooms.len(), 1);
        assert_eq!(rooms[0].id, created_room.id);
        assert_eq!(rooms[0].host_name, "550e8400-e29b-41d4-a716-446655440000"); // No mapping, so UUID
        assert_eq!(rooms[0].status, "ONLINE");
        assert_eq!(rooms[0].player_count, 1);
    }

    #[tokio::test]
    async fn test_join_room_handler_room_not_found() {
        let room_repository = Arc::new(InMemoryRoomRepository::new());
        let app_state = AppStateBuilder::new()
            .with_room_repository(room_repository)
            .build_with_test_defaults();

        let app = Router::new()
            .route("/room/:room_id", axum::routing::post(join_room))
            .with_state(app_state);

        let request = Request::builder()
            .method("POST")
            .uri("/room/nonexistent-room")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn test_join_room_handler_room_full() {
        let room_repository = Arc::new(InMemoryRoomRepository::new());
        let app_state = AppStateBuilder::new()
            .with_room_repository(room_repository.clone())
            .build_with_test_defaults();

        // Create a room and fill it to capacity using the service directly
        let service = Arc::clone(&app_state.room_service);
        let request = RoomCreateRequest {
            host_uuid: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        };
        let created_room = service.create_room(request).await.unwrap();

        // Fill room to capacity (3 more players to reach 4 total)
        for _ in 0..3 {
            service
                .join_room(created_room.id.clone(), "test-player".to_string())
                .await
                .unwrap();
        }

        let app = Router::new()
            .route("/room/:room_id", axum::routing::post(join_room))
            .with_state(app_state);

        // Try to join the full room
        let request = Request::builder()
            .method("POST")
            .uri(format!("/room/{}", created_room.id))
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
