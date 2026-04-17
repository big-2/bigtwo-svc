CREATE TABLE IF NOT EXISTS active_games (
    room_id TEXT PRIMARY KEY,
    game_state JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_active_games_updated_at
    ON active_games (updated_at);
