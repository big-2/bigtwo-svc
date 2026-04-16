use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::game::{
    cards::Card,
    core::{Game, GameError},
};

#[derive(Debug, Clone)]
pub struct GameMoveUpdate {
    pub game: Game,
    pub player_won: bool,
    pub winning_hand: Option<Vec<Card>>,
}

#[async_trait]
pub trait GameStateRepository: Send + Sync {
    async fn create_game(
        &self,
        room_id: &str,
        player_data: &[(String, String)],
    ) -> Result<Game, GameError>;

    async fn update_game(&self, room_id: &str, game: Game) -> Result<(), GameError>;

    async fn get_game(&self, room_id: &str) -> Result<Option<Game>, GameError>;

    async fn remove_game(&self, room_id: &str) -> Result<Option<Game>, GameError>;

    async fn try_play_move(
        &self,
        room_id: &str,
        player_uuid: &str,
        cards: &[Card],
    ) -> Result<Option<GameMoveUpdate>, GameError>;
}

pub struct GameRepository {
    /// A mapping from room ID to game
    games: Arc<RwLock<HashMap<String, Game>>>,
}

impl Default for GameRepository {
    fn default() -> Self {
        Self::new()
    }
}

impl GameRepository {
    pub fn new() -> Self {
        Self {
            games: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl GameStateRepository for GameRepository {
    async fn create_game(
        &self,
        room_id: &str,
        player_data: &[(String, String)],
    ) -> Result<Game, GameError> {
        let mut games = self.games.write().await;
        let game = Game::new_game(room_id.to_string(), player_data)?;
        games.insert(room_id.to_string(), game.clone());
        Ok(game)
    }

    async fn update_game(&self, room_id: &str, game: Game) -> Result<(), GameError> {
        let mut games = self.games.write().await;
        games.insert(room_id.to_string(), game);
        Ok(())
    }

    async fn get_game(&self, room_id: &str) -> Result<Option<Game>, GameError> {
        let games = self.games.read().await;
        Ok(games.get(room_id).cloned())
    }

    async fn remove_game(&self, room_id: &str) -> Result<Option<Game>, GameError> {
        let mut games = self.games.write().await;
        Ok(games.remove(room_id))
    }

    async fn try_play_move(
        &self,
        room_id: &str,
        player_uuid: &str,
        cards: &[Card],
    ) -> Result<Option<GameMoveUpdate>, GameError> {
        let mut games = self.games.write().await;

        let Some(game) = games.get_mut(room_id) else {
            return Ok(None);
        };

        let player_won = game.play_cards(player_uuid, cards)?;
        let winning_hand = player_won.then(|| game.last_played_cards());
        let updated_game = game.clone();

        Ok(Some(GameMoveUpdate {
            game: updated_game,
            player_won,
            winning_hand,
        }))
    }
}

#[allow(dead_code)] // Used by the binary target when PostgreSQL is configured.
pub struct PostgresGameRepository {
    pool: sqlx::PgPool,
}

#[allow(dead_code)] // Used by the binary target when PostgreSQL is configured.
impl PostgresGameRepository {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    fn serialize_game(game: &Game) -> Result<String, GameError> {
        serde_json::to_string(game).map_err(|e| GameError::StorageError(e.to_string()))
    }

    fn deserialize_game(game_json: String) -> Result<Game, GameError> {
        serde_json::from_str(&game_json).map_err(|e| GameError::StorageError(e.to_string()))
    }
}

#[async_trait]
impl GameStateRepository for PostgresGameRepository {
    async fn create_game(
        &self,
        room_id: &str,
        player_data: &[(String, String)],
    ) -> Result<Game, GameError> {
        let game = Game::new_game(room_id.to_string(), player_data)?;
        self.update_game(room_id, game.clone()).await?;
        Ok(game)
    }

    async fn update_game(&self, room_id: &str, game: Game) -> Result<(), GameError> {
        let game_json = Self::serialize_game(&game)?;

        sqlx::query(
            r#"
            INSERT INTO active_games (room_id, game_state, created_at, updated_at)
            VALUES ($1, $2::jsonb, $3, NOW())
            ON CONFLICT (room_id)
            DO UPDATE SET
                game_state = EXCLUDED.game_state,
                updated_at = NOW()
            "#,
        )
        .bind(room_id)
        .bind(game_json)
        .bind(game.created_at())
        .execute(&self.pool)
        .await
        .map_err(|e| GameError::StorageError(e.to_string()))?;

        Ok(())
    }

    async fn get_game(&self, room_id: &str) -> Result<Option<Game>, GameError> {
        let row = sqlx::query_scalar::<_, String>(
            r#"
            SELECT game_state::text
            FROM active_games
            WHERE room_id = $1
            "#,
        )
        .bind(room_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| GameError::StorageError(e.to_string()))?;

        row.map(Self::deserialize_game).transpose()
    }

    async fn remove_game(&self, room_id: &str) -> Result<Option<Game>, GameError> {
        let row = sqlx::query_scalar::<_, String>(
            r#"
            DELETE FROM active_games
            WHERE room_id = $1
            RETURNING game_state::text
            "#,
        )
        .bind(room_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| GameError::StorageError(e.to_string()))?;

        row.map(Self::deserialize_game).transpose()
    }

    async fn try_play_move(
        &self,
        room_id: &str,
        player_uuid: &str,
        cards: &[Card],
    ) -> Result<Option<GameMoveUpdate>, GameError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| GameError::StorageError(e.to_string()))?;

        let row = sqlx::query_scalar::<_, String>(
            r#"
            SELECT game_state::text
            FROM active_games
            WHERE room_id = $1
            FOR UPDATE
            "#,
        )
        .bind(room_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| GameError::StorageError(e.to_string()))?;

        let Some(game_json) = row else {
            tx.commit()
                .await
                .map_err(|e| GameError::StorageError(e.to_string()))?;
            return Ok(None);
        };

        let mut game = Self::deserialize_game(game_json)?;
        let player_won = game.play_cards(player_uuid, cards)?;
        let winning_hand = player_won.then(|| game.last_played_cards());
        let game_json = Self::serialize_game(&game)?;
        sqlx::query(
            r#"
            UPDATE active_games
            SET game_state = $2::jsonb,
                updated_at = NOW()
            WHERE room_id = $1
            "#,
        )
        .bind(room_id)
        .bind(game_json)
        .execute(&mut *tx)
        .await
        .map_err(|e| GameError::StorageError(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| GameError::StorageError(e.to_string()))?;

        Ok(Some(GameMoveUpdate {
            game,
            player_won,
            winning_hand,
        }))
    }
}
