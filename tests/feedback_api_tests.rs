//! HTTP-level integration tests for the post-game feedback endpoints.
//!
//! These drive the same router `main.rs` serves (via `feedback::routes`) through
//! `tower::ServiceExt::oneshot`, so auth middleware, extractors, status codes and
//! JSON bodies are all exercised end to end.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use bigtwo::{
    feedback::{self, FeedbackRepository, FeedbackService, InMemoryFeedbackRepository},
    game::GameService,
    room::{
        activity_room_subscriber::ActivityRoomSubscriber, activity_tracker::ActivityTracker,
        repository::InMemoryRoomRepository, service::RoomService,
    },
    session::{repository::InMemorySessionRepository, service::SessionService},
    shared::AppState,
    user::mapping_service::InMemoryPlayerMappingService,
    websockets::InMemoryConnectionManager,
};
use serde_json::{json, Value};
use tower::ServiceExt;

struct TestApp {
    router: Router,
    session_service: Arc<SessionService>,
    feedback_repository: Arc<InMemoryFeedbackRepository>,
}

impl TestApp {
    fn new() -> Self {
        let player_mapping = Arc::new(InMemoryPlayerMappingService::new());
        let session_service = Arc::new(SessionService::new(
            Arc::new(InMemorySessionRepository::new()),
            player_mapping.clone(),
        ));
        let feedback_repository = Arc::new(InMemoryFeedbackRepository::new());
        let feedback_service = Arc::new(FeedbackService::new(
            feedback_repository.clone() as Arc<dyn FeedbackRepository>
        ));

        let room_repository = Arc::new(InMemoryRoomRepository::new());
        let app_state = AppState::builder()
            .with_session_service(session_service.clone())
            .with_player_mapping(player_mapping.clone())
            .with_room_service(Arc::new(RoomService::new(room_repository.clone())))
            .with_connection_manager(Arc::new(InMemoryConnectionManager::new()))
            .with_game_service(Arc::new(GameService::new(player_mapping)))
            .with_activity_subscriber(Arc::new(ActivityRoomSubscriber::new(Arc::new(
                ActivityTracker::new(room_repository),
            ))))
            .with_feedback_service(feedback_service)
            .build()
            .expect("test AppState should build");

        let router = feedback::routes(app_state.clone()).with_state(app_state);

        Self {
            router,
            session_service,
            feedback_repository,
        }
    }

    /// Creates a real session and returns its JWT, the same value the web
    /// client stores in `localStorage['session_id']`.
    async fn new_token(&self) -> String {
        self.session_service
            .create_session()
            .await
            .expect("session creation should succeed")
            .session_id
    }

    async fn post_feedback(&self, token: Option<&str>, body: Value) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/feedback")
            .header(header::CONTENT_TYPE, "application/json");

        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {}", token));
        }

        let request = builder
            .body(Body::from(body.to_string()))
            .expect("request should build");

        self.send(request).await
    }

    async fn get_status(&self, token: &str) -> (StatusCode, Value) {
        let request = Request::builder()
            .method("GET")
            .uri("/feedback/status")
            .header(header::AUTHORIZATION, format!("Bearer {}", token))
            .body(Body::empty())
            .expect("request should build");

        self.send(request).await
    }

    async fn send(&self, request: Request<Body>) -> (StatusCode, Value) {
        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("router should respond");

        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable");
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

        (status, value)
    }
}

#[tokio::test]
async fn submitting_feedback_returns_201_and_stores_the_row() {
    let app = TestApp::new();
    let token = app.new_token().await;

    let (status, body) = app
        .post_feedback(
            Some(&token),
            json!({
                "rating": "down",
                "chips": ["bots_too_hard", "slow"],
                "comment": "expert bot is relentless",
                "won": false,
                "bot_difficulty": "expert",
                "had_bots": true,
                "app_version": "2026.08.24"
            }),
        )
        .await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["submitted"], json!(true));
    assert_eq!(app.feedback_repository.count().await, 1);
}

#[tokio::test]
async fn a_bare_thumbs_up_is_a_valid_submission() {
    let app = TestApp::new();
    let token = app.new_token().await;

    let (status, _) = app
        .post_feedback(Some(&token), json!({ "rating": "up" }))
        .await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(app.feedback_repository.count().await, 1);
}

#[tokio::test]
async fn second_submission_returns_409_and_does_not_store_another_row() {
    let app = TestApp::new();
    let token = app.new_token().await;

    let (first, _) = app
        .post_feedback(Some(&token), json!({ "rating": "up" }))
        .await;
    assert_eq!(first, StatusCode::CREATED);

    let (second, body) = app
        .post_feedback(Some(&token), json!({ "rating": "down" }))
        .await;

    assert_eq!(second, StatusCode::CONFLICT);
    assert_eq!(body["error"], json!("feedback_already_submitted"));
    assert_eq!(
        app.feedback_repository.count().await,
        1,
        "the duplicate must not be stored"
    );
}

#[tokio::test]
async fn status_flips_from_false_to_true_after_submitting() {
    let app = TestApp::new();
    let token = app.new_token().await;

    let (status, body) = app.get_status(&token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["submitted"], json!(false));

    app.post_feedback(Some(&token), json!({ "rating": "up" }))
        .await;

    let (status, body) = app.get_status(&token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["submitted"], json!(true));
}

