use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::{
    session::SessionClaims,
    shared::{AppError, AppState},
};

#[derive(Debug, Deserialize)]
pub struct RecentGamesQuery {
    pub limit: Option<u32>,
    pub before: Option<DateTime<Utc>>,
}

pub async fn get_my_stats(
    State(state): State<AppState>,
    Extension(claims): Extension<SessionClaims>,
) -> Result<Json<crate::stats::PlayerProfileStatsResponse>, AppError> {
    let player_uuid = state
        .session_service
        .get_player_uuid_by_session(&claims.session_id)
        .await?
        .ok_or_else(|| AppError::Unauthorized("No player UUID for session".to_string()))?;

    let stats = state
        .stats_service
        .get_player_profile_stats(&player_uuid, &claims.username)
        .await
        .map_err(|e| AppError::DatabaseError(format!("Failed to get player stats: {}", e)))?
        .ok_or_else(|| {
            AppError::NotFound("No completed game stats found for player".to_string())
        })?;

    Ok(Json(stats))
}

pub async fn get_my_recent_games(
    State(state): State<AppState>,
    Extension(claims): Extension<SessionClaims>,
    Query(query): Query<RecentGamesQuery>,
) -> Result<Json<crate::stats::PlayerRecentGamesResponse>, AppError> {
    let player_uuid = state
        .session_service
        .get_player_uuid_by_session(&claims.session_id)
        .await?
        .ok_or_else(|| AppError::Unauthorized("No player UUID for session".to_string()))?;

    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    let games = state
        .stats_service
        .get_recent_games_for_player(&player_uuid, limit, query.before)
        .await
        .map_err(|e| AppError::DatabaseError(format!("Failed to get recent games: {}", e)))?;

    Ok(Json(games))
}

pub async fn get_completed_game(
    State(state): State<AppState>,
    Extension(claims): Extension<SessionClaims>,
    Path(game_id): Path<String>,
) -> Result<Json<crate::stats::CompletedGameDetailResponse>, AppError> {
    let player_uuid = state
        .session_service
        .get_player_uuid_by_session(&claims.session_id)
        .await?
        .ok_or_else(|| AppError::Unauthorized("No player UUID for session".to_string()))?;

    let game = state
        .stats_service
        .get_completed_game_for_player(&player_uuid, &game_id)
        .await
        .map_err(|e| AppError::DatabaseError(format!("Failed to get completed game: {}", e)))?
        .ok_or_else(|| AppError::NotFound("Completed game not found".to_string()))?;

    Ok(Json(game))
}
