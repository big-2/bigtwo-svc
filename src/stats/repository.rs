use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
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
        PlayerStatsSplits, PlayerStatsSummary, RoomStats,
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
    ) -> Result<Option<PlayerProfileStatsResponse>, StatsError>;
    async fn get_recent_games_for_player(
        &self,
        player_uuid: &str,
        limit: u32,
        before: Option<DateTime<Utc>>,
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
struct InMemoryProfileAggregate {
    games_played: u64,
    wins: u64,
    current_win_streak: u64,
    best_win_streak: u64,
    total_turns: u64,
    total_passes: u64,
    total_plays: u64,
    total_cards_played: u64,
    human_games_played: u64,
    human_wins: u64,
    bot_games_played: u64,
    bot_wins: u64,
}

#[derive(Debug, Default)]
pub struct InMemoryGameHistoryRepository {
    games: Arc<RwLock<HashMap<String, GameResult>>>,
    profile_aggregates: Arc<RwLock<HashMap<String, InMemoryProfileAggregate>>>,
}

impl InMemoryGameHistoryRepository {
    pub fn new() -> Self {
        Self {
            games: Arc::new(RwLock::new(HashMap::new())),
            profile_aggregates: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl GameHistoryRepository for InMemoryGameHistoryRepository {
    async fn record_completed_game(&self, game_result: &GameResult) -> Result<bool, StatsError> {
        {
            let games = self.games.read().await;
            if games.contains_key(&game_result.game_id) {
                return Ok(false);
            }
        }

        {
            let mut games = self.games.write().await;
            if games.contains_key(&game_result.game_id) {
                return Ok(false);
            }
            games.insert(game_result.game_id.clone(), game_result.clone());
        }

        let mut aggregates = self.profile_aggregates.write().await;
        for player in &game_result.players {
            let aggregate = aggregates.entry(player.uuid.clone()).or_default();
            aggregate.games_played += 1;
            aggregate.wins += u64::from(player.won);
            aggregate.total_turns += player.turns_taken as u64;
            aggregate.total_passes += player.passes as u64;
            aggregate.total_plays += player.plays as u64;
            aggregate.total_cards_played += player.cards_played as u64;

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
    ) -> Result<Option<PlayerProfileStatsResponse>, StatsError> {
        let aggregate = {
            let aggregates = self.profile_aggregates.read().await;
            aggregates.get(player_uuid).cloned()
        };
        let Some(aggregate) = aggregate else {
            return Ok(None);
        };

        let recent_games = self
            .get_recent_games_for_player(player_uuid, 25, None)
            .await?
            .games;

        let wins_last_10 = recent_games
            .iter()
            .take(10)
            .filter(|game| game.winner_uuid == player_uuid)
            .count() as u64;
        let wins_last_25 = recent_games
            .iter()
            .take(25)
            .filter(|game| game.winner_uuid == player_uuid)
            .count() as u64;

        Ok(Some(PlayerProfileStatsResponse {
            player_uuid: player_uuid.to_string(),
            display_name: display_name.to_string(),
            summary: PlayerStatsSummary {
                games_played: aggregate.games_played,
                wins: aggregate.wins,
                win_rate: ratio(aggregate.wins, aggregate.games_played),
                current_win_streak: aggregate.current_win_streak,
                best_win_streak: aggregate.best_win_streak,
            },
            play_style: PlayerPlayStyle {
                total_turns: aggregate.total_turns,
                total_passes: aggregate.total_passes,
                pass_rate: ratio(aggregate.total_passes, aggregate.total_turns),
                total_plays: aggregate.total_plays,
                total_cards_played: aggregate.total_cards_played,
                average_cards_per_play: ratio(aggregate.total_cards_played, aggregate.total_plays),
            },
            splits: PlayerStatsSplits {
                human_only: PlayerSplitSummary {
                    games_played: aggregate.human_games_played,
                    wins: aggregate.human_wins,
                    win_rate: ratio(aggregate.human_wins, aggregate.human_games_played),
                },
                with_bots: PlayerSplitSummary {
                    games_played: aggregate.bot_games_played,
                    wins: aggregate.bot_wins,
                    win_rate: ratio(aggregate.bot_wins, aggregate.bot_games_played),
                },
            },
            recent_form: PlayerRecentForm {
                last_10: PlayerRecentWindow {
                    wins: wins_last_10,
                    win_rate: ratio(wins_last_10, recent_games.len().min(10) as u64),
                },
                last_25: PlayerRecentWindow {
                    wins: wins_last_25,
                    win_rate: ratio(wins_last_25, recent_games.len().min(25) as u64),
                },
            },
        }))
    }

    async fn get_recent_games_for_player(
        &self,
        player_uuid: &str,
        limit: u32,
        before: Option<DateTime<Utc>>,
    ) -> Result<PlayerRecentGamesResponse, StatsError> {
        let before = before.unwrap_or_else(Utc::now);
        let games = self.games.read().await;
        let mut relevant_games: Vec<_> = games
            .values()
            .filter(|game| {
                game.completed_at < before && game.players.iter().any(|p| p.uuid == player_uuid)
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
                            won: p.won,
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
        let games = self.games.read().await;
        let Some(game) = games.get(game_id) else {
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

impl PostgresGameHistoryRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl GameHistoryRepository for PostgresGameHistoryRepository {
    async fn record_completed_game(&self, game_result: &GameResult) -> Result<bool, StatsError> {
        debug!(game_id = %game_result.game_id, "Persisting completed game history");

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
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                "#,
            )
            .bind(&game_result.game_id)
            .bind(&player.uuid)
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
    ) -> Result<Option<PlayerProfileStatsResponse>, StatsError> {
        let row = sqlx::query(
            r#"
            SELECT
                games_played,
                wins,
                current_win_streak,
                best_win_streak,
                total_turns,
                total_passes,
                total_plays,
                total_cards_played,
                human_games_played,
                human_wins,
                bot_games_played,
                bot_wins
            FROM player_profile_stats
            WHERE player_uuid = $1
            "#,
        )
        .bind(player_uuid)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StatsError::Repository(e.to_string()))?;

        let Some(row) = row else {
            return Ok(None);
        };

        let recent_rows = sqlx::query(
            r#"
            SELECT won
            FROM completed_game_players
            WHERE player_uuid = $1
            ORDER BY completed_at DESC
            LIMIT 25
            "#,
        )
        .bind(player_uuid)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StatsError::Repository(e.to_string()))?;

        let games_played = row.get::<i64, _>("games_played") as u64;
        let wins = row.get::<i64, _>("wins") as u64;
        let total_turns = row.get::<i64, _>("total_turns") as u64;
        let total_passes = row.get::<i64, _>("total_passes") as u64;
        let total_plays = row.get::<i64, _>("total_plays") as u64;
        let total_cards_played = row.get::<i64, _>("total_cards_played") as u64;
        let human_games_played = row.get::<i64, _>("human_games_played") as u64;
        let human_wins = row.get::<i64, _>("human_wins") as u64;
        let bot_games_played = row.get::<i64, _>("bot_games_played") as u64;
        let bot_wins = row.get::<i64, _>("bot_wins") as u64;

        let wins_last_10 = recent_rows
            .iter()
            .take(10)
            .filter(|r| r.get::<bool, _>("won"))
            .count() as u64;
        let wins_last_25 = recent_rows
            .iter()
            .take(25)
            .filter(|r| r.get::<bool, _>("won"))
            .count() as u64;

        Ok(Some(PlayerProfileStatsResponse {
            player_uuid: player_uuid.to_string(),
            display_name: display_name.to_string(),
            summary: PlayerStatsSummary {
                games_played,
                wins,
                win_rate: ratio(wins, games_played),
                current_win_streak: row.get::<i64, _>("current_win_streak") as u64,
                best_win_streak: row.get::<i64, _>("best_win_streak") as u64,
            },
            play_style: PlayerPlayStyle {
                total_turns,
                total_passes,
                pass_rate: ratio(total_passes, total_turns),
                total_plays,
                total_cards_played,
                average_cards_per_play: ratio(total_cards_played, total_plays),
            },
            splits: PlayerStatsSplits {
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
            },
            recent_form: PlayerRecentForm {
                last_10: PlayerRecentWindow {
                    wins: wins_last_10,
                    win_rate: ratio(wins_last_10, recent_rows.len().min(10) as u64),
                },
                last_25: PlayerRecentWindow {
                    wins: wins_last_25,
                    win_rate: ratio(wins_last_25, recent_rows.len().min(25) as u64),
                },
            },
        }))
    }

    async fn get_recent_games_for_player(
        &self,
        player_uuid: &str,
        limit: u32,
        before: Option<DateTime<Utc>>,
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
            ORDER BY cg.completed_at DESC
            LIMIT $3
            "#,
        )
        .bind(player_uuid)
        .bind(before)
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
                SELECT player_uuid, won
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
                    won: opponent.get("won"),
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
            SELECT sequence, player_uuid, action_type, cards
            FROM completed_game_moves
            WHERE game_id = $1
            ORDER BY sequence ASC
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
                    won: row.get("won"),
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
                won: player.won,
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
                action: mv.action,
                cards: mv.cards.clone(),
            })
            .collect(),
    }
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
        assert_eq!(winner.current_win_streak, 1);
        assert_eq!(winner.best_win_streak, 1);

        let loser = stats.player_stats.get("player-2").unwrap();
        assert_eq!(loser.wins, 0);
        assert_eq!(loser.current_win_streak, 0);
        assert_eq!(loser.total_score, 5);
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
            .get_player_profile_stats("player-1", "Player 1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stats.summary.games_played, 1);
        assert_eq!(stats.summary.wins, 1);
    }
}
