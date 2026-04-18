use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn};

#[cfg(test)]
use super::PlayerGameResult;
use super::{
    models::{
        CompletedGameDetailMove, CompletedGameDetailPlayer, CompletedGameDetailResponse,
        GameOpponentSummary, PlayerPlayStyle, PlayerProfileStatsResponse, PlayerRecentForm,
        PlayerRecentGameSummary, PlayerRecentGamesResponse, PlayerRecentWindow, PlayerSplitSummary,
        PlayerStatsSplits, PlayerStatsSummary, RoomStats, StatsGameFilter,
    },
    GameResult, StatsError,
};

#[async_trait]
pub trait StatsRepository: Send + Sync {
    async fn record_game(&self, game_result: GameResult) -> Result<RoomStats, StatsError>;
    async fn get_room_stats(&self, room_id: &str) -> Result<Option<RoomStats>, StatsError>;
    async fn reset_room_stats(&self, room_id: &str) -> Result<(), StatsError>;
}

#[async_trait]
pub trait GameHistoryRepository: Send + Sync {
    async fn record_completed_game(&self, game_result: &GameResult) -> Result<bool, StatsError>;
    async fn get_player_profile_stats(
        &self,
        player_uuid: &str,
        display_name: &str,
        filter: StatsGameFilter,
    ) -> Result<Option<PlayerProfileStatsResponse>, StatsError>;
    async fn get_recent_games_for_player(
        &self,
        player_uuid: &str,
        limit: u32,
        before: Option<DateTime<Utc>>,
        filter: StatsGameFilter,
    ) -> Result<PlayerRecentGamesResponse, StatsError>;
    async fn get_completed_game_for_player(
        &self,
        player_uuid: &str,
        game_id: &str,
    ) -> Result<Option<CompletedGameDetailResponse>, StatsError>;
}

#[derive(Debug, Default)]
pub struct InMemoryStatsRepository {
    rooms: Arc<RwLock<HashMap<String, RoomStats>>>,
}

