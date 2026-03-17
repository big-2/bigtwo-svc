CREATE TABLE IF NOT EXISTS player_profile_stats (
    player_uuid VARCHAR(36) PRIMARY KEY,
    games_played BIGINT NOT NULL DEFAULT 0,
    wins BIGINT NOT NULL DEFAULT 0,
    current_win_streak BIGINT NOT NULL DEFAULT 0,
    best_win_streak BIGINT NOT NULL DEFAULT 0,
    total_turns BIGINT NOT NULL DEFAULT 0,
    total_passes BIGINT NOT NULL DEFAULT 0,
    total_plays BIGINT NOT NULL DEFAULT 0,
    total_cards_played BIGINT NOT NULL DEFAULT 0,
    human_games_played BIGINT NOT NULL DEFAULT 0,
    human_wins BIGINT NOT NULL DEFAULT 0,
    bot_games_played BIGINT NOT NULL DEFAULT 0,
    bot_wins BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
