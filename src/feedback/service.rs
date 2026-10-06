use chrono::Utc;
use std::sync::Arc;
use tracing::{info, warn};

use super::errors::FeedbackError;
use super::models::{FeedbackRecord, SubmitFeedbackRequest, ValidatedFeedback};
use super::repository::FeedbackRepository;

pub struct FeedbackService {
    repository: Arc<dyn FeedbackRepository>,
}

impl FeedbackService {
    pub fn new(repository: Arc<dyn FeedbackRepository>) -> Self {
        Self { repository }
    }

    /// Validates and stores a submission for `player_uuid`.
    pub async fn submit(
        &self,
        player_uuid: &str,
        session_id: &str,
        request: SubmitFeedbackRequest,
    ) -> Result<(), FeedbackError> {
        let validated: ValidatedFeedback = request.validate()?;

        // Short-circuit before doing any game resolution work.
        if self.repository.has_submitted(player_uuid).await? {
            return Err(FeedbackError::AlreadySubmitted);
        }

        let game_id = match validated.room_id.as_deref() {
            Some(room_id) => match self.repository.resolve_recent_game_id(room_id).await {
                Ok(game_id) => game_id,
                // Context is a nice-to-have; never fail a submission over it.
                Err(e) => {
                    warn!(room_id = %room_id, error = %e, "Failed to resolve game for feedback");
                    None
                }
            },
            None => None,
        };

        let record = FeedbackRecord {
            player_uuid: player_uuid.to_string(),
            session_id: session_id.to_string(),
            game_id,
            room_id: validated.room_id,
            rating: validated.rating,
            chips: validated
                .chips
                .iter()
                .map(|chip| chip.as_str().to_string())
                .collect(),
            comment: validated.comment,
            won: validated.won,
            bot_difficulty: validated.bot_difficulty,
            had_bots: validated.had_bots,
            client: validated.client,
            app_version: validated.app_version,
            created_at: Utc::now(),
        };

        self.repository.insert_feedback(record).await?;

        info!(player_uuid = %player_uuid, "Stored post-game feedback");
        Ok(())
    }

    pub async fn has_submitted(&self, player_uuid: &str) -> Result<bool, FeedbackError> {
        self.repository.has_submitted(player_uuid).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feedback::models::FeedbackRating;
    use crate::feedback::repository::InMemoryFeedbackRepository;

    fn service_with_repo() -> (FeedbackService, Arc<InMemoryFeedbackRepository>) {
        let repository = Arc::new(InMemoryFeedbackRepository::new());
        (
            FeedbackService::new(repository.clone() as Arc<dyn FeedbackRepository>),
            repository,
        )
    }

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

    #[tokio::test]
    async fn stores_a_submission() {
        let (service, repository) = service_with_repo();

        service
            .submit("player-1", "session-1", request("up"))
            .await
            .expect("submit should succeed");

        let stored = repository
            .get_for_player("player-1")
            .await
            .expect("record should exist");
        assert_eq!(stored.rating, FeedbackRating::Up);
        assert_eq!(stored.session_id, "session-1");
        assert_eq!(stored.client, "web");
    }

    #[tokio::test]
    async fn second_submission_from_same_player_is_rejected() {
        let (service, repository) = service_with_repo();

        service
            .submit("player-1", "session-1", request("up"))
            .await
            .expect("first submit should succeed");

        let error = service
            .submit("player-1", "session-1", request("down"))
            .await
            .expect_err("second submit should be rejected");

        assert!(matches!(error, FeedbackError::AlreadySubmitted));
        assert_eq!(repository.count().await, 1, "must not store a second row");
    }

    #[tokio::test]
    async fn different_players_may_each_submit() {
        let (service, repository) = service_with_repo();

        service
            .submit("player-1", "session-1", request("up"))
            .await
            .expect("player 1 should submit");
        service
            .submit("player-2", "session-2", request("down"))
            .await
            .expect("player 2 should submit");

        assert_eq!(repository.count().await, 2);
    }

    #[tokio::test]
    async fn has_submitted_reflects_state() {
        let (service, _repository) = service_with_repo();

        assert!(!service.has_submitted("player-1").await.unwrap());
        service
            .submit("player-1", "session-1", request("up"))
            .await
            .unwrap();
        assert!(service.has_submitted("player-1").await.unwrap());
    }

    #[tokio::test]
    async fn resolves_game_id_from_room_id() {
        let (service, repository) = service_with_repo();
        repository.seed_recent_game("room-7", "game-42").await;

        let mut req = request("down");
        req.room_id = Some("room-7".to_string());
        service.submit("player-1", "session-1", req).await.unwrap();

        let stored = repository.get_for_player("player-1").await.unwrap();
        assert_eq!(stored.game_id.as_deref(), Some("game-42"));
        assert_eq!(stored.room_id.as_deref(), Some("room-7"));
    }

    #[tokio::test]
    async fn stores_without_game_id_when_room_has_no_recent_game() {
        let (service, repository) = service_with_repo();

        let mut req = request("down");
        req.room_id = Some("room-unknown".to_string());
        service.submit("player-1", "session-1", req).await.unwrap();

        let stored = repository.get_for_player("player-1").await.unwrap();
        assert_eq!(stored.game_id, None);
    }

    #[tokio::test]
    async fn invalid_request_is_rejected_before_storage() {
        let (service, repository) = service_with_repo();

        let error = service
            .submit("player-1", "session-1", request("maybe"))
            .await
            .expect_err("invalid rating should be rejected");

        assert!(matches!(error, FeedbackError::Validation(_)));
        assert_eq!(repository.count().await, 0);
    }

    #[tokio::test]
    async fn persists_context_fields() {
        let (service, repository) = service_with_repo();

        let mut req = request("down");
        req.chips = vec!["bots_too_hard".to_string(), "slow".to_string()];
        req.comment = Some("expert bot never passes".to_string());
        req.won = Some(false);
        req.bot_difficulty = Some("expert".to_string());
        req.had_bots = Some(true);
        req.app_version = Some("2026.08.24".to_string());

        service.submit("player-1", "session-1", req).await.unwrap();

        let stored = repository.get_for_player("player-1").await.unwrap();
        assert_eq!(stored.chips, vec!["bots_too_hard", "slow"]);
        assert_eq!(stored.comment.as_deref(), Some("expert bot never passes"));
        assert_eq!(stored.won, Some(false));
        assert_eq!(stored.bot_difficulty.as_deref(), Some("expert"));
        assert_eq!(stored.had_bots, Some(true));
        assert_eq!(stored.app_version.as_deref(), Some("2026.08.24"));
    }
}