impl InMemoryStatsRepository {
    pub fn new() -> Self {
        Self {
            rooms: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl StatsRepository for InMemoryStatsRepository {
    async fn record_game(&self, game_result: GameResult) -> Result<RoomStats, StatsError> {
        let mut rooms = self.rooms.write().await;
        let room_stats = rooms
            .entry(game_result.room_id.clone())
            .or_insert_with(|| RoomStats {
                room_id: game_result.room_id.clone(),
                ..RoomStats::default()
            });

        room_stats.games_played += 1;

        for player_result in &game_result.players {
            let player_stats = room_stats
                .player_stats
                .entry(player_result.uuid.clone())
                .or_insert_with(|| super::PlayerStats {
                    uuid: player_result.uuid.clone(),
                    ..super::PlayerStats::default()
                });

            player_stats.games_played += 1;
            player_stats.cards_remaining = player_stats
                .cards_remaining
                .saturating_add(player_result.cards_remaining as u32);
            player_stats.total_score += player_result.final_score;

            if player_result.uuid == game_result.winner_uuid {
                player_stats.wins += 1;
                player_stats.current_win_streak += 1;
                player_stats.best_win_streak = player_stats
                    .best_win_streak
                    .max(player_stats.current_win_streak);
            } else {
                player_stats.current_win_streak = 0;
            }
        }

        Ok(room_stats.clone())
    }

    async fn get_room_stats(&self, room_id: &str) -> Result<Option<RoomStats>, StatsError> {
        let rooms = self.rooms.read().await;
        Ok(rooms.get(room_id).cloned())
    }

    async fn reset_room_stats(&self, room_id: &str) -> Result<(), StatsError> {
        let mut rooms = self.rooms.write().await;
        rooms.remove(room_id);
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
struct InMemoryProfileAggregate {
    games_played: u64,
    wins: u64,
    current_win_streak: u64,
    best_win_streak: u64,
    total_passes: u64,
    total_single_plays: u64,
    total_pair_plays: u64,
    total_triple_plays: u64,
    total_five_card_plays: u64,
    human_games_played: u64,
    human_wins: u64,
    bot_games_played: u64,
    bot_wins: u64,
}

#[derive(Debug, Default, Clone, Copy)]
struct PlayTypeBreakdown {
    single_plays: u64,
    pair_plays: u64,
    triple_plays: u64,
    five_card_plays: u64,
}

#[derive(Debug, Default)]
struct InMemoryCompletedGamesStore {
    games: HashMap<String, GameResult>,
    order: VecDeque<String>,
}

fn read_in_memory_history_limit() -> usize {
    std::env::var("IN_MEMORY_COMPLETED_GAME_HISTORY_LIMIT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(512)
}

#[derive(Debug, Default)]
pub struct InMemoryGameHistoryRepository {
    completed_games: Arc<RwLock<InMemoryCompletedGamesStore>>,
    profile_aggregates: Arc<RwLock<HashMap<String, InMemoryProfileAggregate>>>,
    max_completed_games: usize,
}

impl InMemoryGameHistoryRepository {
    pub fn new() -> Self {
        Self::with_limit(read_in_memory_history_limit())
    }

    pub fn with_limit(max_completed_games: usize) -> Self {
        Self {
            completed_games: Arc::new(RwLock::new(InMemoryCompletedGamesStore::default())),
            profile_aggregates: Arc::new(RwLock::new(HashMap::new())),
            max_completed_games,
        }
    }

    async fn list_games_for_player(&self, player_uuid: &str) -> Vec<GameResult> {
        let completed_games = self.completed_games.read().await;
        let mut relevant_games: Vec<_> = completed_games
            .games
            .values()
            .filter(|game| game.players.iter().any(|p| p.uuid == player_uuid))
            .cloned()
            .collect();
        relevant_games.sort_by_key(|game| game.completed_at);
        relevant_games
    }
}

#[async_trait]
impl GameHistoryRepository for InMemoryGameHistoryRepository {
    async fn record_completed_game(&self, game_result: &GameResult) -> Result<bool, StatsError> {
        {
            let mut completed_games = self.completed_games.write().await;
            if completed_games.games.contains_key(&game_result.game_id) {
                return Ok(false);
            }

            completed_games
                .games
                .insert(game_result.game_id.clone(), game_result.clone());
            completed_games.order.push_back(game_result.game_id.clone());

            while completed_games.games.len() > self.max_completed_games {
                if let Some(oldest_game_id) = completed_games.order.pop_front() {
                    completed_games.games.remove(&oldest_game_id);
                }
            }
        }

        let play_breakdown_by_player = calculate_play_type_breakdown(game_result);
        let mut aggregates = self.profile_aggregates.write().await;
        for player in &game_result.players {
            let aggregate = aggregates.entry(player.uuid.clone()).or_default();
            let play_breakdown = play_breakdown_by_player
                .get(&player.uuid)
                .copied()
                .unwrap_or_default();
            aggregate.games_played += 1;
            aggregate.wins += u64::from(player.won);
            aggregate.total_passes += player.passes as u64;
            aggregate.total_single_plays += play_breakdown.single_plays;
            aggregate.total_pair_plays += play_breakdown.pair_plays;
            aggregate.total_triple_plays += play_breakdown.triple_plays;
            aggregate.total_five_card_plays += play_breakdown.five_card_plays;

            if player.won {
                aggregate.current_win_streak += 1;
                aggregate.best_win_streak =
                    aggregate.best_win_streak.max(aggregate.current_win_streak);
            } else {
                aggregate.current_win_streak = 0;
            }

            if game_result.had_bots {
                aggregate.bot_games_played += 1;
                aggregate.bot_wins += u64::from(player.won);
            } else {
                aggregate.human_games_played += 1;
                aggregate.human_wins += u64::from(player.won);
            }
        }

        Ok(true)
    }

    async fn get_player_profile_stats(
        &self,
        player_uuid: &str,
        display_name: &str,
        filter: StatsGameFilter,
    ) -> Result<Option<PlayerProfileStatsResponse>, StatsError> {
        let all_games = self.list_games_for_player(player_uuid).await;
        if all_games.is_empty() {
            return Ok(None);
        }

        let filtered_games: Vec<_> = all_games
            .iter()
            .filter(|game| filter.matches_game(game.had_bots))
            .cloned()
            .collect();
        let wins = filtered_games
            .iter()
            .filter(|game| game.winner_uuid == player_uuid)
            .count() as u64;
        let games_played = filtered_games.len() as u64;
        let (current_win_streak, best_win_streak) =
            build_streaks_from_games(&filtered_games, player_uuid);

        Ok(Some(PlayerProfileStatsResponse {
            player_uuid: player_uuid.to_string(),
            display_name: display_name.to_string(),
            summary: PlayerStatsSummary {
                games_played,
                wins,
                win_rate: ratio(wins, games_played),
                current_win_streak,
                best_win_streak,
            },
            play_style: build_in_memory_play_style(&filtered_games, player_uuid),
            splits: build_split_summary(&all_games, player_uuid),
            recent_form: build_recent_form_from_games(&filtered_games, player_uuid),
        }))
    }

    async fn get_recent_games_for_player(
        &self,
        player_uuid: &str,
        limit: u32,
        before: Option<DateTime<Utc>>,
        filter: StatsGameFilter,
    ) -> Result<PlayerRecentGamesResponse, StatsError> {
        let before = before.unwrap_or_else(Utc::now);
        let completed_games = self.completed_games.read().await;
        let mut relevant_games: Vec<_> = completed_games
            .games
            .values()
            .filter(|game| {
                game.completed_at < before
                    && filter.matches_game(game.had_bots)
                    && game.players.iter().any(|p| p.uuid == player_uuid)
            })
            .cloned()
            .collect();
        relevant_games.sort_by_key(|game| std::cmp::Reverse(game.completed_at));

        let selected: Vec<_> = relevant_games.into_iter().take(limit as usize).collect();
        let next_before = selected.last().map(|game| game.completed_at);

        let games = selected
            .into_iter()
            .filter_map(|game| {
                let player = game.players.iter().find(|p| p.uuid == player_uuid)?;
                Some(PlayerRecentGameSummary {
                    game_id: game.game_id.clone(),
                    completed_at: game.completed_at,
                    winner_uuid: game.winner_uuid.clone(),
                    cards_remaining: player.cards_remaining,
                    final_score: player.final_score,
                    had_bots: game.had_bots,
                    opponents: game
                        .players
                        .iter()
                        .filter(|p| p.uuid != player_uuid)
                        .map(|p| GameOpponentSummary {
                            player_uuid: p.uuid.clone(),
                            display_name: Some(p.display_name.clone()),
                            won: p.won,
                            is_bot: crate::bot::types::BotPlayer::is_bot_uuid(&p.uuid),
                        })
                        .collect(),
                })
            })
            .collect();

        Ok(PlayerRecentGamesResponse { games, next_before })
    }

    async fn get_completed_game_for_player(
        &self,
        player_uuid: &str,
        game_id: &str,
    ) -> Result<Option<CompletedGameDetailResponse>, StatsError> {
        let completed_games = self.completed_games.read().await;
        let Some(game) = completed_games.games.get(game_id) else {
            return Ok(None);
        };
        if !game.players.iter().any(|player| player.uuid == player_uuid) {
            return Ok(None);
        }

        Ok(Some(build_game_detail_response(game)))
    }
}

pub struct PostgresGameHistoryRepository {
    pool: PgPool,
}

#[derive(Debug, Clone)]
struct PlayerHistoryRow {
    won: bool,
    passes: u64,
    completed_at: DateTime<Utc>,
    had_bots: bool,
}

impl PostgresGameHistoryRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn get_play_type_breakdown_from_moves(
        &self,
        player_uuid: &str,
        filter: StatsGameFilter,
    ) -> Result<PlayTypeBreakdown, StatsError> {
        let row = sqlx::query(
            r#"
            SELECT
                COALESCE(SUM(CASE WHEN jsonb_array_length(cards) = 1 THEN 1 ELSE 0 END), 0) AS total_single_plays,
                COALESCE(SUM(CASE WHEN jsonb_array_length(cards) = 2 THEN 1 ELSE 0 END), 0) AS total_pair_plays,
                COALESCE(SUM(CASE WHEN jsonb_array_length(cards) = 3 THEN 1 ELSE 0 END), 0) AS total_triple_plays,
                COALESCE(SUM(CASE WHEN jsonb_array_length(cards) = 5 THEN 1 ELSE 0 END), 0) AS total_five_card_plays
            FROM completed_game_moves
            JOIN completed_games ON completed_games.game_id = completed_game_moves.game_id
            WHERE player_uuid = $1
              AND action_type = 'play'
              AND ($2::boolean IS NULL OR completed_games.had_bots = $2)
            "#,
        )
        .bind(player_uuid)
        .bind(filter.as_optional_had_bots())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| StatsError::Repository(e.to_string()))?;

        Ok(PlayTypeBreakdown {
            single_plays: row.get::<i64, _>("total_single_plays") as u64,
            pair_plays: row.get::<i64, _>("total_pair_plays") as u64,
            triple_plays: row.get::<i64, _>("total_triple_plays") as u64,
            five_card_plays: row.get::<i64, _>("total_five_card_plays") as u64,
        })
    }

    async fn fetch_player_history_rows(
        &self,
        player_uuid: &str,
        filter: StatsGameFilter,
    ) -> Result<Vec<PlayerHistoryRow>, StatsError> {
        let rows = sqlx::query(
            r#"
            SELECT
                cgp.won,
                cgp.passes,
                cg.completed_at,
                cg.had_bots
            FROM completed_game_players cgp
            JOIN completed_games cg ON cg.game_id = cgp.game_id
            WHERE cgp.player_uuid = $1
              AND ($2::boolean IS NULL OR cg.had_bots = $2)
            ORDER BY cg.completed_at DESC
            "#,
        )
        .bind(player_uuid)
        .bind(filter.as_optional_had_bots())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StatsError::Repository(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|row| PlayerHistoryRow {
                won: row.get("won"),
                passes: row.get::<i64, _>("passes") as u64,
                completed_at: row.get("completed_at"),
                had_bots: row.get("had_bots"),
            })
            .collect())
    }
}

#[async_trait]
impl GameHistoryRepository for PostgresGameHistoryRepository {
    async fn record_completed_game(&self, game_result: &GameResult) -> Result<bool, StatsError> {
        debug!(game_id = %game_result.game_id, "Persisting completed game history");
        let play_breakdown_by_player = calculate_play_type_breakdown(game_result);

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| StatsError::Repository(e.to_string()))?;

        let insert_result = sqlx::query(
            r#"
            INSERT INTO completed_games (
                game_id,
                room_id,
                game_number,
                winner_uuid,
                started_at,
                completed_at,
                had_bots
            ) VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
        )
        .bind(&game_result.game_id)
        .bind(&game_result.room_id)
        .bind(game_result.game_number as i32)
        .bind(&game_result.winner_uuid)
        .bind(game_result.started_at)
        .bind(game_result.completed_at)
        .bind(game_result.had_bots)
        .execute(&mut *tx)
        .await;

        match insert_result {
            Ok(_) => {}
            Err(sqlx::Error::Database(err)) if err.code().as_deref() == Some("23505") => {
                return Ok(false);
            }
            Err(e) => {
                warn!(error = %e, game_id = %game_result.game_id, "Failed to insert completed game");
                return Err(StatsError::Repository(e.to_string()));
            }
        }

        for player in &game_result.players {
            sqlx::query(
                r#"
                INSERT INTO completed_game_players (
                    game_id,
                    player_uuid,
                    display_name,
                    won,
                    cards_remaining,
                    raw_score,
                    final_score,
                    turns_taken,
                    passes,
                    plays,
                    cards_played,
                    started_first,
                    had_bots,
                    completed_at
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
                "#,
            )
            .bind(&game_result.game_id)
            .bind(&player.uuid)
            .bind(&player.display_name)
            .bind(player.won)
            .bind(player.cards_remaining as i16)
            .bind(player.raw_score)
            .bind(player.final_score)
            .bind(player.turns_taken as i64)
            .bind(player.passes as i64)
            .bind(player.plays as i64)
            .bind(player.cards_played as i64)
            .bind(player.started_first)
            .bind(game_result.had_bots)
            .bind(game_result.completed_at)
            .execute(&mut *tx)
            .await
            .map_err(|e| StatsError::Repository(e.to_string()))?;
        }

        for mv in &game_result.moves {
            let action = match mv.action {
                crate::game::MoveAction::Pass => "pass",
                crate::game::MoveAction::Play => "play",
            };
            let cards_json = serde_json::to_value(&mv.cards)
                .map_err(|e| StatsError::Repository(e.to_string()))?;

            sqlx::query(
                r#"
                INSERT INTO completed_game_moves (
                    game_id,
                    sequence,
                    player_uuid,
                    action_type,
                    cards
                ) VALUES ($1, $2, $3, $4, $5)
                "#,
            )
            .bind(&game_result.game_id)
            .bind(mv.sequence as i64)
            .bind(&mv.player_uuid)
            .bind(action)
            .bind(cards_json)
            .execute(&mut *tx)
            .await
            .map_err(|e| StatsError::Repository(e.to_string()))?;
        }

        for player in &game_result.players {
            let play_breakdown = play_breakdown_by_player
                .get(&player.uuid)
                .copied()
                .unwrap_or_default();
            sqlx::query(
                r#"
                INSERT INTO player_profile_stats (
                    player_uuid,
                    games_played,
                    wins,
                    current_win_streak,
                    best_win_streak,
                    total_turns,
                    total_passes,
                    total_plays,
                    total_cards_played,
                    total_single_plays,
                    total_pair_plays,
                    total_triple_plays,
                    total_five_card_plays,
                    human_games_played,
                    human_wins,
                    bot_games_played,
                    bot_wins,
                    updated_at
                ) VALUES (
                    $1,
                    1,
                    $2,
                    $2,
                    $2,
                    $3,
                    $4,
                    $5,
                    $6,
                    $7,
                    $8,
                    $9,
                    $10,
                    $11,
                    $12,
                    $13,
                    $14,
                    NOW()
                )
                ON CONFLICT (player_uuid) DO UPDATE SET
                    games_played = player_profile_stats.games_played + 1,
                    wins = player_profile_stats.wins + EXCLUDED.wins,
                    current_win_streak = CASE
                        WHEN EXCLUDED.wins = 1 THEN player_profile_stats.current_win_streak + 1
                        ELSE 0
                    END,
                    best_win_streak = GREATEST(
                        player_profile_stats.best_win_streak,
                        CASE
                            WHEN EXCLUDED.wins = 1 THEN player_profile_stats.current_win_streak + 1
                            ELSE player_profile_stats.best_win_streak
                        END
                    ),
                    total_turns = player_profile_stats.total_turns + EXCLUDED.total_turns,
                    total_passes = player_profile_stats.total_passes + EXCLUDED.total_passes,
                    total_plays = player_profile_stats.total_plays + EXCLUDED.total_plays,
                    total_cards_played = player_profile_stats.total_cards_played + EXCLUDED.total_cards_played,
                    total_single_plays = player_profile_stats.total_single_plays + EXCLUDED.total_single_plays,
                    total_pair_plays = player_profile_stats.total_pair_plays + EXCLUDED.total_pair_plays,
                    total_triple_plays = player_profile_stats.total_triple_plays + EXCLUDED.total_triple_plays,
                    total_five_card_plays = player_profile_stats.total_five_card_plays + EXCLUDED.total_five_card_plays,
                    human_games_played = player_profile_stats.human_games_played + EXCLUDED.human_games_played,
                    human_wins = player_profile_stats.human_wins + EXCLUDED.human_wins,
                    bot_games_played = player_profile_stats.bot_games_played + EXCLUDED.bot_games_played,
                    bot_wins = player_profile_stats.bot_wins + EXCLUDED.bot_wins,
                    updated_at = NOW()
                "#,
            )
            .bind(&player.uuid)
            .bind(if player.won { 1_i64 } else { 0_i64 })
            .bind(player.turns_taken as i64)
            .bind(player.passes as i64)
            .bind(player.plays as i64)
            .bind(player.cards_played as i64)
            .bind(play_breakdown.single_plays as i64)
            .bind(play_breakdown.pair_plays as i64)
            .bind(play_breakdown.triple_plays as i64)
            .bind(play_breakdown.five_card_plays as i64)
            .bind(if game_result.had_bots { 0_i64 } else { 1_i64 })
            .bind(if !game_result.had_bots && player.won {
                1_i64
            } else {
                0_i64
            })
            .bind(if game_result.had_bots { 1_i64 } else { 0_i64 })
            .bind(if game_result.had_bots && player.won {
                1_i64
            } else {
                0_i64
            })
            .execute(&mut *tx)
            .await
            .map_err(|e| StatsError::Repository(e.to_string()))?;
        }

        tx.commit()
            .await
            .map_err(|e| StatsError::Repository(e.to_string()))?;

        Ok(true)
    }

    async fn get_player_profile_stats(
        &self,
        player_uuid: &str,
        display_name: &str,
        filter: StatsGameFilter,
    ) -> Result<Option<PlayerProfileStatsResponse>, StatsError> {
        let history_rows = self
            .fetch_player_history_rows(player_uuid, StatsGameFilter::All)
            .await?;
        if history_rows.is_empty() {
            return Ok(None);
        }

        let filtered_history: Vec<_> = history_rows
            .iter()
            .filter(|row| filter.matches_game(row.had_bots))
            .cloned()
            .collect();
        let wins = filtered_history.iter().filter(|row| row.won).count() as u64;
        let games_played = filtered_history.len() as u64;
        let total_passes = filtered_history.iter().map(|row| row.passes).sum();
        let (current_win_streak, best_win_streak) =
            build_streaks_from_history_rows(&filtered_history);
        let play_breakdown = self
            .get_play_type_breakdown_from_moves(player_uuid, filter)
            .await?;

        Ok(Some(PlayerProfileStatsResponse {
            player_uuid: player_uuid.to_string(),
            display_name: display_name.to_string(),
            summary: PlayerStatsSummary {
                games_played,
                wins,
                win_rate: ratio(wins, games_played),
                current_win_streak,
                best_win_streak,
            },
            play_style: PlayerPlayStyle {
                total_passes,
                total_single_plays: play_breakdown.single_plays,
                total_pair_plays: play_breakdown.pair_plays,
                total_triple_plays: play_breakdown.triple_plays,
                total_five_card_plays: play_breakdown.five_card_plays,
            },
            splits: build_split_summary_from_history_rows(&history_rows),
            recent_form: build_recent_form_from_history_rows(&filtered_history),
        }))
    }

    async fn get_recent_games_for_player(
        &self,
        player_uuid: &str,
        limit: u32,
        before: Option<DateTime<Utc>>,
        filter: StatsGameFilter,
    ) -> Result<PlayerRecentGamesResponse, StatsError> {
        let before = before.unwrap_or_else(Utc::now);
        let rows = sqlx::query(
            r#"
            SELECT
                cgp.game_id,
                cgp.cards_remaining,
                cgp.final_score,
                cg.winner_uuid,
                cg.completed_at,
                cg.had_bots
            FROM completed_game_players cgp
            JOIN completed_games cg ON cg.game_id = cgp.game_id
            WHERE cgp.player_uuid = $1
              AND cg.completed_at < $2
              AND ($3::boolean IS NULL OR cg.had_bots = $3)
            ORDER BY cg.completed_at DESC
            LIMIT $4
            "#,
        )
        .bind(player_uuid)
        .bind(before)
        .bind(filter.as_optional_had_bots())
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StatsError::Repository(e.to_string()))?;

        let mut games = Vec::with_capacity(rows.len());
        let mut next_before = None;
        for row in rows {
            let game_id = row.get::<String, _>("game_id");
            let completed_at = row.get::<DateTime<Utc>, _>("completed_at");
            let opponent_rows = sqlx::query(
                r#"
                SELECT player_uuid, display_name, won
                FROM completed_game_players
                WHERE game_id = $1 AND player_uuid <> $2
                ORDER BY player_uuid ASC
                "#,
            )
            .bind(&game_id)
            .bind(player_uuid)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StatsError::Repository(e.to_string()))?;

            let opponents = opponent_rows
                .into_iter()
                .map(|opponent| GameOpponentSummary {
                    player_uuid: opponent.get("player_uuid"),
                    display_name: opponent.get("display_name"),
                    won: opponent.get("won"),
                    is_bot: crate::bot::types::BotPlayer::is_bot_uuid(
                        &opponent.get::<String, _>("player_uuid"),
                    ),
                })
                .collect();

            next_before = Some(completed_at);
            games.push(PlayerRecentGameSummary {
                game_id,
                completed_at,
                winner_uuid: row.get("winner_uuid"),
                cards_remaining: row.get::<i16, _>("cards_remaining") as u8,
                final_score: row.get("final_score"),
                had_bots: row.get("had_bots"),
                opponents,
            });
        }

        Ok(PlayerRecentGamesResponse { games, next_before })
    }

    async fn get_completed_game_for_player(
        &self,
        player_uuid: &str,
        game_id: &str,
    ) -> Result<Option<CompletedGameDetailResponse>, StatsError> {
        let membership = sqlx::query(
            "SELECT 1 FROM completed_game_players WHERE game_id = $1 AND player_uuid = $2",
        )
        .bind(game_id)
        .bind(player_uuid)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StatsError::Repository(e.to_string()))?;

        if membership.is_none() {
            return Ok(None);
        }

        let game_row = sqlx::query(
            r#"
            SELECT game_id, room_id, game_number, winner_uuid, started_at, completed_at, had_bots
            FROM completed_games
            WHERE game_id = $1
            "#,
        )
        .bind(game_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StatsError::Repository(e.to_string()))?;

        let Some(game_row) = game_row else {
            return Ok(None);
        };

        let player_rows = sqlx::query(
            r#"
            SELECT
                player_uuid,
                display_name,
                won,
                cards_remaining,
                raw_score,
                final_score,
                turns_taken,
                passes,
                plays,
                cards_played,
                started_first
            FROM completed_game_players
            WHERE game_id = $1
            ORDER BY player_uuid ASC
            "#,
        )
        .bind(game_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StatsError::Repository(e.to_string()))?;

        let move_rows = sqlx::query(
            r#"
            SELECT
                cgm.sequence,
                cgm.player_uuid,
                cgp.display_name,
                cgm.action_type,
                cgm.cards
            FROM completed_game_moves cgm
            LEFT JOIN completed_game_players cgp
                ON cgp.game_id = cgm.game_id
               AND cgp.player_uuid = cgm.player_uuid
            WHERE cgm.game_id = $1
            ORDER BY cgm.sequence ASC
            "#,
        )
        .bind(game_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StatsError::Repository(e.to_string()))?;

        Ok(Some(CompletedGameDetailResponse {
            game_id: game_row.get("game_id"),
            room_id: game_row.get("room_id"),
            game_number: game_row.get::<i32, _>("game_number") as u32,
            winner_uuid: game_row.get("winner_uuid"),
            started_at: game_row.get("started_at"),
            completed_at: game_row.get("completed_at"),
            had_bots: game_row.get("had_bots"),
            players: player_rows
                .into_iter()
                .map(|row| CompletedGameDetailPlayer {
                    player_uuid: row.get("player_uuid"),
                    display_name: row.get("display_name"),
                    won: row.get("won"),
                    is_bot: crate::bot::types::BotPlayer::is_bot_uuid(
                        &row.get::<String, _>("player_uuid"),
                    ),
                    cards_remaining: row.get::<i16, _>("cards_remaining") as u8,
                    raw_score: row.get("raw_score"),
                    final_score: row.get("final_score"),
                    turns_taken: row.get::<i64, _>("turns_taken") as u32,
                    passes: row.get::<i64, _>("passes") as u32,
                    plays: row.get::<i64, _>("plays") as u32,
                    cards_played: row.get::<i64, _>("cards_played") as u32,
                    started_first: row.get("started_first"),
                })
                .collect(),
            moves: move_rows
                .into_iter()
                .map(|row| CompletedGameDetailMove {
                    sequence: row.get::<i64, _>("sequence") as u32,
                    player_uuid: row.get("player_uuid"),
                    display_name: row.get("display_name"),
                    is_bot: crate::bot::types::BotPlayer::is_bot_uuid(
                        &row.get::<String, _>("player_uuid"),
                    ),
                    action: match row.get::<String, _>("action_type").as_str() {
                        "pass" => crate::game::MoveAction::Pass,
                        _ => crate::game::MoveAction::Play,
                    },
                    cards: serde_json::from_value(row.get("cards")).unwrap_or_else(|_| Vec::new()),
                })
                .collect(),
        }))
    }
}