#[tokio::test]
async fn one_players_submission_does_not_suppress_another_players_widget() {
    let app = TestApp::new();
    let first_token = app.new_token().await;
    let second_token = app.new_token().await;

    app.post_feedback(Some(&first_token), json!({ "rating": "up" }))
        .await;

    let (_, body) = app.get_status(&second_token).await;
    assert_eq!(body["submitted"], json!(false));

    let (status, _) = app
        .post_feedback(Some(&second_token), json!({ "rating": "down" }))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(app.feedback_repository.count().await, 2);
}

#[tokio::test]
async fn requests_without_a_token_are_rejected() {
    let app = TestApp::new();

    let (status, _) = app.post_feedback(None, json!({ "rating": "up" })).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(app.feedback_repository.count().await, 0);
}

#[tokio::test]
async fn status_without_a_token_is_rejected() {
    let app = TestApp::new();

    let request = Request::builder()
        .method("GET")
        .uri("/feedback/status")
        .body(Body::empty())
        .unwrap();

    let (status, _) = app.send(request).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// A structurally invalid token fails JWT decoding, which this codebase maps to
/// `AppError::JwtError` -> 400 rather than 401. That is pre-existing behaviour
/// shared by every authenticated route; asserted here so a future change to the
/// auth error mapping is caught rather than silently altering the client contract.
#[tokio::test]
async fn a_malformed_token_is_rejected() {
    let app = TestApp::new();

    let (status, _) = app
        .post_feedback(Some("not-a-real-jwt"), json!({ "rating": "up" }))
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(app.feedback_repository.count().await, 0);
}

/// A well-formed token whose session is not in the repository is a genuine
/// authentication failure, and must be a 401 so the client's axios interceptor
/// clears the stale session.
#[tokio::test]
async fn a_token_for_an_unknown_session_is_unauthorized() {
    let app = TestApp::new();
    let orphan_token = TestApp::new().new_token().await;

    let (status, _) = app
        .post_feedback(Some(&orphan_token), json!({ "rating": "up" }))
        .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(app.feedback_repository.count().await, 0);
}

#[tokio::test]
async fn an_unknown_rating_is_rejected() {
    let app = TestApp::new();
    let token = app.new_token().await;

    let (status, _) = app
        .post_feedback(Some(&token), json!({ "rating": "sideways" }))
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(app.feedback_repository.count().await, 0);
}

#[tokio::test]
async fn an_overlong_comment_is_rejected() {
    let app = TestApp::new();
    let token = app.new_token().await;

    let (status, _) = app
        .post_feedback(
            Some(&token),
            json!({ "rating": "down", "comment": "a".repeat(501) }),
        )
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(app.feedback_repository.count().await, 0);
}

#[tokio::test]
async fn too_many_chips_are_rejected() {
    let app = TestApp::new();
    let token = app.new_token().await;

    let (status, _) = app
        .post_feedback(
            Some(&token),
            json!({
                "rating": "down",
                "chips": ["bug", "slow", "not_fun", "confusing_ui", "bots_too_hard", "bots_too_easy"]
            }),
        )
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(app.feedback_repository.count().await, 0);
}

#[tokio::test]
async fn unknown_chips_are_dropped_rather_than_rejected() {
    let app = TestApp::new();
    let token = app.new_token().await;

    let (status, _) = app
        .post_feedback(
            Some(&token),
            json!({ "rating": "down", "chips": ["bug", "chip_from_a_future_client"] }),
        )
        .await;

    assert_eq!(status, StatusCode::CREATED);

    let stored = app
        .feedback_repository
        .get_for_player(&player_uuid_of(&app, &token).await)
        .await
        .expect("record should exist");
    assert_eq!(stored.chips, vec!["bug"]);
}

#[tokio::test]
async fn an_unknown_bot_difficulty_is_rejected() {
    let app = TestApp::new();
    let token = app.new_token().await;

    let (status, _) = app
        .post_feedback(
            Some(&token),
            json!({ "rating": "up", "bot_difficulty": "nightmare" }),
        )
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn room_id_is_resolved_to_the_recent_game() {
    let app = TestApp::new();
    let token = app.new_token().await;
    app.feedback_repository
        .seed_recent_game("room-123", "game-abc")
        .await;

    let (status, _) = app
        .post_feedback(
            Some(&token),
            json!({ "rating": "up", "room_id": "room-123" }),
        )
        .await;

    assert_eq!(status, StatusCode::CREATED);

    let stored = app
        .feedback_repository
        .get_for_player(&player_uuid_of(&app, &token).await)
        .await
        .expect("record should exist");
    assert_eq!(stored.game_id.as_deref(), Some("game-abc"));
    assert_eq!(stored.room_id.as_deref(), Some("room-123"));
}

#[tokio::test]
async fn a_room_with_no_recent_game_still_accepts_feedback() {
    let app = TestApp::new();
    let token = app.new_token().await;

    let (status, _) = app
        .post_feedback(
            Some(&token),
            json!({ "rating": "up", "room_id": "room-with-no-history" }),
        )
        .await;

    assert_eq!(status, StatusCode::CREATED);

    let stored = app
        .feedback_repository
        .get_for_player(&player_uuid_of(&app, &token).await)
        .await
        .expect("record should exist");
    assert_eq!(stored.game_id, None);
}

/// The player identifier is the session id carried inside the JWT.
async fn player_uuid_of(app: &TestApp, token: &str) -> String {
    app.session_service
        .validate_session(token)
        .await
        .expect("token should validate")
        .session_id
}
