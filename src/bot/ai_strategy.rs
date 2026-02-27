use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, time::Duration};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::game::{Card, Game};

use super::{basic_strategy::BasicBotStrategy, types::BotStrategy};

const DEFAULT_AI_BOT_PREDICT_PATH: &str = "/api/v1/predict";
const DEFAULT_AI_BOT_TIMEOUT_MS: u64 = 1200;

#[derive(Debug, Serialize)]
struct PredictRequest {
    api_version: &'static str,
    request_id: String,
    bot_id: String,
    hand_cards: Vec<String>,
    player_card_counts: Vec<usize>,
    current_player_index: usize,
    last_non_pass_cards: Vec<String>,
    consecutive_passes: usize,
    played_hands_count: usize,
    played_cards_by_player: Vec<Vec<String>>,
    legal_actions: Vec<Vec<String>>,
    allow_pass: bool,
}

#[derive(Debug, Deserialize)]
struct PredictResponse {
    #[serde(default)]
    cards: Vec<String>,
    #[serde(default)]
    pass_move: bool,
}

/// Bot strategy backed by an external HTTP inference service.
///
/// If the service is unavailable or returns an invalid move, this strategy falls
/// back to `BasicBotStrategy` to preserve game progression.
pub struct AiBotStrategy {
    client: Client,
    endpoint: Option<String>,
    fallback_strategy: BasicBotStrategy,
}

impl AiBotStrategy {
    pub fn new() -> Self {
        let endpoint = build_predict_url_from_env();
        let timeout_ms = read_timeout_ms_from_env();
        let client = Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .unwrap_or_else(|e| {
                warn!(
                    error = %e,
                    "Failed to build AI bot HTTP client with custom config, using default client"
                );
                Client::new()
            });

        Self {
            client,
            endpoint,
            fallback_strategy: BasicBotStrategy::new(),
        }
    }

    fn allow_pass(game: &Game) -> bool {
        game.consecutive_passes() < 3 && !game.played_hands().is_empty()
    }

    fn build_request(
        &self,
        game: &Game,
        bot_uuid: &str,
        legal_actions: Vec<Vec<String>>,
        allow_pass: bool,
    ) -> Option<PredictRequest> {
        let bot_player = game
            .players()
            .iter()
            .find(|player| player.uuid == bot_uuid)?;
        let mut hand_cards = bot_player.cards.clone();
        hand_cards.sort();

        let current_player_uuid = game.current_player_turn();
        let current_player_index = game
            .players()
            .iter()
            .position(|player| player.uuid == current_player_uuid)?;

        let player_card_counts: Vec<usize> = game
            .players()
            .iter()
            .map(|player| player.cards.len())
            .collect();

        // Service contract currently assumes 4-player games.
        if player_card_counts.len() != 4 {
            return None;
        }
        let played_cards_by_player = Self::played_cards_by_player(game)?;

        Some(PredictRequest {
            api_version: "v1",
            request_id: Uuid::new_v4().to_string(),
            bot_id: bot_uuid.to_string(),
            hand_cards: hand_cards.iter().map(|card| card.to_string()).collect(),
            player_card_counts,
            current_player_index,
            last_non_pass_cards: game
                .last_non_pass_cards()
                .iter()
                .map(|card| card.to_string())
                .collect(),
            consecutive_passes: game.consecutive_passes(),
            played_hands_count: game.played_hands().len(),
            played_cards_by_player,
            legal_actions,
            allow_pass,
        })
    }

    fn played_cards_by_player(game: &Game) -> Option<Vec<Vec<String>>> {
        game.players()
            .iter()
            .map(|player| {
                let starting_cards = game.starting_hands().get(&player.uuid)?;
                let mut played_cards: Vec<Card> = starting_cards
                    .iter()
                    .copied()
                    .filter(|card| !player.cards.contains(card))
                    .collect();
                played_cards.sort();
                Some(
                    played_cards
                        .into_iter()
                        .map(|card| card.to_string())
                        .collect(),
                )
            })
            .collect()
    }