fn build_game_detail_response(game: &GameResult) -> CompletedGameDetailResponse {
    let display_names: HashMap<String, String> = game
        .players
        .iter()
        .map(|player| (player.uuid.clone(), player.display_name.clone()))
        .collect();

    CompletedGameDetailResponse {
        game_id: game.game_id.clone(),
        room_id: game.room_id.clone(),
        game_number: game.game_number,
        winner_uuid: game.winner_uuid.clone(),
        started_at: game.started_at,
        completed_at: game.completed_at,
        had_bots: game.had_bots,
        players: game
            .players
            .iter()
            .map(|player| CompletedGameDetailPlayer {
                player_uuid: player.uuid.clone(),
                display_name: Some(player.display_name.clone()),
                won: player.won,
                is_bot: crate::bot::types::BotPlayer::is_bot_uuid(&player.uuid),
                cards_remaining: player.cards_remaining,
                raw_score: player.raw_score,
                final_score: player.final_score,
                turns_taken: player.turns_taken,
                passes: player.passes,
                plays: player.plays,
                cards_played: player.cards_played,
                started_first: player.started_first,
            })
            .collect(),
        moves: game
            .moves
            .iter()
            .map(|mv| CompletedGameDetailMove {
                sequence: mv.sequence,
                player_uuid: mv.player_uuid.clone(),
                display_name: display_names.get(&mv.player_uuid).cloned(),
                is_bot: crate::bot::types::BotPlayer::is_bot_uuid(&mv.player_uuid),
                action: mv.action,
                cards: mv.cards.clone(),
            })
            .collect(),
    }
}

