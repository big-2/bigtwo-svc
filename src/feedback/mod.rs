use axum::{
    middleware,
    routing::{get, post},
    Router,
};

use crate::session;
use crate::shared::AppState;

pub mod errors;
pub mod handlers;
pub mod models;
pub mod repository;
pub mod service;

#[allow(unused_imports)] // Used by tests and by consumers matching on feedback failures
pub use errors::FeedbackError;
#[allow(unused_imports)] // Public re-exports for handlers, tests and future consumers
pub use models::{
    FeedbackChip, FeedbackRating, FeedbackRecord, FeedbackStatusResponse, SubmitFeedbackRequest,
    SubmitFeedbackResponse, MAX_CHIPS, MAX_COMMENT_LEN,
};
#[allow(unused_imports)] // Used by main wiring and integration tests
pub use repository::{FeedbackRepository, InMemoryFeedbackRepository, PostgresFeedbackRepository};
pub use service::FeedbackService;

/// Feedback routes, both behind JWT auth. Defined here rather than inline in
/// `main.rs` so integration tests exercise the exact same router the binary
/// serves.
pub fn routes(app_state: AppState) -> Router<AppState> {
    Router::new()
        .route(
            "/feedback",
            post(handlers::submit_feedback).layer(middleware::from_fn_with_state(
                app_state.clone(),
                session::jwt_auth,
            )),
        )
        .route(
            "/feedback/status",
            get(handlers::get_feedback_status)
                .layer(middleware::from_fn_with_state(app_state, session::jwt_auth)),
        )
}
