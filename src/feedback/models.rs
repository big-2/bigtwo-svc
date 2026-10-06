use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::errors::FeedbackError;

/// Maximum number of chips accepted on a single submission.
pub const MAX_CHIPS: usize = 5;
/// Maximum length of the optional free-text comment, after trimming.
pub const MAX_COMMENT_LEN: usize = 500;

/// Overall sentiment. Deliberately binary - stars cluster at the extremes and
/// carry less information than a thumb.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedbackRating {
    Up,
    Down,
}

impl FeedbackRating {
    pub fn as_str(&self) -> &'static str {
        match self {
            FeedbackRating::Up => "up",
            FeedbackRating::Down => "down",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "up" => Some(FeedbackRating::Up),
            "down" => Some(FeedbackRating::Down),
            _ => None,
        }
    }
}

/// Known chip vocabulary. Unknown values sent by an older or newer client are
/// dropped rather than rejected, so the vocabulary can change without breaking
/// clients that are already deployed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackChip {
    Fun,
    GoodBots,
    Smooth,
    BotsTooEasy,
    BotsTooHard,
    Bug,
    Slow,
    ConfusingUi,
    NotFun,
}

impl FeedbackChip {
    pub fn as_str(&self) -> &'static str {
        match self {
            FeedbackChip::Fun => "fun",
            FeedbackChip::GoodBots => "good_bots",
            FeedbackChip::Smooth => "smooth",
            FeedbackChip::BotsTooEasy => "bots_too_easy",
            FeedbackChip::BotsTooHard => "bots_too_hard",
            FeedbackChip::Bug => "bug",
            FeedbackChip::Slow => "slow",
            FeedbackChip::ConfusingUi => "confusing_ui",
            FeedbackChip::NotFun => "not_fun",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "fun" => Some(FeedbackChip::Fun),
            "good_bots" => Some(FeedbackChip::GoodBots),
            "smooth" => Some(FeedbackChip::Smooth),
            "bots_too_easy" => Some(FeedbackChip::BotsTooEasy),
            "bots_too_hard" => Some(FeedbackChip::BotsTooHard),
            "bug" => Some(FeedbackChip::Bug),
            "slow" => Some(FeedbackChip::Slow),
            "confusing_ui" => Some(FeedbackChip::ConfusingUi),
            "not_fun" => Some(FeedbackChip::NotFun),
            _ => None,
        }
    }
}

/// Bot difficulties accepted for context. Mirrors the values the room API and
/// the websocket `BOT_ADDED` payload already use.
const KNOWN_BOT_DIFFICULTIES: [&str; 5] = ["easy", "medium", "hard", "ai", "expert"];

#[derive(Debug, Clone, Deserialize)]
pub struct SubmitFeedbackRequest {
    pub rating: String,
    #[serde(default)]
    pub chips: Vec<String>,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub room_id: Option<String>,
    #[serde(default)]
    pub won: Option<bool>,
    #[serde(default)]
    pub bot_difficulty: Option<String>,
    #[serde(default)]
    pub had_bots: Option<bool>,
    #[serde(default)]
    pub app_version: Option<String>,
    #[serde(default)]
    pub client: Option<String>,
}

/// A request that has passed validation. Constructing one is the only way to
/// reach the repository, so handlers cannot skip validation by accident.
#[derive(Debug, Clone)]
pub struct ValidatedFeedback {
    pub rating: FeedbackRating,
    pub chips: Vec<FeedbackChip>,
    pub comment: Option<String>,
    pub room_id: Option<String>,
    pub won: Option<bool>,
    pub bot_difficulty: Option<String>,
    pub had_bots: Option<bool>,
    pub app_version: Option<String>,
    pub client: String,
}

impl SubmitFeedbackRequest {
    pub fn validate(self) -> Result<ValidatedFeedback, FeedbackError> {
        let rating = FeedbackRating::parse(self.rating.trim()).ok_or_else(|| {
            FeedbackError::Validation("rating must be either \"up\" or \"down\"".to_string())
        })?;

        if self.chips.len() > MAX_CHIPS {
            return Err(FeedbackError::Validation(format!(
                "at most {} chips may be submitted",
                MAX_CHIPS
            )));
        }

        // Unknown chips are dropped, not rejected: an older client must not
        // start failing when the vocabulary changes.
        let mut chips: Vec<FeedbackChip> = Vec::new();
        for raw in &self.chips {
            if let Some(chip) = FeedbackChip::parse(raw.trim()) {
                if !chips.contains(&chip) {
                    chips.push(chip);
                }
            }
        }

        let comment = match self.comment {
            Some(raw) => {
                let trimmed = raw.trim();
                if trimmed.chars().count() > MAX_COMMENT_LEN {
                    return Err(FeedbackError::Validation(format!(
                        "comment must be at most {} characters",
                        MAX_COMMENT_LEN
                    )));
                }
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            }
            None => None,
        };

        let bot_difficulty = match self.bot_difficulty {
            Some(raw) => {
                let normalized = raw.trim().to_lowercase();
                if normalized.is_empty() {
                    None
                } else if KNOWN_BOT_DIFFICULTIES.contains(&normalized.as_str()) {
                    Some(normalized)
                } else {
                    return Err(FeedbackError::Validation(format!(
                        "unknown bot_difficulty \"{}\"",
                        raw.trim()
                    )));
                }
            }
            None => None,
        };

        let room_id = self
            .room_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        let app_version = self
            .app_version
            .map(|value| value.trim().chars().take(64).collect::<String>())
            .filter(|value| !value.is_empty());

        let client = self
            .client
            .map(|value| value.trim().chars().take(32).collect::<String>())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "web".to_string());