impl StatsGameFilter {
    fn as_optional_had_bots(self) -> Option<bool> {
        match self {
            Self::All => None,
            Self::HumanOnly => Some(false),
            Self::WithBots => Some(true),
        }
    }
}

fn calculate_play_type_breakdown(game_result: &GameResult) -> HashMap<String, PlayTypeBreakdown> {
    let mut breakdown_by_player: HashMap<String, PlayTypeBreakdown> = HashMap::new();

    for mv in &game_result.moves {
        if !matches!(mv.action, crate::game::MoveAction::Play) {
            continue;
        }

        let breakdown = breakdown_by_player
            .entry(mv.player_uuid.clone())
            .or_default();
        match mv.cards.len() {
            1 => breakdown.single_plays += 1,
            2 => breakdown.pair_plays += 1,
            3 => breakdown.triple_plays += 1,
            5 => breakdown.five_card_plays += 1,
            _ => {}
        }
    }

    breakdown_by_player
}

fn build_split_summary(all_games: &[GameResult], player_uuid: &str) -> PlayerStatsSplits {
    let human_games_played = all_games.iter().filter(|game| !game.had_bots).count() as u64;
    let human_wins = all_games
        .iter()
        .filter(|game| !game.had_bots && game.winner_uuid == player_uuid)
        .count() as u64;
    let bot_games_played = all_games.iter().filter(|game| game.had_bots).count() as u64;
    let bot_wins = all_games
        .iter()
        .filter(|game| game.had_bots && game.winner_uuid == player_uuid)
        .count() as u64;

    PlayerStatsSplits {
        human_only: PlayerSplitSummary {
            games_played: human_games_played,
            wins: human_wins,
            win_rate: ratio(human_wins, human_games_played),
        },
        with_bots: PlayerSplitSummary {
            games_played: bot_games_played,
            wins: bot_wins,
            win_rate: ratio(bot_wins, bot_games_played),
        },
    }
}

