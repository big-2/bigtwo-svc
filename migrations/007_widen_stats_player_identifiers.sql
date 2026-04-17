ALTER TABLE completed_games
    ALTER COLUMN winner_uuid TYPE TEXT;

ALTER TABLE completed_game_players
    ALTER COLUMN player_uuid TYPE TEXT;

ALTER TABLE completed_game_moves
    ALTER COLUMN player_uuid TYPE TEXT;

ALTER TABLE player_profile_stats
    ALTER COLUMN player_uuid TYPE TEXT;

ALTER TABLE player_profile_stats
    ADD COLUMN IF NOT EXISTS total_single_plays BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS total_pair_plays BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS total_triple_plays BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS total_five_card_plays BIGINT NOT NULL DEFAULT 0;

ALTER TABLE completed_game_players
    ADD COLUMN IF NOT EXISTS display_name TEXT;

UPDATE player_profile_stats AS pps
SET
    total_single_plays = COALESCE(src.total_single_plays, 0),
    total_pair_plays = COALESCE(src.total_pair_plays, 0),
    total_triple_plays = COALESCE(src.total_triple_plays, 0),
    total_five_card_plays = COALESCE(src.total_five_card_plays, 0)
FROM (
    SELECT
        player_uuid,
        COALESCE(SUM(CASE WHEN jsonb_array_length(cards) = 1 THEN 1 ELSE 0 END), 0) AS total_single_plays,
        COALESCE(SUM(CASE WHEN jsonb_array_length(cards) = 2 THEN 1 ELSE 0 END), 0) AS total_pair_plays,
        COALESCE(SUM(CASE WHEN jsonb_array_length(cards) = 3 THEN 1 ELSE 0 END), 0) AS total_triple_plays,
        COALESCE(SUM(CASE WHEN jsonb_array_length(cards) = 5 THEN 1 ELSE 0 END), 0) AS total_five_card_plays
    FROM completed_game_moves
    WHERE action_type = 'play'
    GROUP BY player_uuid
) AS src
WHERE src.player_uuid = pps.player_uuid
  AND pps.total_plays > 0
  AND pps.total_single_plays = 0
  AND pps.total_pair_plays = 0
  AND pps.total_triple_plays = 0
  AND pps.total_five_card_plays = 0;