    async fn predict(&self, request: &PredictRequest) -> Result<PredictResponse, String> {
        let endpoint = self
            .endpoint
            .as_ref()
            .ok_or_else(|| "AI_BOT_SERVICE_URL is not configured".to_string())?;

        let response = self
            .client
            .post(endpoint)
            .json(request)
            .send()
            .await
            .map_err(|e| format!("request error for url ({endpoint}): {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("status={status}, url={endpoint}, body={body}"));
        }

        response
            .json::<PredictResponse>()
            .await
            .map_err(|e| format!("response decode error for url ({endpoint}): {e}"))
    }

    fn move_key(cards: &[Card]) -> String {
        let mut sorted = cards.to_vec();
        sorted.sort();
        sorted
            .iter()
            .map(|card| card.to_string())
            .collect::<Vec<_>>()
            .join(",")
    }

    fn parse_card_strings(card_strings: &[String]) -> Result<Vec<Card>, String> {
        card_strings
            .iter()
            .map(|card| Card::from_string(card).map_err(|_| format!("invalid card: {card}")))
            .collect()
    }

    async fn fallback_move(&self, game: &Game, bot_uuid: &str, reason: &str) -> Option<Vec<Card>> {
        warn!(
            bot_uuid = %bot_uuid,
            reason = %reason,
            "AI bot fallback strategy engaged"
        );
        self.fallback_strategy.decide_move(game, bot_uuid).await
    }
}

impl Default for AiBotStrategy {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BotStrategy for AiBotStrategy {
    async fn decide_move(&self, game: &Game, bot_uuid: &str) -> Option<Vec<Card>> {
        if game.current_player_turn() != bot_uuid {
            debug!(bot_uuid = %bot_uuid, "Not AI bot's turn");
            return None;
        }

        let legal_moves = BasicBotStrategy::find_legal_moves_for_bot_turn(game, bot_uuid);
        let allow_pass = Self::allow_pass(game);
        let mut legal_actions: Vec<Vec<String>> = legal_moves
            .iter()
            .map(|cards| cards.iter().map(|card| card.to_string()).collect())
            .collect();

        if allow_pass {
            legal_actions.push(Vec::new());
        }

        let legal_move_keys: HashSet<String> = legal_moves
            .iter()
            .map(|cards| Self::move_key(cards))
            .collect();

        let request = match self.build_request(game, bot_uuid, legal_actions, allow_pass) {
            Some(request) => request,
            None => {
                return self
                    .fallback_move(game, bot_uuid, "invalid request state")
                    .await
            }
        };

        debug!(
            bot_uuid = %bot_uuid,
            endpoint = %self.endpoint.as_deref().unwrap_or("<unset>"),
            legal_move_count = legal_moves.len(),
            allow_pass = allow_pass,
            "Calling AI bot service for move prediction"
        );

        let response = match self.predict(&request).await {
            Ok(response) => response,
            Err(error) => return self.fallback_move(game, bot_uuid, &error).await,
        };

        if response.pass_move && !response.cards.is_empty() {
            warn!(
                bot_uuid = %bot_uuid,
                cards = ?response.cards,
                "AI service returned contradictory move (pass + cards); validating cards"
            );
        }

        if response.cards.is_empty() {
            if allow_pass {
                info!(bot_uuid = %bot_uuid, "AI bot chose to pass");
                return None;
            }
            return self
                .fallback_move(game, bot_uuid, "AI attempted illegal pass")
                .await;
        }

        let mut cards = match Self::parse_card_strings(&response.cards) {
            Ok(cards) => cards,
            Err(error) => return self.fallback_move(game, bot_uuid, &error).await,
        };
        cards.sort();

        if !legal_move_keys.contains(&Self::move_key(&cards)) {
            return self
                .fallback_move(game, bot_uuid, "AI returned move outside legal action set")
                .await;
        }

        info!(
            bot_uuid = %bot_uuid,
            cards = ?cards,
            "AI bot selected move from remote service"
        );
        Some(cards)
    }

    fn strategy_name(&self) -> &'static str {
        "AiBotStrategy"
    }
}

fn build_predict_url_from_env() -> Option<String> {
    build_predict_url(
        std::env::var("AI_BOT_SERVICE_URL").ok().as_deref(),
        std::env::var("AI_BOT_SERVICE_PREDICT_PATH").ok().as_deref(),
    )
}

fn build_predict_url(base_url_env: Option<&str>, path_env: Option<&str>) -> Option<String> {
    let base_url = base_url_env.map(str::trim).filter(|url| !url.is_empty())?;
    let predict_path = normalize_predict_path(path_env);
    Some(format!(
        "{}{}",
        base_url.trim_end_matches('/'),
        predict_path
    ))
}

fn normalize_predict_path(path_env: Option<&str>) -> String {
    let predict_path = path_env
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_AI_BOT_PREDICT_PATH);

    if predict_path.starts_with('/') {
        predict_path.to_string()
    } else {
        format!("/{}", predict_path)
    }
}

