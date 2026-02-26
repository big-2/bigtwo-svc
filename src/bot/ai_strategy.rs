use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, time::Duration};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::game::{Card, Game};

use super::{basic_strategy::BasicBotStrategy, types::BotStrategy};

const DEFAULT_AI_BOT_SERVICE_URL: &str = "http://127.0.0.1:8001";
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
    endpoint: String,
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
            legal_actions,
            allow_pass,
        })
    }

    async fn predict(&self, request: &PredictRequest) -> Result<PredictResponse, String> {
        let response = self
            .client
            .post(&self.endpoint)
            .json(request)
            .send()
            .await
            .map_err(|e| format!("request error: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("status={status}, body={body}"));
        }

        response
            .json::<PredictResponse>()
            .await
            .map_err(|e| format!("response decode error: {e}"))
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
            endpoint = %self.endpoint,
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

fn build_predict_url_from_env() -> String {
    let base_url = std::env::var("AI_BOT_SERVICE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_AI_BOT_SERVICE_URL.to_string());
    let predict_path = std::env::var("AI_BOT_SERVICE_PREDICT_PATH")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_AI_BOT_PREDICT_PATH.to_string());
    let normalized_path = if predict_path.starts_with('/') {
        predict_path
    } else {
        format!("/{}", predict_path)
    };

    format!("{}{}", base_url.trim_end_matches('/'), normalized_path)
}

fn read_timeout_ms_from_env() -> u64 {
    std::env::var("AI_BOT_HTTP_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_AI_BOT_TIMEOUT_MS)
}
