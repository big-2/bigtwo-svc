use thiserror::Error;

use crate::shared::AppError;

#[derive(Debug, Error)]
pub enum FeedbackError {
    #[error("Repository error: {0}")]
    Repository(String),

    #[error("Validation error: {0}")]
    Validation(String),

    /// The player already has a feedback row. Expected in normal operation
    /// (second browser tab, replayed request) - not something to alert on.
    #[error("feedback_already_submitted")]
    AlreadySubmitted,
}

impl From<FeedbackError> for AppError {
    fn from(error: FeedbackError) -> Self {
        match error {
            FeedbackError::Validation(message) => AppError::BadRequest(message),
            FeedbackError::AlreadySubmitted => {
                AppError::Conflict("feedback_already_submitted".to_string())
            }
            FeedbackError::Repository(message) => {
                AppError::DatabaseError(format!("Failed to store feedback: {}", message))
            }
        }
    }
}
