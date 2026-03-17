CREATE TABLE IF NOT EXISTS completed_games (
    game_id TEXT PRIMARY KEY,
    room_id TEXT NOT NULL,
    game_number INTEGER NOT NULL,
    winner_uuid VARCHAR(36) NOT NULL,
    started_at TIMESTAMPTZ NOT NULL,
    completed_at TIMESTAMPTZ NOT NULL,
    had_bots BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE INDEX IF NOT EXISTS idx_completed_games_room_completed_at
    ON completed_games (room_id, completed_at DESC);

CREATE TABLE IF NOT EXISTS completed_game_players (
    game_id TEXT NOT NULL REFERENCES completed_games(game_id) ON DELETE CASCADE,
    player_uuid VARCHAR(36) NOT NULL,
    placement SMALLINT NOT NULL,
    won BOOLEAN NOT NULL,
    cards_remaining SMALLINT NOT NULL,
    raw_score INTEGER NOT NULL,
    final_score INTEGER NOT NULL,
    turns_taken BIGINT NOT NULL,
    passes BIGINT NOT NULL,
    plays BIGINT NOT NULL,
    cards_played BIGINT NOT NULL,
    started_first BOOLEAN NOT NULL,
    had_bots BOOLEAN NOT NULL DEFAULT FALSE,
    completed_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (game_id, player_uuid)
);

CREATE INDEX IF NOT EXISTS idx_completed_game_players_player_completed_at
    ON completed_game_players (player_uuid, completed_at DESC);

CREATE TABLE IF NOT EXISTS completed_game_moves (
    game_id TEXT NOT NULL REFERENCES completed_games(game_id) ON DELETE CASCADE,
    sequence BIGINT NOT NULL,
    player_uuid VARCHAR(36) NOT NULL,
    action_type TEXT NOT NULL,
    cards JSONB NOT NULL,
    PRIMARY KEY (game_id, sequence)
);

CREATE INDEX IF NOT EXISTS idx_completed_game_moves_player
    ON completed_game_moves (player_uuid);
