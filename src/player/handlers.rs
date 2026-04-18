use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::{
    bot::types::BotPlayer,
    session::SessionClaims,
    shared::{AppError, AppState},
    stats::{
        CompletedGameDetailResponse, PlayerProfileStatsResponse, PlayerRecentGamesResponse,
        StatsGameFilter,
    },
};

#[derive(Debug, Deserialize)]
pub struct StatsQuery {
    pub filter: Option<StatsGameFilter>,
}

#[derive(Debug, Deserialize)]
pub struct RecentGamesQuery {
    pub limit: Option<u32>,
    pub before: Option<DateTime<Utc>>,
    pub filter: Option<StatsGameFilter>,
}

pub async fn get_my_stats(
    State(state): State<AppState>,
    Extension(claims): Extension<SessionClaims>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<PlayerProfileStatsResponse>, AppError> {
    let player_uuid = state
        .session_service
        .get_player_uuid_by_session(&claims.session_id)
        .await?
        .ok_or_else(|| AppError::Unauthorized("No player UUID for session".to_string()))?;

    let stats = state
        .stats_service
        .get_player_profile_stats(
            &player_uuid,
            &claims.username,
            query.filter.unwrap_or_default(),
        )
        .await
        .map_err(|e| AppError::DatabaseError(format!("Failed to get player stats: {}", e)))?;

    let stats = stats.unwrap_or_else(|| empty_player_profile(&player_uuid, &claims.username));

    Ok(Json(stats))
}

pub async fn get_my_recent_games(
    State(state): State<AppState>,
    Extension(claims): Extension<SessionClaims>,
    Query(query): Query<RecentGamesQuery>,
) -> Result<Json<PlayerRecentGamesResponse>, AppError> {
    let player_uuid = state
        .session_service
        .get_player_uuid_by_session(&claims.session_id)
        .await?
        .ok_or_else(|| AppError::Unauthorized("No player UUID for session".to_string()))?;

    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    let mut games = state
        .stats_service
        .get_recent_games_for_player(
            &player_uuid,
            limit,
            query.before,
            query.filter.unwrap_or_default(),
        )
        .await
        .map_err(|e| AppError::DatabaseError(format!("Failed to get recent games: {}", e)))?;

    populate_recent_game_display_names(&state, &player_uuid, &claims.username, &mut games).await;

    Ok(Json(games))
}

pub async fn get_completed_game(
    State(state): State<AppState>,
    Extension(claims): Extension<SessionClaims>,
    Path(game_id): Path<String>,
) -> Result<Json<CompletedGameDetailResponse>, AppError> {
    let player_uuid = state
        .session_service
        .get_player_uuid_by_session(&claims.session_id)
        .await?
        .ok_or_else(|| AppError::Unauthorized("No player UUID for session".to_string()))?;

    let mut game = state
        .stats_service
        .get_completed_game_for_player(&player_uuid, &game_id)
        .await
        .map_err(|e| AppError::DatabaseError(format!("Failed to get completed game: {}", e)))?
        .ok_or_else(|| AppError::NotFound("Completed game not found".to_string()))?;

    populate_completed_game_display_names(&state, &player_uuid, &claims.username, &mut game).await;

    Ok(Json(game))
}

fn empty_player_profile(player_uuid: &str, display_name: &str) -> PlayerProfileStatsResponse {
    PlayerProfileStatsResponse {
        player_uuid: player_uuid.to_string(),
        display_name: display_name.to_string(),
        summary: Default::default(),
        play_style: Default::default(),
        splits: Default::default(),
        recent_form: Default::default(),
    }
}

async fn populate_recent_game_display_names(
    state: &AppState,
    current_player_uuid: &str,
    current_display_name: &str,
    games: &mut PlayerRecentGamesResponse,
) {
    for game in &mut games.games {
        for opponent in &mut game.opponents {
            populate_display_name_if_missing(
                state,
                &opponent.player_uuid,
                current_player_uuid,
                current_display_name,
                &mut opponent.display_name,
            )
            .await;
        }
    }
}

async fn populate_completed_game_display_names(
    state: &AppState,
    current_player_uuid: &str,
    current_display_name: &str,
    game: &mut CompletedGameDetailResponse,
) {
    for player in &mut game.players {
        populate_display_name_if_missing(
            state,
            &player.player_uuid,
            current_player_uuid,
            current_display_name,
            &mut player.display_name,
        )
        .await;
    }

    for mv in &mut game.moves {
        populate_display_name_if_missing(
            state,
            &mv.player_uuid,
            current_player_uuid,
            current_display_name,
            &mut mv.display_name,
        )
        .await;
    }
}

async fn populate_display_name_if_missing(
    state: &AppState,
    player_uuid: &str,
    current_player_uuid: &str,
    current_display_name: &str,
    display_name: &mut Option<String>,
) {
    if display_name.is_none() {
        *display_name = Some(
            resolve_player_display_name(
                state,
                player_uuid,
                current_player_uuid,
                current_display_name,
            )
            .await,
        );
    }
}

async fn resolve_player_display_name(
    state: &AppState,
    player_uuid: &str,
    current_player_uuid: &str,
    current_display_name: &str,
) -> String {
    if player_uuid == current_player_uuid {
        return current_display_name.to_string();
    }

    if let Some(display_name) = state
        .session_service
        .get_playername_by_uuid(player_uuid)
        .await
    {
        return display_name;
    }

    fallback_player_display_name(player_uuid)
}

fn fallback_player_display_name(player_uuid: &str) -> String {
    let is_bot = BotPlayer::is_bot_uuid(player_uuid);
    let prefix = if is_bot { "Bot" } else { "Player" };
    let trimmed = player_uuid.strip_prefix("bot-").unwrap_or(player_uuid);
    let short_id: String = trimmed.chars().take(8).collect();

    if short_id.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix} {short_id}")
    }
}