fn build_split_summary_from_history_rows(rows: &[PlayerHistoryRow]) -> PlayerStatsSplits {
    let human_games_played = rows.iter().filter(|row| !row.had_bots).count() as u64;
    let human_wins = rows.iter().filter(|row| !row.had_bots && row.won).count() as u64;
    let bot_games_played = rows.iter().filter(|row| row.had_bots).count() as u64;
    let bot_wins = rows.iter().filter(|row| row.had_bots && row.won).count() as u64;

    PlayerStatsSplits {
        human_only: PlayerSplitSummary {
            games_played: human_games_played,
            wins: human_wins,
            win_rate: ratio(human_wins, human_games_played),
        },
        with_bots: PlayerSplitSummary {
            games_played: bot_games_played,
            wins: bot_wins,
            win_rate: ratio(bot_wins, bot_games_played),
        },
    }
}

fn build_recent_form_from_games(
    filtered_games: &[GameResult],
    player_uuid: &str,
) -> PlayerRecentForm {
    let recent_desc: Vec<_> = filtered_games.iter().rev().collect();
    let wins_last_10 = recent_desc
        .iter()
        .take(10)
        .filter(|game| game.winner_uuid == player_uuid)
        .count() as u64;
    let wins_last_25 = recent_desc
        .iter()
        .take(25)
        .filter(|game| game.winner_uuid == player_uuid)
        .count() as u64;

    PlayerRecentForm {
        last_10: PlayerRecentWindow {
            wins: wins_last_10,
            win_rate: ratio(wins_last_10, recent_desc.len().min(10) as u64),
        },
        last_25: PlayerRecentWindow {
            wins: wins_last_25,
            win_rate: ratio(wins_last_25, recent_desc.len().min(25) as u64),
        },
    }
}