fn read_timeout_ms_from_env() -> u64 {
    std::env::var("AI_BOT_HTTP_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_AI_BOT_TIMEOUT_MS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{Player, Rank, Suit};
    use std::collections::HashMap;

    fn create_game_with_partial_progress() -> Game {
        let p1_current = vec![Card::new(Rank::Four, Suit::Diamonds)];
        let p2_current = vec![
            Card::new(Rank::Five, Suit::Diamonds),
            Card::new(Rank::Six, Suit::Diamonds),
        ];
        let p3_current = vec![Card::new(Rank::Eight, Suit::Diamonds)];
        let p4_current = vec![];

        let players = vec![
            Player {
                name: "p1".to_string(),
                uuid: "bot-1".to_string(),
                cards: p1_current,
            },
            Player {
                name: "p2".to_string(),
                uuid: "p2".to_string(),
                cards: p2_current,
            },
            Player {
                name: "p3".to_string(),
                uuid: "p3".to_string(),
                cards: p3_current,
            },
            Player {
                name: "p4".to_string(),
                uuid: "p4".to_string(),
                cards: p4_current,
            },
        ];

        let starting_hands = HashMap::from([
            (
                "bot-1".to_string(),
                vec![
                    Card::new(Rank::Three, Suit::Diamonds),
                    Card::new(Rank::Four, Suit::Diamonds),
                ],
            ),
            (
                "p2".to_string(),
                vec![
                    Card::new(Rank::Five, Suit::Diamonds),
                    Card::new(Rank::Six, Suit::Diamonds),
                ],
            ),
            (
                "p3".to_string(),
                vec![
                    Card::new(Rank::Seven, Suit::Diamonds),
                    Card::new(Rank::Eight, Suit::Diamonds),
                ],
            ),
            (
                "p4".to_string(),
                vec![
                    Card::new(Rank::Nine, Suit::Diamonds),
                    Card::new(Rank::Ten, Suit::Diamonds),
                ],
            ),
        ]);

        Game::new("room-1".to_string(), players, 0, 0, vec![], starting_hands)
    }

    #[test]
    fn played_cards_by_player_is_derived_from_starting_hands() {
        let game = create_game_with_partial_progress();

        let played =
            AiBotStrategy::played_cards_by_player(&game).expect("should build played cards");

        assert_eq!(played.len(), 4);
        assert_eq!(played[0], vec!["3D".to_string()]);
        assert!(played[1].is_empty());
        assert_eq!(played[2], vec!["7D".to_string()]);
        assert_eq!(played[3], vec!["9D".to_string(), "TD".to_string()]);
    }

    #[test]
    fn build_request_includes_played_cards_by_player() {
        let game = create_game_with_partial_progress();
        let strategy = AiBotStrategy::new();

        let request = strategy
            .build_request(
                &game,
                "bot-1",
                vec![vec!["4D".to_string()], vec!["3D".to_string()]],
                false,
            )
            .expect("request should be built");

        assert_eq!(request.played_cards_by_player.len(), 4);
        assert_eq!(request.played_cards_by_player[0], vec!["3D".to_string()]);
        assert_eq!(
            request.played_cards_by_player[3],
            vec!["9D".to_string(), "TD".to_string()]
        );
    }

    #[test]
    fn build_predict_url_returns_none_without_service_url() {
        assert!(build_predict_url(None, None).is_none());
    }

    #[test]
    fn build_predict_url_uses_default_predict_path() {
        let url = build_predict_url(Some("http://bot-svc:8001"), None);
        assert_eq!(url, Some("http://bot-svc:8001/api/v1/predict".to_string()));
    }

    #[test]
    fn build_predict_url_normalizes_predict_path() {
        let url = build_predict_url(Some("http://bot-svc:8001/"), Some("v1/predict"));
        assert_eq!(url, Some("http://bot-svc:8001/v1/predict".to_string()));
    }

    #[test]
    fn normalize_predict_path_adds_leading_slash() {
        assert_eq!(
            normalize_predict_path(Some("api/v1/predict")),
            "/api/v1/predict".to_string()
        );
        assert_eq!(
            normalize_predict_path(Some("/api/v1/predict")),
            "/api/v1/predict".to_string()
        );
    }

    #[test]
    fn normalize_predict_path_defaults_when_empty() {
        assert_eq!(
            normalize_predict_path(Some("   ")),
            "/api/v1/predict".to_string()
        );
    }
}