        Ok(ValidatedFeedback {
            rating,
            chips,
            comment,
            room_id,
            won: self.won,
            bot_difficulty,
            had_bots: self.had_bots,
            app_version,
            client,
        })
    }
}

/// A feedback row as it is handed to the repository.
#[derive(Debug, Clone)]
pub struct FeedbackRecord {
    pub player_uuid: String,
    pub session_id: String,
    pub game_id: Option<String>,
    pub room_id: Option<String>,
    pub rating: FeedbackRating,
    pub chips: Vec<String>,
    pub comment: Option<String>,
    pub won: Option<bool>,
    pub bot_difficulty: Option<String>,
    pub had_bots: Option<bool>,
    pub client: String,
    pub app_version: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubmitFeedbackResponse {
    pub submitted: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeedbackStatusResponse {
    pub submitted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(rating: &str) -> SubmitFeedbackRequest {
        SubmitFeedbackRequest {
            rating: rating.to_string(),
            chips: Vec::new(),
            comment: None,
            room_id: None,
            won: None,
            bot_difficulty: None,
            had_bots: None,
            app_version: None,
            client: None,
        }
    }

    #[test]
    fn accepts_bare_thumbs_up() {
        let validated = request("up").validate().expect("should validate");
        assert_eq!(validated.rating, FeedbackRating::Up);
        assert!(validated.chips.is_empty());
        assert_eq!(validated.comment, None);
        assert_eq!(validated.client, "web");
    }

    #[test]
    fn rejects_unknown_rating() {
        let error = request("sideways").validate().expect_err("should reject");
        assert!(matches!(error, FeedbackError::Validation(_)));
    }

    #[test]
    fn drops_unknown_chips_but_keeps_known_ones() {
        let mut req = request("down");
        req.chips = vec![
            "bots_too_hard".to_string(),
            "chip_from_a_future_client".to_string(),
            "bug".to_string(),
        ];

        let validated = req.validate().expect("should validate");
        assert_eq!(
            validated.chips,
            vec![FeedbackChip::BotsTooHard, FeedbackChip::Bug]
        );
    }

    #[test]
    fn deduplicates_repeated_chips() {
        let mut req = request("up");
        req.chips = vec!["fun".to_string(), "fun".to_string()];

        let validated = req.validate().expect("should validate");
        assert_eq!(validated.chips, vec![FeedbackChip::Fun]);
    }

    #[test]
    fn rejects_more_than_max_chips() {
        let mut req = request("up");
        req.chips = vec!["fun".to_string(); MAX_CHIPS + 1];

        let error = req.validate().expect_err("should reject");
        assert!(matches!(error, FeedbackError::Validation(_)));
    }

    #[test]
    fn accepts_comment_at_the_limit() {
        let mut req = request("down");
        req.comment = Some("a".repeat(MAX_COMMENT_LEN));

        let validated = req.validate().expect("should validate");
        assert_eq!(validated.comment.map(|c| c.len()), Some(MAX_COMMENT_LEN));
    }

    #[test]
    fn rejects_comment_over_the_limit() {
        let mut req = request("down");
        req.comment = Some("a".repeat(MAX_COMMENT_LEN + 1));

        let error = req.validate().expect_err("should reject");
        assert!(matches!(error, FeedbackError::Validation(_)));
    }

    #[test]
    fn measures_comment_length_in_characters_not_bytes() {
        let mut req = request("down");
        // Multi-byte characters must not be counted as several characters.
        req.comment = Some("é".repeat(MAX_COMMENT_LEN));

        assert!(req.validate().is_ok());
    }

    #[test]
    fn treats_whitespace_only_comment_as_absent() {
        let mut req = request("up");
        req.comment = Some("   \n  ".to_string());

        let validated = req.validate().expect("should validate");
        assert_eq!(validated.comment, None);
    }

    #[test]
    fn trims_comment_before_storing() {
        let mut req = request("up");
        req.comment = Some("  good game  ".to_string());

        let validated = req.validate().expect("should validate");
        assert_eq!(validated.comment.as_deref(), Some("good game"));
    }

    #[test]
    fn normalizes_bot_difficulty_case() {
        let mut req = request("up");
        req.bot_difficulty = Some("Expert".to_string());

        let validated = req.validate().expect("should validate");
        assert_eq!(validated.bot_difficulty.as_deref(), Some("expert"));
    }

    #[test]
    fn rejects_unknown_bot_difficulty() {
        let mut req = request("up");
        req.bot_difficulty = Some("nightmare".to_string());

        let error = req.validate().expect_err("should reject");
        assert!(matches!(error, FeedbackError::Validation(_)));
    }

    #[test]
    fn drops_empty_room_id() {
        let mut req = request("up");
        req.room_id = Some("   ".to_string());

        let validated = req.validate().expect("should validate");
        assert_eq!(validated.room_id, None);
    }

    #[test]
    fn every_chip_round_trips_through_parse() {
        let all = [
            FeedbackChip::Fun,
            FeedbackChip::GoodBots,
            FeedbackChip::Smooth,
            FeedbackChip::BotsTooEasy,
            FeedbackChip::BotsTooHard,
            FeedbackChip::Bug,
            FeedbackChip::Slow,
            FeedbackChip::ConfusingUi,
            FeedbackChip::NotFun,
        ];

        for chip in all {
            assert_eq!(FeedbackChip::parse(chip.as_str()), Some(chip));
        }
    }
}