fn build_recent_form_from_history_rows(rows: &[PlayerHistoryRow]) -> PlayerRecentForm {
    let wins_last_10 = rows.iter().take(10).filter(|row| row.won).count() as u64;
    let wins_last_25 = rows.iter().take(25).filter(|row| row.won).count() as u64;

    PlayerRecentForm {
        last_10: PlayerRecentWindow {
            wins: wins_last_10,
            win_rate: ratio(wins_last_10, rows.len().min(10) as u64),
        },
        last_25: PlayerRecentWindow {
            wins: wins_last_25,
            win_rate: ratio(wins_last_25, rows.len().min(25) as u64),
        },
    }
}

fn build_streaks_from_games(filtered_games: &[GameResult], player_uuid: &str) -> (u64, u64) {
    let wins_by_game: Vec<_> = filtered_games
        .iter()
        .map(|game| game.winner_uuid == player_uuid)
        .collect();
    build_streaks_from_results(&wins_by_game)
}

fn build_streaks_from_history_rows(rows: &[PlayerHistoryRow]) -> (u64, u64) {
    let mut ordered_rows = rows.to_vec();
    ordered_rows.sort_by_key(|row| row.completed_at);
    let wins_by_game: Vec<_> = ordered_rows.iter().map(|row| row.won).collect();
    build_streaks_from_results(&wins_by_game)
}

fn build_streaks_from_results(results: &[bool]) -> (u64, u64) {
    let mut current = 0_u64;
    let mut best = 0_u64;

    for won in results {
        if *won {
            current += 1;
            best = best.max(current);
        } else {
            current = 0;
        }
    }

    (current, best)
}

