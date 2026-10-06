-- Post-game feedback collected from the web client.
-- One row per player, enforced by uq_game_feedback_player.

CREATE TABLE IF NOT EXISTS game_feedback (
    id             BIGSERIAL PRIMARY KEY,
    player_uuid    TEXT NOT NULL,
    session_id     TEXT NOT NULL,
    game_id        TEXT REFERENCES completed_games(game_id) ON DELETE SET NULL,
    room_id        TEXT,
    rating         TEXT NOT NULL CHECK (rating IN ('up', 'down')),
    chips          TEXT[] NOT NULL DEFAULT '{}',
    comment        TEXT,
    won            BOOLEAN,
    bot_difficulty TEXT,
    had_bots       BOOLEAN,
    client         TEXT NOT NULL DEFAULT 'web',
    app_version    TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Enforces "one submission per player, ever".
CREATE UNIQUE INDEX IF NOT EXISTS uq_game_feedback_player
    ON game_feedback (player_uuid);

CREATE INDEX IF NOT EXISTS idx_game_feedback_created_at
    ON game_feedback (created_at DESC);

CREATE INDEX IF NOT EXISTS idx_game_feedback_rating_created_at
    ON game_feedback (rating, created_at DESC);
