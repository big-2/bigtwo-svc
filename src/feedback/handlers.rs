use axum::{extract::State, http::StatusCode, Extension, Json};
use tracing::instrument;

use crate::{
    session::SessionClaims,
    shared::{AppError, AppState},
};

use super::models::{FeedbackStatusResponse, SubmitFeedbackRequest, SubmitFeedbackResponse};

/// POST /feedback
///
/// One submission per player, ever. A repeat submission returns 409 so the
/// client can correct its local "already submitted" flag.
#[instrument(skip(state, claims, request))]
pub async fn submit_feedback(
    State(state): State<AppState>,
    Extension(claims): Extension<SessionClaims>,
    Json(request): Json<SubmitFeedbackRequest>,
) -> Result<(StatusCode, Json<SubmitFeedbackResponse>), AppError> {
    let player_uuid = state
        .session_service
        .get_player_uuid_by_session(&claims.session_id)
        .await?
        .ok_or_else(|| AppError::Unauthorized("No player UUID for session".to_string()))?;

    state
        .feedback_service
        .submit(&player_uuid, &claims.session_id, request)
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(SubmitFeedbackResponse { submitted: true }),
    ))
}

/// GET /feedback/status
///
/// Called lazily by the client, only when it is about to show the widget.
#[instrument(skip(state, claims))]
pub async fn get_feedback_status(
    State(state): State<AppState>,
    Extension(claims): Extension<SessionClaims>,
) -> Result<Json<FeedbackStatusResponse>, AppError> {
    let player_uuid = state
        .session_service
        .get_player_uuid_by_session(&claims.session_id)
        .await?
        .ok_or_else(|| AppError::Unauthorized("No player UUID for session".to_string()))?;

    let submitted = state.feedback_service.has_submitted(&player_uuid).await?;

    Ok(Json(FeedbackStatusResponse { submitted }))
}