fn build_in_memory_play_style(filtered_games: &[GameResult], player_uuid: &str) -> PlayerPlayStyle {
    let mut play_style = PlayerPlayStyle::default();

    for game in filtered_games {
        if let Some(player) = game
            .players
            .iter()
            .find(|player| player.uuid == player_uuid)
        {
            play_style.total_passes += player.passes as u64;
        }

        let play_breakdown = calculate_play_type_breakdown(game);
        if let Some(breakdown) = play_breakdown.get(player_uuid) {
            play_style.total_single_plays += breakdown.single_plays;
            play_style.total_pair_plays += breakdown.pair_plays;
            play_style.total_triple_plays += breakdown.triple_plays;
            play_style.total_five_card_plays += breakdown.five_card_plays;
        }
    }

    play_style
}

fn ratio(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_game(
        room_id: &str,
        winner_uuid: &str,
        players: Vec<(String, u8, i32, i32)>,
    ) -> GameResult {
        GameResult {
            game_id: format!("{room_id}-1"),
            room_id: room_id.to_string(),
            game_number: 1,
            winner_uuid: winner_uuid.to_string(),
            players: players
                .into_iter()
                .map(
                    |(uuid, cards_remaining, raw_score, final_score)| PlayerGameResult {
                        uuid: uuid.clone(),
                        display_name: uuid.clone(),
                        won: uuid == winner_uuid,
                        cards_remaining,
                        raw_score,
                        final_score,
                        turns_taken: 0,
                        passes: 0,
                        plays: 0,
                        cards_played: 0,
                        started_first: false,
                    },
                )
                .collect(),
            moves: vec![],
            started_at: Utc::now(),
            completed_at: Utc::now(),
            had_bots: false,
        }
    }

    #[tokio::test]
    async fn records_game_and_updates_stats() {
        let repo = InMemoryStatsRepository::new();
        let game = sample_game(
            "room-1",
            "player-1",
            vec![
                ("player-1".to_string(), 0, 0, 0),
                ("player-2".to_string(), 5, 5, 5),
            ],
        );

        repo.record_game(game).await.unwrap();

        let stats = repo.get_room_stats("room-1").await.unwrap().unwrap();
        assert_eq!(stats.games_played, 1);
        assert_eq!(stats.player_stats.len(), 2);

        let winner = stats.player_stats.get("player-1").unwrap();
        assert_eq!(winner.wins, 1);
        assert_eq!(winner.cards_remaining, 0);
        assert_eq!(winner.current_win_streak, 1);
        assert_eq!(winner.best_win_streak, 1);

        let loser = stats.player_stats.get("player-2").unwrap();
        assert_eq!(loser.wins, 0);
        assert_eq!(loser.cards_remaining, 5);
        assert_eq!(loser.current_win_streak, 0);
        assert_eq!(loser.total_score, 5);

        let second_game = sample_game(
            "room-1",
            "player-1",
            vec![
                ("player-1".to_string(), 0, 0, 0),
                ("player-2".to_string(), 3, 3, 3),
            ],
        );

        repo.record_game(second_game).await.unwrap();

        let stats = repo.get_room_stats("room-1").await.unwrap().unwrap();
        let loser = stats.player_stats.get("player-2").unwrap();
        assert_eq!(stats.games_played, 2);
        assert_eq!(loser.cards_remaining, 8);
        assert_eq!(loser.total_score, 8);
    }

    #[tokio::test]
    async fn in_memory_history_is_idempotent() {
        let repo = InMemoryGameHistoryRepository::new();
        let game = sample_game(
            "room-1",
            "player-1",
            vec![
                ("player-1".to_string(), 0, 0, 0),
                ("player-2".to_string(), 5, 5, 5),
            ],
        );

        assert!(repo.record_completed_game(&game).await.unwrap());
        assert!(!repo.record_completed_game(&game).await.unwrap());

        let stats = repo
            .get_player_profile_stats("player-1", "Player 1", StatsGameFilter::All)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stats.summary.games_played, 1);
        assert_eq!(stats.summary.wins, 1);
    }

    #[tokio::test]
    async fn in_memory_history_tracks_play_breakdown_totals() {
        let repo = InMemoryGameHistoryRepository::new();
        let mut game = sample_game(
            "room-1",
            "player-1",
            vec![
                ("player-1".to_string(), 0, 0, 0),
                ("player-2".to_string(), 5, 5, 5),
            ],
        );

        game.players[0].turns_taken = 5;
        game.players[0].passes = 1;
        game.players[0].plays = 4;
        game.players[0].cards_played = 11;
        game.moves = vec![
            crate::stats::GameMoveResult {
                sequence: 1,
                player_uuid: "player-1".to_string(),
                action: crate::game::MoveAction::Play,
                cards: vec![crate::game::Card::from_string("3D").unwrap()],
            },
            crate::stats::GameMoveResult {
                sequence: 2,
                player_uuid: "player-1".to_string(),
                action: crate::game::MoveAction::Play,
                cards: vec![
                    crate::game::Card::from_string("4D").unwrap(),
                    crate::game::Card::from_string("4C").unwrap(),
                ],
            },
            crate::stats::GameMoveResult {
                sequence: 3,
                player_uuid: "player-1".to_string(),
                action: crate::game::MoveAction::Play,
                cards: vec![
                    crate::game::Card::from_string("5D").unwrap(),
                    crate::game::Card::from_string("5C").unwrap(),
                    crate::game::Card::from_string("5H").unwrap(),
                ],
            },
            crate::stats::GameMoveResult {
                sequence: 4,
                player_uuid: "player-1".to_string(),
                action: crate::game::MoveAction::Play,
                cards: vec![
                    crate::game::Card::from_string("6D").unwrap(),
                    crate::game::Card::from_string("7D").unwrap(),
                    crate::game::Card::from_string("8D").unwrap(),
                    crate::game::Card::from_string("9D").unwrap(),
                    crate::game::Card::from_string("TD").unwrap(),
                ],
            },
            crate::stats::GameMoveResult {
                sequence: 5,
                player_uuid: "player-2".to_string(),
                action: crate::game::MoveAction::Pass,
                cards: vec![],
            },
        ];

        repo.record_completed_game(&game).await.unwrap();

        let stats = repo
            .get_player_profile_stats("player-1", "Player 1", StatsGameFilter::All)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stats.play_style.total_single_plays, 1);
        assert_eq!(stats.play_style.total_pair_plays, 1);
        assert_eq!(stats.play_style.total_triple_plays, 1);
        assert_eq!(stats.play_style.total_five_card_plays, 1);
        assert_eq!(stats.play_style.total_passes, 1);
    }

    #[tokio::test]
    async fn in_memory_history_evicts_old_games_when_limit_is_reached() {
        let repo = InMemoryGameHistoryRepository::with_limit(1);
        let base_time = Utc::now() - chrono::Duration::minutes(2);
        let first_game = GameResult {
            started_at: base_time - chrono::Duration::seconds(30),
            completed_at: base_time,
            ..sample_game(
                "room-1",
                "player-1",
                vec![
                    ("player-1".to_string(), 0, 0, 0),
                    ("player-2".to_string(), 5, 5, 5),
                ],
            )
        };
        let second_game = GameResult {
            game_id: "room-1:2".to_string(),
            started_at: base_time + chrono::Duration::seconds(30),
            completed_at: base_time + chrono::Duration::minutes(1),
            ..sample_game(
                "room-1",
                "player-2",
                vec![
                    ("player-1".to_string(), 4, 4, 4),
                    ("player-2".to_string(), 0, 0, 0),
                ],
            )
        };

        assert!(repo.record_completed_game(&first_game).await.unwrap());
        assert!(repo.record_completed_game(&second_game).await.unwrap());

        let recent_games = repo
            .get_recent_games_for_player("player-1", 10, None, StatsGameFilter::All)
            .await
            .unwrap();
        assert_eq!(recent_games.games.len(), 1);
        assert_eq!(recent_games.games[0].game_id, second_game.game_id);

        let first_game_detail = repo
            .get_completed_game_for_player("player-1", &first_game.game_id)
            .await
            .unwrap();
        assert!(first_game_detail.is_none());

        let stats = repo
            .get_player_profile_stats("player-1", "Player 1", StatsGameFilter::All)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stats.summary.games_played, 1);
    }

    #[tokio::test]
    async fn in_memory_history_filters_profile_stats_and_recent_games_by_game_type() {
        let repo = InMemoryGameHistoryRepository::new();
        let base_time = Utc::now() - chrono::Duration::minutes(3);

        let human_only_game = GameResult {
            started_at: base_time - chrono::Duration::seconds(30),
            completed_at: base_time,
            ..sample_game(
                "room-human",
                "player-1",
                vec![
                    ("player-1".to_string(), 0, 0, 0),
                    ("player-2".to_string(), 5, 5, 5),
                ],
            )
        };
        let with_bots_game = GameResult {
            game_id: "room-bot:2".to_string(),
            room_id: "room-bot".to_string(),
            game_number: 2,
            started_at: base_time + chrono::Duration::minutes(1) - chrono::Duration::seconds(30),
            completed_at: base_time + chrono::Duration::minutes(1),
            had_bots: true,
            ..sample_game(
                "room-bot",
                "player-2",
                vec![
                    ("player-1".to_string(), 4, 4, 4),
                    ("bot-1".to_string(), 0, 0, 0),
                    ("player-2".to_string(), 0, 0, 0),
                ],
            )
        };

        repo.record_completed_game(&human_only_game).await.unwrap();
        repo.record_completed_game(&with_bots_game).await.unwrap();

        let human_only_stats = repo
            .get_player_profile_stats("player-1", "Player 1", StatsGameFilter::HumanOnly)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(human_only_stats.summary.games_played, 1);
        assert_eq!(human_only_stats.summary.wins, 1);

        let with_bots_stats = repo
            .get_player_profile_stats("player-1", "Player 1", StatsGameFilter::WithBots)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(with_bots_stats.summary.games_played, 1);
        assert_eq!(with_bots_stats.summary.wins, 0);

        let recent_with_bots = repo
            .get_recent_games_for_player("player-1", 10, None, StatsGameFilter::WithBots)
            .await
            .unwrap();
        assert_eq!(recent_with_bots.games.len(), 1);
        assert!(recent_with_bots.games[0].had_bots);
    }

    #[tokio::test]
    async fn in_memory_history_preserves_historical_display_names() {
        let repo = InMemoryGameHistoryRepository::new();
        let game = GameResult {
            game_id: "room-1-1".to_string(),
            room_id: "room-1".to_string(),
            game_number: 1,
            winner_uuid: "player-1".to_string(),
            players: vec![
                PlayerGameResult {
                    uuid: "player-1".to_string(),
                    display_name: "Current Player".to_string(),
                    won: true,
                    cards_remaining: 0,
                    raw_score: 0,
                    final_score: 0,
                    turns_taken: 0,
                    passes: 0,
                    plays: 0,
                    cards_played: 0,
                    started_first: true,
                },
                PlayerGameResult {
                    uuid: "player-2".to_string(),
                    display_name: "Archived Opponent".to_string(),
                    won: false,
                    cards_remaining: 5,
                    raw_score: 5,
                    final_score: 5,
                    turns_taken: 0,
                    passes: 0,
                    plays: 0,
                    cards_played: 0,
                    started_first: false,
                },
            ],
            moves: vec![crate::stats::GameMoveResult {
                sequence: 1,
                player_uuid: "player-2".to_string(),
                action: crate::game::MoveAction::Pass,
                cards: vec![],
            }],
            started_at: Utc::now(),
            completed_at: Utc::now(),
            had_bots: false,
        };

        repo.record_completed_game(&game).await.unwrap();

        let recent = repo
            .get_recent_games_for_player("player-1", 10, None, StatsGameFilter::All)
            .await
            .unwrap();
        assert_eq!(
            recent.games[0].opponents[0].display_name.as_deref(),
            Some("Archived Opponent")
        );

        let detail = repo
            .get_completed_game_for_player("player-1", "room-1-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            detail
                .players
                .iter()
                .find(|player| player.player_uuid == "player-2")
                .and_then(|player| player.display_name.as_deref()),
            Some("Archived Opponent")
        );
        assert_eq!(
            detail.moves[0].display_name.as_deref(),
            Some("Archived Opponent")
        );
    }
}
